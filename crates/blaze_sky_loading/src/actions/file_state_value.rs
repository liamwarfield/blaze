//! Port of `FileStateValue`, `FileStateType` and `FileContentsProxy`.
//!
//! Java: com/google/devtools/build/lib/actions/{FileStateValue,FileStateType,
//! FileContentsProxy}.java.

use std::fmt;
use std::sync::Arc;

use blaze_util::fingerprint::Fingerprint;
use blaze_util::io::timestamp_granularity_monitor::TimestampGranularityMonitor;
use blaze_vfs::{FileStatus, PathFragment, RootedPath, StatMetadata, Symlinks};

use crate::io::IOException;

/// An enum indicating the type of a path on the file system.
// Java: com/google/devtools/build/lib/actions/FileStateType.java:18 FileStateType
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileStateType {
    RegularFile,
    /// A special file such as a socket, fifo, or device. See [`FileStatus::is_special_file`].
    SpecialFile,
    Directory,
    Symlink,
    Nonexistent,
}

impl FileStateType {
    // Java: com/google/devtools/build/lib/actions/FileStateType.java:35 FileStateType.getHumanReadableName
    pub fn get_human_readable_name(self) -> &'static str {
        match self {
            FileStateType::RegularFile => "file",
            FileStateType::SpecialFile => "special file",
            FileStateType::Directory => "directory",
            FileStateType::Symlink => "symlink",
            FileStateType::Nonexistent => "non-existent path",
        }
    }

    /// Returns true if this type does not correspond to a non-existent path.
    // Java: com/google/devtools/build/lib/actions/FileStateType.java:40 FileStateType.exists
    pub fn exists(self) -> bool {
        self != FileStateType::Nonexistent
    }

    /// Returns true if this value corresponds to a symlink.
    // Java: com/google/devtools/build/lib/actions/FileStateType.java:45 FileStateType.isSymlink
    pub fn is_symlink(self) -> bool {
        self == FileStateType::Symlink
    }

    /// Returns true if this value corresponds to a regular file. If so, its parent directory is
    /// guaranteed to exist.
    // Java: com/google/devtools/build/lib/actions/FileStateType.java:53 FileStateType.isFile
    pub fn is_file(self) -> bool {
        self == FileStateType::RegularFile
    }

    /// Returns true if this value corresponds to a special file. If so, its parent directory is
    /// guaranteed to exist.
    // Java: com/google/devtools/build/lib/actions/FileStateType.java:61 FileStateType.isSpecialFile
    pub fn is_special_file(self) -> bool {
        self == FileStateType::SpecialFile
    }

    /// Returns true if the file is a directory. If so, its parent directory is guaranteed to
    /// exist.
    // Java: com/google/devtools/build/lib/actions/FileStateType.java:68 FileStateType.isDirectory
    pub fn is_directory(self) -> bool {
        self == FileStateType::Directory
    }
}

/// In case we can't get a fast digest from the filesystem, we store this metadata as a proxy to
/// the file contents. Currently it is a pair of a relevant timestamp and a "node id". On Linux
/// the former is the ctime and the latter is the inode number. We might want to add the device
/// number in the future.
// Java: com/google/devtools/build/lib/actions/FileContentsProxy.java:35 FileContentsProxy
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileContentsProxy {
    ctime: i64,
    mtime: i64,
    node_id: i64,
}

impl FileContentsProxy {
    // Java: com/google/devtools/build/lib/actions/FileContentsProxy.java:40 FileContentsProxy.<init>
    pub fn new(ctime: i64, mtime: i64, node_id: i64) -> FileContentsProxy {
        FileContentsProxy {
            ctime,
            mtime,
            node_id,
        }
    }

    // Java: com/google/devtools/build/lib/actions/FileContentsProxy.java:46 FileContentsProxy.create
    pub fn create(stat: &FileStatus) -> FileContentsProxy {
        FileContentsProxy::new(
            // Note: there are file systems that return mtime for getLastChangeTime() instead of
            // ctime, such as the JavaIoFileSystem.
            stat.get_last_change_time(),
            stat.get_last_modified_time(),
            stat.get_node_id(),
        )
    }

