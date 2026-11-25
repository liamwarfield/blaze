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
    ///
    /// Equality is Java's: digest, size and location index only; the expiration time and the
    /// in-memory flag are not compared.
    /// PORT: Java's `equals` also checks the class (`RemoteFileArtifactValue.equals` accepts a
    /// value with materialization data, the reverse does not); the merged variant cannot tell
    /// the two classes apart, so the comparison here is the symmetric one.
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:756 RemoteFileArtifactValue.equals
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:903 RemoteFileArtifactValueWithMaterializationData.equals
    RemoteFile {
        digest: Vec<u8>,
        size: i64,
        location_index: i32,
        expiration_time_millis: NotInEquality<Option<i64>>,
        in_memory_output: NotInEquality<bool>,
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
    /// `TreeArtifactValue.TreeArtifactCompositeFileArtifactValue`: the metadata of a tree
    /// artifact whose children are all remote or that has a resolved path.
    ///
    /// PORT: a local tree without a resolved path keeps [`FileArtifactValue::HashedDirectory`],
    /// which answers every method the port reads as the composite value does.
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:224 TreeArtifactCompositeFileArtifactValue
    TreeArtifactComposite {
        digest: Vec<u8>,
        is_remote: NotInEquality<bool>,
        resolved_path: Option<PathFragment>,
    },
}

/// A field that `equals` does not compare (Java's `TreeArtifactCompositeFileArtifactValue.equals`
/// compares the digest and the resolved path only; the remote values' `equals` compares digest,
/// size and location index only).
#[derive(Clone, Copy, Debug)]
pub struct NotInEquality<T>(pub T);

impl<T> PartialEq for NotInEquality<T> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<T> Eq for NotInEquality<T> {}

