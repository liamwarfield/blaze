//! The subset of `FileArtifactValue`, `FileStateType` and `FileContentsProxy` the execution layer
//! reads and the action cache creates.
//!
//! PORT: seam copy. `blaze_actions` owns the full port of `lib/actions/FileArtifactValue.java`;
//! when `blaze_exec` depends on it, this enum is replaced by (or converted from) that type.

use std::fmt;
use std::io;

use blaze_vfs::{FileStatus, Path, PathFragment, Symlinks};

/// Port of `FileStateType`.
// Java: com/google/devtools/build/lib/actions/FileStateType.java:20 FileStateType
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileStateType {
    RegularFile,
    SpecialFile,
    Directory,
    Symlink,
    Nonexistent,
}

impl FileStateType {
    /// `getHumanReadableName()`.
    pub fn get_human_readable_name(self) -> &'static str {
        match self {
            FileStateType::RegularFile => "file",
            FileStateType::SpecialFile => "special file",
            FileStateType::Directory => "directory",
            FileStateType::Symlink => "symlink",
            FileStateType::Nonexistent => "non-existent path",
        }
    }
    /// `exists()`.
    pub fn exists(self) -> bool {
        self != FileStateType::Nonexistent
    }
    /// `isSymlink()`.
    pub fn is_symlink(self) -> bool {
        self == FileStateType::Symlink
    }
    /// `isFile()`.
    pub fn is_file(self) -> bool {
        self == FileStateType::RegularFile
    }
    /// `isSpecialFile()`.
    pub fn is_special_file(self) -> bool {
        self == FileStateType::SpecialFile
    }
    /// `isDirectory()`.
    pub fn is_directory(self) -> bool {
        self == FileStateType::Directory
    }
}

/// Port of `FileContentsProxy`: ctime, mtime and inode of a file, to detect modification.
// Java: com/google/devtools/build/lib/actions/FileContentsProxy.java:30 FileContentsProxy
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileContentsProxy {
    ctime: i64,
    mtime: i64,
    node_id: i64,
}

impl FileContentsProxy {
    /// `new FileContentsProxy(ctime, mtime, nodeId)`.
    pub fn new(ctime: i64, mtime: i64, node_id: i64) -> FileContentsProxy {
        FileContentsProxy {
            ctime,
            mtime,
            node_id,
        }
    }

    /// `create(FileStatus)`.
    // Java: com/google/devtools/build/lib/actions/FileContentsProxy.java:46 FileContentsProxy.create
    pub fn create(stat: &FileStatus) -> FileContentsProxy {
        FileContentsProxy::new(
            stat.get_last_change_time(),
            stat.get_last_modified_time(),
            stat.get_node_id(),
        )
    }

    /// `isModified(other)`: only node id and mtime are compared (ctime changes on chmod).
    // Java: com/google/devtools/build/lib/actions/FileContentsProxy.java:68 FileContentsProxy.isModified
    pub fn is_modified(&self, other: &FileContentsProxy) -> bool {
        self.node_id != other.node_id || self.mtime != other.mtime
    }

    /// `getCTime()`.
    pub fn get_ctime(&self) -> i64 {
        self.ctime
    }
    /// `getMTime()`.
    pub fn get_mtime(&self) -> i64 {
        self.mtime
    }
    /// `getNodeId()`.
    pub fn get_node_id(&self) -> i64 {
        self.node_id
    }

    /// `prettyPrint()`.
    // Java: com/google/devtools/build/lib/actions/FileContentsProxy.java:95 FileContentsProxy.prettyPrint
    pub fn pretty_print(&self) -> String {
        format!(
            "ctime of {} and mtime of {} and nodeId of {}",
            self.ctime, self.mtime, self.node_id
        )
    }
}

