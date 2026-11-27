//! A [`RecursivePackageProvider`] over a prepopulated Skyframe graph (SkyQuery's universe).
//!
//! Java: com/google/devtools/build/lib/skyframe/{GraphBackedRecursivePackageProvider,
//! TraversalInfoRootPackageExtractor,RootPackageExtractor}.java.

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_cmdline::{PackageIdentifier, RepositoryName, TargetPattern, TargetPatternType};
use blaze_events::{Event, ExtendedEventHandler};
use blaze_packages::exceptions::NoSuchPackageException;
use blaze_packages::package::Package;
use blaze_packages::package::package_piece::PackagePieceIdentifierForBuildFile;
use blaze_packages::target::{InputFile, Target};
use blaze_skyframe::{SkyKey, WalkableGraph};
use blaze_vfs::{PathFragment, Root, RootedPath};

use crate::ignored_subdirectories::IgnoredSubdirectories;
use crate::package_function::{PackageKey, PackageValue};
use crate::package_lookup::{PackageLookupKey, PackageLookupValue};
use crate::package_pieces::{PackagePieceForBuildFileKey, PackagePieceValueForBuildFile};
use crate::parallel_visitor::{self, ParallelVisitor, Visit};
use crate::prepare_deps::{CollectPackagesUnderDirectoryKey, CollectPackagesUnderDirectoryValue};
use crate::repos::{RepositoryDirectoryKey, RepositoryDirectoryValue};
use crate::target_pattern_function::{ProviderError, RecursivePackageProvider};

/// The patterns of the universe (Java `GraphBackedRecursivePackageProvider.UniverseTargetPattern`).
// Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:67 GraphBackedRecursivePackageProvider.UniverseTargetPattern
pub enum UniverseTargetPattern {
    /// `UniverseTargetPattern.of(patterns)`.
    Of(Vec<TargetPattern>),
    /// `UniverseTargetPattern.all()`.
    All,
}

impl UniverseTargetPattern {
    fn patterns(&self) -> &[TargetPattern] {
        match self {
            UniverseTargetPattern::Of(p) => p,
            UniverseTargetPattern::All => &[],
        }
    }

    fn allow_all(&self) -> bool {
        matches!(self, UniverseTargetPattern::All)
    }
}

fn downcast<T: Send + Sync + 'static>(v: blaze_skyframe::SkyValue) -> Option<Arc<T>> {
    v.downcast::<T>().ok()
}

/// A package provider that reads packages from a graph in which the universe was loaded.
// Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:58 GraphBackedRecursivePackageProvider
pub struct GraphBackedRecursivePackageProvider {
    graph: Arc<dyn WalkableGraph>,
    pkg_roots: Vec<Root>,
    universe_target_patterns: UniverseTargetPattern,
}