    /// The change time (milliseconds).
    pub fn get_ctime(&self) -> i64 {
        self.ctime
    }

    /// The modification time (milliseconds).
    pub fn get_mtime(&self) -> i64 {
        self.mtime
    }

    /// The node id (inode number).
    pub fn get_node_id(&self) -> i64 {
        self.node_id
    }

    /// Can be used when hardlink reference counter changes should not be considered a file
    /// modification. Is only comparing mtime and not ctime and is therefore not detecting changed
    /// metadata like permission.
    // Java: com/google/devtools/build/lib/actions/FileContentsProxy.java:72 FileContentsProxy.isModified
    pub fn is_modified(&self, other: &FileContentsProxy) -> bool {
        // true if nodeId are different or inode has a new mtime
        self.node_id != other.node_id || self.mtime != other.mtime
    }

    /// PORT: package-private in Java; public here for `FileStateValue.getValueFingerprint`'s
    /// callers in other crates.
    // Java: com/google/devtools/build/lib/actions/FileContentsProxy.java:86 FileContentsProxy.addToFingerprint
    pub fn add_to_fingerprint(&self, fp: &mut Fingerprint) {
        fp.add_long(self.ctime);
        fp.add_long(self.mtime);
        fp.add_long(self.node_id);
    }

    // Java: com/google/devtools/build/lib/actions/FileContentsProxy.java:96 FileContentsProxy.prettyPrint
    pub fn pretty_print(&self) -> String {
        format!(
            "ctime of {} and mtime of {} and nodeId of {}",
            self.ctime, self.mtime, self.node_id
        )
    }
}

impl fmt::Display for FileContentsProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.pretty_print())
    }
}

/// The result of [`FileStateValue::compare_contents`].
// Java: com/google/devtools/build/lib/actions/FileStateValue.java:230 FileStateValue.ContentsComparisonResult
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentsComparisonResult {
    /// Both values describe the same file contents and can be used interchangeably, even if not
    /// equal.
    Same,
    /// Both values have identical metadata except for ctime, which may have changed due to
    /// hardlinks.
    SameExceptChangeTime,
    /// The values may describe different contents.
    Different,
}

/// Encapsulates the filesystem operations needed to get state for a path. This is equivalent to
/// an 'lstat' that does not follow symlinks to determine what type of file the path is.
///
/// Java's `FileStateValue` is also a `RegularFileValue` (a `FileValue` for a path whose real
/// path is itself); here that is [`crate::actions::FileValue::Regular`].
// Java: com/google/devtools/build/lib/actions/FileStateValue.java:67 FileStateValue
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FileStateValue {
    /// Regular files when a digest is provided.
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:303 FileStateValue.RegularFileStateValueWithDigest
    RegularFileWithDigest { size: i64, digest: Vec<u8> },
    /// Regular files when a [`FileContentsProxy`] is provided.
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:375 FileStateValue.RegularFileStateValueWithContentsProxy
    RegularFileWithContentsProxy {
        size: i64,
        contents_proxy: FileContentsProxy,
    },
    /// Regular files when its metadata is backed by a `FileArtifactValue` (the metadata of a
    /// `FileStatusWithMetadata`).
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:448 FileStateValue.RegularFileStateValueWithMetadata
    RegularFileWithMetadata(FileStateMetadata),
    /// Special files that exist.
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:517 FileStateValue.SpecialFileStateValue
    SpecialFile { contents_proxy: FileContentsProxy },
    /// Directories that exist.
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:586 FileStateValue.DirectoryFileStateValue
    Directory,
    /// Symlinks.
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:625 FileStateValue.SymlinkFileStateValue
    Symlink { symlink_target: PathFragment },
    /// Nonexistent files.
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:675 FileStateValue.NonexistentFileStateValue
    Nonexistent,
}

