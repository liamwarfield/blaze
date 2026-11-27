//! Ports of `query2/RBuildFilesVisitor.java` and `query2/RBuildFilesFunction.java`:
//! `rbuildfiles(path, ...)`, the BUILD files of the packages whose loading transitively depends
//! on the given files, read from the reverse edges of the universe's graph.

use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::label_constants;
use blaze_packages::target::Target;
use blaze_protos::failure_details::query;
use blaze_query_engine::{
    Argument, ArgumentType, Callback, QueryEnvironment, QueryEnvironmentFutures, QueryError,
    QueryException, QueryExpression, QueryExpressionContext, QueryFunction, QueryFunctionBase,
    QueryTaskFuture, Uniquifier,
};
use blaze_sky_loading::actions::FileStateValue;
use blaze_sky_loading::directory_listing::{DirectoryListingKey, DirectoryListingStateKey};
use blaze_sky_loading::file::FileKey;
use blaze_sky_loading::file_state::FileStateKey;
use blaze_sky_loading::package_function::PackageKey;
use blaze_sky_loading::package_lookup::{PackageLookupKey, PackageLookupValue};
use blaze_sky_loading::parallel_visitor::{self, ParallelVisitor, Visit};
use blaze_sky_loading::prepare_deps::{
    COLLECT_TARGETS_IN_PACKAGE, PREPARE_DEPS_OF_PATTERN, PREPARE_DEPS_OF_PATTERNS,
    PREPARE_DEPS_OF_TARGETS_UNDER_DIRECTORY,
};
use blaze_sky_loading::recursive_pkg::PACKAGE_ERROR_MESSAGE;
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_sky_loading::sky_functions;
use blaze_sky_loading::transitive_traversal::TRANSITIVE_TRAVERSAL;
use blaze_skyframe::{SkyKey, WalkableGraph};
use blaze_vfs::{PathFragment, RootedPath};
use indexmap::{IndexMap, IndexSet};

use crate::sky_query::parallel_sky_query_utils::VISIT_BATCH_SIZE;
use crate::sky_query::sky_query_environment::SkyQueryEnvironment;

/// Each target in the full output of this visitor corresponds to BUILD file InputFile of a
/// unique package. So the processResultsBatchSize we choose to pass to the ParallelVisitor ctor
/// influences how many packages each leaf task doing processPartialResults will have to deal
/// with at once. A value of 100 was chosen experimentally.
// Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:68 RBuildFilesVisitor.PROCESS_RESULTS_BATCH_SIZE
const PROCESS_RESULTS_BATCH_SIZE: usize = 100;

/// We don't expect to find any additional BUILD files so we skip visitation of these nodes.
// Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:72 RBuildFilesVisitor.NODES_TO_PRUNE_TRAVERSAL
fn is_node_to_prune_traversal(name: blaze_skyframe::SkyFunctionName) -> bool {
    name == TRANSITIVE_TRAVERSAL
        || name == COLLECT_TARGETS_IN_PACKAGE
        || name == PREPARE_DEPS_OF_TARGETS_UNDER_DIRECTORY
        || name == PACKAGE_ERROR_MESSAGE
        || name == PREPARE_DEPS_OF_PATTERN
        || name == PREPARE_DEPS_OF_PATTERNS
}

/// A visitor that finds the packages whose loading depends on given files.
// Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:62 RBuildFilesVisitor
pub struct RBuildFilesVisitor<'a> {
    env: &'a SkyQueryEnvironment,
    visit_uniquifier: &'a dyn Uniquifier<SkyKey>,
    result_uniquifier: &'a dyn Uniquifier<SkyKey>,
    callback: &'a dyn Callback<Target>,
}

