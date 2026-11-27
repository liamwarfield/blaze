//! RECURSIVE_PKG (the packages beneath a directory), PACKAGE_ERROR_MESSAGE and PACKAGE_ERROR.
//!
//! Java: com/google/devtools/build/lib/skyframe/{RecursivePkgFunction,RecursivePkgValue,
//! RecursivePkgKey,RecursiveDirectoryTraversalFunction,ProcessPackageDirectory,
//! ProcessPackageDirectoryResult,PackageErrorMessageFunction,PackageErrorMessageValue,
//! PackageErrorFunction}.java and com/google/devtools/build/lib/io/ProcessPackageDirectoryException.java.

use std::fmt;

use blaze_cmdline::label_constants::EXTERNAL_PACKAGE_NAME;
use blaze_cmdline::{PackageIdentifier, RepositoryName};
use blaze_events::Event;
use blaze_packages::exceptions::{DetailedExitCode, NoSuchPackageException, PackageLoadingCode};
use blaze_skyframe::{
    ComputeError, Environment, FunctionHermeticity, Key, SkyFunctionException, SkyFunctionName,
    SkyKey,
};
use blaze_vfs::{DirentType, PathFragment, Root, RootedPath};

use crate::directories::LoadingDirectories;
use crate::directory_listing::{DirectoryListingKey, Dirents};
use crate::file::FileKey;
use crate::file_chain_uniqueness::FileSymlinkInfiniteExpansionUniquenessKey;
use crate::ignored_subdirectories::IgnoredSubdirectories;
use crate::io::IOException;
use crate::package_function::PackageKey;
use crate::package_lookup::PackageLookupKey;
use crate::precomputed::STARLARK_SEMANTICS;

/// Function name of RECURSIVE_PKG.
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:94 SkyFunctions.RECURSIVE_PKG
pub const RECURSIVE_PKG: SkyFunctionName = SkyFunctionName("RECURSIVE_PKG");
/// Function name of PACKAGE_ERROR.
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:59 SkyFunctions.PACKAGE_ERROR
pub const PACKAGE_ERROR: SkyFunctionName = SkyFunctionName("PACKAGE_ERROR");
/// Function name of PACKAGE_ERROR_MESSAGE.
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:60 SkyFunctions.PACKAGE_ERROR_MESSAGE
pub const PACKAGE_ERROR_MESSAGE: SkyFunctionName = SkyFunctionName("PACKAGE_ERROR_MESSAGE");

/// A directory whose processing hit a filesystem inconsistency.
// Java: com/google/devtools/build/lib/io/ProcessPackageDirectoryException.java:27 ProcessPackageDirectoryException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ProcessPackageDirectoryException {
    message: String,
}

