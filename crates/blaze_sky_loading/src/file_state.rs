//! FILE_STATE: `FileStateFunction`, its key, and `ExternalFilesHelper`.
//!
//! Java: com/google/devtools/build/lib/skyframe/{FileStateFunction,ExternalFilesHelper}.java,
//! com/google/devtools/build/lib/vfs/FileStateKey.java.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::RepositoryName;
use blaze_cmdline::label_constants::EXTERNAL_REPOSITORY_LOCATION;
use blaze_skyframe::{ComputeError, Environment, Key, SkyFunctionException, SkyFunctionName};
use blaze_vfs::{Path, RootedPath};
use parking_lot::RwLock;

use crate::actions::FileStateValue;
use crate::directories::LoadingDirectories;
use crate::io::IOException;
use crate::pkgcache::PathPackageLocator;
use crate::repos::RepositoryDirectoryKey;
use crate::sky_functions;

/// The SkyKey for [`FileStateValue`]: a [`RootedPath`]. `Debug` prints the rooted path, as Java
/// (where `RootedPath` itself is the key) does.
// Java: com/google/devtools/build/lib/vfs/FileStateKey.java:25 FileStateKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct FileStateKey(pub RootedPath);

impl FileStateKey {
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:205 FileStateValue.key
    pub fn key(rooted_path: RootedPath) -> FileStateKey {
        FileStateKey(rooted_path)
    }

    // Java: com/google/devtools/build/lib/vfs/FileStateKey.java:33 FileStateKey.argument
    pub fn argument(&self) -> &RootedPath {
        &self.0
    }
}

impl fmt::Debug for FileStateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Key for FileStateKey {
    type Value = FileStateValue;
    const FUNCTION: SkyFunctionName = sky_functions::FILE_STATE;
}

/// The action to take when an external path is encountered. See [`FileType`] for the definition
/// of "external".
// Java: com/google/devtools/build/lib/skyframe/ExternalFilesHelper.java:130 ExternalFilesHelper.ExternalFileAction
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalFileAction {
    /// For paths of type [`FileType::ExternalRepo`], introduce a Skyframe dependency on the
    /// 'external' package.
    ///
    /// This is the default for Bazel, since it's required for correctness of the external
    /// repositories feature.
    DependOnExternalPkgForExternalRepoPaths,
    /// For paths of type [`FileType::ExternalOther`] or [`FileType::Output`], assume the path
    /// does not exist and will never exist.
    AssumeNonExistentAndImmutableForExternalPaths,
}

/// Classification of a path encountered by Bazel.
// Java: com/google/devtools/build/lib/skyframe/ExternalFilesHelper.java:148 ExternalFilesHelper.FileType
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    /// A path to a file located inside of Bazel, e.g. the bundled builtins_bzl root (see
    /// [`crate::bundled_fs`]). These files must not change throughout the lifetime of the server.
    Bundled,
    /// A path inside the package roots.
    Internal,
    /// A path in Bazel's output tree that's a proper output of an action (*not* a source file in
    /// an external repository).
    Output,
    /// A path in the part of Bazel's output tree that contains (/ symlinks to) to external
    /// repositories.
    ExternalRepo,
    /// The directory containing the repo contents cache entries as well as direct children
    /// corresponding to individual predeclared input hashes.
    RepoContentsCacheDirs,
    /// None of the above. We encounter these paths when outputs, source files or external repos
    /// symlink to files outside aforementioned Bazel-managed directories.
    ExternalOther,
}

/// Thrown by [`ExternalFilesHelper::maybe_handle_external_file`] when an applicable path is
/// processed (see [`ExternalFileAction::AssumeNonExistentAndImmutableForExternalPaths`]).
// Java: com/google/devtools/build/lib/skyframe/ExternalFilesHelper.java:212 ExternalFilesHelper.NonexistentImmutableExternalFileException
#[derive(Debug)]
pub struct NonexistentImmutableExternalFileException;

/// Classifies paths and adds the dependencies Skyframe needs for paths in external repositories.
///
/// PORT: the "external files knowledge" (which kinds of external paths were seen, used by the
/// next command's diff awareness) and the logging of external paths are skipped (incremental).
// Java: com/google/devtools/build/lib/skyframe/ExternalFilesHelper.java:47 ExternalFilesHelper
pub struct ExternalFilesHelper {
    pkg_locator: Arc<RwLock<PathPackageLocator>>,
    external_file_action: ExternalFileAction,
    directories: LoadingDirectories,
    repo_contents_cache_path: Option<Path>,
}

