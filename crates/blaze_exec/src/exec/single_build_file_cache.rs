//! Port of `lib/exec/SingleBuildFileCache.java`.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use blaze_vfs::{Path, PathFragment, Symlinks};
use parking_lot::Mutex;

use crate::actions::action_adapters::as_blaze_artifact;
use crate::actions::{
    ActionInputRef, FileArtifactValue, FilesetOutputTree, InputMetadataProvider,
    RunfilesArtifactValue, TreeArtifactValue,
};

/// The message prefix of Java's `DigestOfDirectoryException` (an `IOException`).
// Java: com/google/devtools/build/lib/actions/DigestOfDirectoryException.java:20 DigestOfDirectoryException
pub const INPUT_IS_A_DIRECTORY: &str = "Input is a directory: ";

/// Container class for caching I/O around `ActionInput`s: the metadata, or the exception that
/// reading it raised.
// Java: com/google/devtools/build/lib/exec/SingleBuildFileCache.java:157 SingleBuildFileCache.ActionInputMetadata
#[derive(Clone)]
struct ActionInputMetadata {
    input: ActionInputRef,
    metadata: Result<FileArtifactValue, (io::ErrorKind, String)>,
}

impl ActionInputMetadata {
    // Java: com/google/devtools/build/lib/exec/SingleBuildFileCache.java:175 ActionInputMetadata.getMetadata
    fn get_metadata(&self) -> io::Result<FileArtifactValue> {
        match &self.metadata {
            Ok(metadata) => Ok(metadata.clone()),
            Err((kind, message)) => Err(io::Error::new(*kind, message.clone())),
        }
    }
}

/// An in-memory cache to ensure we do I/O for source files only once during a single build.
///
/// Simply maintains a cached mapping from filename to metadata that may be populated only once.
// Java: com/google/devtools/build/lib/exec/SingleBuildFileCache.java:46 SingleBuildFileCache
pub struct SingleBuildFileCache {
    exec_root: Path,
    relative_output_path: PathFragment,
    // If we can't get the digest, we store the exception. This avoids extra file IO for files
    // that are allowed to be missing, as we first check a likely non-existent content file
    // first.
    path_to_metadata: Mutex<HashMap<PathFragment, ActionInputMetadata>>,
}

impl SingleBuildFileCache {
    /// `new SingleBuildFileCache(cwd, relativeOutputPath, fs, xattrProvider)`: `exec_root` is
    /// `fs.getPath(cwd)`.
    // Java: com/google/devtools/build/lib/exec/SingleBuildFileCache.java:62 SingleBuildFileCache.<init>
    pub fn new(exec_root: Path, relative_output_path: PathFragment) -> SingleBuildFileCache {
        SingleBuildFileCache {
            exec_root,
            relative_output_path,
            path_to_metadata: Mutex::new(HashMap::new()),
        }
    }

    /// The `relativeOutputPath` this cache was created with
    /// (`directories.getRelativeOutputPath()`).
    pub fn get_relative_output_path(&self) -> &PathFragment {
        &self.relative_output_path
    }

    /// `ActionInputHelper.toInputPath(input, execRoot)`.
    // Java: com/google/devtools/build/lib/actions/ActionInputHelper.java:147 ActionInputHelper.toInputPath
    fn to_input_path(&self, input: &ActionInputRef) -> Path {
        match as_blaze_artifact(input) {
            Some(artifact) => artifact.get_path(),
            None => self.exec_root.get_relative(input.get_exec_path()),
        }
    }
}

impl InputMetadataProvider for SingleBuildFileCache {
    // Java: com/google/devtools/build/lib/exec/SingleBuildFileCache.java:71 SingleBuildFileCache.getInputMetadataChecked
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        if let Some(artifact) = input.as_artifact() {
            if !artifact.is_source_artifact() {
                panic!(
                    "IllegalStateException: SingleBuildFileCache does not support derived artifact '{}'",
                    input.get_exec_path_string()
                );
            }
        } else if input
            .get_exec_path()
            .starts_with(&self.relative_output_path)
        {
            panic!(
                "IllegalStateException: SingleBuildFileCache does not support action input '{}' in the output tree",
                input.get_exec_path()
            );
        }

