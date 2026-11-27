//! GLOBS: Skyframe globbing of all globs of a package in one node (Bazel's default
//! `GlobbingStrategy.SINGLE_GLOBS_HYBRID`).
//!
//! Java: com/google/devtools/build/lib/skyframe/{GlobsFunction,GlobsValue,GlobDescriptor,
//! GlobException,InvalidGlobPatternException}.java and the state machines of
//! com/google/devtools/build/lib/packages/producers/ (GlobComputationProducer, FragmentProducer,
//! PatternWithWildcardProducer, PatternWithoutWildcardProducer, DirectoryDirentProducer,
//! SymlinkProducer, GlobError).
//!
//! PORT: Java drives one state machine per glob, in parallel. Blaze's evaluator is blocking, so
//! each glob is a work queue of the same steps (`tasks.enqueue` pushes onto the queue) on the
//! calling thread. The result is a set, so the order of the steps does not matter.

use std::collections::{HashSet, VecDeque};
use std::fmt;

use blaze_cmdline::PackageIdentifier;
use blaze_packages::build::unix_glob;
use blaze_packages::package::GlobberOperation;
use blaze_skyframe::{
    ComputeError, Environment, FunctionHermeticity, Key, SkyFunctionException, SkyFunctionName,
};
use blaze_vfs::{DirentType, PathFragment, Root, RootedPath};

use crate::directory_listing::DirectoryListingKey;
use crate::file::FileKey;
use crate::file_chain_uniqueness::FileSymlinkInfiniteExpansionUniquenessKey;
use crate::ignored_subdirectories::{IgnoredSubdirectories, IgnoredSubdirectoriesKey};
use crate::io::IOException;
use crate::package_lookup::{PackageLookupKey, PackageLookupValue};
use crate::sky_functions;

/// An exception that indicates that a glob pattern is syntactically invalid.
// Java: com/google/devtools/build/lib/skyframe/InvalidGlobPatternException.java:21 InvalidGlobPatternException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid glob pattern '{pattern}': {error}")]
pub struct InvalidGlobPatternException {
    pattern: String,
    error: String,
}

/// Java `Globber.Operation.toString()` (the enum constant name).
fn operation_name(operation: GlobberOperation) -> &'static str {
    match operation {
        GlobberOperation::Files => "FILES",
        GlobberOperation::FilesAndDirs => "FILES_AND_DIRS",
        GlobberOperation::Subpackages => "SUBPACKAGES",
    }
}

/// One glob pattern of a package and its operation.
// Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:62 GlobsValue.GlobRequest
// Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:91 GlobsValue.GlobRequest.equals
// Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:103 GlobsValue.GlobRequest.hashCode
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct GlobRequest {
    pattern: String,
    glob_operation: GlobberOperation,
}

impl GlobRequest {
    // Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:73 GlobsValue.GlobRequest.getPattern
    pub fn get_pattern(&self) -> &str {
        &self.pattern
    }

    // Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:77 GlobsValue.GlobRequest.getGlobOperation
    pub fn get_glob_operation(&self) -> GlobberOperation {
        self.glob_operation
    }

    /// Creates a [`GlobRequest`], checking the pattern.
    // Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:111 GlobsValue.GlobRequest.create
    pub fn create(
        pattern: &str,
        glob_operation: GlobberOperation,
    ) -> Result<GlobRequest, InvalidGlobPatternException> {
        if pattern.contains('?') {
            return Err(InvalidGlobPatternException {
                pattern: pattern.to_owned(),
                error: "wildcard ? forbidden".to_owned(),
            });
        }
        if let Some(error) = unix_glob::check_pattern_for_error(pattern) {
            return Err(InvalidGlobPatternException {
                pattern: pattern.to_owned(),
                error,
            });
        }
        Ok(GlobRequest {
            pattern: pattern.to_owned(),
            glob_operation,
        })
    }

    fn sort_key(&self) -> (&str, u8) {
        (&self.pattern, self.glob_operation as u8)
    }
}

// Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:84 GlobsValue.GlobRequest.toString
impl fmt::Display for GlobRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GlobRequest: {} {}",
            self.pattern,
            operation_name(self.glob_operation)
        )
    }
}

impl fmt::Debug for GlobRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// The matches of all globs of a package, relative to the package directory.
// Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:39 GlobsValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobsValue {
    matches: HashSet<PathFragment>,
}

impl GlobsValue {
    pub fn new(matches: HashSet<PathFragment>) -> Self {
        GlobsValue { matches }
    }

    // Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:48 GlobsValue.getMatches
    pub fn get_matches(&self) -> &HashSet<PathFragment> {
        &self.matches
    }
}

/// The key of GLOBS: a package and the set of its glob requests.
///
/// The requests are kept sorted and deduplicated, so that equality is set equality (Java
/// `ImmutableSet.equals`).
// Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:148 GlobsValue.Key
// Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:211 GlobsValue.Key.equals
// Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:224 GlobsValue.Key.hashCode
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct GlobsKey {
    package_identifier: PackageIdentifier,
    package_root: Root,
    glob_requests: Vec<GlobRequest>,
}

impl GlobsKey {
    // Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:136 GlobsValue.key
    pub fn key(
        package_identifier: PackageIdentifier,
        package_root: Root,
        glob_requests: impl IntoIterator<Item = GlobRequest>,
    ) -> GlobsKey {
        let mut glob_requests: Vec<GlobRequest> = glob_requests.into_iter().collect();
        glob_requests.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        glob_requests.dedup();
        GlobsKey {
            package_identifier,
            package_root,
            glob_requests,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:184 GlobsValue.Key.getPackageIdentifier
    pub fn get_package_identifier(&self) -> &PackageIdentifier {
        &self.package_identifier
    }

    // Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:189 GlobsValue.Key.getPackageRoot
    pub fn get_package_root(&self) -> &Root {
        &self.package_root
    }

    // Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:197 GlobsValue.Key.getGlobRequests
    pub fn get_glob_requests(&self) -> &[GlobRequest] {
        &self.glob_requests
    }
}

// Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:228 GlobsValue.Key.toString
impl fmt::Debug for GlobsKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut requests: Vec<String> = self.glob_requests.iter().map(|r| r.to_string()).collect();
        requests.sort();
        write!(
            f,
            "<GlobsKey packageRoot = {}, packageIdentifier = {}, globRequests = [{}]>",
            self.package_root,
            self.package_identifier,
            requests.join(",")
        )
    }
}

impl Key for GlobsKey {
    type Value = GlobsValue;
    // Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:206 GlobsValue.Key.functionName
    const FUNCTION: SkyFunctionName = sky_functions::GLOBS;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:55 SkyFunctions.GLOBS (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: the set of matches.
    // Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:48 GlobsValue.equals
    // Java: com/google/devtools/build/lib/skyframe/GlobsValue.java:59 GlobsValue.hashCode
    fn values_equal(old: &GlobsValue, new: &GlobsValue) -> bool {
        old == new
    }
}

/// Tagged union of possible errors that can be accepted while globbing (Java `GlobError`). Both
/// kinds are `IOException`s in Blaze ([`IOException::inconsistent_filesystem`] and
/// [`IOException::file_symlink_infinite_expansion`]).
// Java: com/google/devtools/build/lib/packages/producers/GlobError.java:21 GlobError
type GlobError = IOException;

/// Wraps the first error of a glob computation (Java `GlobException.handleExceptions`).
// Java: com/google/devtools/build/lib/skyframe/GlobException.java:39 GlobException.handleExceptions
fn handle_exceptions(error: Option<GlobError>) -> Result<(), ComputeError> {
    match error {
        None => Ok(()),
        Some(e) if e.is_inconsistent_filesystem() => Err(SkyFunctionException::transient(e).into()),
        Some(e) => Err(SkyFunctionException::persistent(e).into()),
    }
}

/// Java `GlobComputationProducer.GlobDetail`: the constant information of one glob.
// Java: com/google/devtools/build/lib/packages/producers/GlobComputationProducer.java:140 GlobComputationProducer.GlobDetail
struct GlobDetail<'a> {
    package_identifier: &'a PackageIdentifier,
    package_root: &'a Root,
    pattern_fragments: Vec<String>,
    contains_multiple_double_stars: bool,
    ignored_subdirectories: &'a IgnoredSubdirectories,
    glob_operation: GlobberOperation,
}