impl ExternalFilesHelper {
    // Java: com/google/devtools/build/lib/skyframe/ExternalFilesHelper.java:80 ExternalFilesHelper.create
    pub fn create(
        pkg_locator: Arc<RwLock<PathPackageLocator>>,
        external_file_action: ExternalFileAction,
        directories: LoadingDirectories,
        repo_contents_cache_path: Option<Path>,
    ) -> ExternalFilesHelper {
        ExternalFilesHelper {
            pkg_locator,
            external_file_action,
            directories,
            repo_contents_cache_path,
        }
    }

    /// Returns the path of the repo contents cache directory, or `None` if the repo contents cache
    /// is disabled. Fetched repos may be materialized as symlinks into this directory.
    // Java: com/google/devtools/build/lib/skyframe/ExternalFilesHelper.java:122 ExternalFilesHelper.getRepoContentsCachePath
    pub fn get_repo_contents_cache_path(&self) -> Option<&Path> {
        self.repo_contents_cache_path.as_ref()
    }

    // Java: com/google/devtools/build/lib/skyframe/ExternalFilesHelper.java:262 ExternalFilesHelper.getAndNoteFileType
    pub fn get_and_note_file_type(&self, rooted_path: &RootedPath) -> FileType {
        // PORT: skipped, incremental (noting which kinds of files were seen).
        self.detect_file_type(rooted_path)
    }

    /// Returns the classification of a path.
    // Java: com/google/devtools/build/lib/skyframe/ExternalFilesHelper.java:283 ExternalFilesHelper.detectFileType
    fn detect_file_type(&self, rooted_path: &RootedPath) -> FileType {
        if crate::bundled_fs::is_bundled_file_system(rooted_path.get_root().get_file_system()) {
            return FileType::Bundled;
        }
        let package_locator = self.pkg_locator.read().clone();
        if package_locator
            .get_path_entries()
            .contains(rooted_path.get_root())
        {
            return FileType::Internal;
        }
        let path = rooted_path.as_path();
        if let Some(repo_contents_cache_path) = &self.repo_contents_cache_path {
            if path.starts_with(repo_contents_cache_path)
                && !path
                    .relative_to(repo_contents_cache_path)
                    .is_multi_segment()
            {
                return FileType::RepoContentsCacheDirs;
            }
        }
        // The outputBase may be null if we're not actually running a build.
        let Some(output_base) = package_locator.get_output_base() else {
            return FileType::ExternalOther;
        };
        if path.starts_with(output_base) {
            let external_repo_dir = output_base.get_relative(&EXTERNAL_REPOSITORY_LOCATION);
            if path.starts_with(&external_repo_dir) {
                return FileType::ExternalRepo;
            } else {
                return FileType::Output;
            }
        }
        FileType::ExternalOther
    }

    /// If this instance is configured with
    /// [`ExternalFileAction::DependOnExternalPkgForExternalRepoPaths`] and `rooted_path` isn't
    /// under a package root then this adds a dependency on the repository directory. If the
    /// action is [`ExternalFileAction::AssumeNonExistentAndImmutableForExternalPaths`], it
    /// returns `Ok(Err(NonexistentImmutableExternalFileException))` instead.
    // Java: com/google/devtools/build/lib/skyframe/ExternalFilesHelper.java:318 ExternalFilesHelper.maybeHandleExternalFile
    pub fn maybe_handle_external_file(
        &self,
        rooted_path: &RootedPath,
        env: &mut Environment<'_>,
    ) -> Result<
        Result<FileType, NonexistentImmutableExternalFileException>,
        blaze_skyframe::Interrupted,
    > {
        let file_type = self.get_and_note_file_type(rooted_path);
        match file_type {
            FileType::Bundled | FileType::Internal | FileType::RepoContentsCacheDirs => {}
            FileType::ExternalOther | FileType::Output => {
                // PORT: the EXTERNAL_OTHER logging is skipped.
                if self.external_file_action
                    == ExternalFileAction::AssumeNonExistentAndImmutableForExternalPaths
                {
                    return Ok(Err(NonexistentImmutableExternalFileException));
                }
            }
            FileType::ExternalRepo => {
                blaze_util::check_state!(
                    self.external_file_action
                        == ExternalFileAction::DependOnExternalPkgForExternalRepoPaths,
                    "{:?}",
                    self.external_file_action
                );
                self.add_external_files_dependencies(rooted_path, env)?;
            }
        }
        Ok(Ok(file_type))
    }