impl<'a> RBuildFilesVisitor<'a> {
    // Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:87 RBuildFilesVisitor.<init>
    pub fn new(
        env: &'a SkyQueryEnvironment,
        visit_uniquifier: &'a dyn Uniquifier<SkyKey>,
        result_uniquifier: &'a dyn Uniquifier<SkyKey>,
        callback: &'a dyn Callback<Target>,
    ) -> Self {
        RBuildFilesVisitor {
            env,
            visit_uniquifier,
            result_uniquifier,
            callback,
        }
    }

    /// Packages may depend on the existence of subpackages, but these edges aren't relevant to
    /// rbuildfiles. They may also depend on files transitively through globs, but these cannot
    /// be included in load statements and so we don't traverse through these either.
    // Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:139 RBuildFilesVisitor.processNonPackageRdepAndDetermineVisitations
    fn process_non_package_rdep_and_determine_visitations(
        rdep: &SkyKey,
        keys_to_visit_next: &mut IndexSet<SkyKey>,
    ) {
        let name = rdep.function_name();
        if name != sky_functions::PACKAGE_LOOKUP
            && name != sky_functions::GLOB
            && name != sky_functions::GLOBS
        {
            keys_to_visit_next.insert(rdep.clone());
        }
    }

    // Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:166 RBuildFilesVisitor.visitFileIdentifiersAndWaitForCompletion
    pub fn visit_file_identifiers_and_wait_for_completion(
        &self,
        graph: &dyn WalkableGraph,
        file_keys: &[PathFragment],
    ) -> Result<(), QueryError> {
        parallel_visitor::visit_and_wait_for_completion(
            self,
            get_sky_keys_for_file_fragments(
                graph, file_keys, /* include_ancestor_keys= */ false,
            ),
        )
    }
}

impl ParallelVisitor for RBuildFilesVisitor<'_> {
    type Input = SkyKey;
    type VisitKey = SkyKey;
    type OutputKey = PackageIdentifier;
    type OutputResult = Target;
    type Error = QueryError;

    fn visit_batch_size(&self) -> usize {
        // Java env.getVisitBatchSizeForParallelVisitation().
        VISIT_BATCH_SIZE
    }

    fn process_results_batch_size(&self) -> usize {
        PROCESS_RESULTS_BATCH_SIZE
    }

    // Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:105 RBuildFilesVisitor.getVisitResult
    fn get_visit_result(
        &self,
        values: Vec<SkyKey>,
    ) -> Result<Visit<PackageIdentifier, SkyKey>, QueryError> {
        let reverse_deps = self.env.graph().get_reverse_deps(&values);
        // CompactHashSet: insertion order.
        let mut keys_to_use_for_result: IndexSet<PackageIdentifier> = IndexSet::new();
        let mut keys_to_visit_next: IndexSet<SkyKey> = IndexSet::new();
        let external_package_key = SkyKey::new(PackageKey::key(
            label_constants::EXTERNAL_PACKAGE_IDENTIFIER.clone(),
        ));
        for (_, rdeps) in reverse_deps {
            for rdep in rdeps {
                // PORT: Java checks Thread.interrupted() here (the loop is cpu bound); Blaze's
                // query has no interrupt point inside a visit.
                if rdep.function_name() == sky_functions::PACKAGE {
                    if self.result_uniquifier.unique(&rdep)? {
                        let Some(package_key) = rdep.downcast::<PackageKey>() else {
                            // Java casts the argument to PackageIdentifier; a package piece key
                            // (lazy macro expansion) is not one.
                            blaze_util::unported!("RBuildFilesVisitor with package piece keys");
                        };
                        keys_to_use_for_result.insert(package_key.0.clone());
                    }
                    // PackageValue(//p) has a transitive dep on the PackageValue(//external), so
                    // we need to make sure these dep paths are traversed. These dep paths go
                    // through the singleton WorkspaceNameValue(), and that node has a direct dep
                    // on PackageValue(//external), so it suffices to ensure we visit
                    // PackageValue(//external).
                    if rdep == external_package_key {
                        keys_to_visit_next.insert(rdep);
                    }
                } else if !is_node_to_prune_traversal(rdep.function_name()) {
                    Self::process_non_package_rdep_and_determine_visitations(
                        &rdep,
                        &mut keys_to_visit_next,
                    );
                }
            }
        }
        Ok(Visit {
            keys_to_use_for_result: keys_to_use_for_result.into_iter().collect(),
            keys_to_visit: keys_to_visit_next.into_iter().collect(),
        })
    }

    // Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:135 RBuildFilesVisitor.preprocessInitialVisit
    fn preprocess_initial_visit(&self, visitation_keys: Vec<SkyKey>) -> Vec<SkyKey> {
        visitation_keys
    }

    // Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:154 RBuildFilesVisitor.outputKeysToOutputValues
    fn output_keys_to_output_values(
        &self,
        target_keys: Vec<PackageIdentifier>,
    ) -> Result<Vec<Target>, QueryError> {
        let unique: Vec<PackageIdentifier> = target_keys
            .into_iter()
            .collect::<IndexSet<_>>()
            .into_iter()
            .collect();
        Ok(self.env.get_build_file_targets_for_package_keys(&unique))
    }

    // Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:160 RBuildFilesVisitor.noteAndReturnUniqueVisitationKeys
    fn note_and_return_unique_visitation_keys(
        &self,
        prospective_visitation_keys: Vec<SkyKey>,
    ) -> Result<Vec<SkyKey>, QueryError> {
        Ok(self
            .visit_uniquifier
            .unique_all(&prospective_visitation_keys)?)
    }

    fn process(&self, partial_result: Vec<Target>) -> Result<(), QueryError> {
        self.callback.process(&partial_result)
    }
}