/// One pending step (Java: an enqueued sub-`StateMachine`).
enum Task {
    // Java: com/google/devtools/build/lib/packages/producers/FragmentProducer.java:115 FragmentProducer.step
    Fragment {
        base: PathFragment,
        fragment_index: usize,
    },
    // Java: com/google/devtools/build/lib/packages/producers/DirectoryDirentProducer.java:76 DirectoryDirentProducer.step
    DirectoryDirent {
        dirent_path: PathFragment,
        fragment_index: usize,
    },
}

/// The state shared by the steps of one glob (Java: the producers' `ResultSink`s, the visited
/// set and the Skyframe environment).
struct GlobRun<'a, 'e, 'x> {
    detail: GlobDetail<'a>,
    env: &'e mut Environment<'x>,
    /// Java `FragmentProducer.visitedGlobSubTasks`; `None` unless the pattern has several `**`.
    visited_glob_sub_tasks: Option<HashSet<(PathFragment, usize)>>,
    queue: VecDeque<Task>,
    /// Matches including the package fragment.
    path_fragments_with_package_fragment: HashSet<PathFragment>,
    /// The first error accepted.
    error: Option<GlobError>,
    /// A dependency was missing (in error): the computation is incomplete.
    values_missing: bool,
}

impl GlobRun<'_, '_, '_> {
    // Java: com/google/devtools/build/lib/packages/producers/GlobComputationProducer.java:125 GlobComputationProducer.acceptPathFragmentWithPackageFragment
    // Java: com/google/devtools/build/lib/packages/producers/FragmentProducer.java:39 FragmentProducer.ResultSink.acceptPathFragmentWithPackageFragment
    fn accept_path_fragment_with_package_fragment(&mut self, path_fragment: PathFragment) {
        self.path_fragments_with_package_fragment
            .insert(path_fragment);
    }

    // Java: com/google/devtools/build/lib/packages/producers/GlobComputationProducer.java:130 GlobComputationProducer.acceptGlobError
    // Java: com/google/devtools/build/lib/packages/producers/FragmentProducer.java:41 FragmentProducer.ResultSink.acceptGlobError
    fn accept_glob_error(&mut self, error: GlobError) {
        // Keeps the first reported error if there are multiple.
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    // Java: com/google/devtools/build/lib/packages/producers/FragmentProducer.java:90 FragmentProducer.FragmentProducer
    fn enqueue_fragment(&mut self, base: PathFragment, fragment_index: usize) {
        self.queue.push_back(Task::Fragment {
            base,
            fragment_index,
        });
    }

    fn run(&mut self) -> Result<(), ComputeError> {
        while let Some(task) = self.queue.pop_front() {
            match task {
                Task::Fragment {
                    base,
                    fragment_index,
                } => self.fragment_step(base, fragment_index)?,
                Task::DirectoryDirent {
                    dirent_path,
                    fragment_index,
                } => self.directory_dirent_step(dirent_path, fragment_index)?,
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/producers/FragmentProducer.java:115 FragmentProducer.step
    fn fragment_step(
        &mut self,
        base: PathFragment,
        fragment_index: usize,
    ) -> Result<(), ComputeError> {
        assert!(fragment_index < self.detail.pattern_fragments.len());

        let pattern_fragment = self.detail.pattern_fragments[fragment_index].clone();
        if pattern_fragment != "**" {
            return self.handle_pattern_fragment(&base, fragment_index, &pattern_fragment);
        }

        // It is valid that `**` matches nothing, which is handled in the if-else block below.
        if fragment_index < self.detail.pattern_fragments.len() - 1 {
            // In the case when `**` is not the last pattern, skip `**` and directly move onto the
            // next pattern fragment.
            let unvisited = match &mut self.visited_glob_sub_tasks {
                None => true,
                Some(visited) => visited.insert((base.clone(), fragment_index + 1)),
            };
            if unvisited {
                self.enqueue_fragment(base.clone(), fragment_index + 1);
            }
        } else {
            // In the case when `**` is the last pattern, add `base` to result when operator is
            // FILES_AND_DIRS.
            if self.detail.glob_operation == GlobberOperation::FilesAndDirs
                && base != *self.detail.package_identifier.get_package_fragment()
            {
                self.accept_path_fragment_with_package_fragment(base.clone());
            }
        }

        // Handle the case when `**` does not match an empty fragment.
        self.handle_pattern_fragment(&base, fragment_index, &pattern_fragment)
    }

    // Java: com/google/devtools/build/lib/packages/producers/FragmentProducer.java:145 FragmentProducer.handlePatternFragment
    fn handle_pattern_fragment(
        &mut self,
        base: &PathFragment,
        fragment_index: usize,
        pattern_fragment: &str,
    ) -> Result<(), ComputeError> {
        if !pattern_fragment.contains('*') && !pattern_fragment.contains('?') {
            return self.pattern_without_wildcard(base.get_child(pattern_fragment), fragment_index);
        }
        self.pattern_with_wildcard(base, fragment_index)
    }

    /// Returns if a matching path at the given pattern index should be added to the result.
    // Java: com/google/devtools/build/lib/packages/producers/FragmentProducer.java:158 FragmentProducer.shouldAddFileMatchingToResult
    fn should_add_file_matching_to_result(&self, fragment_index: usize) -> bool {
        if self.detail.glob_operation == GlobberOperation::Subpackages {
            return false;
        }
        if fragment_index < self.detail.pattern_fragments.len() - 1 {
            return false;
        }
        true
    }

    // Java: com/google/devtools/build/lib/packages/producers/PatternWithoutWildcardProducer.java:72 PatternWithoutWildcardProducer.step
    // Java: com/google/devtools/build/lib/packages/producers/PatternWithoutWildcardProducer.java:55 PatternWithoutWildcardProducer.PatternWithoutWildcardProducer
    // Java: com/google/devtools/build/lib/packages/producers/PatternWithoutWildcardProducer.java:76 PatternWithoutWildcardProducer.accept
    fn pattern_without_wildcard(
        &mut self,
        file_path: PathFragment,
        fragment_index: usize,
    ) -> Result<(), ComputeError> {
        let file_value = self.env.get_value(FileKey::key(RootedPath::to_rooted_path(
            self.detail.package_root.clone(),
            &file_path,
        )))?;
        // Java: com/google/devtools/build/lib/packages/producers/PatternWithoutWildcardProducer.java:82 PatternWithoutWildcardProducer.processFileValue
        let Some(file_value) = file_value else {
            self.values_missing = true;
            return Ok(());
        };
        if !file_value.exists() {
            // Early exit if the file does not exist.
            return Ok(());
        }

        if file_value.is_directory() {
            return self.directory_dirent(file_path, fragment_index);
        }
        if self.should_add_file_matching_to_result(fragment_index) {
            self.accept_path_fragment_with_package_fragment(file_path);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/producers/PatternWithWildcardProducer.java:81 PatternWithWildcardProducer.step
    // Java: com/google/devtools/build/lib/packages/producers/PatternWithWildcardProducer.java:76 PatternWithWildcardProducer.PatternWithWildcardProducer
    // Java: com/google/devtools/build/lib/packages/producers/PatternWithWildcardProducer.java:97 PatternWithWildcardProducer.accept
    fn pattern_with_wildcard(
        &mut self,
        base: &PathFragment,
        fragment_index: usize,
    ) -> Result<(), ComputeError> {
        let directory_listing_value =
            self.env
                .get_value(DirectoryListingKey::key(RootedPath::to_rooted_path(
                    self.detail.package_root.clone(),
                    base,
                )))?;
        // Java: com/google/devtools/build/lib/packages/producers/PatternWithWildcardProducer.java:102 PatternWithWildcardProducer.processDirectoryListingValue
        let Some(directory_listing_value) = directory_listing_value else {
            self.values_missing = true;
            return Ok(());
        };
        let pattern_fragment = self.detail.pattern_fragments[fragment_index].clone();
        let mut symlink_keys: Vec<FileKey> = Vec::new();
        for dirent in directory_listing_value.get_dirents().iter() {
            if dirent.get_type() == DirentType::Unknown {
                continue;
            }

            let dirent_name = dirent.get_name();

            // PORT: Java passes the GlobsFunction-wide regex pattern cache.
            if !unix_glob::matches(&pattern_fragment, dirent_name) {
                continue;
            }

            if dirent.get_type() == DirentType::Symlink {
                symlink_keys.push(FileKey::key(RootedPath::to_rooted_path(
                    self.detail.package_root.clone(),
                    &base.get_child(dirent_name),
                )));
            } else if dirent.get_type() == DirentType::Directory {
                self.queue.push_back(Task::DirectoryDirent {
                    dirent_path: base.get_child(dirent_name),
                    fragment_index,
                });
            } else if self.should_add_file_matching_to_result(fragment_index) {
                self.accept_path_fragment_with_package_fragment(base.get_child(dirent_name));
            }
        }

        if symlink_keys.is_empty() {
            return Ok(());
        }
        // Java: SymlinkProducer for each symlink dirent, then processSymlinks once.
        let symlinks_count = symlink_keys.len();
        let mut symlinks = Vec::with_capacity(symlinks_count);
        for symlink_key in symlink_keys {
            // Java: com/google/devtools/build/lib/packages/producers/SymlinkProducer.java:66 SymlinkProducer.accept
            // Java: com/google/devtools/build/lib/packages/producers/SymlinkProducer.java:55 SymlinkProducer.SymlinkProducer
            // Java: com/google/devtools/build/lib/packages/producers/SymlinkProducer.java:60 SymlinkProducer.step
            let Some(symlink_value) = self.env.get_value(symlink_key.clone())? else {
                self.values_missing = true;
                continue;
            };
            if !symlink_value.is_symlink() {
                // Java: com/google/devtools/build/lib/packages/producers/SymlinkProducer.java:46 SymlinkProducer.ResultSink.acceptInconsistentFilesystemException
                // Java: com/google/devtools/build/lib/packages/producers/PatternWithWildcardProducer.java:159 PatternWithWildcardProducer.acceptInconsistentFilesystemException
                self.accept_glob_error(IOException::inconsistent_filesystem(format!(
                    "readdir and stat disagree about whether {} is a symlink.",
                    symlink_key.argument().as_path()
                )));
                continue;
            }
            // Java: com/google/devtools/build/lib/packages/producers/SymlinkProducer.java:44 SymlinkProducer.ResultSink.acceptSymlinkFileValue
            // Java: com/google/devtools/build/lib/packages/producers/PatternWithWildcardProducer.java:154 PatternWithWildcardProducer.acceptSymlinkFileValue
            symlinks.push((symlink_key, symlink_value));
        }
        self.process_symlinks(symlinks, symlinks_count, fragment_index)
    }

    // Java: com/google/devtools/build/lib/packages/producers/PatternWithWildcardProducer.java:155 PatternWithWildcardProducer.processSymlinks
    fn process_symlinks(
        &mut self,
        symlinks: Vec<(FileKey, std::sync::Arc<crate::actions::FileValue>)>,
        symlinks_count: usize,
        fragment_index: usize,
    ) -> Result<(), ComputeError> {
        if symlinks.is_empty() || symlinks.len() < symlinks_count {
            // It is possible that some symlinks cannot be accepted due to inconsistent filesystem
            // error. In this case, since the `InconsistentFilesystemException` is accepted and
            // glob function computation will error out, it is unnecessary to proceed.
            return Ok(());
        }

        for (symlink_key, symlink_value) in symlinks {
            if !symlink_value.exists() {
                // Tolerate when the symlink is pointing to a non-existing path.
                continue;
            }

            // This check is more strict than necessary: we raise an error if globbing traverses
            // into a directory for any reason, even though it's only necessary if that reason
            // was the resolution of a recursive glob ("**"). Fixing this would require plumbing
            // the ancestor symlink information through DirectoryListingValue.
            if symlink_value.is_directory()
                && let Some(chain) = symlink_value.unbounded_ancestor_symlink_expansion_chain()
            {
                let _ = self
                    .env
                    .get_value(FileSymlinkInfiniteExpansionUniquenessKey::key(chain))?;
                self.accept_glob_error(IOException::file_symlink_infinite_expansion(
                    symlink_value
                        .path_to_unbounded_ancestor_symlink_expansion_chain()
                        .expect("set with the chain")
                        .to_vec(),
                    chain.to_vec(),
                ));
                return Ok(());
            }

            // Use the symlink path instead of the target path.
            let dirent_path = symlink_key.argument().get_root_relative_path().clone();
            if symlink_value.is_directory() {
                self.queue.push_back(Task::DirectoryDirent {
                    dirent_path,
                    fragment_index,
                });
            } else if self.should_add_file_matching_to_result(fragment_index) {
                self.accept_path_fragment_with_package_fragment(dirent_path);
            }
        }
        Ok(())
    }

    /// Java `new DirectoryDirentProducer(...)` returned as the next state (runs immediately).
    fn directory_dirent(
        &mut self,
        dirent_path: PathFragment,
        fragment_index: usize,
    ) -> Result<(), ComputeError> {
        self.directory_dirent_step(dirent_path, fragment_index)
    }

    // Java: com/google/devtools/build/lib/packages/producers/DirectoryDirentProducer.java:76 DirectoryDirentProducer.step
    fn directory_dirent_step(
        &mut self,
        dirent_path: PathFragment,
        fragment_index: usize,
    ) -> Result<(), ComputeError> {
        // Upstream logic should already have appended some dirent to the package fragment when
        // constructing this `direntPath`.
        assert!(dirent_path != *self.detail.package_identifier.get_package_fragment());
        // Check whether the next directory path should be ignored according to
        // IgnoredSubdirectories.
        if self
            .detail
            .ignored_subdirectories
            .matching_entry(&dirent_path)
            .is_some()
        {
            return Ok(());
        }

        let package_lookup_value =
            self.env
                .get_value(PackageLookupKey::key(PackageIdentifier::create(
                    self.detail.package_identifier.get_repository().clone(),
                    dirent_path.clone(),
                )))?;
        // Java: checkSubpackageExistence
        let Some(package_lookup_value) = package_lookup_value else {
            self.values_missing = true;
            return Ok(());
        };
        if matches!(
            *package_lookup_value,
            PackageLookupValue::IncorrectRepositoryReference { .. }
        ) {
            // Cross repository boundary, so glob expansion should not descend into that subdir.
            return Ok(());
        }

        if package_lookup_value.package_exists() {
            // Cross the package boundary. The subdir contains a BUILD file and thus defines
            // another package, so glob expansion should not descend into that subdir.
            if self.detail.glob_operation == GlobberOperation::Subpackages {
                // If this is a subpackages() call, we need to check whether this subpackage is a
                // glob matching before returning.
                if self.should_add_result(fragment_index, /* is_subpackage= */ true) {
                    self.accept_path_fragment_with_package_fragment(dirent_path);
                }
            }
            return Ok(());
        }

        self.add_results_or_create_next_fragment_producer(dirent_path, fragment_index);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/producers/DirectoryDirentProducer.java:117 DirectoryDirentProducer.addResultsOrCreateNextFragmentProducer
    fn add_results_or_create_next_fragment_producer(
        &mut self,
        dirent_path: PathFragment,
        fragment_index: usize,
    ) {
        // Even for directory dirent, we need to check whether this path is a matching result.
        if self.should_add_result(fragment_index, /* is_subpackage= */ false) {
            self.accept_path_fragment_with_package_fragment(dirent_path.clone());
        }

        let next_fragment_index = if self.detail.pattern_fragments[fragment_index] == "**" {
            fragment_index
        } else {
            fragment_index + 1
        };
        if next_fragment_index == self.detail.pattern_fragments.len() {
            // When the last glob pattern is not double star, we have already processed all
            // pattern fragments, so execution enters this block and immediately returns.
            return;
        }

        let unvisited = match &mut self.visited_glob_sub_tasks {
            None => true,
            Some(visited) => visited.insert((dirent_path.clone(), next_fragment_index)),
        };
        if unvisited {
            // Create the next unvisited `FragmentProducer` if it has not been processed.
            self.enqueue_fragment(dirent_path, next_fragment_index);
        }
    }

    /// Returns `true` if the dirent can be added as a glob matching result.
    // Java: com/google/devtools/build/lib/packages/producers/DirectoryDirentProducer.java:148 DirectoryDirentProducer.shouldAddResult
    fn should_add_result(&self, fragment_index: usize, is_subpackage: bool) -> bool {
        match self.detail.glob_operation {
            // The dirent is always a directory, so it can never match FILE operation.
            GlobberOperation::Files => false,
            GlobberOperation::Subpackages => {
                is_subpackage
                    && all_remain_pattern_fragments_double_star(
                        &self.detail.pattern_fragments,
                        fragment_index,
                    )
            }
            GlobberOperation::FilesAndDirs => {
                fragment_index == self.detail.pattern_fragments.len() - 1 && !is_subpackage
            }
        }
    }
}

// Java: com/google/devtools/build/lib/packages/producers/DirectoryDirentProducer.java:160 DirectoryDirentProducer.allRemainPatternFragmentsDoubleStar
fn all_remain_pattern_fragments_double_star(pattern_fragments: &[String], index: usize) -> bool {
    if index == pattern_fragments.len() - 1 {
        // Already covered all pattern fragments at this point, so we don't need to check
        // additional pattern fragments.
        return true;
    }
    pattern_fragments[index + 1..].iter().all(|p| p == "**")
}

/// Computes one glob and adds its matches (relative to the package) to `matchings` (Java
/// `GlobComputationProducer`). Returns whether all dependencies were available.
// Java: com/google/devtools/build/lib/packages/producers/GlobComputationProducer.java:85 GlobComputationProducer.step
fn compute_glob(
    globs_key: &GlobsKey,
    glob_request: &GlobRequest,
    ignored_subdirectories: &IgnoredSubdirectories,
    env: &mut Environment<'_>,
    matchings: &mut HashSet<PathFragment>,
    error: &mut Option<GlobError>,
) -> Result<bool, ComputeError> {
    // Java GlobDescriptor.create(packageId, packageRoot, subdir = EMPTY_FRAGMENT, ...).
    // Java: com/google/devtools/build/lib/skyframe/GlobDescriptor.java:50 GlobDescriptor.create
    // (its getters are GlobDetail's fields; GlobsFunction never makes it a Skyframe key)
    // PORT: skipped, GlobsFunction uses the descriptor as a value, not a key (no interning, equality or key name): GlobDescriptor.intern, GlobDescriptor.equals, GlobDescriptor.hashCode
    // PORT: skipped, GlobsFunction uses the descriptor as a value, not a key (no interning, equality or key name): GlobDescriptor.functionName, GlobDescriptor.getSkyKeyInterner, GlobDescriptor.toString
    // PORT: skipped, the fields of GlobDetail: GlobDescriptor.getPackageId, GlobDescriptor.getPackageRoot, GlobDescriptor.getSubdir, GlobDescriptor.getPattern, GlobDescriptor.globberOperation
    let patterns: Vec<String> = glob_request
        .get_pattern()
        .split('/')
        .map(str::to_owned)
        .collect();
    let contains_multiple_double_stars = patterns.iter().filter(|p| *p == "**").count() > 1;
    // Java: com/google/devtools/build/lib/packages/producers/GlobComputationProducer.java:174 GlobComputationProducer.GlobDetail.create
    // Java: com/google/devtools/build/lib/packages/producers/GlobComputationProducer.java:82 GlobComputationProducer.GlobComputationProducer
    let detail = GlobDetail {
        package_identifier: &globs_key.package_identifier,
        package_root: &globs_key.package_root,
        pattern_fragments: patterns,
        contains_multiple_double_stars,
        ignored_subdirectories,
        glob_operation: glob_request.get_glob_operation(),
    };
    let package_fragment = globs_key.package_identifier.get_package_fragment().clone();
    let mut run = GlobRun {
        visited_glob_sub_tasks: detail.contains_multiple_double_stars.then(HashSet::new),
        detail,
        env,
        queue: VecDeque::new(),
        path_fragments_with_package_fragment: HashSet::new(),
        error: None,
        values_missing: false,
    };
    run.enqueue_fragment(
        package_fragment.get_relative(&PathFragment::empty_fragment()),
        0,
    );
    run.run()?;

    // Java: ResultSink.acceptGlobError keeps the first error of all globs.
    // Java: com/google/devtools/build/lib/packages/producers/GlobComputationProducer.java:70 GlobComputationProducer.ResultSink.acceptGlobError
    // Java: com/google/devtools/build/lib/skyframe/GlobsFunction.java:97 GlobsFunction.State.acceptGlobError
    if error.is_none() {
        *error = run.error.take();
    }
    // Java: buildAndBubbleUpResult. Removes the package fragment from all accepted matching path
    // fragments; matchings are ignored once an error was accepted.
    // Java: com/google/devtools/build/lib/packages/producers/GlobComputationProducer.java:139 GlobComputationProducer.buildAndBubbleUpResult
    // Java: com/google/devtools/build/lib/packages/producers/GlobComputationProducer.java:68 GlobComputationProducer.ResultSink.acceptPathFragmentsWithoutPackageFragment
    // Java: com/google/devtools/build/lib/skyframe/GlobsFunction.java:87 GlobsFunction.State.acceptPathFragmentsWithoutPackageFragment
    if error.is_none() {
        for p in &run.path_fragments_with_package_fragment {
            matchings.insert(p.relative_to(&package_fragment));
        }
    }
    Ok(!run.values_missing)
}

/// GLOBS (Java `GlobsFunction.compute`).
// Java: com/google/devtools/build/lib/skyframe/GlobsFunction.java:107 GlobsFunction.compute
pub fn globs_function(
    globs_key: &GlobsKey,
    env: &mut Environment<'_>,
) -> Result<Option<GlobsValue>, ComputeError> {
    let repository_name = globs_key.get_package_identifier().get_repository().clone();
    let ignored_subdirectories = env.get_value(IgnoredSubdirectoriesKey::key(repository_name))?;
    if env.values_missing() {
        return Ok(None);
    }
    let Some(ignored_subdirectories) = ignored_subdirectories else {
        return Ok(None);
    };
    let ignored_subdirectories = ignored_subdirectories.as_ignored_subdirectories();

    let mut matchings = HashSet::new();
    let mut error = None;
    let mut all_complete = true;
    for glob_request in globs_key.get_glob_requests() {
        // TODO(b/290998109): Support non-empty subdir when replacing Glob with Globs in
        // IncludeParser.
        if !compute_glob(
            globs_key,
            glob_request,
            ignored_subdirectories,
            env,
            &mut matchings,
            &mut error,
        )? {
            all_complete = false;
        }
    }

    if !all_complete {
        handle_exceptions(error)?;
        return Ok(None);
    }

    handle_exceptions(error)?;
    Ok(Some(GlobsValue::new(matchings)))
}