/// Port of the `FileArtifactValue` hierarchy (the subclasses the execution layer needs).
// Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:84 FileArtifactValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileArtifactValue {
    /// `RegularFileArtifactValue`.
    RegularFile {
        digest: Option<Vec<u8>>,
        proxy: Option<FileContentsProxy>,
        size: i64,
    },
    /// `DirectoryArtifactValue` (mtime only).
    Directory { mtime: i64 },
    /// `HashedDirectoryArtifactValue`.
    HashedDirectory { digest: Vec<u8> },
    /// `UnresolvedSymlinkArtifactValue`.
    UnresolvedSymlink { target: String, digest: Vec<u8> },
    /// `RemoteFileArtifactValue` / `RemoteFileArtifactValueWithMaterializationData`.
    RemoteFile {
        digest: Vec<u8>,
        size: i64,
        location_index: i32,
        expiration_time_millis: Option<i64>,
        in_memory_output: bool,
    },
    /// `ResolvedSymlinkArtifactValue`: a value with the path it was resolved through.
    ResolvedSymlink {
        delegate: Box<FileArtifactValue>,
        resolved_path: PathFragment,
    },
    /// `RunfilesProxyArtifactValue`.
    RunfilesProxy { digest: Vec<u8> },
    /// `FileArtifactValue.ConstantMetadataValue.INSTANCE`.
    ConstantMetadata,
    /// `ProxyFileArtifactValue`.
    Proxy {
        delegate: Box<FileArtifactValue>,
        path: blaze_vfs::Path,
    },
    /// `RUNFILES_TREE_MARKER`.
    RunfilesTreeMarker,
    /// `MISSING_FILE_MARKER`.
    MissingFileMarker,
}

impl FileArtifactValue {
    /// `getType()`.
    pub fn get_type(&self) -> FileStateType {
        match self {
            FileArtifactValue::RegularFile { .. } => FileStateType::RegularFile,
            FileArtifactValue::Directory { .. } => FileStateType::Directory,
            FileArtifactValue::HashedDirectory { .. } => FileStateType::Directory,
            FileArtifactValue::UnresolvedSymlink { .. } => FileStateType::Symlink,
            FileArtifactValue::RemoteFile { .. } => FileStateType::RegularFile,
            FileArtifactValue::ResolvedSymlink { delegate, .. } => delegate.get_type(),
            FileArtifactValue::RunfilesProxy { .. } => FileStateType::Directory,
            FileArtifactValue::ConstantMetadata => FileStateType::RegularFile,
            FileArtifactValue::Proxy { delegate, .. } => delegate.get_type(),
            // Java: SingletonMarkerValue.getType() is NONEXISTENT.
            FileArtifactValue::RunfilesTreeMarker | FileArtifactValue::MissingFileMarker => {
                FileStateType::Nonexistent
            }
        }
    }

    /// `getDigest()`: `None` when unknown.
    pub fn get_digest(&self) -> Option<&[u8]> {
        match self {
            FileArtifactValue::RegularFile { digest, .. } => digest.as_deref(),
            FileArtifactValue::Directory { .. } => None,
            FileArtifactValue::HashedDirectory { digest } => Some(digest),
            FileArtifactValue::UnresolvedSymlink { digest, .. } => Some(digest),
            FileArtifactValue::RemoteFile { digest, .. } => Some(digest),
            FileArtifactValue::ResolvedSymlink { delegate, .. } => delegate.get_digest(),
            FileArtifactValue::RunfilesProxy { digest } => Some(digest),
            // Java: ConstantMetadataValue.DIGEST = new byte[1].
            FileArtifactValue::ConstantMetadata => Some(&[0u8]),
            FileArtifactValue::Proxy { delegate, .. } => delegate.get_digest(),
            FileArtifactValue::RunfilesTreeMarker | FileArtifactValue::MissingFileMarker => None,
        }
    }

    /// `getSize()`.
    pub fn get_size(&self) -> i64 {
        match self {
            FileArtifactValue::RegularFile { size, .. } => *size,
            FileArtifactValue::RemoteFile { size, .. } => *size,
            FileArtifactValue::ResolvedSymlink { delegate, .. } => delegate.get_size(),
            FileArtifactValue::Proxy { delegate, .. } => delegate.get_size(),
            _ => 0,
        }
    }

    /// `getContentsProxy()`.
    pub fn get_contents_proxy(&self) -> Option<FileContentsProxy> {
        match self {
            FileArtifactValue::RegularFile { proxy, .. } => *proxy,
            FileArtifactValue::ResolvedSymlink { delegate, .. } => delegate.get_contents_proxy(),
            FileArtifactValue::Proxy { delegate, .. } => delegate.get_contents_proxy(),
            _ => None,
        }
    }

