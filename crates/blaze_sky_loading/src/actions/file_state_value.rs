//! Port of `FileStateValue`, `FileStateType` and `FileContentsProxy`.
//!
//! Java: com/google/devtools/build/lib/actions/{FileStateValue,FileStateType,
//! FileContentsProxy}.java. Not ported: `RegularFileStateValueWithMetadata` (needs
//! `FileArtifactValue`, only produced by `FileStatusWithMetadata` file systems such as remote
//! output services), `compareContents` (PORT: skipped, incremental),
//! `TimestampGranularityMonitor` notifications (PORT: skipped, incremental).

use std::fmt;

use blaze_util::fingerprint::Fingerprint;
use blaze_vfs::{FileStatus, PathFragment, RootedPath, Symlinks};

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

impl FileStateValue {
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:69 FileStateValue.DIRECTORY_FILE_STATE_NODE
    pub const DIRECTORY_FILE_STATE_NODE: FileStateValue = FileStateValue::Directory;
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:73 FileStateValue.NONEXISTENT_FILE_STATE_NODE
    pub const NONEXISTENT_FILE_STATE_NODE: FileStateValue = FileStateValue::Nonexistent;

    /// Creates the state of `rooted_path` from the file system, without a syscall cache
    /// (Java `SyscallCache.NO_CACHE`, which skips `getType` and stats).
    ///
    /// PORT: the `SyscallCache` parameter is not ported; Blaze always stats (Java's
    /// `DefaultSyscallCache` only caches the same answers). The `TimestampGranularityMonitor` is
    /// skipped (incremental).
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:78 FileStateValue.create
    pub fn create(rooted_path: &RootedPath) -> Result<FileStateValue, IOException> {
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
            Self::create_with_stat_no_follow(rooted_path, &stat)
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
            Self::create_with_stat_no_follow(rooted_path, &stat)
        }
    }

    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:114 FileStateValue.createWithStatNoFollow
    pub fn create_with_stat_no_follow(
        rooted_path: &RootedPath,
        stat_no_follow: &FileStatus,
    ) -> Result<FileStateValue, IOException> {
        let path = rooted_path.as_path();
        if stat_no_follow.is_file() {
            return if stat_no_follow.is_special_file() {
                // Java: SpecialFileStateValue.fromStat
                Ok(FileStateValue::SpecialFile {
                    contents_proxy: FileContentsProxy::create(stat_no_follow),
                })
            } else {
                Self::create_regular_file_state_value_from_path(&path, stat_no_follow)
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
    ) -> Result<FileStateValue, IOException> {
        blaze_util::check_state!(stat.is_file(), "{}", path);
        // PORT: `FileStatusWithMetadata` (RegularFileStateValueWithMetadata) is never produced by
        // Blaze's file systems.
        match Self::try_get_digest(path) {
            Ok(None) => Ok(FileStateValue::RegularFileWithContentsProxy {
                size: stat.get_size(),
                contents_proxy: FileContentsProxy::create(stat),
            }),
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

    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:214 FileStateValue.getType
    pub fn get_type(&self) -> FileStateType {
        match self {
            FileStateValue::RegularFileWithDigest { .. }
            | FileStateValue::RegularFileWithContentsProxy { .. } => FileStateType::RegularFile,
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
            FileStateValue::SpecialFile { .. } => 0,
            _ => panic!("IllegalStateException"),
        }
    }

    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:287 FileStateValue.getDigest
    pub fn get_digest(&self) -> Option<&[u8]> {
        match self {
            FileStateValue::RegularFileWithDigest { digest, .. } => Some(digest),
            FileStateValue::RegularFileWithContentsProxy { .. }
            | FileStateValue::SpecialFile { .. } => None,
            _ => panic!("IllegalStateException"),
        }
    }

    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:227 FileStateValue.getContentsProxy
    pub fn get_contents_proxy(&self) -> Option<&FileContentsProxy> {
        match self {
            FileStateValue::RegularFileWithDigest { .. } | FileStateValue::Symlink { .. } => None,
            FileStateValue::RegularFileWithContentsProxy { contents_proxy, .. }
            | FileStateValue::SpecialFile { contents_proxy } => Some(contents_proxy),
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
            _ => f.write_str(&self.pretty_print()),
        }
    }
}