impl GraphBackedRecursivePackageProvider {
    /// PORT: Java takes a `RootPackageExtractor`; SkyQuery passes `TraversalInfoRootPackageExtractor`,
    /// the only one, which is used directly.
    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:108 GraphBackedRecursivePackageProvider.<init>
    pub fn new(
        graph: Arc<dyn WalkableGraph>,
        universe_target_patterns: UniverseTargetPattern,
        pkg_roots: Vec<Root>,
    ) -> Self {
        GraphBackedRecursivePackageProvider {
            graph,
            pkg_roots,
            universe_target_patterns,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:132 GraphBackedRecursivePackageProvider.getPackageInternal
    fn get_package_internal(
        &self,
        package_name: &PackageIdentifier,
    ) -> Result<Option<Arc<Package>>, NoSuchPackageException> {
        let key = SkyKey::new(PackageKey::key(package_name.clone()));
        if let Some(pkg_value) = self
            .graph
            .get_value(&key)
            .and_then(downcast::<PackageValue>)
        {
            return Ok(Some(pkg_value.get_package().clone()));
        }
        if let Some(e) = self.graph.get_exception(&key)
            && let Some(nspe) = e.downcast_ref::<NoSuchPackageException>()
        {
            return Err(nspe.clone());
        }
        if self.graph.is_cycle(&key) {
            return Err(NoSuchPackageException::new(
                package_name.clone(),
                "Package depends on a cycle",
            ));
        }
        Ok(None)
    }

    /// Returns the BUILD file of the package, from the package or its BUILD file piece.
    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:149 GraphBackedRecursivePackageProvider.getBuildFile
    pub fn get_build_file_of_package(
        &self,
        package_name: &PackageIdentifier,
    ) -> Result<Arc<InputFile>, NoSuchPackageException> {
        // Do we have a full package?
        if let Some(pkg) = self.get_package_internal(package_name)? {
            return Ok(pkg.get_build_file().clone());
        }
        // Do we have a PackagePiece.ForBuildFile?
        let piece_key = SkyKey::new(PackagePieceForBuildFileKey(
            PackagePieceIdentifierForBuildFile::new(package_name.clone()),
        ));
        if let Some(package_piece_value) = self
            .graph
            .get_value(&piece_key)
            .and_then(downcast::<PackagePieceValueForBuildFile>)
        {
            return Ok(package_piece_value
                .for_build_file
                .get_build_file()
                .expect("the BUILD file piece has a BUILD file")
                .clone());
        }
        if let Some(exception) = self.graph.get_exception(&piece_key) {
            if let Some(e) = exception.downcast_ref::<NoSuchPackageException>() {
                return Err(e.clone());
            }
            // PORT: Java also rethrows a NoSuchPackagePieceException, which Blaze's package
            // pieces do not throw as a separate type.
        }
        if self.graph.is_cycle(&piece_key) {
            return Err(NoSuchPackageException::new(
                package_name.clone(),
                "Package depends on a cycle",
            ));
        }
        // If both the package key and the package piece for BUILD file key do not exist in the
        // graph, then packageName must not correspond to any package, because the SkyQuery
        // environment has already loaded the universe.
        Err(NoSuchPackageException::build_file_not_found(
            package_name.clone(),
            "BUILD file not found on package path",
        ))
    }

    /// Java `PackageProvider.getBuildFile(Target)` (the default method).
    // Java: com/google/devtools/build/lib/pkgcache/PackageProvider.java:108 PackageProvider.getBuildFile
    pub fn get_build_file(&self, target: &Target) -> Arc<InputFile> {
        // PORT: Blaze's query targets belong to monolithic packages (lazy macro expansion of
        // targets in package pieces is not reached by SkyQuery).
        match self.get_build_file_of_package(target.get_label().get_package_identifier()) {
            Ok(f) => f,
            Err(e) => panic!(
                "IllegalStateException: Bug in package loading machinery: failed to load package \
                 piece for BUILD file of already-loaded target {}: {}",
                target.get_label(),
                e.get_message()
            ),
        }
    }

    /// Java `TargetProvider.getSiblingTargetsInPackage` (the `PackageProvider` default).
    // Java: com/google/devtools/build/lib/pkgcache/PackageProvider.java:147 PackageProvider.getSiblingTargetsInPackage
    pub fn get_sibling_targets_in_package(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        target: &Target,
    ) -> Result<Vec<Target>, NoSuchPackageException> {
        let pkg = match self.get_package(event_handler, target.get_label().get_package_identifier())
        {
            Ok(pkg) => pkg,
            Err(ProviderError::NoSuchThing(
                blaze_packages::exceptions::NoSuchThingException::Package(e),
            )) => {
                return Err(e);
            }
            Err(e) => panic!("{e:?}"),
        };
        Ok(pkg.get_targets().values().cloned().collect())
    }

    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:217 GraphBackedRecursivePackageProvider.bulkIsPackage
    pub fn bulk_is_package(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        pkg_ids: &[PackageIdentifier],
    ) -> Vec<PackageIdentifier> {
        let mut result_builder = Vec::new();
        let package_lookup_keys: Vec<SkyKey> = pkg_ids
            .iter()
            .map(|p| SkyKey::new(PackageLookupKey::key(p.clone())))
            .collect();
        let successful_package_lookup_values =
            self.graph.get_successful_values(&package_lookup_keys);
        let mut non_successful_package_lookup_keys: Vec<SkyKey> = Vec::new();
        for (pkg_id, package_lookup_key) in pkg_ids.iter().zip(&package_lookup_keys) {
            let package_lookup_value = successful_package_lookup_values
                .iter()
                .find(|(k, _)| k == package_lookup_key)
                .and_then(|(_, v)| downcast::<PackageLookupValue>(v.clone()));
            match package_lookup_value {
                // Could be because the node legitimately isn't in the graph (which definitely
                // implies the package doesn't exist) but could also be because the node exists
                // but is in error. In the latter case, we want to print that error message out.
                None => non_successful_package_lookup_keys.push(package_lookup_key.clone()),
                Some(v) => {
                    if v.package_exists() && !result_builder.contains(pkg_id) {
                        result_builder.push(pkg_id.clone());
                    }
                }
            }
        }
        if !non_successful_package_lookup_keys.is_empty() {
            for (_, exception) in self
                .graph
                .get_missing_and_exceptions(&non_successful_package_lookup_keys)
            {
                if let Some(exception) = exception
                    && let Some(e) = exception.downcast_ref::<NoSuchPackageException>()
                {
                    event_handler.handle(&Event::error(e.get_message()));
                }
            }
        }
        result_builder
    }

    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:259 GraphBackedRecursivePackageProvider.checkValidDirectoryAndGetRoots
    fn check_valid_directory_and_get_roots(
        &self,
        repository: &RepositoryName,
        directory: &PathFragment,
    ) -> Vec<Root> {
        // Check that this package is covered by at least one of our universe patterns.
        let mut in_universe = false;
        if self.universe_target_patterns.allow_all() {
            in_universe = true;
        } else {
            for pattern in self.universe_target_patterns.patterns() {
                if pattern.get_type() != TargetPatternType::TargetsBelowDirectory {
                    continue;
                }
                let package_identifier =
                    PackageIdentifier::create(repository.clone(), directory.clone());
                if pattern
                    .as_targets_below_directory()
                    .expect("TARGETS_BELOW_DIRECTORY")
                    .contains_all_transitive_subdirectories(&package_identifier)
                {
                    in_universe = true;
                    break;
                }
            }
        }
        if !in_universe {
            return Vec::new();
        }
        if repository.is_main() {
            self.pkg_roots.clone()
        } else {
            match self
                .graph
                .get_value(&SkyKey::new(RepositoryDirectoryKey::key(
                    repository.clone(),
                )))
                .and_then(downcast::<RepositoryDirectoryValue>)
                .as_deref()
            {
                Some(RepositoryDirectoryValue::Success { root, .. }) => vec![root.clone()],
                // If this key doesn't exist, the repository is outside the universe, so we
                // return "nothing".
                _ => Vec::new(),
            }
        }
    }
}

impl RecursivePackageProvider for GraphBackedRecursivePackageProvider {
    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:120 GraphBackedRecursivePackageProvider.getPackage
    fn get_package(
        &self,
        _event_handler: &dyn ExtendedEventHandler,
        package_name: &PackageIdentifier,
    ) -> Result<Arc<Package>, ProviderError> {
        if let Some(pkg) = self.get_package_internal(package_name)? {
            return Ok(pkg);
        }
        // If the package key does not exist in the graph, then it must not correspond to any
        // package, because the SkyQuery environment has already loaded the universe.
        Err(NoSuchPackageException::build_file_not_found(
            package_name.clone(),
            "BUILD file not found on package path",
        )
        .into())
    }

    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:254 GraphBackedRecursivePackageProvider.isPackage
    fn is_package(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        package_id: &PackageIdentifier,
    ) -> Result<bool, ProviderError> {
        Ok(!self
            .bulk_is_package(event_handler, std::slice::from_ref(package_id))
            .is_empty())
    }

    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:298 GraphBackedRecursivePackageProvider.streamPackagesUnderDirectory
    fn stream_packages_under_directory(
        &self,
        results: &mut dyn FnMut(Vec<PackageIdentifier>),
        event_handler: &dyn ExtendedEventHandler,
        repository: &RepositoryName,
        directory: &PathFragment,
        ignored_subdirectories: &IgnoredSubdirectories,
        excluded_subdirectories: &[PathFragment],
    ) -> Result<(), ProviderError> {
        traversal_info_root_package_extractor_stream_packages_from_roots(
            results,
            &*self.graph,
            &self.check_valid_directory_and_get_roots(repository, directory),
            event_handler,
            repository,
            directory,
            ignored_subdirectories,
            excluded_subdirectories,
        );
        Ok(())
    }

    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:181 GraphBackedRecursivePackageProvider.bulkGetPackages
    fn bulk_get_packages(
        &self,
        _event_handler: &dyn ExtendedEventHandler,
        pkg_ids: &[PackageIdentifier],
    ) -> Result<Vec<(PackageIdentifier, Arc<Package>)>, ProviderError> {
        let mut pkg_keys: Vec<SkyKey> = Vec::new();
        for p in pkg_ids {
            let k = SkyKey::new(PackageKey::key(p.clone()));
            if !pkg_keys.contains(&k) {
                pkg_keys.push(k);
            }
        }
        let packages = self.graph.get_successful_values(&pkg_keys);
        let mut pkg_results = Vec::new();
        for (key, value) in &packages {
            let pkg_id = key
                .downcast::<PackageKey>()
                .expect("a PACKAGE key")
                .0
                .clone();
            let pkg_value = downcast::<PackageValue>(value.clone()).expect("a PackageValue");
            pkg_results.push((pkg_id, pkg_value.get_package().clone()));
        }
        // PORT: skipped, logging ("Unable to find %s in the batch lookup of %s").
        let unknown_keys: Vec<SkyKey> = pkg_keys
            .iter()
            .filter(|k| !packages.iter().any(|(pk, _)| pk == *k))
            .cloned()
            .collect();
        for (key, exception) in self.graph.get_missing_and_exceptions(&unknown_keys) {
            let pkg_identifier = key
                .downcast::<PackageKey>()
                .expect("a PACKAGE key")
                .0
                .clone();
            match exception {
                // If the package key does not exist in the graph, then it must not correspond to
                // any package, because the SkyQuery environment has already loaded the universe.
                None => {
                    return Err(NoSuchPackageException::build_file_not_found(
                        pkg_identifier,
                        "Package not found",
                    )
                    .into());
                }
                Some(e) => {
                    if let Some(e) = e.downcast_ref::<NoSuchPackageException>() {
                        return Err(e.clone().into());
                    }
                    // Java: Throwables.propagate(exception).
                    panic!("{e}");
                }
            }
        }
        Ok(pkg_results)
    }
}

/// A directory to look for packages in, with the forbidden (ignored) and excluded directories
/// beneath it (Java `TraversalInfo`).
// Java: com/google/devtools/build/lib/skyframe/TraversalInfoRootPackageExtractor.java:214 TraversalInfoRootPackageExtractor.TraversalInfo
#[derive(Clone, PartialEq, Eq)]
struct TraversalInfo {
    rooted_dir: RootedPath,
    // Set of forbidden directories. The graph is assumed to be prepopulated with
    // CollectPackagesUnderDirectoryValue nodes whose keys have forbidden packages embedded in
    // them. Therefore, we need to be careful to request and use the same sort of keys here in our
    // traversal.
    forbidden_subdirectories: IgnoredSubdirectories,
    // Set of directories, targets under which should be excluded from the traversal results.
    // Excluded directory information isn't part of the graph keys in the prepopulated graph, so we
    // need to perform the filtering ourselves.
    excluded_subdirectories: Vec<PathFragment>,
}

/// Java `TreeSet<TraversalInfo>(comparing(rootedDir.getRootRelativePath()))`: sorted by the
/// root-relative path, one entry per path (the comparator's equality).
type TraversalInfoSet = BTreeMap<String, TraversalInfo>;

fn traversal_info_set_add(set: &mut TraversalInfoSet, info: TraversalInfo) {
    // PathFragment's compareTo compares the segments; the path strings compare the same way for
    // normalized relative paths except for '/' against bytes below it, so the segments are
    // compared through a key that orders '/' first.
    let key = info
        .rooted_dir
        .get_root_relative_path()
        .get_path_string()
        .replace('/', "\u{0}");
    set.entry(key).or_insert(info);
}

/// The packages beneath `directory` in `roots`, read from the COLLECT_PACKAGES_UNDER_DIRECTORY
/// nodes of the graph.
// Java: com/google/devtools/build/lib/skyframe/TraversalInfoRootPackageExtractor.java:56 TraversalInfoRootPackageExtractor.streamPackagesFromRoots
#[allow(clippy::too_many_arguments)]
fn traversal_info_root_package_extractor_stream_packages_from_roots(
    results: &mut dyn FnMut(Vec<PackageIdentifier>),
    graph: &dyn WalkableGraph,
    roots: &[Root],
    event_handler: &dyn ExtendedEventHandler,
    repository: &RepositoryName,
    directory: &PathFragment,
    forbidden_subdirectories: &IgnoredSubdirectories,
    excluded_subdirectories: &[PathFragment],
) {
    let mut dirs_to_check_for_packages: TraversalInfoSet = BTreeMap::new();
    for root in roots {
        let rooted_dir = RootedPath::to_rooted_path(root.clone(), directory);
        traversal_info_set_add(
            &mut dirs_to_check_for_packages,
            TraversalInfo {
                rooted_dir,
                forbidden_subdirectories: forbidden_subdirectories.clone(),
                excluded_subdirectories: excluded_subdirectories.to_vec(),
            },
        );
    }
    let visitor = PackageCollectingParallelVisitor {
        results: std::cell::RefCell::new(results),
        event_handler,
        repository,
        graph,
    };
    // The visitor throws no checked exception (MarkerRuntimeException).
    let _ = parallel_visitor::visit_and_wait_for_completion(
        &visitor,
        dirs_to_check_for_packages.into_values().collect(),
    );
}

// Java: com/google/devtools/build/lib/skyframe/TraversalInfoRootPackageExtractor.java:52 TraversalInfoRootPackageExtractor.PACKAGE_ID_OUTPUT_BATCH_SIZE
const PACKAGE_ID_OUTPUT_BATCH_SIZE: usize = 100;
// Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:75 RecursivePackageProviderBackedTargetPatternResolver.MAX_PACKAGES_BULK_GET
const MAX_PACKAGES_BULK_GET: usize = 1000;

// Java: com/google/devtools/build/lib/skyframe/TraversalInfoRootPackageExtractor.java:94 TraversalInfoRootPackageExtractor.PackageCollectingParallelVisitor
struct PackageCollectingParallelVisitor<'a, 'r> {
    results: std::cell::RefCell<&'r mut dyn FnMut(Vec<PackageIdentifier>)>,
    event_handler: &'a dyn ExtendedEventHandler,
    repository: &'a RepositoryName,
    graph: &'a dyn WalkableGraph,
}

impl ParallelVisitor for PackageCollectingParallelVisitor<'_, '_> {
    type Input = TraversalInfo;
    type VisitKey = TraversalInfo;
    type OutputKey = TraversalInfo;
    type OutputResult = PackageIdentifier;
    type Error = std::convert::Infallible;

    fn visit_batch_size(&self) -> usize {
        MAX_PACKAGES_BULK_GET
    }

    fn process_results_batch_size(&self) -> usize {
        PACKAGE_ID_OUTPUT_BATCH_SIZE
    }

    // Java: com/google/devtools/build/lib/skyframe/TraversalInfoRootPackageExtractor.java:131 TraversalInfoRootPackageExtractor.PackageCollectingParallelVisitor.outputKeysToOutputValues
    fn output_keys_to_output_values(
        &self,
        target_keys: Vec<TraversalInfo>,
    ) -> Result<Vec<PackageIdentifier>, Self::Error> {
        Ok(target_keys
            .into_iter()
            .map(|result_info| {
                PackageIdentifier::create(
                    self.repository.clone(),
                    result_info.rooted_dir.get_root_relative_path().clone(),
                )
            })
            .collect())
    }

    // Java: com/google/devtools/build/lib/skyframe/TraversalInfoRootPackageExtractor.java:143 TraversalInfoRootPackageExtractor.PackageCollectingParallelVisitor.getVisitResult
    fn get_visit_result(
        &self,
        dirs_to_check_for_packages: Vec<TraversalInfo>,
    ) -> Result<Visit<TraversalInfo, TraversalInfo>, Self::Error> {
        let traversal_to_key_map: Vec<(TraversalInfo, SkyKey)> = dirs_to_check_for_packages
            .into_iter()
            .map(|traversal_info| {
                let key = SkyKey::new(CollectPackagesUnderDirectoryKey::key(
                    self.repository.clone(),
                    traversal_info.rooted_dir.clone(),
                    traversal_info.forbidden_subdirectories.clone(),
                ));
                (traversal_info, key)
            })
            .collect();
        let keys: Vec<SkyKey> = traversal_to_key_map
            .iter()
            .map(|(_, k)| k.clone())
            .collect();
        let values = self.graph.get_successful_values(&keys);

        // NOTE: Use a TreeSet to ensure a deterministic (sorted) iteration order when we recurse.
        let mut result_package_ids: Vec<TraversalInfo> = Vec::new();
        let mut subdirs_to_check_for_packages: TraversalInfoSet = BTreeMap::new();
        for (info, key) in traversal_to_key_map {
            let collect_packages_value = values
                .iter()
                .find(|(k, _)| *k == key)
                .and_then(|(_, v)| downcast::<CollectPackagesUnderDirectoryValue>(v.clone()));
            let Some(collect_packages_value) = collect_packages_value else {
                continue;
            };
            if collect_packages_value.is_directory_package() {
                result_package_ids.push(info.clone());
            }
            if let Some(error_message) = collect_packages_value.get_error_message() {
                self.event_handler.handle(&Event::error(error_message));
            }
            for (subdirectory, contains) in
                collect_packages_value.get_subdirectory_transitively_contains_packages_or_errors()
            {
                if !*contains {
                    continue;
                }
                let subdirectory_relative_path = subdirectory.get_root_relative_path();
                let forbidden_subdirectories_beneath_this_subdirectory = info
                    .forbidden_subdirectories
                    .filter_for_directory(subdirectory_relative_path);
                let mut excluded_subdirectories_beneath_this_subdirectory: Vec<PathFragment> =
                    Vec::new();
                for path_fragment in &info.excluded_subdirectories {
                    if path_fragment.starts_with(subdirectory_relative_path)
                        && !excluded_subdirectories_beneath_this_subdirectory
                            .contains(path_fragment)
                    {
                        excluded_subdirectories_beneath_this_subdirectory
                            .push(path_fragment.clone());
                    }
                }
                if !excluded_subdirectories_beneath_this_subdirectory
                    .contains(subdirectory_relative_path)
                {
                    traversal_info_set_add(
                        &mut subdirs_to_check_for_packages,
                        TraversalInfo {
                            rooted_dir: subdirectory.clone(),
                            forbidden_subdirectories:
                                forbidden_subdirectories_beneath_this_subdirectory,
                            excluded_subdirectories:
                                excluded_subdirectories_beneath_this_subdirectory,
                        },
                    );
                }
            }
        }
        Ok(Visit {
            keys_to_use_for_result: result_package_ids,
            keys_to_visit: subdirs_to_check_for_packages.into_values().collect(),
        })
    }

    // Java: com/google/devtools/build/lib/skyframe/TraversalInfoRootPackageExtractor.java:202 TraversalInfoRootPackageExtractor.PackageCollectingParallelVisitor.preprocessInitialVisit
    fn preprocess_initial_visit(&self, infos: Vec<TraversalInfo>) -> Vec<TraversalInfo> {
        infos
    }

    // Java: com/google/devtools/build/lib/skyframe/TraversalInfoRootPackageExtractor.java:207 TraversalInfoRootPackageExtractor.PackageCollectingParallelVisitor.noteAndReturnUniqueVisitationKeys
    fn note_and_return_unique_visitation_keys(
        &self,
        prospective_visitation_keys: Vec<TraversalInfo>,
    ) -> Result<Vec<TraversalInfo>, Self::Error> {
        Ok(prospective_visitation_keys)
    }

    fn process(&self, partial_result: Vec<PackageIdentifier>) -> Result<(), Self::Error> {
        (self.results.borrow_mut())(partial_result);
        Ok(())
    }
}