/// The `FileArtifactValue` held by [`FileStateValue::RegularFileWithMetadata`], with its size and
/// digest read once (they do not change; the contents proxy, which does, is read on each call).
#[derive(Clone, Debug)]
pub struct FileStateMetadata {
    metadata: Arc<dyn StatMetadata>,
    size: i64,
    digest: Option<Vec<u8>>,
}

impl FileStateMetadata {
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:452 RegularFileStateValueWithMetadata.<init>
    pub fn new(metadata: Arc<dyn StatMetadata>) -> FileStateMetadata {
        FileStateMetadata {
            size: metadata.get_size(),
            digest: metadata.get_digest(),
            metadata,
        }
    }

    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:477 RegularFileStateValueWithMetadata.getMetadata
    pub fn get_metadata(&self) -> &Arc<dyn StatMetadata> {
        &self.metadata
    }
}

// Java: com/google/devtools/build/lib/actions/FileStateValue.java:481 RegularFileStateValueWithMetadata.equals
impl PartialEq for FileStateMetadata {
    fn eq(&self, other: &FileStateMetadata) -> bool {
        Arc::ptr_eq(&self.metadata, &other.metadata) || other.metadata.eq_dyn(&*self.metadata)
    }
}

impl Eq for FileStateMetadata {}

// Java: com/google/devtools/build/lib/actions/FileStateValue.java:492 RegularFileStateValueWithMetadata.hashCode
impl std::hash::Hash for FileStateMetadata {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.metadata.hash_dyn(state);
    }
}

impl FileStateValue {
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:69 FileStateValue.DIRECTORY_FILE_STATE_NODE
    pub const DIRECTORY_FILE_STATE_NODE: FileStateValue = FileStateValue::Directory;
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:73 FileStateValue.NONEXISTENT_FILE_STATE_NODE
    pub const NONEXISTENT_FILE_STATE_NODE: FileStateValue = FileStateValue::Nonexistent;

    /// Creates the state of `rooted_path` from the file system, without a syscall cache
    /// (Java `SyscallCache.NO_CACHE`, which skips `getType` and stats).
    ///
    /// PORT: the `SyscallCache` parameter is not ported; Blaze always stats (Java's
    /// `DefaultSyscallCache` only caches the same answers). Java's `tsgm` is null here (see
    /// [`FileStateValue::create_with_tsgm`]).
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:78 FileStateValue.create
    pub fn create(rooted_path: &RootedPath) -> Result<FileStateValue, IOException> {
        Self::create_with_tsgm(rooted_path, None)
    }

    /// [`FileStateValue::create`] with Java's `TimestampGranularityMonitor` argument: a regular
    /// file without a digest notifies the monitor of its change time.
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:78 FileStateValue.create
    pub fn create_with_tsgm(
        rooted_path: &RootedPath,
        tsgm: Option<&TimestampGranularityMonitor>,
    ) -> Result<FileStateValue, IOException> {
        let path = rooted_path.as_path();
        // NO_CACHE.getType returns FILESYSTEM_OP_SKIPPED.
        let stat = path
            .stat_if_found(Symlinks::Nofollow)
            .map_err(|e| IOException::from_io(&e))?;
        let Some(stat) = stat else {
            return Ok(FileStateValue::NONEXISTENT_FILE_STATE_NODE);
        };
        // Java: SyscallCache.statusToDirentType
        if stat.is_file() {
            Self::create_with_stat_no_follow_and_tsgm(rooted_path, &stat, tsgm)
        } else if stat.is_directory() {
            Ok(FileStateValue::DIRECTORY_FILE_STATE_NODE)
        } else if stat.is_symbolic_link() {
            Ok(FileStateValue::Symlink {
                symlink_target: path
                    .read_symbolic_link_unchecked()
                    .map_err(|e| IOException::from_io(&e))?,
            })
        } else {
            // UNKNOWN
            Self::create_with_stat_no_follow_and_tsgm(rooted_path, &stat, tsgm)
        }
    }

    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:114 FileStateValue.createWithStatNoFollow
    pub fn create_with_stat_no_follow(
        rooted_path: &RootedPath,
        stat_no_follow: &FileStatus,
    ) -> Result<FileStateValue, IOException> {
        Self::create_with_stat_no_follow_and_tsgm(rooted_path, stat_no_follow, None)
    }