        // PORT: Caffeine's `Cache.get(key, mappingFunction)` computes under the entry's lock; the
        // I/O is done outside the map lock here, so two threads may both stat a file whose entry
        // is absent. The first result is kept.
        let exec_path = input.get_exec_path().clone();
        if let Some(cached) = self.path_to_metadata.lock().get(&exec_path).cloned() {
            return cached.get_metadata().map(Some);
        }
        let path = self.to_input_path(input);
        let metadata = match path
            .stat_with(Symlinks::Follow)
            .and_then(|stat| FileArtifactValue::create_from_stat(&path, &stat))
        {
            Err(e) => Err((e.kind(), e.to_string())),
            Ok(metadata) => {
                if metadata.get_type().is_directory() {
                    // new DigestOfDirectoryException("Input is a directory: " + execPath)
                    Err((
                        io::ErrorKind::IsADirectory,
                        format!("{INPUT_IS_A_DIRECTORY}{exec_path}"),
                    ))
                } else {
                    Ok(metadata)
                }
            }
        };
        let computed = ActionInputMetadata {
            input: input.clone(),
            metadata,
        };
        self.path_to_metadata
            .lock()
            .entry(exec_path)
            .or_insert(computed)
            .get_metadata()
            .map(Some)
    }

    // Java: com/google/devtools/build/lib/exec/SingleBuildFileCache.java:108 SingleBuildFileCache.getTreeMetadata
    fn get_tree_metadata(&self, _input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        None
    }

    // Java: com/google/devtools/build/lib/exec/SingleBuildFileCache.java:121 SingleBuildFileCache.getFileset
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        None
    }

    // Java: com/google/devtools/build/lib/exec/SingleBuildFileCache.java:132 SingleBuildFileCache.getRunfilesMetadata
    fn get_runfiles_metadata(&self, _input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        None
    }

    // Java: com/google/devtools/build/lib/exec/SingleBuildFileCache.java:144 SingleBuildFileCache.getInput
    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        self.path_to_metadata
            .lock()
            .get(exec_path)
            .map(|metadata| metadata.input.clone())
    }
}

/// Port of `SingleBuildFileCacheTest`.
///
/// PORT: Java uses an `InMemoryFileSystem` rooted at `/`; these tests use the real file system
/// under a fresh temporary directory as the exec root, with exec-root-relative inputs.
#[cfg(test)]
mod tests {
    use blaze_vfs::{DigestHashFunction, FileSystemExt, UnixFileSystem};
    use sha2::Digest as _;

    use super::*;
    use crate::actions::BasicActionInput;

    fn set_up() -> (tempfile::TempDir, SingleBuildFileCache) {
        let dir = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let exec_root = fs.get_path(dir.path().to_str().unwrap());
        std::fs::write(dir.path().join("empty"), b"").unwrap();
        let under_test =
            SingleBuildFileCache::new(exec_root, PathFragment::create("dummy-output-path"));
        (dir, under_test)
    }

    fn from_path(path: &str) -> ActionInputRef {
        ActionInputRef::new(BasicActionInput::from_path(path))
    }

    #[test]
    fn test_non_existent_path() {
        let (_dir, under_test) = set_up();
        let empty = from_path("noexist");
        // non existent file should raise exception
        assert!(under_test.get_input_metadata(&empty).is_err());
    }

    #[test]
    fn test_directory() {
        let (dir, under_test) = set_up();
        std::fs::create_dir(dir.path().join("directory")).unwrap();
        let input = from_path("directory");
        // directory should raise exception
        let expected = under_test.get_input_metadata(&input).unwrap_err();
        // PORT: Java's input is the absolute path `/directory`; here it is exec-root relative.
        assert_eq!(expected.to_string(), "Input is a directory: directory");
    }

    #[test]
    fn test_cache() {
        let (dir, under_test) = set_up();
        let empty = from_path("empty");
        let first = under_test.get_input_metadata(&empty).unwrap().unwrap();
        // PORT: Java counts `getInputStream` calls of the file system; here the file is changed
        // after the first read and the cached metadata must not change.
        std::fs::write(dir.path().join("empty"), b"changed").unwrap();
        let second = under_test.get_input_metadata(&empty).unwrap().unwrap();
        assert_eq!(first.get_digest(), second.get_digest());
        assert_eq!(second.get_size(), 0);
    }

    #[test]
    fn test_basic() {
        let (_dir, under_test) = set_up();
        let empty = from_path("empty");
        let metadata = under_test.get_input_metadata(&empty).unwrap().unwrap();
        assert_eq!(metadata.get_size(), 0);
        let expected = sha2::Sha256::digest(b"").to_vec();
        assert_eq!(metadata.get_digest(), Some(expected.as_slice()));
    }

    #[test]
    #[ignore = "needs a FileSystem whose getDigest is overridden (digest without reading the file)"]
    fn test_unreadable_file_when_file_system_supports_digest() {
        unimplemented!(
            "not ported: SingleBuildFileCacheTest.testUnreadableFileWhenFileSystemSupportsDigest"
        )
    }
}
