//! Port of `lib/actions/ActionOutputDirectoryHelper.java`: creates the output directories of
//! actions, replacing files or symlinks left in the way by earlier builds.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;

use blaze_actions::Artifact;
use blaze_vfs::{Path, PathFragment, Symlinks};
use parking_lot::Mutex;

/// Whether a directory was found or created in this invocation.
// Java: com/google/devtools/build/lib/actions/ActionOutputDirectoryHelper.java:47 ActionOutputDirectoryHelper.DirectoryState
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectoryState {
    Found,
    Created,
}

/// Helper class to create directories for action outputs.
///
/// PORT: Java bounds `knownDirectories` with a Caffeine cache (`--directory_creation_cache`,
/// default `maximumSize=10000`); an evicted entry only makes a later call re-check the
/// directory, so the map here is unbounded. Java's `Striped<Lock>` (64 stripes) guarding the
/// check-then-delete of `forceCreateDirectoryAndParents` is one lock.
// Java: com/google/devtools/build/lib/actions/ActionOutputDirectoryHelper.java:39 ActionOutputDirectoryHelper
pub struct ActionOutputDirectoryHelper {
    known_directories: Mutex<HashMap<PathFragment, DirectoryState>>,
    output_directory_deletion_lock: Mutex<()>,
}

/// An exception that occurred while attempting to create an output directory.
// Java: com/google/devtools/build/lib/actions/ActionOutputDirectoryHelper.java:317 ActionOutputDirectoryHelper.CreateOutputDirectoryException
#[derive(Debug)]
pub struct CreateOutputDirectoryException {
    directory_path: PathFragment,
    cause: io::Error,
}

impl CreateOutputDirectoryException {
    /// Returns the path to the output directory for which the exception occurred.
    // Java: com/google/devtools/build/lib/actions/ActionOutputDirectoryHelper.java:327 CreateOutputDirectoryException.getDirectoryPath
    pub fn get_directory_path(&self) -> &PathFragment {
        &self.directory_path
    }

    /// The `IOException` cause (`getMessage()` is the cause's message).
    pub fn get_cause(&self) -> &io::Error {
        &self.cause
    }
}

impl fmt::Display for CreateOutputDirectoryException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.cause, f)
    }
}

impl std::error::Error for CreateOutputDirectoryException {}

impl Default for ActionOutputDirectoryHelper {
    fn default() -> Self {
        ActionOutputDirectoryHelper::new()
    }
}

impl ActionOutputDirectoryHelper {
    // Java: com/google/devtools/build/lib/actions/ActionOutputDirectoryHelper.java:52 ActionOutputDirectoryHelper.<init>
    pub fn new() -> ActionOutputDirectoryHelper {
        ActionOutputDirectoryHelper {
            known_directories: Mutex::new(HashMap::new()),
            output_directory_deletion_lock: Mutex::new(()),
        }
    }

    /// Invalidates the cached creation of tree artifact directories when an action is going to
    /// be rewound.
    // Java: com/google/devtools/build/lib/actions/ActionOutputDirectoryHelper.java:110 ActionOutputDirectoryHelper.invalidateTreeArtifactDirectoryCreation
    pub fn invalidate_tree_artifact_directory_creation(&self, action_outputs: &[Artifact]) {
        for output in action_outputs {
            if output.is_tree_artifact() {
                self.known_directories
                    .lock()
                    .remove(output.get_path().as_fragment());
            }
        }
    }

    /// Creates output directories, including missing ancestor directories, for the given set of
    /// action outputs.
    ///
    /// For a non-tree output, the parent directory is created; for a tree output, the root
    /// directory for the tree is created.
    ///
    /// If a path to be created already exists but is not a directory, it is recursively deleted
    /// and an empty directory is created in its place. If the path exists but is a non-writable
    /// directory, it is made writable.
    // Java: com/google/devtools/build/lib/actions/ActionOutputDirectoryHelper.java:149 ActionOutputDirectoryHelper.createOutputDirectories
    pub fn create_output_directories(
        &self,
        action_outputs: &[Artifact],
    ) -> Result<(), CreateOutputDirectoryException> {
        let mut done: HashSet<Path> = HashSet::new(); // avoid redundant calls for the same directory.
        for output_file in action_outputs {
            // Given we know that we are not using action file system, we can get safely get paths
            // directly from the artifacts.
            let output_dir = if output_file.is_tree_artifact() {
                output_file.get_path()
            } else {
                output_file
                    .get_path()
                    .get_parent_directory()
                    .expect("output has a parent directory")
            };
            if done.insert(output_dir.clone()) {
                let root_path = output_file
                    .get_root()
                    .get_root()
                    .as_path()
                    .expect("output root is a path")
                    .as_fragment()
                    .clone();
                self.create_output_directory(&output_dir, &root_path)?;
            }
        }
        Ok(())
    }