    /// [`FileStateValue::create_with_stat_no_follow`] with Java's `tsgm` argument.
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:114 FileStateValue.createWithStatNoFollow
    pub fn create_with_stat_no_follow_and_tsgm(
        rooted_path: &RootedPath,
        stat_no_follow: &FileStatus,
        tsgm: Option<&TimestampGranularityMonitor>,
    ) -> Result<FileStateValue, IOException> {
        let path = rooted_path.as_path();
        if stat_no_follow.is_file() {
            return if stat_no_follow.is_special_file() {
                // Java: com/google/devtools/build/lib/actions/FileStateValue.java:525 SpecialFileStateValue.fromStat
                if let Some(tsgm) = tsgm {
                    tsgm.notify_dependence_on_file_time(stat_no_follow.get_last_change_time());
                }
                Ok(FileStateValue::SpecialFile {
                    contents_proxy: FileContentsProxy::create(stat_no_follow),
                })
            } else {
                Self::create_regular_file_state_value_from_path(&path, stat_no_follow, tsgm)
            };
        } else if stat_no_follow.is_directory() {
            return Ok(FileStateValue::DIRECTORY_FILE_STATE_NODE);
        } else if stat_no_follow.is_symbolic_link() {
            return Ok(FileStateValue::Symlink {
                symlink_target: path
                    .read_symbolic_link_unchecked()
                    .map_err(|e| IOException::from_io(&e))?,
            });
        }
        Err(IOException::inconsistent_filesystem(format!(
            "according to stat, existing path {} is neither a file nor directory nor symlink.",
            path
        )))
    }

    /// Creates a value corresponding to the given existing file.
    ///
    /// We use digests only if a fast digest lookup is available from the filesystem. If not, we
    /// fall back to mtime-based digests. This avoids the case where Blaze must read all files
    /// involved in the build in order to check for modifications in the case where fast digest
    /// lookups are not available.
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:147 FileStateValue.createRegularFileStateValueFromPath
    fn create_regular_file_state_value_from_path(
        path: &blaze_vfs::Path,
        stat: &FileStatus,
        tsgm: Option<&TimestampGranularityMonitor>,
    ) -> Result<FileStateValue, IOException> {
        blaze_util::check_state!(stat.is_file(), "{}", path);
        // Java: `stat instanceof FileStatusWithMetadata`. PORT: the metadata is asked of the file
        // system separately (see `blaze_vfs::StatMetadata`).
        if let Some(metadata) = path
            .stat_metadata(Symlinks::Nofollow)
            .map_err(|e| IOException::from_io(&e))?
        {
            return Ok(FileStateValue::RegularFileWithMetadata(
                FileStateMetadata::new(metadata),
            ));
        }
        match Self::try_get_digest(path) {
            Ok(None) => {
                // Note that TimestampGranularityMonitor#notifyDependenceOnFileTime is a
                // thread-safe method.
                if let Some(tsgm) = tsgm {
                    tsgm.notify_dependence_on_file_time(stat.get_last_change_time());
                }
                Ok(FileStateValue::RegularFileWithContentsProxy {
                    size: stat.get_size(),
                    contents_proxy: FileContentsProxy::create(stat),
                })
            }
            // We are careful here to avoid putting the value ID into FileMetadata if we already
            // have a digest. Arbitrary filesystems may do weird things with the value ID; a digest
            // is more robust.
            Ok(Some(digest)) => Ok(FileStateValue::RegularFileWithDigest {
                size: stat.get_size(),
                digest,
            }),
            Err(e) => {
                let error_message = format!("error '{}'", e);
                Err(IOException::inconsistent_filesystem(format!(
                    "'stat' said {} is a file but then we later encountered {} which indicates \
                     that {} is no longer a file. Did you delete it during the build?",
                    path, error_message, path
                )))
            }
        }
    }

    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:189 FileStateValue.tryGetDigest
    fn try_get_digest(path: &blaze_vfs::Path) -> std::io::Result<Option<Vec<u8>>> {
        // A plain FileStatus has no digest (FileStatusWithDigestAdapter.getDigest() is null), so
        // ask the file system for a fast digest.
        match path.get_fast_digest() {
            Ok(d) => Ok(d),
            Err(ioe) => {
                if !path.is_readable()? {
                    return Ok(None);
                }
                Err(ioe)
            }
        }
    }

