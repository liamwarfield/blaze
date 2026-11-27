//! Port of `skyframe/IncrementalPackageRoots.java`: the execroot symlinks of a build with merged
//! analysis and execution (Skymeld), planted as the top-level targets finish their analysis.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use blaze_actions::actions::artifact_factory::PackageRootLookup;
use blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata;
use blaze_buildtool::buildtool::symlink_forest::{SymlinkForest, SymlinkPlantingException};
use blaze_cmdline::PackageIdentifier;
use blaze_collect::NestedSet;
use blaze_events::Postable;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_sky_exec::events::{TopLevelTargetReadyForSymlinkPlanting, downcast_event};
use blaze_sky_loading::ignored_subdirectories::IgnoredSubdirectories;
use blaze_vfs::{Path, Root};
use indexmap::IndexSet;
use parking_lot::Mutex;

/// The state dropped when the analysis is finished (Java's `donePackages` and
/// `lazilyPlantedSymlinks`, both nulled by `dropIntermediateStatesAndUnregisterFromEventBus`).
struct PlantingState {
    /// The `NestedSet` nodes already traversed (Java `Set<NestedSet.Node>`), by node id.
    done_packages: HashSet<usize>,
    /// Only tracks the symlinks lazily planted after the first eager planting wave.
    lazily_planted_symlinks: IndexSet<Path>,
}

/// An implementation of `PackageRoots` that allows incremental updating of the packageRootsMap.
///
/// This class is also in charge of planting the necessary symlinks.
///
/// PORT: Java plants the lazily needed symlinks on a pool of 200 threads and waits for them; the
/// order of planting is not observable, so they are planted on the posting thread here. Java's
/// `stateLock` and the pool's lock are the mutexes of [`PlantingState`].
// Java: com/google/devtools/build/lib/skyframe/IncrementalPackageRoots.java:63 IncrementalPackageRoots
pub struct IncrementalPackageRoots {
    // We only keep track of PackageIdentifier from external repos here as a memory optimization:
    // packages belong to the main repository all share the same root, which is singleSourceRoot.
    thread_safe_external_repo_package_roots_map: Mutex<HashMap<PackageIdentifier, Root>>,
    state: Mutex<Option<PlantingState>>,
    execroot: Path,
    single_source_root: Root,
    prefix: String,
    ignored_paths: IgnoredSubdirectories,
    use_sibling_repository_layout: bool,
    allow_external_repositories: bool,
    fallback_package_root_lookup: Option<Arc<dyn PackageRootLookup>>,
    // "maybe" because some conflicts in a case-insensitive FS may not be in a case-sensitive one.
    maybe_conflicting_base_names_lowercase: Mutex<IndexSet<String>>,
}

/// `AbruptExitException` of the symlink planting (`throwAbruptExitException`).
#[derive(Debug, Clone)]
pub struct SymlinkForestAbruptExit {
    pub failure_detail: FailureDetail,
}