    /// Creates a writable output directory, including missing ancestor directories.
    // Java: com/google/devtools/build/lib/actions/ActionOutputDirectoryHelper.java:183 ActionOutputDirectoryHelper.createOutputDirectory
    pub fn create_output_directory(
        &self,
        output_dir: &Path,
        root_path: &PathFragment,
    ) -> Result<(), CreateOutputDirectoryException> {
        match self.create_and_check_for_symlinks(output_dir, root_path) {
            Ok(()) => Ok(()),
            // Fall through to plan B.
            Err(_) => self.force_create_directory_and_parents(output_dir, root_path),
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionOutputDirectoryHelper.java:192 ActionOutputDirectoryHelper.forceCreateDirectoryAndParents
    fn force_create_directory_and_parents(
        &self,
        output_dir: &Path,
        root_path: &PathFragment,
    ) -> Result<(), CreateOutputDirectoryException> {
        // Possibly some direct ancestors are not directories. In that case, we traverse the
        // ancestors downward, deleting any non-directories. This handles the case where a file
        // becomes a directory. The traversal is done downward because otherwise we may delete
        // files through a symlink in a parent directory. Since Blaze never creates such
        // directories within a build, we have no idea where on disk we're actually deleting.
        //
        // Symlinks should not be followed so in order to clean up symlinks pointing to Fileset
        // outputs from previous builds. See bug [incremental build of Fileset fails if
        // Fileset.out was changed to be a subdirectory of the old value].
        let result = (|| -> io::Result<()> {
            let mut p = root_path.clone();
            let relative = output_dir.as_fragment().relative_to(root_path);
            for segment in relative.segments() {
                p = p.get_relative(&PathFragment::create(segment));
                let path = Path::create(p.clone(), output_dir.get_file_system());
                // This lock ensures that the only thread that observes a filesystem transition
                // in which the path p first exists and then does not is the thread that calls
                // p.delete() and causes the transition.
                let _lock = self.output_directory_deletion_lock.lock();
                let Some(stat) = path.stat_if_found(Symlinks::Nofollow)? else {
                    // Missing entry: Break out and create expected directories.
                    break;
                };
                if stat.is_directory() {
                    // If this directory used to be a tree artifact it won't be writable.
                    path.set_writable(true)?;
                    self.known_directories
                        .lock()
                        .insert(p.clone(), DirectoryState::Found);
                } else {
                    // p may be a file or symlink (possibly from a Fileset in a previous build).
                    path.delete()?; // throws IOException
                    break;
                }
            }
            output_dir.create_directory_and_parents()
        })();
        result.map_err(|cause| CreateOutputDirectoryException {
            directory_path: output_dir.as_fragment().clone(),
            cause,
        })
    }

    /// Create an output directory and ensure that no symlinks exists between the output root and
    /// the output file. These are all expected to be regular directories. Violations of this
    /// expectations can only come from state left behind by previous invocations or external
    /// filesystem mutation.
    // Java: com/google/devtools/build/lib/actions/ActionOutputDirectoryHelper.java:260 ActionOutputDirectoryHelper.createAndCheckForSymlinks
    fn create_and_check_for_symlinks(&self, dir: &Path, root: &PathFragment) -> io::Result<()> {
        let fs = dir.get_file_system();
        // If the output root has not been created yet, do so now.
        if !self.known_directories.lock().contains_key(root) {
            let root_path = Path::create(root.clone(), fs);
            let stat = root_path.stat_nullable(Symlinks::Nofollow);
            if stat.is_none() {
                root_path.create_directory_and_parents()?;
                self.known_directories
                    .lock()
                    .insert(root.clone(), DirectoryState::Created);
            } else {
                self.known_directories
                    .lock()
                    .insert(root.clone(), DirectoryState::Found);
            }
        }

        // Walk up until the first known directory is found (must be root or below).
        let mut check_dirs: Vec<PathFragment> = Vec::new();
        let mut dir_fragment = dir.as_fragment().clone();
        while dir_fragment != *root && !self.known_directories.lock().contains_key(&dir_fragment) {
            check_dirs.push(dir_fragment.clone());
            dir_fragment = dir_fragment
                .get_parent_directory()
                .expect("output directory below its root");
        }

        // Check in reverse order (parent directory first).
        let parent_created =
            self.known_directories.lock().get(&dir_fragment) == Some(&DirectoryState::Created);
        for path_fragment in check_dirs.into_iter().rev() {
            let path = Path::create(path_fragment.clone(), fs);
            if parent_created {
                // If we have created this directory's parent, we know that it doesn't exist or
                // else we would know about it already. Even if a parallel thread has created it in
                // the meantime, createDirectory() will succeed and we can assume that a regular
                // directory exists afterwards.
                path.create_directory()?;
                self.known_directories
                    .lock()
                    .insert(path_fragment, DirectoryState::Created);
                continue;
            }

            // Otherwise, check whether the directory already exists.
            // Note: while we could also optimistically try to create the directory upfront,
            // benchmarks indicate that doing it this way is faster.
            match path.stat_if_found(Symlinks::Nofollow)? {
                Some(stat) => {
                    // Already exists, but make sure it's a directory.
                    if !stat.is_directory() {
                        return Err(io::Error::other(format!(
                            "{} (File exists)",
                            path.get_path_string()
                        )));
                    }
                    // Adjust permissions on the directory if necessary.
                    // Avoid touching permissions for group/other, which may have been overridden
                    // by umask(2) when this directory was originally created.
                    // PORT: Java's `perms == -1` (permissions unknown) does not occur for a Unix
                    // stat.
                    let perms = stat.get_permissions();
                    if (perms & 0o700) != 0o700 {
                        path.chmod(perms | 0o700)?;
                    }
                    self.known_directories
                        .lock()
                        .insert(path_fragment, DirectoryState::Found);
                }
                None => {
                    // Create the directory. Even if a parallel thread has created it in the
                    // meantime, createDirectory() will succeed and we can assume that a regular
                    // directory exists afterwards.
                    path.create_directory()?;
                    self.known_directories
                        .lock()
                        .insert(path_fragment, DirectoryState::Created);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Port of the `ActionOutputDirectoryHelper` behaviour `SkyframeActionExecutorTest` and
    //! `ActionOutputDirectoryHelperTest` exercise: a stale file in the way is deleted, a
    //! read-only directory is made writable, a symlink in the way is replaced.

    use std::os::unix::fs::PermissionsExt;

    use blaze_vfs::{DigestHashFunction, FileSystemExt, Path, PathFragment, UnixFileSystem};

    use super::ActionOutputDirectoryHelper;

    fn root() -> (tempfile::TempDir, Path) {
        let dir = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root = fs.get_path(dir.path().to_str().unwrap()).get_child("out");
        std::fs::create_dir_all(root.get_path_string()).unwrap();
        (dir, root)
    }

    #[test]
    fn creates_missing_directories() {
        let (_d, root) = root();
        let helper = ActionOutputDirectoryHelper::new();
        let dir = root.get_relative(&PathFragment::create("a/b/c"));
        helper
            .create_output_directory(&dir, root.as_fragment())
            .unwrap();
        assert!(dir.is_directory());
    }

    #[test]
    fn deletes_file_in_the_way() {
        // Build 1 produced file `p/foo`; build 2 needs directory `p/foo`.
        let (_d, root) = root();
        std::fs::create_dir_all(format!("{}/p", root.get_path_string())).unwrap();
        std::fs::write(format!("{}/p/foo", root.get_path_string()), "x").unwrap();
        let helper = ActionOutputDirectoryHelper::new();
        let dir = root.get_relative(&PathFragment::create("p/foo"));
        helper
            .create_output_directory(&dir, root.as_fragment())
            .unwrap();
        assert!(dir.is_directory());
    }

    #[test]
    fn replaces_symlink_in_the_way() {
        let (_d, root) = root();
        let elsewhere = root.get_parent_directory().unwrap().get_child("elsewhere");
        std::fs::create_dir_all(elsewhere.get_path_string()).unwrap();
        std::os::unix::fs::symlink(
            elsewhere.get_path_string(),
            format!("{}/p", root.get_path_string()),
        )
        .unwrap();
        let helper = ActionOutputDirectoryHelper::new();
        let dir = root.get_relative(&PathFragment::create("p/q"));
        helper
            .create_output_directory(&dir, root.as_fragment())
            .unwrap();
        let p = root.get_child("p");
        assert!(!p.is_symbolic_link());
        assert!(dir.is_directory());
        // Nothing was written through the stale symlink.
        assert!(!elsewhere.get_child("q").exists());
    }

    #[test]
    fn makes_read_only_directory_writable() {
        let (_d, root) = root();
        let p = format!("{}/p", root.get_path_string());
        std::fs::create_dir_all(&p).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o555)).unwrap();
        let helper = ActionOutputDirectoryHelper::new();
        let dir = root.get_relative(&PathFragment::create("p/q"));
        helper
            .create_output_directory(&dir, root.as_fragment())
            .unwrap();
        assert!(dir.is_directory());
        let mode = std::fs::metadata(&p).unwrap().permissions().mode();
        assert_eq!(mode & 0o700, 0o700);
    }
}