/// The FILE_STATE and FILE keys of the given files, under the root of the package each is in
/// (and, with `include_ancestor_keys`, the directory listing keys of their ancestors).
// Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:200 RBuildFilesVisitor.getSkyKeysForFileFragments
pub fn get_sky_keys_for_file_fragments(
    graph: &dyn WalkableGraph,
    path_fragments: &[PathFragment],
    include_ancestor_keys: bool,
) -> Vec<SkyKey> {
    if path_fragments.is_empty() {
        return Vec::new();
    }
    // PORT: Java's result is a HashSet; here in insertion order.
    let mut result: IndexSet<SkyKey> = IndexSet::new();
    // ListMultimap<PathFragment, PathFragment>: ancestors in first-seen order.
    let mut current_ancestor_to_original_path: IndexMap<PathFragment, Vec<PathFragment>> =
        IndexMap::new();
    for path_fragment in path_fragments {
        check_workspace_file(&mut result, path_fragment);
        if let Some(parent_path_fragment) = path_fragment.get_parent_directory() {
            current_ancestor_to_original_path
                .entry(parent_path_fragment)
                .or_default()
                .push(path_fragment.clone());
        }
    }
    // Used to find directories that have been added in the diff.
    let mut paths_to_check_for_newly_added_directories: IndexSet<RootedPath> = IndexSet::new();
    // We look at each ancestor directory of every file, and use the
    // currentAncestorToOriginalPath map to avoid doing unnecessary work with common ancestors.
    // If we don't find a package with the first level of ancestors, we go up a level, until we
    // find the first package for every file. If a file doesn't have a parent package, the file
    // is ignored.
    while !current_ancestor_to_original_path.is_empty() {
        let pkg_lookup_keys: Vec<SkyKey> = current_ancestor_to_original_path
            .keys()
            .map(get_pkg_lookup_key_for_directory)
            .collect();
        let lookup_values = graph.get_successful_values(&pkg_lookup_keys);
        for (key, value) in lookup_values {
            let package_lookup_value = value
                .downcast::<PackageLookupValue>()
                .expect("a PackageLookupValue");
            if package_lookup_value.package_exists() {
                let current_fragment = key
                    .downcast::<PackageLookupKey>()
                    .expect("a PACKAGE_LOOKUP key")
                    .0
                    .get_package_fragment()
                    .clone();
                let original_files = current_ancestor_to_original_path
                    .get(&current_fragment)
                    .cloned()
                    .unwrap_or_default();
                blaze_util::check_state!(!original_files.is_empty(), "{:?}", key);
                for file_name in original_files {
                    let rooted_path = RootedPath::to_rooted_path(
                        package_lookup_value.get_root().clone(),
                        &file_name,
                    );
                    result.insert(SkyKey::new(FileStateKey::key(rooted_path.clone())));
                    // Include the File key too in case the FileState is never considered due to
                    // a missing parent directory.
                    result.insert(SkyKey::new(FileKey::key(rooted_path.clone())));
                    if include_ancestor_keys {
                        let parent_path = rooted_path
                            .get_parent_directory()
                            .expect("a file has a parent directory");
                        result.insert(SkyKey::new(DirectoryListingStateKey::key(
                            parent_path.clone(),
                        )));
                        // Include the DirectoryListing key too in case the
                        // DirectoryListingState is never considered due to a missing parent
                        // directory.
                        result.insert(SkyKey::new(DirectoryListingKey::key(parent_path)));
                        let mut path_to_check_if_newly_added = file_name.clone();
                        while path_to_check_if_newly_added.get_path_string().len()
                            > current_fragment.get_path_string().len()
                        {
                            paths_to_check_for_newly_added_directories.insert(
                                RootedPath::to_rooted_path(
                                    package_lookup_value.get_root().clone(),
                                    &path_to_check_if_newly_added,
                                ),
                            );
                            path_to_check_if_newly_added = path_to_check_if_newly_added
                                .get_parent_directory()
                                .expect("a path longer than its package has a parent");
                        }
                    }
                }
                current_ancestor_to_original_path.shift_remove(&current_fragment);
            }
        }
        current_ancestor_to_original_path = go_up_one_directory(current_ancestor_to_original_path);
    }
    if include_ancestor_keys {
        include_ancestor_keys_in_result(
            graph,
            &mut result,
            paths_to_check_for_newly_added_directories,
        );
    }
    result.into_iter().collect()
}

// Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:272 RBuildFilesVisitor.goUpOneDirectory
fn go_up_one_directory(
    current_to_original: IndexMap<PathFragment, Vec<PathFragment>>,
) -> IndexMap<PathFragment, Vec<PathFragment>> {
    let mut new_current_to_original: IndexMap<PathFragment, Vec<PathFragment>> = IndexMap::new();
    for (path_fragment, originals) in current_to_original {
        if let Some(parent) = path_fragment.get_parent_directory() {
            new_current_to_original
                .entry(parent)
                .or_default()
                .extend(originals);
        }
    }
    new_current_to_original
}

/// The WORKSPACE file is a transitive dependency of every package. Unfortunately, there is no
/// specific SkyValue that we can use to figure out under which package path entries it lives so
/// we add a dependency on the main repo mapping key.
// Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:284 RBuildFilesVisitor.checkWorkspaceFile
fn check_workspace_file(result: &mut IndexSet<SkyKey>, file: &PathFragment) {
    if *file == *label_constants::WORKSPACE_FILE_NAME
        || *file == *label_constants::WORKSPACE_DOT_BAZEL_FILE_NAME
    {
        result.insert(SkyKey::new(RepositoryMappingKey::key(
            blaze_cmdline::repository_name::MAIN.clone(),
        )));
    }
}