    /// `getUnresolvedSymlinkTarget()`.
    pub fn get_unresolved_symlink_target(&self) -> Option<&str> {
        match self {
            FileArtifactValue::UnresolvedSymlink { target, .. } => Some(target),
            _ => None,
        }
    }

    /// `isRemote()`.
    pub fn is_remote(&self) -> bool {
        match self {
            FileArtifactValue::RemoteFile { .. } => true,
            FileArtifactValue::ResolvedSymlink { delegate, .. } => delegate.is_remote(),
            _ => false,
        }
    }

    /// `getLocationIndex()`.
    pub fn get_location_index(&self) -> i32 {
        match self {
            FileArtifactValue::RemoteFile { location_index, .. } => *location_index,
            FileArtifactValue::ResolvedSymlink { delegate, .. } => delegate.get_location_index(),
            _ => 0,
        }
    }

    /// `getResolvedPath()`.
    pub fn get_resolved_path(&self) -> Option<&PathFragment> {
        match self {
            FileArtifactValue::ResolvedSymlink { resolved_path, .. } => Some(resolved_path),
            _ => None,
        }
    }

    /// `createForNormalFile(digest, proxy, size)`.
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:398 FileArtifactValue.createForNormalFile
    pub fn create_for_normal_file(
        digest: Option<Vec<u8>>,
        proxy: Option<FileContentsProxy>,
        size: i64,
    ) -> FileArtifactValue {
        FileArtifactValue::RegularFile {
            digest,
            proxy,
            size,
        }
    }

    /// `createForVirtualActionInput(digest, size)`.
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:385 FileArtifactValue.createForVirtualActionInput
    pub fn create_for_virtual_action_input(digest: Vec<u8>, size: i64) -> FileArtifactValue {
        FileArtifactValue::RegularFile {
            digest: Some(digest),
            proxy: None,
            size,
        }
    }

    /// `createForDirectoryWithMtime(mtime)`.
    pub fn create_for_directory_with_mtime(mtime: i64) -> FileArtifactValue {
        FileArtifactValue::Directory { mtime }
    }

    /// `createForRemoteFile(digest, size, locationIndex)`.
    pub fn create_for_remote_file(
        digest: Vec<u8>,
        size: i64,
        location_index: i32,
    ) -> FileArtifactValue {
        FileArtifactValue::RemoteFile {
            digest,
            size,
            location_index,
            expiration_time_millis: None,
            in_memory_output: false,
        }
    }

    /// `createForRemoteFileWithMaterializationData(...)`.
    pub fn create_for_remote_file_with_materialization_data(
        digest: Vec<u8>,
        size: i64,
        location_index: i32,
        expiration_time_millis: Option<i64>,
        in_memory_output: bool,
    ) -> FileArtifactValue {
        FileArtifactValue::RemoteFile {
            digest,
            size,
            location_index,
            expiration_time_millis,
            in_memory_output,
        }
    }

    /// `createFromExistingWithResolvedPath(delegate, resolvedPath)`.
    pub fn create_from_existing_with_resolved_path(
        delegate: FileArtifactValue,
        resolved_path: PathFragment,
    ) -> FileArtifactValue {
        FileArtifactValue::ResolvedSymlink {
            delegate: Box::new(delegate),
            resolved_path,
        }
    }

    /// `createFromStat(path, stat, xattrProvider)` (digest computed from the file: no xattrs).
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:353 FileArtifactValue.createFromStat
    pub fn create_from_stat(path: &Path, stat: &FileStatus) -> io::Result<FileArtifactValue> {
        if !stat.is_file() {
            return Ok(FileArtifactValue::Directory {
                mtime: path.get_last_modified_time()?,
            });
        }
        let digest = path.get_digest()?;
        Ok(FileArtifactValue::create_for_normal_file(
            Some(digest),
            Some(FileContentsProxy::create(stat)),
            stat.get_size(),
        ))
    }

    /// `createForTesting(Path)`.
    pub fn create_for_testing(path: &Path) -> io::Result<FileArtifactValue> {
        let stat = path.stat_with(Symlinks::Follow)?;
        FileArtifactValue::create_from_stat(path, &stat)
    }
}

impl fmt::Display for FileArtifactValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}
