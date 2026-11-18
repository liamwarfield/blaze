//! Port of `BuildEventArtifactUploader` and `LocalFilesArtifactUploader`.

use std::collections::HashMap;

use blaze_vfs::Path;

use super::build_event::{LocalFile, LocalFileType};
use super::build_event_context::{FileUriPathConverter, PathConverter};

/// Uploads artifacts referenced by the Build Event Protocol (BEP).
///
/// PORT: Java returns `ListenableFuture`s and reference-counts the uploader. Blaze's only
/// uploader is the local one, whose futures are immediate, so the results are plain values and
/// `retain`/`release` are not ported.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEventArtifactUploader.java:34 BuildEventArtifactUploader
pub trait BuildEventArtifactUploader: Send + Sync {
    /// Asynchronously uploads a set of files referenced by the protobuf representation of a
    /// build event. Returns a [`PathConverter`] that must be used to convert local paths to
    /// remote URIs.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventArtifactUploader.java:44 BuildEventArtifactUploader.upload
    fn upload(&self, files: HashMap<Path, LocalFile>) -> Box<dyn PathConverter>;

    /// Return true if the upload may be "slow". Examples of slowness include writes to remote
    /// storage.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventArtifactUploader.java:85 BuildEventArtifactUploader.mayBeSlow
    fn may_be_slow(&self) -> bool;

    /// Uploads the referenced local files of an event (the first `LocalFile` per path wins).
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventArtifactUploader.java:91 BuildEventArtifactUploader.uploadReferencedLocalFiles
    fn upload_referenced_local_files(&self, local_files: &[LocalFile]) -> Box<dyn PathConverter> {
        let mut local_file_map: HashMap<Path, LocalFile> =
            HashMap::with_capacity(local_files.len());
        for local_file in local_files {
            local_file_map
                .entry(local_file.path.clone())
                .or_insert_with(|| local_file.clone());
        }
        self.upload(local_file_map)
    }
}

/// An uploader that simply turns paths into local file URIs.
// Java: com/google/devtools/build/lib/buildeventstream/LocalFilesArtifactUploader.java:29 LocalFilesArtifactUploader
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalFilesArtifactUploader;

impl BuildEventArtifactUploader for LocalFilesArtifactUploader {
    // Java: com/google/devtools/build/lib/buildeventstream/LocalFilesArtifactUploader.java:34 LocalFilesArtifactUploader.upload
    fn upload(&self, files: HashMap<Path, LocalFile>) -> Box<dyn PathConverter> {
        Box::new(PathConverterImpl { paths: files })
    }

    // Java: com/google/devtools/build/lib/buildeventstream/LocalFilesArtifactUploader.java:49 LocalFilesArtifactUploader.mayBeSlow
    fn may_be_slow(&self) -> bool {
        false
    }
}

// Java: com/google/devtools/build/lib/buildeventstream/LocalFilesArtifactUploader.java:53 LocalFilesArtifactUploader.PathConverterImpl
struct PathConverterImpl {
    paths: HashMap<Path, LocalFile>,
}

impl PathConverter for PathConverterImpl {
    // Java: com/google/devtools/build/lib/buildeventstream/LocalFilesArtifactUploader.java:62 PathConverterImpl.apply
    fn apply(&self, path: &Path) -> Option<String> {
        let local_file = self.paths.get(path)?;
        let type_ = local_file.type_;
        if type_ == LocalFileType::OutputDirectory || type_ == LocalFileType::OutputSymlink {
            return None;
        }
        FileUriPathConverter.apply(path)
    }
}