    /// For files that are under $OUTPUT_BASE/external, add a dependency on the corresponding repo
    /// so that if the repo is refetched, the {File,DirectoryListing}StateValue's of files
    /// underneath will be re-evaluated.
    ///
    /// Note that we don't add a dependency on the parent directory at the package root boundary,
    /// so the only transitive dependencies from files inside the package roots to external files
    /// are through symlinks. So the upwards transitive closure of external files is small.
    // Java: com/google/devtools/build/lib/skyframe/ExternalFilesHelper.java:356 ExternalFilesHelper.addExternalFilesDependencies
    fn add_external_files_dependencies(
        &self,
        rooted_path: &RootedPath,
        env: &mut Environment<'_>,
    ) -> Result<(), blaze_skyframe::Interrupted> {
        if let Some(repository_name) = self.get_external_repo_name(rooted_path) {
            env.get_value(RepositoryDirectoryKey(repository_name))?;
        }
        Ok(())
    }

    /// For a file of type [`FileType::ExternalRepo`], returns the name of the external repository
    /// it is in or `None` if the path is not in a valid external repository.
    // Java: com/google/devtools/build/lib/skyframe/ExternalFilesHelper.java:369 ExternalFilesHelper.getExternalRepoName
    pub fn get_external_repo_name(&self, rooted_path: &RootedPath) -> Option<RepositoryName> {
        let repository_path = rooted_path
            .as_path()
            .relative_to(&self.get_external_directory());
        if repository_path.is_empty() {
            // We are the top of the repository path (<outputBase>/external), not in an actual
            // external repository path.
            return None;
        }
        // The directory of a repository with an invalid name can never exist.
        RepositoryName::create(repository_path.get_segment(0)).ok()
    }

    // Java: com/google/devtools/build/lib/skyframe/ExternalFilesHelper.java:411 ExternalFilesHelper.getExternalDirectory
    fn get_external_directory(&self) -> Path {
        self.directories.get_external_repository_directory()
    }
}

/// Used to declare all the exception types that can be wrapped in the exception thrown by
/// FileStateFunction: an `InconsistentFilesystemException` is catastrophic, other IOExceptions
/// are not; both are transient.
// Java: com/google/devtools/build/lib/skyframe/FileStateFunction.java:82 FileStateFunction.FileStateFunctionException
fn file_state_function_exception(e: IOException) -> SkyFunctionException {
    if e.is_inconsistent_filesystem() {
        SkyFunctionException::catastrophic(e, blaze_skyframe::Transience::Transient)
    } else {
        SkyFunctionException::transient(e)
    }
}

/// A SkyFunction for [`FileStateValue`]s.
///
/// Merely calls [`FileStateValue::create`], but also has special handling for files outside the
/// package roots (see [`ExternalFilesHelper`]).
///
/// PORT: the `SyscallCache` and `TimestampGranularityMonitor` constructor arguments are not
/// ported; files are always stat'ed (Java's NO_CACHE path, used for external repos).
// Java: com/google/devtools/build/lib/skyframe/FileStateFunction.java:35 FileStateFunction
pub struct FileStateFunction {
    external_files_helper: Arc<ExternalFilesHelper>,
}

impl FileStateFunction {
    // Java: com/google/devtools/build/lib/skyframe/FileStateFunction.java:41 FileStateFunction.<init>
    pub fn new(external_files_helper: Arc<ExternalFilesHelper>) -> FileStateFunction {
        FileStateFunction {
            external_files_helper,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/FileStateFunction.java:56 FileStateFunction.compute
    pub fn compute(
        &self,
        sky_key: &FileStateKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<FileStateValue>, ComputeError> {
        let rooted_path = sky_key.argument();
        let file_type = match self
            .external_files_helper
            .maybe_handle_external_file(rooted_path, env)?
        {
            Ok(file_type) => file_type,
            Err(NonexistentImmutableExternalFileException) => {
                return Ok(Some(FileStateValue::NONEXISTENT_FILE_STATE_NODE));
            }
        };
        if env.values_missing() {
            return Ok(None);
        }
        // PORT: Java does not use the syscall cache for EXTERNAL_REPO files (they get generated
        // during the build); Blaze never caches, so both branches stat.
        let _ = file_type;
        match FileStateValue::create(rooted_path) {
            Ok(v) => Ok(Some(v)),
            Err(e) => Err(file_state_function_exception(e).into()),
        }
    }
}