impl FileArtifactValue {
    /// `getType()`.
    pub fn get_type(&self) -> FileStateType {
        match self {
            FileArtifactValue::RegularFile { .. } => FileStateType::RegularFile,
            FileArtifactValue::Directory { .. } => FileStateType::Directory,
            FileArtifactValue::HashedDirectory { .. } => FileStateType::Directory,
            // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:253 TreeArtifactCompositeFileArtifactValue.getType
            FileArtifactValue::TreeArtifactComposite { .. } => FileStateType::Directory,
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
            FileArtifactValue::TreeArtifactComposite { digest, .. } => Some(digest),
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
            // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:297 TreeArtifactCompositeFileArtifactValue.isRemote
            FileArtifactValue::TreeArtifactComposite { is_remote, .. } => is_remote.0,
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
            // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:303 TreeArtifactCompositeFileArtifactValue.getResolvedPath
            FileArtifactValue::TreeArtifactComposite { resolved_path, .. } => {
                resolved_path.as_ref()
            }
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

    /// `createForDirectoryWithHash(digest)`.
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:413 FileArtifactValue.createForDirectoryWithHash
    pub fn create_for_directory_with_hash(digest: Vec<u8>) -> FileArtifactValue {
        FileArtifactValue::HashedDirectory { digest }
    }

    /// `createForNormalFileUsingPath(path, size, xattrProvider)`: the digest is the file's fast
    /// digest or read from the file (`DigestUtils.getDigestWithManualFallback`), with no contents
    /// proxy.
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:407 FileArtifactValue.createForNormalFileUsingPath
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:364 FileArtifactValue.create
    pub fn create_for_normal_file_using_path(
        path: &Path,
        size: i64,
    ) -> io::Result<FileArtifactValue> {
        let digest =
            crate::actions::cache::digest_utils::DigestUtils::get_digest_with_manual_fallback(
                path,
            )?;
        Ok(FileArtifactValue::create_for_normal_file(
            Some(digest),
            /* proxy= */ None,
            size,
        ))
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
            expiration_time_millis: NotInEquality(None),
            in_memory_output: NotInEquality(false),
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
            expiration_time_millis: NotInEquality(expiration_time_millis),
            in_memory_output: NotInEquality(in_memory_output),
        }
    }

    /// `createFromExistingWithResolvedPath(delegate, resolvedPath)`: `new
    /// ResolvedSymlinkArtifactValue(delegate, resolvedPath)`, whose constructor keeps the delegate
    /// of a delegate that is itself a `ResolvedSymlinkArtifactValue`.
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:444 FileArtifactValue.createFromExistingWithResolvedPath
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:951 ResolvedSymlinkArtifactValue.<init>
    pub fn create_from_existing_with_resolved_path(
        delegate: FileArtifactValue,
        resolved_path: PathFragment,
    ) -> FileArtifactValue {
        // Singleton: SingletonMarkerValue (RUNFILES_TREE_MARKER, MISSING_FILE_MARKER) and
        // ConstantMetadataValue.
        blaze_util::check_argument!(
            !matches!(
                delegate,
                FileArtifactValue::RunfilesTreeMarker
                    | FileArtifactValue::MissingFileMarker
                    | FileArtifactValue::ConstantMetadata
            ),
            "delegate is a singleton: {}",
            delegate
        );
        blaze_util::check_argument!(
            resolved_path.is_absolute(),
            "resolved path is not absolute: {}",
            resolved_path
        );
        blaze_util::check_argument!(
            delegate
                .get_resolved_path()
                .is_none_or(|p| *p == resolved_path),
            "delegate has a different resolved path: {}",
            delegate
        );
        let delegate = match delegate {
            FileArtifactValue::ResolvedSymlink {
                delegate: resolved_delegate,
                ..
            } => *resolved_delegate,
            other => other,
        };
        FileArtifactValue::ResolvedSymlink {
            delegate: Box::new(delegate),
            resolved_path,
        }
    }

    /// `createFromStat(path, stat, xattrProvider)`: the digest is the file's fast digest or
    /// read from the file (`DigestUtils.getDigestWithManualFallback`).
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:353 FileArtifactValue.createFromStat
    pub fn create_from_stat(path: &Path, stat: &FileStatus) -> io::Result<FileArtifactValue> {
        if !stat.is_file() {
            return Ok(FileArtifactValue::Directory {
                mtime: path.get_last_modified_time()?,
            });
        }
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:364 FileArtifactValue.create
        let digest =
            crate::actions::cache::digest_utils::DigestUtils::get_digest_with_manual_fallback(
                path,
            )?;
        Ok(FileArtifactValue::create_for_normal_file(
            Some(digest),
            Some(FileContentsProxy::create(stat)),
            stat.get_size(),
        ))
    }

    /// `createForUnresolvedSymlink(Path)`: the value of the symlink at `path`, whose digest is
    /// the (tweaked) digest of its target string.
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:394 FileArtifactValue.createForUnresolvedSymlink
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:1149 UnresolvedSymlinkArtifactValue.<init>
    pub fn create_for_unresolved_symlink(symlink: &Path) -> io::Result<FileArtifactValue> {
        let symlink_target = symlink.read_symbolic_link()?.get_path_string().to_owned();

        // `hashString(symlinkTarget, ISO_8859_1)`: the bytes of the internal string (Guava encodes
        // with `String.getBytes(charset)`, so a char above U+00FF hashes as '?').
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:1157 UnresolvedSymlinkArtifactValue.<init>
        let mut digest = symlink
            .get_file_system()
            .get_digest_function()
            .hash_reader(&mut &*blaze_util::string_encoding::latin1_encode_lossy(
                &symlink_target,
            ))?;

        // We need to be able to tell the difference between a symlink and a file containing the
        // same text. So we transform the digest a bit. This works because if one wants to craft
        // a file with the same digest as a symlink, one would need to mount a preimage attack on
        // the digest function (this would be different if we tweaked the data before applying
        // the hash function)
        digest[0] ^= 0xff;

        Ok(FileArtifactValue::UnresolvedSymlink {
            target: symlink_target,
            digest,
        })
    }

    /// `createForTesting(Path)`.
    pub fn create_for_testing(path: &Path) -> io::Result<FileArtifactValue> {
        let stat = path.stat_with(Symlinks::Follow)?;
        FileArtifactValue::create_from_stat(path, &stat)
    }
}

impl FileArtifactValue {
    /// Returns whether the file contents have changed since the digest was computed, as far as
    /// this value can tell (`path` is the file's current location).
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:228 FileArtifactValue.wasModifiedSinceDigest
    pub fn was_modified_since_digest(&self, path: &Path) -> io::Result<bool> {
        match self {
            // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:631 RegularFileArtifactValue.wasModifiedSinceDigest
            FileArtifactValue::RegularFile { digest, proxy, .. } => {
                let Some(proxy) = proxy else {
                    return Ok(false);
                };
                let stat = path.stat_if_found(Symlinks::Follow)?;
                let Some(stat) = stat.filter(|stat| stat.is_file()) else {
                    // The file no longer exists or changed type, so it certainly has changed.
                    return Ok(true);
                };
                let new_proxy = FileContentsProxy::create(&stat);
                if *proxy == new_proxy {
                    // If the proxy is the same, then the file certainly hasn't been modified. This is
                    // the common case, so we check it first.
                    return Ok(false);
                }
                if proxy.is_modified(&new_proxy) {
                    // If the non-ctime information in the proxy changed, the file has certainly been
                    // modified between the time the digest was computed and now.
                    return Ok(true);
                }
                // At this point the ctime changed, so some of the file's metadata has changed since
                // we computed the digest. Returning true here would allow us to cautiously report
                // modification even in complex ABA scenarios (file modified, then modified back with
                // its mtime reset). However, we would also report modification in case a hardlink to
                // the file was created or removed, such as by the hermetic Linux sandbox or certain
                // optimized copy actions.
                // As a compromise, we check whether the current state of the file differs from the
                // previous one, ignoring any inbetween modifications that may have happened.
                //
                // Note that this path is always taken when using the hermetic Linux sandbox, but the
                // associated cost should amortize over the next build as the digest will be cached
                // under the new stat.
                //
                // Java: DigestUtils.getDigestWithManualFallback(path, SyscallCache.NO_CACHE, stat)
                let new_digest =
                    crate::actions::cache::digest_utils::DigestUtils::get_digest_with_manual_fallback(path)?;
                Ok(digest.as_deref() != Some(&new_digest[..]))
            }
            // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:513 DirectoryArtifactValue.wasModifiedSinceDigest
            FileArtifactValue::Directory { .. } => Ok(false),
            // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:567 HashedDirectoryArtifactValue.wasModifiedSinceDigest
            //
            // TODO(ulfjack): Ideally, we'd attempt to detect intra-build modifications here. I'm
            // consciously deferring work here as this code will most likely change again, and we're
            // already doing better than before by detecting inter-build modifications.
            FileArtifactValue::HashedDirectory { .. } => Ok(false),
            // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:274 TreeArtifactCompositeFileArtifactValue.wasModifiedSinceDigest
            FileArtifactValue::TreeArtifactComposite { .. } => Ok(false),
            // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:1213 UnresolvedSymlinkArtifactValue.wasModifiedSinceDigest
            FileArtifactValue::UnresolvedSymlink { digest, .. } => {
                match FileArtifactValue::create_for_unresolved_symlink(path) {
                    Ok(new_metadata) => Ok(Some(&digest[..]) != new_metadata.get_digest()),
                    Err(_) => Ok(true),
                }
            }
            // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:801 RemoteFileArtifactValue.wasModifiedSinceDigest
            FileArtifactValue::RemoteFile { .. } => Ok(false),
            // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:1017 ResolvedSymlinkArtifactValue.wasModifiedSinceDigest
            FileArtifactValue::ResolvedSymlink { delegate, .. } => {
                delegate.was_modified_since_digest(path)
            }
            // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:706 RunfilesProxyArtifactValue.wasModifiedSinceDigest
            FileArtifactValue::RunfilesProxy { .. } => Ok(false),
            // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:1461 ConstantMetadataValue.wasModifiedSinceDigest
            FileArtifactValue::ConstantMetadata => panic!(
                "UnsupportedOperationException: ConstantMetadataValue doesn't support wasModifiedSinceDigest {path}"
            ),
            // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:1377 ProxyFileArtifactValue.wasModifiedSinceDigest
            FileArtifactValue::Proxy { delegate, .. } => delegate.was_modified_since_digest(path),
            // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:1415 SingletonMarkerValue.wasModifiedSinceDigest
            FileArtifactValue::RunfilesTreeMarker | FileArtifactValue::MissingFileMarker => {
                Ok(false)
            }
        }
    }
}

impl fmt::Display for FileArtifactValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}