    /// Compares the file contents described by this value and `other` regardless of how each
    /// value represents them, using either a digest or a proxy.
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:249 FileStateValue.compareContents
    pub fn compare_contents(&self, other: &FileStateValue) -> ContentsComparisonResult {
        if self == other {
            return ContentsComparisonResult::Same;
        }
        if self.get_type() != other.get_type() {
            return ContentsComparisonResult::Different;
        }
        match self.get_type() {
            FileStateType::RegularFile | FileStateType::SpecialFile => {
                if self.get_size() != other.get_size() {
                    return ContentsComparisonResult::Different;
                }
                // Prefer comparing digests, the most robust signal, when both values have one.
                if let (Some(this_digest), Some(other_digest)) =
                    (self.get_digest(), other.get_digest())
                {
                    return if this_digest == other_digest {
                        ContentsComparisonResult::Same
                    } else {
                        ContentsComparisonResult::Different
                    };
                }
                let (Some(this_proxy), Some(other_proxy)) =
                    (self.get_contents_proxy(), other.get_contents_proxy())
                else {
                    return ContentsComparisonResult::Different;
                };
                if this_proxy == other_proxy {
                    return ContentsComparisonResult::Same;
                }
                if this_proxy.is_modified(&other_proxy) {
                    ContentsComparisonResult::Different
                } else {
                    ContentsComparisonResult::SameExceptChangeTime
                }
            }
            // Java: case SYMLINK, DIRECTORY, NONEXISTENT -> DIFFERENT (equals was false).
            _ => ContentsComparisonResult::Different,
        }
    }

    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:214 FileStateValue.getType
    pub fn get_type(&self) -> FileStateType {
        match self {
            FileStateValue::RegularFileWithDigest { .. }
            | FileStateValue::RegularFileWithContentsProxy { .. }
            | FileStateValue::RegularFileWithMetadata(_) => FileStateType::RegularFile,
            FileStateValue::SpecialFile { .. } => FileStateType::SpecialFile,
            FileStateValue::Directory => FileStateType::Directory,
            FileStateValue::Symlink { .. } => FileStateType::Symlink,
            FileStateValue::Nonexistent => FileStateType::Nonexistent,
        }
    }

    /// Returns the target of the symlink, or panics if this is not a symlink.
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:217 FileStateValue.getSymlinkTarget
    pub fn get_symlink_target(&self) -> &PathFragment {
        match self {
            FileStateValue::Symlink { symlink_target } => symlink_target,
            _ => panic!("IllegalStateException"),
        }
    }

    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:222 FileStateValue.getSize
    pub fn get_size(&self) -> i64 {
        match self {
            FileStateValue::RegularFileWithDigest { size, .. }
            | FileStateValue::RegularFileWithContentsProxy { size, .. } => *size,
            FileStateValue::RegularFileWithMetadata(m) => m.size,
            FileStateValue::SpecialFile { .. } => 0,
            _ => panic!("IllegalStateException"),
        }
    }

    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:287 FileStateValue.getDigest
    pub fn get_digest(&self) -> Option<&[u8]> {
        match self {
            FileStateValue::RegularFileWithDigest { digest, .. } => Some(digest),
            FileStateValue::RegularFileWithMetadata(m) => m.digest.as_deref(),
            FileStateValue::RegularFileWithContentsProxy { .. }
            | FileStateValue::SpecialFile { .. } => None,
            _ => panic!("IllegalStateException"),
        }
    }

    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:227 FileStateValue.getContentsProxy
    pub fn get_contents_proxy(&self) -> Option<FileContentsProxy> {
        match self {
            FileStateValue::RegularFileWithDigest { .. } | FileStateValue::Symlink { .. } => None,
            FileStateValue::RegularFileWithContentsProxy { contents_proxy, .. }
            | FileStateValue::SpecialFile { contents_proxy } => Some(*contents_proxy),
            FileStateValue::RegularFileWithMetadata(m) => m
                .metadata
                .get_contents_proxy()
                .map(|(ctime, mtime, node_id)| FileContentsProxy::new(ctime, mtime, node_id)),
            FileStateValue::Directory | FileStateValue::Nonexistent => {
                panic!("UnsupportedOperationException")
            }
        }
    }