impl ProcessPackageDirectoryException {
    // Java: com/google/devtools/build/lib/io/ProcessPackageDirectoryException.java:31 ProcessPackageDirectoryException.<init>
    pub fn new(directory: &RootedPath, e: &IOException) -> Self {
        ProcessPackageDirectoryException {
            message: format!(
                "Directory '{}' could not be processed: {}",
                directory.as_path().get_path_string(),
                e.get_message()
            ),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/io/ProcessPackageDirectoryException.java:46 ProcessPackageDirectoryException.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> DetailedExitCode {
        DetailedExitCode::package_loading(
            self.message.clone(),
            PackageLoadingCode::TransientInconsistentFilesystemError,
        )
    }
}

/// The key of RECURSIVE_PKG: a directory, its repository and the ignored paths beneath it.
// Java: com/google/devtools/build/lib/skyframe/RecursivePkgKey.java:28 RecursivePkgKey
// Java: com/google/devtools/build/lib/skyframe/RecursivePkgKey.java:67 RecursivePkgKey.equals
// Java: com/google/devtools/build/lib/skyframe/RecursivePkgKey.java:81 RecursivePkgKey.hashCode
// Java: com/google/devtools/build/lib/skyframe/RecursivePkgSkyKey.java:34 RecursivePkgSkyKey.equals
// Java: com/google/devtools/build/lib/skyframe/RecursivePkgSkyKey.java:42 RecursivePkgSkyKey.hashCode
// PORT: the function name of a RecursivePkgSkyKey is its Rust key type (`Key::FUNCTION`), which
// the evaluator compares with the key; the fields are compared by the derived `PartialEq`.
// Java: com/google/devtools/build/lib/skyframe/RecursivePkgSkyKey.java:24 RecursivePkgSkyKey.<init>
// Java: com/google/devtools/build/lib/skyframe/RecursivePkgValue.java:77 RecursivePkgValue.Key.<init>
// PORT: skipped, the key is a plain struct key (Key::FUNCTION; no interning): RecursivePkgValue.Key.create, RecursivePkgValue.Key.intern, RecursivePkgValue.Key.functionName, RecursivePkgValue.Key.getSkyKeyInterner
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RecursivePkgKey {
    repository_name: RepositoryName,
    rooted_path: RootedPath,
    excluded_paths: IgnoredSubdirectories,
}

impl RecursivePkgKey {
    // Java: com/google/devtools/build/lib/skyframe/RecursivePkgValue.java:59 RecursivePkgValue.key
    pub fn key(
        repository_name: RepositoryName,
        rooted_path: RootedPath,
        excluded_paths: IgnoredSubdirectories,
    ) -> Self {
        RecursivePkgKey {
            repository_name,
            rooted_path,
            excluded_paths,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePkgKey.java:50 RecursivePkgKey.getRepositoryName
    pub fn get_repository_name(&self) -> &RepositoryName {
        &self.repository_name
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePkgKey.java:54 RecursivePkgKey.getRootedPath
    pub fn get_rooted_path(&self) -> &RootedPath {
        &self.rooted_path
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePkgKey.java:58 RecursivePkgKey.getExcludedPaths
    pub fn get_excluded_paths(&self) -> &IgnoredSubdirectories {
        &self.excluded_paths
    }
}

impl RecursivePkgKey {
    /// Java `RecursivePkgKey.toString` (the plain key, without a function name).
    // Java: com/google/devtools/build/lib/skyframe/RecursivePkgKey.java:63 RecursivePkgKey.toString
    pub fn to_key_string(&self) -> String {
        format!("rootedPath={}, excludedPaths=<omitted>", self.rooted_path)
    }
}

/// As a SkyKey: Java `RecursivePkgSkyKey.toString`, the function name and the key.
// Java: com/google/devtools/build/lib/skyframe/RecursivePkgSkyKey.java:29 RecursivePkgSkyKey.toString
impl fmt::Debug for RecursivePkgKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", RECURSIVE_PKG, self.to_key_string())
    }
}

impl Key for RecursivePkgKey {
    type Value = RecursivePkgValue;
    const FUNCTION: SkyFunctionName = RECURSIVE_PKG;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:94 SkyFunctions.RECURSIVE_PKG (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: `RecursivePkgValue` has no `equals`; `create` returns the singleton `EMPTY` for no
    /// packages and no errors, and a new object otherwise. So only two empty, error-free values are
    /// equal.
    // Java: com/google/devtools/build/lib/skyframe/RecursivePkgValue.java:51 RecursivePkgValue.create
    fn values_equal(old: &RecursivePkgValue, new: &RecursivePkgValue) -> bool {
        let is_empty = |v: &RecursivePkgValue| v.get_packages().is_empty() && !v.has_errors();
        is_empty(old) && is_empty(new)
    }
}

/// The packages beneath a directory (Java `RecursivePkgValue`). `packages` is Java's
/// `NestedSet<String>` in `STABLE_ORDER`, flattened: each child's packages (in child order)
/// before the directory's own package, duplicates removed keeping the first.
// Java: com/google/devtools/build/lib/skyframe/RecursivePkgValue.java:35 RecursivePkgValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecursivePkgValue {
    packages: Vec<String>,
    has_errors: bool,
}

impl RecursivePkgValue {
    /// A value with these fields (tests).
    #[cfg(test)]
    pub(crate) fn for_testing(packages: Vec<String>, has_errors: bool) -> Self {
        RecursivePkgValue {
            packages,
            has_errors,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePkgValue.java:64 RecursivePkgValue.getPackages
    pub fn get_packages(&self) -> &[String] {
        &self.packages
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePkgValue.java:68 RecursivePkgValue.hasErrors
    pub fn has_errors(&self) -> bool {
        self.has_errors
    }
}

/// The result of processing a directory (Java `ProcessPackageDirectoryResult`).
// Java: com/google/devtools/build/lib/skyframe/ProcessPackageDirectoryResult.java:27 ProcessPackageDirectoryResult
pub(crate) struct ProcessPackageDirectoryResult {
    pub(crate) package_exists: bool,
    /// Java `getChildDeps()`: the subdirectories to visit, as the RECURSIVE_PKG keys whose fields
    /// are Java's `getSkyKeyForSubdirectory` arguments (callers map them to their own keys).
    pub(crate) child_deps: Vec<RecursivePkgKey>,
}

impl ProcessPackageDirectoryResult {
    // Java: com/google/devtools/build/lib/skyframe/ProcessPackageDirectoryResult.java:33 ProcessPackageDirectoryResult.EMPTY_RESULT
    fn empty_result() -> Self {
        ProcessPackageDirectoryResult {
            package_exists: false,
            child_deps: Vec::new(),
        }
    }
}

const SENTINEL_FILE_NAME_FOR_NOT_TRAVERSING_SYMLINKS: &str =
    "DONT_FOLLOW_SYMLINKS_WHEN_TRAVERSING_THIS_DIRECTORY_VIA_A_RECURSIVE_TARGET_PATTERN";

/// Computes whether a directory is a package and which subdirectories to visit (Java
/// `ProcessPackageDirectory`, with RECURSIVE_PKG's key transformer).
// Java: com/google/devtools/build/lib/skyframe/ProcessPackageDirectory.java:56 ProcessPackageDirectory
pub(crate) struct ProcessPackageDirectory<'a> {
    pub(crate) directories: &'a LoadingDirectories,
}

/// A processing outcome: a result, or values missing.
pub(crate) type ProcessResult = Result<Option<ProcessPackageDirectoryResult>, ComputeError>;

impl ProcessPackageDirectory<'_> {
    // Java: com/google/devtools/build/lib/skyframe/ProcessPackageDirectory.java:83 ProcessPackageDirectory.getPackageExistenceAndSubdirDeps
    pub(crate) fn get_package_existence_and_subdir_deps(
        &self,
        rooted_path: &RootedPath,
        repository_name: &RepositoryName,
        excluded_paths: &IgnoredSubdirectories,
        env: &mut Environment<'_>,
    ) -> ProcessResult {
        let root_relative_path = rooted_path.get_root_relative_path().clone();
        let file_value = match env
            .get_value_or_throw::<FileKey, IOException>(FileKey::key(rooted_path.clone()))?
        {
            Err(e) if e.is_inconsistent_filesystem() => {
                return Err(process_package_directory_sky_function_exception(
                    rooted_path,
                    &e,
                ));
            }
            Err(e) => {
                return Ok(Some(report_error_and_return(
                    "Failed to get information about path",
                    e.get_message(),
                    &root_relative_path,
                    env,
                )));
            }
            Ok(v) => v,
        };
        if env.values_missing() {
            return Ok(None);
        }
        let Some(file_value) = file_value else {
            return Ok(None);
        };
        if !file_value.is_directory() {
            return Ok(Some(ProcessPackageDirectoryResult::empty_result()));
        }
        if let Some(chain) = file_value.unbounded_ancestor_symlink_expansion_chain() {
            env.get_value(FileSymlinkInfiniteExpansionUniquenessKey::key(chain))?;
            if env.values_missing() {
                return Ok(None);
            }
            let symlink_exception = IOException::file_symlink_infinite_expansion(
                file_value
                    .path_to_unbounded_ancestor_symlink_expansion_chain()
                    .expect("set with the chain")
                    .to_vec(),
                chain.to_vec(),
            );
            return Ok(Some(report_error_and_return(
                symlink_exception.get_message(),
                symlink_exception.get_message(),
                &root_relative_path,
                env,
            )));
        }

        let package_id =
            PackageIdentifier::create(repository_name.clone(), root_relative_path.clone());
        if package_id.get_repository().is_main()
            && self.is_convenience_symlink(&file_value, rooted_path, env)?
        {
            return Ok(Some(ProcessPackageDirectoryResult::empty_result()));
        }
        if env.values_missing() {
            return Ok(None);
        }

        let pkg_lookup_key = PackageLookupKey::key(package_id);
        let dir_listing_key = DirectoryListingKey::key(rooted_path.clone());
        let lookups = env.get_values_and_exceptions(&[
            SkyKey::new(pkg_lookup_key.clone()),
            SkyKey::new(dir_listing_key.clone()),
        ])?;
        enum Caught {
            NoSuchPackage(NoSuchPackageException),
            InconsistentFilesystem(IOException),
        }
        let pkg_lookup_value = match lookups.get_or_throw_with(&pkg_lookup_key, |e| {
            if let Some(e) = e.downcast_ref::<NoSuchPackageException>() {
                Some(Caught::NoSuchPackage(e.clone()))
            } else {
                e.downcast_ref::<IOException>()
                    .filter(|e| e.is_inconsistent_filesystem())
                    .map(|e| Caught::InconsistentFilesystem(e.clone()))
            }
        }) {
            Err(Caught::NoSuchPackage(e)) => {
                return Ok(Some(report_error_and_return(
                    "Failed to load package",
                    &e.get_message(),
                    &root_relative_path,
                    env,
                )));
            }
            Err(Caught::InconsistentFilesystem(e)) => {
                return Err(process_package_directory_sky_function_exception(
                    rooted_path,
                    &e,
                ));
            }
            Ok(v) => v,
        };
        let dir_listing_value = match lookups
            .get_or_throw_typed::<DirectoryListingKey, IOException>(&dir_listing_key)
        {
            Err(e) if e.is_file_symlink_exception() => {
                // DirectoryListingFunction only throws FileSymlinkCycleException when FileFunction
                // throws it, but FileFunction was evaluated for rootedPath above, and didn't throw
                // there. It shouldn't be able to avoid throwing there but throw here.
                panic!(
                    "IllegalStateException: Symlink cycle found after not being found for \"{}\": {}",
                    rooted_path, e
                );
            }
            Err(e) => {
                return Ok(Some(report_error_and_return(
                    "Failed to list directory contents",
                    e.get_message(),
                    &root_relative_path,
                    env,
                )));
            }
            Ok(v) => v,
        };
        let starlark_semantics = STARLARK_SEMANTICS.get(env)?;
        if env.values_missing() {
            return Ok(None);
        }
        let (Some(pkg_lookup_value), Some(dir_listing_value), Some(starlark_semantics)) =
            (pkg_lookup_value, dir_listing_value, starlark_semantics)
        else {
            return Ok(None);
        };
        Ok(Some(ProcessPackageDirectoryResult {
            package_exists: pkg_lookup_value.package_exists()
                && pkg_lookup_value.get_root() == rooted_path.get_root(),
            child_deps: get_subdir_deps(
                dir_listing_value.get_dirents(),
                rooted_path,
                repository_name,
                excluded_paths,
                starlark_semantics.experimental_sibling_repository_layout,
            ),
        }))
    }

    // Java: com/google/devtools/build/lib/skyframe/ProcessPackageDirectory.java:191 ProcessPackageDirectory.isConvenienceSymlink
    fn is_convenience_symlink(
        &self,
        file_value: &crate::actions::FileValue,
        rooted_path: &RootedPath,
        env: &mut Environment<'_>,
    ) -> Result<bool, blaze_skyframe::Interrupted> {
        if !file_value.is_symlink() {
            return Ok(false);
        }
        let link_target = file_value.get_unresolved_link_target();
        // Java BlazeDirectories.getExecRootBase(): <output_base>/execroot.
        let exec_root_base = self.directories.get_output_base().get_child("execroot");
        if link_target.starts_with(exec_root_base.as_fragment()) {
            return Ok(true);
        }
        let root_relative_path = rooted_path.get_root_relative_path();
        let root = rooted_path.get_root();
        let base_name = root_relative_path.get_base_name();
        if base_name.ends_with("-bin") && is_in_exec_root(link_target, root, 4, env)? {
            return Ok(true);
        }
        if base_name.ends_with("-genfiles") && is_in_exec_root(link_target, root, 4, env)? {
            return Ok(true);
        }
        if base_name.ends_with("-out") && is_in_exec_root(link_target, root, 2, env)? {
            return Ok(true);
        }
        if base_name.ends_with("-testlogs") && is_in_exec_root(link_target, root, 4, env)? {
            return Ok(true);
        }
        // Java BlazeDirectories.getWorkingDirectory(): the workspace.
        if base_name.ends_with(&format!(
            "-{}",
            self.directories.get_workspace().get_base_name()
        )) && is_in_exec_root(link_target, root, 1, env)?
        {
            return Ok(true);
        }
        Ok(false)
    }
}

// Java: com/google/devtools/build/lib/skyframe/ProcessPackageDirectory.java:238 ProcessPackageDirectory.isInExecRoot
fn is_in_exec_root(
    path: &PathFragment,
    root: &Root,
    depth: usize,
    env: &mut Environment<'_>,
) -> Result<bool, blaze_skyframe::Interrupted> {
    let segment_count = path.segment_count();
    if segment_count <= depth {
        return Ok(false);
    }
    let candidate_exec_root = path.sub_fragment(0, segment_count - depth);
    if candidate_exec_root.get_base_name() != "execroot" {
        return Ok(false);
    }
    let absolute_root = Root::absolute_root(root.get_file_system());
    let do_not_build_path = RootedPath::to_rooted_path(
        absolute_root,
        &candidate_exec_root.get_child("DO_NOT_BUILD_HERE"),
    );
    let Some(do_not_build_value) = env.get_value(FileKey::key(do_not_build_path))? else {
        return Ok(false);
    };
    Ok(do_not_build_value.exists())
}

// Java: com/google/devtools/build/lib/skyframe/ProcessPackageDirectory.java:256 ProcessPackageDirectory.getSubdirDeps
fn get_subdir_deps(
    dirents: &Dirents,
    rooted_path: &RootedPath,
    repository_name: &RepositoryName,
    excluded_paths: &IgnoredSubdirectories,
    sibling_repository_layout: bool,
) -> Vec<RecursivePkgKey> {
    let root = rooted_path.get_root();
    let root_relative_path = rooted_path.get_root_relative_path();
    let follow_symlinks = should_follow_symlinks_when_traversing(dirents);
    let mut child_deps = Vec::new();
    for dirent in dirents.iter() {
        let type_ = dirent.get_type();
        if type_ != DirentType::Directory && (type_ != DirentType::Symlink || !follow_symlinks) {
            // Non-directories can never host packages. Symlinks to non-directories are weeded
            // out at the next level of recursion when we check if its FileValue is a directory.
            continue;
        }
        let basename = dirent.get_name();
        let subdirectory = root_relative_path.get_relative_str(basename);
        if !sibling_repository_layout
            && subdirectory == *EXTERNAL_PACKAGE_NAME
            && repository_name.is_main()
        {
            // Subpackages under //external in the main repo can be processed only when
            // --experimental_sibling_repository_layout is set.
            continue;
        }
        // If this subdirectory is one of the excluded paths, don't recurse into it.
        if excluded_paths.matching_entry(&subdirectory).is_some() {
            continue;
        }
        child_deps.push(RecursivePkgKey::key(
            repository_name.clone(),
            RootedPath::to_rooted_path(root.clone(), &subdirectory),
            excluded_paths.filter_for_directory(&subdirectory),
        ));
    }
    child_deps
}

// Java: com/google/devtools/build/lib/skyframe/ProcessPackageDirectory.java:306 ProcessPackageDirectory.reportErrorAndReturn
fn report_error_and_return(
    error_prefix: &str,
    message: &str,
    root_relative_path: &PathFragment,
    env: &Environment<'_>,
) -> ProcessPackageDirectoryResult {
    env.listener().handle(&Event::error(format!(
        "{}, for {}, skipping: {}",
        error_prefix, root_relative_path, message
    )));
    ProcessPackageDirectoryResult::empty_result()
}

// Java: com/google/devtools/build/lib/skyframe/ProcessPackageDirectory.java:313 ProcessPackageDirectory.shouldFollowSymlinksWhenTraversing
fn should_follow_symlinks_when_traversing(dirents: &Dirents) -> bool {
    // This is a special sentinel file whose existence tells Blaze not to follow symlinks when
    // recursively traversing through this directory.
    !dirents
        .iter()
        .any(|d| d.get_name() == SENTINEL_FILE_NAME_FOR_NOT_TRAVERSING_SYMLINKS)
}

// Java: com/google/devtools/build/lib/skyframe/ProcessPackageDirectory.java:332 ProcessPackageDirectory.ProcessPackageDirectorySkyFunctionException
fn process_package_directory_sky_function_exception(
    directory: &RootedPath,
    e: &IOException,
) -> ComputeError {
    SkyFunctionException::catastrophic(
        ProcessPackageDirectoryException::new(directory, e),
        blaze_skyframe::Transience::Persistent,
    )
    .into()
}

/// RECURSIVE_PKG (Java `RecursivePkgFunction` over `RecursiveDirectoryTraversalFunction`).
// Java: com/google/devtools/build/lib/skyframe/RecursivePkgFunction.java:41 RecursivePkgFunction
pub struct RecursivePkgFunction {
    directories: LoadingDirectories,
}

impl RecursivePkgFunction {
    pub fn new(directories: LoadingDirectories) -> Self {
        RecursivePkgFunction { directories }
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePkgFunction.java:47 RecursivePkgFunction.compute
    // Java: com/google/devtools/build/lib/skyframe/RecursivePkgFunction.java:56 RecursivePkgFunction.MyTraversalFunction.<init>
    // Java: com/google/devtools/build/lib/skyframe/RecursiveDirectoryTraversalFunction.java:53 RecursiveDirectoryTraversalFunction.<init>
    // Java: com/google/devtools/build/lib/skyframe/RecursiveDirectoryTraversalFunction.java:58 RecursiveDirectoryTraversalFunction.getProcessPackageDirectoryResult
    // Java: com/google/devtools/build/lib/skyframe/RecursivePkgFunction.java:60 RecursivePkgFunction.MyTraversalFunction.getInitialConsumer
    // Java: com/google/devtools/build/lib/skyframe/RecursiveDirectoryTraversalFunction.java:77 RecursiveDirectoryTraversalFunction.getInitialConsumer
    // PORT: the subdirectory keys are RecursivePkgKeys (`get_subdir_deps`); this function and
    // COLLECT_PACKAGES_UNDER_DIRECTORY wrap them in their own key types.
    // Java: com/google/devtools/build/lib/skyframe/RecursivePkgFunction.java:65 RecursivePkgFunction.MyTraversalFunction.getSkyKeyForSubdirectory
    // Java: com/google/devtools/build/lib/skyframe/RecursiveDirectoryTraversalFunction.java:85 RecursiveDirectoryTraversalFunction.getSkyKeyForSubdirectory
    // Java: com/google/devtools/build/lib/skyframe/RecursiveDirectoryTraversalFunction.java:93 RecursiveDirectoryTraversalFunction.visitDirectory
    pub fn compute(
        &self,
        recursive_pkg_key: &RecursivePkgKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<RecursivePkgValue>, ComputeError> {
        let process_package_directory_result = ProcessPackageDirectory {
            directories: &self.directories,
        }
        .get_package_existence_and_subdir_deps(
            recursive_pkg_key.get_rooted_path(),
            recursive_pkg_key.get_repository_name(),
            recursive_pkg_key.get_excluded_paths(),
            env,
        )?;
        if env.values_missing() {
            return Ok(None);
        }
        let Some(process_package_directory_result) = process_package_directory_result else {
            return Ok(None);
        };
        let child_deps = &process_package_directory_result.child_deps;
        // Java MyPackageDirectoryConsumer: NestedSetBuilder<String>(STABLE_ORDER) + hasErrors.
        let mut own_package: Option<String> = None;
        let mut has_errors = false;
        let child_sky_keys: Vec<SkyKey> = child_deps.iter().cloned().map(SkyKey::new).collect();
        let dependent_sky_values;
        if process_package_directory_result.package_exists {
            let root_relative_path = recursive_pkg_key
                .get_rooted_path()
                .get_root_relative_path()
                .clone();
            let package_error_message_key = PackageErrorMessageKey(PackageIdentifier::create(
                recursive_pkg_key.get_repository_name().clone(),
                root_relative_path.clone(),
            ));
            // In a no-keep-going build during error bubbling, PackageErrorMessageFunction may
            // throw a NoSuchPackageException. Since we don't catch such an exception here, this
            // SkyFunction will return immediately with a missing value, and the
            // NoSuchPackageException will propagate up.
            let mut keys = vec![SkyKey::new(package_error_message_key.clone())];
            keys.extend(child_sky_keys.iter().cloned());
            dependent_sky_values = env.get_values_and_exceptions(&keys)?;
            if env.values_missing() {
                return Ok(None);
            }
            let Some(pkg_error_message_value) =
                dependent_sky_values.get_typed(&package_error_message_key)
            else {
                return Ok(None);
            };
            match &*pkg_error_message_value {
                // Java: com/google/devtools/build/lib/skyframe/RecursivePkgFunction.java:95 RecursivePkgFunction.MyPackageDirectoryConsumer.notePackage
                // Java: com/google/devtools/build/lib/skyframe/RecursiveDirectoryTraversalFunction.java:107 RecursiveDirectoryTraversalFunction.PackageDirectoryConsumer.notePackage
                PackageErrorMessageValue::NoError => {
                    own_package = Some(root_relative_path.get_path_string().to_owned());
                }
                PackageErrorMessageValue::Error => {
                    env.listener().handle(&Event::error(format!(
                        "package contains errors: {}",
                        root_relative_path.get_path_string()
                    )));
                    own_package = Some(root_relative_path.get_path_string().to_owned());
                }
                PackageErrorMessageValue::NoSuchPackageException(msg) => {
                    // The package had errors, but don't fail-fast as there might be subpackages
                    // below the current directory.
                    env.listener().handle(&Event::error(msg.clone()));
                    // Java: com/google/devtools/build/lib/skyframe/RecursivePkgFunction.java:100 RecursivePkgFunction.MyPackageDirectoryConsumer.notePackageError
                    // Java: com/google/devtools/build/lib/skyframe/RecursiveDirectoryTraversalFunction.java:119 RecursiveDirectoryTraversalFunction.PackageDirectoryConsumer.notePackageError
                    has_errors = true;
                }
            }
        } else {
            dependent_sky_values = env.get_values_and_exceptions(&child_sky_keys)?;
            if env.values_missing() {
                return Ok(None);
            }
        }

        // Java aggregateWithSubdirectorySkyValues: the transitive packages of each child, in
        // order, then the directory's own package (NestedSet STABLE_ORDER flattening).
        // Java: com/google/devtools/build/lib/skyframe/RecursivePkgFunction.java:74 RecursivePkgFunction.MyTraversalFunction.aggregateWithSubdirectorySkyValues
        // Java: com/google/devtools/build/lib/skyframe/RecursiveDirectoryTraversalFunction.java:95 RecursiveDirectoryTraversalFunction.aggregateWithSubdirectorySkyValues
        // Java: com/google/devtools/build/lib/skyframe/RecursivePkgFunction.java:105 RecursivePkgFunction.MyPackageDirectoryConsumer.addTransitivePackages
        // Java: com/google/devtools/build/lib/skyframe/RecursivePkgFunction.java:109 RecursivePkgFunction.MyPackageDirectoryConsumer.addTransitiveErrors
        // Java: com/google/devtools/build/lib/skyframe/RecursivePkgFunction.java:113 RecursivePkgFunction.MyPackageDirectoryConsumer.createRecursivePkgValue
        let mut packages: indexmap::IndexSet<String> = indexmap::IndexSet::new();
        for key in child_deps {
            let Some(child_value) = dependent_sky_values.get_typed(key) else {
                return Ok(None);
            };
            packages.extend(child_value.get_packages().iter().cloned());
            if child_value.has_errors() {
                has_errors = true;
            }
        }
        if let Some(own) = own_package {
            packages.insert(own);
        }
        Ok(Some(RecursivePkgValue {
            packages: packages.into_iter().collect(),
            has_errors,
        }))
    }
}

/// The key of PACKAGE_ERROR_MESSAGE. `Debug` prints `PACKAGE_ERROR_MESSAGE:<pkg>`.
// Java: com/google/devtools/build/lib/skyframe/PackageErrorMessageValue.java:74 PackageErrorMessageValue.key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PackageErrorMessageKey(pub PackageIdentifier);

impl fmt::Debug for PackageErrorMessageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", Self::FUNCTION, self.0)
    }
}

impl Key for PackageErrorMessageKey {
    type Value = PackageErrorMessageValue;
    const FUNCTION: SkyFunctionName = PACKAGE_ERROR_MESSAGE;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:61 SkyFunctions.PACKAGE_ERROR_MESSAGE (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: the `NO_ERROR_VALUE` and `ERROR_VALUE` singletons, and the exception message.
    // Java: com/google/devtools/build/lib/skyframe/PackageErrorMessageValue.java:154 PackageErrorMessageValue.NoSuchPackageExceptionValue.equals
    fn values_equal(old: &PackageErrorMessageValue, new: &PackageErrorMessageValue) -> bool {
        old == new
    }
}

/// Whether a package loaded, loaded with errors, or failed (Java `PackageErrorMessageValue`).
// Java: com/google/devtools/build/lib/skyframe/PackageErrorMessageValue.java:30 PackageErrorMessageValue
// The variants are the values of ofPackageWithNoErrors, ofPackageWithErrors and
// ofNoSuchPackageException; getResult is the variant and getNoSuchPackageExceptionMessage the field.
// Java: com/google/devtools/build/lib/skyframe/PackageErrorMessageValue.java:62 PackageErrorMessageValue.ofPackageWithNoErrors
// Java: com/google/devtools/build/lib/skyframe/PackageErrorMessageValue.java:66 PackageErrorMessageValue.ofPackageWithErrors
// Java: com/google/devtools/build/lib/skyframe/PackageErrorMessageValue.java:70 PackageErrorMessageValue.ofNoSuchPackageException
// Java: com/google/devtools/build/lib/skyframe/PackageErrorMessageValue.java:139 PackageErrorMessageValue.NoSuchPackageExceptionValue.<init>
// PORT: skipped, the variant and its field (see above): PackageErrorMessageValue.getResult, PackageErrorMessageValue.getNoSuchPackageExceptionMessage
// PORT: skipped, derived PartialEq; the value is not hashed: PackageErrorMessageValue.hashCode
// PORT: skipped, the key is a newtype key (Key::FUNCTION; no interning): PackageErrorMessageValue.create, PackageErrorMessageValue.intern, PackageErrorMessageValue.functionName, PackageErrorMessageValue.getSkyKeyInterner
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageErrorMessageValue {
    NoError,
    Error,
    /// The message of the package's `NoSuchPackageException`.
    NoSuchPackageException(String),
}

/// PACKAGE_ERROR_MESSAGE (Java `PackageErrorMessageFunction`).
// Java: com/google/devtools/build/lib/skyframe/PackageErrorMessageFunction.java:30 PackageErrorMessageFunction.compute
pub fn package_error_message_function(
    key: &PackageErrorMessageKey,
    env: &mut Environment<'_>,
) -> Result<Option<PackageErrorMessageValue>, ComputeError> {
    let pkg_value = match env
        .get_value_or_throw::<PackageKey, NoSuchPackageException>(PackageKey::key(key.0.clone()))?
    {
        // Note that in a no-keep-going build, this returned value is ignored by Skyframe, and
        // the NoSuchPackageException will be propagated up to the caller.
        Err(e) => {
            return Ok(Some(PackageErrorMessageValue::NoSuchPackageException(
                e.get_message(),
            )));
        }
        Ok(v) => v,
    };
    let Some(pkg_value) = pkg_value else {
        return Ok(None);
    };
    Ok(Some(if pkg_value.get_package().contains_errors() {
        PackageErrorMessageValue::Error
    } else {
        PackageErrorMessageValue::NoError
    }))
}

/// The key of PACKAGE_ERROR. `Debug` prints `PACKAGE_ERROR:<pkg>`.
// Java: com/google/devtools/build/lib/skyframe/PackageErrorFunction.java:45 PackageErrorFunction.key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PackageErrorKey(pub PackageIdentifier);

impl fmt::Debug for PackageErrorKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", Self::FUNCTION, self.0)
    }
}

/// PACKAGE_ERROR never has a value: it always throws.
#[derive(Debug)]
pub struct PackageErrorValue;

impl Key for PackageErrorKey {
    type Value = PackageErrorValue;
    const FUNCTION: SkyFunctionName = PACKAGE_ERROR;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:59 SkyFunctions.PACKAGE_ERROR (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Default `values_equal`: Java's function always throws; there is no value.
    // Java: com/google/devtools/build/lib/skyframe/PackageErrorFunction.java:78 PackageErrorFunction.compute
}

/// PACKAGE_ERROR: throws `BuildFileContainsErrorsException` for a package in error, so that a
/// no-keep-going evaluation stops (Java `PackageErrorFunction`).
// Java: com/google/devtools/build/lib/skyframe/PackageErrorFunction.java:55 PackageErrorFunction.compute
pub fn package_error_function(
    key: &PackageErrorKey,
    env: &mut Environment<'_>,
) -> Result<Option<PackageErrorValue>, ComputeError> {
    let package_identifier = &key.0;
    // Callers must have tried to load the package already and gotten the package successfully.
    match env.get_value_or_throw::<PackageKey, NoSuchPackageException>(PackageKey::key(
        package_identifier.clone(),
    ))? {
        Err(e) => panic!(
            "IllegalStateException: Function should not have been called on package with \
             exception: {}",
            e
        ),
        Ok(None) => Ok(None),
        Ok(Some(pkg)) => {
            assert!(pkg.get_package().contains_errors(), "{:?}", key);
            Err(SkyFunctionException::persistent(
                NoSuchPackageException::build_file_contains_errors(package_identifier.clone()),
            )
            .into())
        }
    }
}