impl IncrementalPackageRoots {
    // Java: com/google/devtools/build/lib/skyframe/IncrementalPackageRoots.java:97 IncrementalPackageRoots.<init>
    // Java: com/google/devtools/build/lib/skyframe/IncrementalPackageRoots.java:141 IncrementalPackageRoots.createAndRegisterToEventBus
    // PORT: the caller registers the returned value on the command's event bus (Java's
    // `eventBus.register(incrementalPackageRoots)`), see `analysis_finished`.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        execroot: Path,
        single_source_root: Root,
        prefix: String,
        ignored_paths: IgnoredSubdirectories,
        use_sibling_repository_layout: bool,
        allow_external_repositories: bool,
        fallback_package_root_lookup: Option<Arc<dyn PackageRootLookup>>,
    ) -> Arc<IncrementalPackageRoots> {
        Arc::new(IncrementalPackageRoots {
            thread_safe_external_repo_package_roots_map: Mutex::new(HashMap::new()),
            state: Mutex::new(Some(PlantingState {
                done_packages: HashSet::new(),
                lazily_planted_symlinks: IndexSet::new(),
            })),
            execroot,
            single_source_root,
            prefix,
            ignored_paths,
            use_sibling_repository_layout,
            allow_external_repositories,
            fallback_package_root_lookup,
            maybe_conflicting_base_names_lowercase: Mutex::new(IndexSet::new()),
        })
    }

    /// Eagerly plant the symlinks to the directories under the single source root. It's possible
    /// that there's a conflict when we plant symlinks eagerly. In that case, we skip planting the
    /// conflicting symlinks eagerly and wait until later in the build to see which of the
    /// conflicting dir we actually need.
    ///
    /// Eagerly planting the symlinks is much cheaper, hence we'd like to do it as much as possible
    /// and only resort to the other route when really necessary.
    // Java: com/google/devtools/build/lib/skyframe/IncrementalPackageRoots.java:190 IncrementalPackageRoots.eagerlyPlantSymlinksToSingleSourceRoot
    pub fn eagerly_plant_symlinks_to_single_source_root(
        &self,
    ) -> Result<(), SymlinkForestAbruptExit> {
        let ignored_paths = &self.ignored_paths;
        match SymlinkForest::eagerly_plant_symlink_forest_single_package_path(
            &self.execroot,
            self.single_source_root
                .as_path()
                .expect("the single source root is a path"),
            &self.prefix,
            &|fragment| ignored_paths.matching_entry(fragment),
            self.use_sibling_repository_layout,
        ) {
            Ok(names) => {
                *self.maybe_conflicting_base_names_lowercase.lock() = names;
                Ok(())
            }
            Err(e) => Err(throw_abrupt_exit_exception(&format!(
                "java.io.IOException: {e}"
            ))),
        }
    }

    // Intentionally don't allow concurrent events here to prevent a race condition between
    // planting a symlink and starting an action that requires that symlink. This race condition
    // is possible because of the various memoizations we use to avoid repeated work.
    // Java: com/google/devtools/build/lib/skyframe/IncrementalPackageRoots.java:264 IncrementalPackageRoots.lazilyPlantSymlinks
    pub fn lazily_plant_symlinks(
        &self,
        event: &TopLevelTargetReadyForSymlinkPlanting,
    ) -> Result<(), SymlinkForestAbruptExit> {
        if !(self.allow_external_repositories
            || !self
                .maybe_conflicting_base_names_lowercase
                .lock()
                .is_empty())
        {
            return Ok(());
        }
        // May still race with analysisFinished, hence the synchronization.
        let mut state = self.state.lock();
        let Some(state) = state.as_mut() else {
            return Ok(());
        };
        self.recursive_register_and_plant_missing_symlinks(
            &event.transitive_packages_for_symlink_planting,
            state,
        )
    }

    /// Drops the intermediate states and stop receiving new events.
    ///
    /// This essentially makes this instance read-only. Should be called when and only when all
    /// analysis work is done in the build to free up some memory.
    // Java: com/google/devtools/build/lib/skyframe/IncrementalPackageRoots.java:303 IncrementalPackageRoots.analysisFinished
    // Java: com/google/devtools/build/lib/skyframe/IncrementalPackageRoots.java:424 IncrementalPackageRoots.dropIntermediateStatesAndUnregisterFromEventBus
    pub fn analysis_finished(&self) {
        *self.state.lock() = None;
        self.maybe_conflicting_base_names_lowercase.lock().clear();
    }

    /// Lazily plant the required symlinks that couldn't be planted in the initial eager planting
    /// wave.
    ///
    /// There are 2 possibilities: either we're planting symlinks to the external repos, or
    /// there's potentially conflicting symlinks detected.
    // Java: com/google/devtools/build/lib/skyframe/IncrementalPackageRoots.java:313 IncrementalPackageRoots.recursiveRegisterAndPlantMissingSymlinks
    fn recursive_register_and_plant_missing_symlinks(
        &self,
        packages: &NestedSet<RepoMappingPackageMetadata>,
        state: &mut PlantingState,
    ) -> Result<(), SymlinkForestAbruptExit> {
        // Optimization to prune subsequent traversals.
        // A false negative does not affect correctness.
        if !state.done_packages.insert(packages.node_id()) {
            return Ok(());
        }

        for pkg in packages.get_leaves() {
            self.plant_single_symlink_for_package(&pkg, &mut state.lazily_planted_symlinks)?;
        }
        for transitive in packages.get_non_leaves() {
            self.recursive_register_and_plant_missing_symlinks(&transitive, state)?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/skyframe/IncrementalPackageRoots.java:341 IncrementalPackageRoots.plantSingleSymlinkForPackage
    fn plant_single_symlink_for_package(
        &self,
        pkg: &RepoMappingPackageMetadata,
        lazily_planted_symlinks: &mut IndexSet<Path>,
    ) -> Result<(), SymlinkForestAbruptExit> {
        let pkg_id = pkg.package_identifier();
        if is_external_repository(pkg_id) {
            let source_root = pkg
                .source_root()
                .unwrap_or_else(|| {
                    panic!("NullPointerException: sourceRoot of the package metadata of {pkg_id}")
                })
                .clone();
            self.thread_safe_external_repo_package_roots_map
                .lock()
                .entry(pkg_id.clone())
                .or_insert_with(|| source_root.clone());
            if let Err(e) = SymlinkForest::plant_single_symlink_for_external_repo(
                pkg_id.get_repository(),
                source_root
                    .as_path()
                    .expect("an external repository's source root is a path"),
                &self.execroot,
                self.use_sibling_repository_layout,
                lazily_planted_symlinks,
            ) {
                return Err(throw_abrupt_exit_exception(&format!(
                    "java.io.IOException: {e}"
                )));
            }
        } else if !self
            .maybe_conflicting_base_names_lowercase
            .lock()
            .is_empty()
        {
            let original_base_name = pkg_id.get_top_level_dir();
            let base_name_lowercase = original_base_name.to_ascii_lowercase();

            // As Skymeld only supports single package path at the moment, we only seek to
            // symlink to the top-level dir i.e. what's directly under the source root.
            let link = self.execroot.get_relative_str(original_base_name);
            let target = self.single_source_root.get_relative_str(original_base_name);

            let ignored_paths = &self.ignored_paths;
            if original_base_name.is_empty()
                || !self
                    .maybe_conflicting_base_names_lowercase
                    .lock()
                    .contains(&base_name_lowercase)
                || !SymlinkForest::symlink_should_be_planted_ignoring(
                    &self.prefix,
                    &|fragment| ignored_paths.matching_entry(fragment),
                    self.use_sibling_repository_layout,
                    original_base_name,
                    &target,
                )
            {
                // We should have already eagerly planted a symlink for this, or there's nothing
                // to do.
                return Ok(());
            }

            if lazily_planted_symlinks.insert(link.clone())
                && let Err(e) = link.create_symbolic_link(&target)
            {
                let mut error_message =
                    format!("Failed to plant a symlink: {} -> {}", link, target);
                if link.exists() && link.is_symbolic_link() {
                    // If the link already exists, it must mean that we're planting from a
                    // case-insensitive file system and this is a legitimate conflict.
                    // TODO(b/295300378) We technically can go deeper here and try to create the
                    // subdirs to try to resolve the conflict, but the complexity isn't worth it
                    // at the moment and the non-skymeld code path isn't doing any better.
                    // Revisit if necessary.
                    if let Ok(existing_target) = link.resolve_symbolic_links()
                        && existing_target != target
                    {
                        error_message.push_str(&format!(
                            ". Found an existing conflicting symlink: {} -> {}",
                            link, existing_target
                        ));
                    }
                }
                let e = SymlinkPlantingException::new(error_message, e);
                return Err(throw_abrupt_exit_exception(&format!(
                    "com.google.devtools.build.lib.buildtool.SymlinkForest$SymlinkPlantingException: {}",
                    e.msg
                )));
            }
        }
        Ok(())
    }
}

/// Returns a lookup function for package roots.
///
/// For packages in the main repository, the lookup unconditionally returns
/// `singleSourceRoot`. For external repositories, the lookup consults
/// `threadSafeExternalRepoPackageRootsMap` and falls back to `fallbackPackageRootLookup` if
/// present.
// Java: com/google/devtools/build/lib/skyframe/IncrementalPackageRoots.java:239 IncrementalPackageRoots.getPackageRootLookup
impl PackageRootLookup for IncrementalPackageRoots {
    fn get_root_for_package(&self, package_id: &PackageIdentifier) -> Option<Root> {
        if package_id.get_repository().is_main() {
            return Some(self.single_source_root.clone());
        }
        if let Some(root) = self
            .thread_safe_external_repo_package_roots_map
            .lock()
            .get(package_id)
        {
            return Some(root.clone());
        }
        if let Some(fallback) = &self.fallback_package_root_lookup
            && let Some(root) = fallback.get_root_for_package(package_id)
        {
            self.thread_safe_external_repo_package_roots_map
                .lock()
                .entry(package_id.clone())
                .or_insert_with(|| root.clone());
            return Some(root);
        }
        None
    }
}

/// The `@Subscribe` methods: `lazilyPlantSymlinks(TopLevelTargetReadyForSymlinkPlanting)`.
///
/// PORT: an exception in a Guava subscriber goes to the runtime's
/// `SubscriberExceptionHandler`, `BugReport.handleCrash`; it is a panic here. Java's
/// `analysisFinished(AnalysisPhaseCompleteEvent)` subscription is
/// [`IncrementalPackageRoots::analysis_finished`], which the caller calls once the top-level
/// keys were evaluated: Blaze evaluates the `BuildDriverKey`s (which post the planting events)
/// after its analysis phase completes (blaze_sky_top `build_view` module comment), so the
/// `AnalysisPhaseCompleteEvent` would come before them.
impl blaze_events::EventBus for IncrementalPackageRoots {
    fn post(&self, event: Arc<dyn Postable>) {
        if let Some(e) = downcast_event::<TopLevelTargetReadyForSymlinkPlanting>(&event)
            && let Err(e) = self.lazily_plant_symlinks(e)
        {
            panic!("AbruptExitException: {}", e.failure_detail.message);
        }
        // @Subscribe analysisFinished(AnalysisPhaseCompleteEvent unused)
        if downcast_event::<blaze_buildtool::buildtool::buildevent::AnalysisPhaseCompleteEvent>(
            &event,
        )
        .is_some()
        {
            self.analysis_finished();
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/IncrementalPackageRoots.java:404 IncrementalPackageRoots.throwAbruptExitException
fn throw_abrupt_exit_exception(exception_to_string: &str) -> SymlinkForestAbruptExit {
    SymlinkForestAbruptExit {
        failure_detail: FailureDetail {
            message: format!("Failed to prepare the symlink forest: {exception_to_string}"),
            category: Some(Category::SymlinkForest(failure_details::SymlinkForest {
                code: failure_details::symlink_forest::Code::CreationFailed as i32,
            })),
        },
    }
}

// Java: com/google/devtools/build/lib/skyframe/IncrementalPackageRoots.java:416 IncrementalPackageRoots.isExternalRepository
fn is_external_repository(pkg_id: &PackageIdentifier) -> bool {
    !pkg_id.get_repository().is_main()
}