    /// Returns a fingerprint of this value (Java: the abstract method and its implementations).
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:291 FileStateValue.getValueFingerprint
    pub fn get_value_fingerprint(&self) -> Vec<u8> {
        match self {
            // Java: com/google/devtools/build/lib/actions/FileStateValue.java:351 RegularFileStateValueWithDigest.getValueFingerprint
            FileStateValue::RegularFileWithDigest { size, digest } => {
                let mut fp = Fingerprint::new();
                fp.add_long(*size);
                fp.add_bytes(digest);
                fp.digest_and_reset()
            }
            // Java: com/google/devtools/build/lib/actions/FileStateValue.java:423 RegularFileStateValueWithContentsProxy.getValueFingerprint
            FileStateValue::RegularFileWithContentsProxy {
                size,
                contents_proxy,
            } => {
                let mut fp = Fingerprint::new();
                fp.add_long(*size);
                contents_proxy.add_to_fingerprint(&mut fp);
                fp.digest_and_reset()
            }
            // Java: com/google/devtools/build/lib/actions/FileStateValue.java:497 RegularFileStateValueWithMetadata.getValueFingerprint
            FileStateValue::RegularFileWithMetadata(m) => {
                let mut fp = Fingerprint::new();
                fp.add_long(m.size);
                // Java: Fingerprint.addBytes(null) throws; a remote file always has a digest.
                fp.add_bytes(m.digest.as_deref().expect("NullPointerException"));
                fp.digest_and_reset()
            }
            // Java: com/google/devtools/build/lib/actions/FileStateValue.java:573 SpecialFileStateValue.getValueFingerprint
            FileStateValue::SpecialFile { contents_proxy } => {
                let mut fp = Fingerprint::new();
                contents_proxy.add_to_fingerprint(&mut fp);
                fp.digest_and_reset()
            }
            // Java: com/google/devtools/build/lib/actions/FileStateValue.java:618 DirectoryFileStateValue.getValueFingerprint
            FileStateValue::Directory => b"DirectoryFileStateValue".to_vec(),
            // Java: com/google/devtools/build/lib/actions/FileStateValue.java:664 SymlinkFileStateValue.getValueFingerprint
            FileStateValue::Symlink { symlink_target } => {
                let mut fp = Fingerprint::new();
                // Java: Fingerprint.addPath(PathFragment) = addString(getPathString()).
                fp.add_string(symlink_target.get_path_string());
                fp.digest_and_reset()
            }
            // Java: com/google/devtools/build/lib/actions/FileStateValue.java:710 NonexistentFileStateValue.getValueFingerprint
            FileStateValue::Nonexistent => b"NonexistentFileStateValue".to_vec(),
        }
    }

    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:298 FileStateValue.prettyPrint
    pub fn pretty_print(&self) -> String {
        match self {
            FileStateValue::RegularFileWithDigest { size, digest } => {
                // Java: Arrays.toString(byte[]) prints signed bytes.
                let bytes: Vec<String> = digest.iter().map(|b| (*b as i8).to_string()).collect();
                format!(
                    "regular file with size of {} and digest of [{}]",
                    size,
                    bytes.join(", ")
                )
            }
            FileStateValue::RegularFileWithContentsProxy {
                size,
                contents_proxy,
            } => format!(
                "regular file with size of {} and {}",
                size,
                contents_proxy.pretty_print()
            ),
            // Java: com/google/devtools/build/lib/actions/FileStateValue.java:509 RegularFileStateValueWithMetadata.prettyPrint
            // PORT: Java prints the FileArtifactValue's toString; Blaze prints its Debug form.
            FileStateValue::RegularFileWithMetadata(m) => {
                format!("regular file with size of {} and {:?}", m.size, m.metadata)
            }
            FileStateValue::SpecialFile { contents_proxy } => {
                format!("special file with {}", contents_proxy.pretty_print())
            }
            FileStateValue::Directory => "directory".to_owned(),
            FileStateValue::Symlink { symlink_target } => format!("symlink to {}", symlink_target),
            FileStateValue::Nonexistent => "nonexistent path".to_owned(),
        }
    }
}