/// Do a single batch fetch of all FileState's corresponding to directories with failed package
/// lookups.
// Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:300 RBuildFilesVisitor.includeAncestorKeysInResult
fn include_ancestor_keys_in_result(
    graph: &dyn WalkableGraph,
    result: &mut IndexSet<SkyKey>,
    file_state_keys_to_fetch: IndexSet<RootedPath>,
) {
    let keys: Vec<SkyKey> = file_state_keys_to_fetch
        .iter()
        .map(|p| SkyKey::new(FileStateKey::key(p.clone())))
        .collect();
    let file_state_values = graph.get_successful_values(&keys);
    for (rooted_path, file_state_key) in file_state_keys_to_fetch.into_iter().zip(&keys) {
        match file_state_values.iter().find(|(k, _)| k == file_state_key) {
            Some((_, value)) => {
                let fsv = value
                    .clone()
                    .downcast::<FileStateValue>()
                    .expect("a FileStateValue");
                if !fsv.get_type().exists() && !fsv.get_type().is_directory() {
                    process_file_state_key_for_missing_directory(result, rooted_path);
                }
            }
            None => process_file_state_key_for_missing_directory(result, rooted_path),
        }
    }
}

/// Add a DirectoryListingState node to our traversal even if the ancestor path too didn't
/// exist prior to the diff.
// Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:318 RBuildFilesVisitor.processFileStateKeyForMissingDirectory
fn process_file_state_key_for_missing_directory(
    result: &mut IndexSet<SkyKey>,
    rooted_path: RootedPath,
) {
    result.insert(SkyKey::new(FileStateKey::key(rooted_path.clone())));
    result.insert(SkyKey::new(FileKey::key(rooted_path.clone())));
    if let Some(parent_path) = rooted_path.get_parent_directory() {
        result.insert(SkyKey::new(DirectoryListingStateKey::key(
            parent_path.clone(),
        )));
        result.insert(SkyKey::new(DirectoryListingKey::key(parent_path)));
    }
}

// Java: com/google/devtools/build/lib/query2/RBuildFilesVisitor.java:341 RBuildFilesVisitor.getPkgLookupKeyForDirectory
fn get_pkg_lookup_key_for_directory(path_fragment: &PathFragment) -> SkyKey {
    SkyKey::new(PackageLookupKey::key(
        PackageIdentifier::create_in_main_repo(path_fragment.clone()),
    ))
}

/// An "rbuildfiles" query expression, which computes the set of packages (as represented by
/// their BUILD source file targets) that depend on the given set of files, either directly or
/// through a transitive chain of loads. Only BUILD and .bzl files in the universe are found.
// Java: com/google/devtools/build/lib/query2/RBuildFilesFunction.java:43 RBuildFilesFunction
pub struct RBuildFilesFunction;

impl QueryFunctionBase for RBuildFilesFunction {
    // Java: com/google/devtools/build/lib/query2/RBuildFilesFunction.java:46 RBuildFilesFunction.getName
    fn get_name(&self) -> String {
        "rbuildfiles".to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/RBuildFilesFunction.java:51 RBuildFilesFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        1
    }

    // Java: com/google/devtools/build/lib/query2/RBuildFilesFunction.java:56 RBuildFilesFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Word]
    }

    fn get_argument_types_cycle(&self) -> bool {
        true
    }
}

impl QueryFunction<Target> for RBuildFilesFunction {
    // Java: com/google/devtools/build/lib/query2/RBuildFilesFunction.java:62 RBuildFilesFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<Target>,
        _context: &QueryExpressionContext<Target>,
        _expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<Target>,
    ) -> QueryTaskFuture<()> {
        let Some(sky_env) = env
            .as_any()
            .and_then(|a| a.downcast_ref::<SkyQueryEnvironment>())
        else {
            return env.immediate_failed_future(QueryException::new(
                None,
                "rbuildfiles can only be used with SkyQueryEnvironment",
                query::Code::RbuildfilesFunctionRequiresSkyquery,
            ));
        };
        let paths: Vec<PathFragment> = args
            .iter()
            .map(|argument| PathFragment::create(argument.get_word()))
            .collect();
        sky_env.get_rbuild_files(&paths, callback)
    }
}