// Java: com/google/devtools/build/lib/actions/FileStateValue.java:293 FileStateValue.toString
impl fmt::Display for FileStateValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Java overrides toString for these two with MoreObjects.toStringHelper.
            // PORT: Java prints the digest array's identity hash ("[B@1b2c3d"); Blaze prints the
            // prettyPrint form of the digest instead.
            FileStateValue::RegularFileWithDigest { size, .. } => write!(
                f,
                "RegularFileStateValueWithDigest{{digest={}, size={}}}",
                self.pretty_print(),
                size
            ),
            FileStateValue::RegularFileWithContentsProxy {
                size,
                contents_proxy,
            } => write!(
                f,
                "RegularFileStateValueWithContentsProxy{{size={}, contentsProxy={}}}",
                size, contents_proxy
            ),
            // Java: com/google/devtools/build/lib/actions/FileStateValue.java:504 RegularFileStateValueWithMetadata.toString
            FileStateValue::RegularFileWithMetadata(m) => write!(
                f,
                "RegularFileStateValueWithMetadata{{metadata={:?}}}",
                m.metadata
            ),
            _ => f.write_str(&self.pretty_print()),
        }
    }
}

#[cfg(test)]
mod regular_file_with_metadata_tests {
    //! Blaze-only tests (Java has no unit test of RegularFileStateValueWithMetadata).
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Remote {
        digest: Vec<u8>,
        size: i64,
    }

    impl StatMetadata for Remote {
        fn get_size(&self) -> i64 {
            self.size
        }
        fn get_digest(&self) -> Option<Vec<u8>> {
            Some(self.digest.clone())
        }
        fn get_contents_proxy(&self) -> Option<(i64, i64, i64)> {
            None
        }
        fn eq_dyn(&self, other: &dyn StatMetadata) -> bool {
            other.as_any().downcast_ref::<Remote>() == Some(self)
        }
        fn hash_dyn(&self, mut state: &mut dyn std::hash::Hasher) {
            use std::hash::Hash;
            self.digest.hash(&mut state);
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    fn value(digest: &[u8], size: i64) -> FileStateValue {
        FileStateValue::RegularFileWithMetadata(FileStateMetadata::new(Arc::new(Remote {
            digest: digest.to_vec(),
            size,
        })))
    }

    #[test]
    fn values_come_from_the_metadata() {
        let v = value(&[1, 2], 7);
        assert_eq!(v.get_type(), FileStateType::RegularFile);
        assert_eq!(v.get_size(), 7);
        assert_eq!(v.get_digest(), Some(&[1u8, 2][..]));
        assert_eq!(v.get_contents_proxy(), None);
        assert_eq!(v, value(&[1, 2], 7));
        assert_ne!(v, value(&[1, 3], 7));
        let mut fp = Fingerprint::new();
        fp.add_long(7);
        fp.add_bytes(&[1, 2]);
        assert_eq!(v.get_value_fingerprint(), fp.digest_and_reset());
        assert_eq!(
            v.compare_contents(&FileStateValue::RegularFileWithDigest {
                size: 7,
                digest: vec![1, 2]
            }),
            ContentsComparisonResult::Same
        );
    }
}
