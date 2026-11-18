//! Port of `com.google.devtools.build.lib.unix.UnixFileSystem` using `std` instead of JNI.
//!
//! Error messages follow the native layer (`unix_jni.cc` `PostException`): `<path> (<strerror>)`.
//! Not ported: xattr-based fast digests (`hashAttributeName`), the Profiler, `Blocker`.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::sync::Arc;

use crate::digest_hash_function::DigestHashFunction;
use crate::dirent::{Dirent, DirentType};
use crate::file_status::FileStatus;
use crate::file_system::{
    ERR_FILE_EXISTS, FileSymlinkLoopException, FileSystem, FileSystemRef, NotASymlinkException,
    is_not_found,
};
use crate::path::NOW_SENTINEL_TIME;
use crate::path_fragment::PathFragment;

const EEXIST: i32 = 17;
const EINVAL: i32 = 22;
const EISDIR: i32 = 21;

/// Formats an OS error the way Bazel's JNI layer does: `<message> (<strerror>)`.
// Java: (native) src/main/native/unix_jni.cc PostException
pub(crate) fn posix_error(message: &str, e: io::Error) -> io::Error {
    let text = e.to_string();
    let strerror = match text.rfind(" (os error ") {
        Some(i) if e.raw_os_error().is_some() => text[..i].to_owned(),
        _ => text,
    };
    io::Error::new(e.kind(), format!("{} ({})", message, strerror))
}

/// The real Unix file system (Java `UnixFileSystem`).
// Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:43 UnixFileSystem
#[derive(Debug)]
pub struct UnixFileSystem {
    digest_function: DigestHashFunction,
}

impl UnixFileSystem {
    /// Creates a Unix file system with the given digest function (no hash xattr).
    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:48 UnixFileSystem.<init>
    pub fn new(digest_function: DigestHashFunction) -> UnixFileSystem {
        UnixFileSystem { digest_function }
    }

    /// Creates a new instance behind a shared [`FileSystemRef`]. Each call yields a distinct file
    /// system identity.
    pub fn create(digest_function: DigestHashFunction) -> FileSystemRef {
        Arc::new(UnixFileSystem::new(digest_function))
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:200 UnixFileSystem.modifyPermissionBits
    fn modify_permission_bits(
        &self,
        path: &PathFragment,
        permission_bits: u32,
        add: bool,
    ) -> io::Result<()> {
        let old_mode = self.stat(path, true)?.get_permissions();
        let new_mode = if add {
            old_mode | permission_bits
        } else {
            old_mode & !permission_bits
        };
        self.chmod(path, new_mode)
    }
}

impl FileSystem for UnixFileSystem {
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:71 FileSystem.getDigestFunction
    fn get_digest_function(&self) -> DigestHashFunction {
        self.digest_function
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:238 UnixFileSystem.supportsModifications
    fn supports_modifications(&self, _path: &PathFragment) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:243 UnixFileSystem.supportsSymbolicLinksNatively
    fn supports_symbolic_links_natively(&self, _path: &PathFragment) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:248 UnixFileSystem.supportsHardLinksNatively
    fn supports_hard_links_natively(&self, _path: &PathFragment) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:253 UnixFileSystem.mayBeCaseOrNormalizationInsensitive
    fn may_be_case_or_normalization_insensitive(&self) -> bool {
        // OS.getCurrent() == OS.DARWIN; Blaze is Linux-only.
        false
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:258 UnixFileSystem.createDirectory
    fn create_directory(&self, path: &PathFragment) -> io::Result<bool> {
        match fs::DirBuilder::new()
            .mode(0o777)
            .create(path.get_path_string())
        {
            Ok(()) => return Ok(true), // successfully created
            Err(e) if e.raw_os_error() == Some(EEXIST) => {}
            Err(e) => return Err(posix_error(path.get_path_string(), e)),
        }
        if self.is_directory(path, false) {
            Ok(false) // directory already existed
        } else {
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{}{}", path, ERR_FILE_EXISTS),
            ))
        }
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:280 UnixFileSystem.createDirectoryAndParents
    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
        let mut dirs_to_create = Vec::new();
        let mut dir = Some(path.clone());
        while let Some(d) = dir {
            if let Some(stat) = self.stat_if_found(&d, true)? {
                if stat.is_directory() {
                    break;
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("{}{}", path, ERR_FILE_EXISTS),
                    ));
                }
            }
            dir = d.get_parent_directory();
            dirs_to_create.push(d);
        }
        while let Some(d) = dirs_to_create.pop() {
            let _ = self.create_directory(&d)?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:338 UnixFileSystem.getFileSize
    fn get_file_size(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<i64> {
        Ok(self.stat(path, follow_symlinks)?.get_size())
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:343 UnixFileSystem.delete
    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        // remove(3): unlink, or rmdir for directories.
        let name = path.get_path_string();
        let result = match fs::remove_file(name) {
            Err(e) if e.raw_os_error() == Some(EISDIR) => fs::remove_dir(name),
            r => r,
        };
        match result {
            Ok(()) => Ok(true),
            Err(e) if is_not_found(&e) => Ok(false),
            Err(e) => Err(posix_error(name, e)),
        }
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:356 UnixFileSystem.getLastModifiedTime
    fn get_last_modified_time(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<i64> {
        Ok(self.stat(path, follow_symlinks)?.get_last_modified_time())
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:361 UnixFileSystem.setLastModifiedTime
    fn set_last_modified_time(&self, path: &PathFragment, new_time: i64) -> io::Result<()> {
        // utimensat(path, {t, t}) on both atime and mtime, following symlinks. std only offers
        // futimens on an open file, so open it (read, falling back to write).
        let name = path.get_path_string();
        let file = fs::File::open(name)
            .or_else(|_| fs::OpenOptions::new().write(true).open(name))
            .map_err(|e| posix_error(name, e))?;
        let t = if new_time == NOW_SENTINEL_TIME {
            std::time::SystemTime::now()
        } else if new_time >= 0 {
            std::time::UNIX_EPOCH + std::time::Duration::from_millis(new_time as u64)
        } else {
            std::time::UNIX_EPOCH - std::time::Duration::from_millis(new_time.unsigned_abs())
        };
        file.set_times(fs::FileTimes::new().set_accessed(t).set_modified(t))
            .map_err(|e| posix_error(name, e))
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:118 UnixFileSystem.stat
    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus> {
        let name = path.get_path_string();
        let m = if follow_symlinks {
            fs::metadata(name)
        } else {
            fs::symlink_metadata(name)
        };
        m.map(|m| FileStatus::from_metadata(&m))
            .map_err(|e| posix_error(name, e))
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:164 UnixFileSystem.statIfFound
    fn stat_if_found(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<FileStatus>> {
        match self.stat(path, follow_symlinks) {
            Ok(s) => Ok(Some(s)),
            Err(e) if is_not_found(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:71 UnixFileSystem.resolveOneLink
    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>> {
        if self.stat(path, false)?.is_symbolic_link() {
            Ok(Some(self.read_symbolic_link(path)?))
        } else {
            Ok(None)
        }
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:299 UnixFileSystem.createSymbolicLink
    fn create_symbolic_link(
        &self,
        link_path: &PathFragment,
        target_fragment: &PathFragment,
    ) -> io::Result<()> {
        std::os::unix::fs::symlink(
            target_fragment.get_safe_path_string(),
            link_path.get_path_string(),
        )
        .map_err(|e| posix_error(link_path.get_path_string(), e))
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:311 UnixFileSystem.readSymbolicLink
    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment> {
        let name = path.get_path_string();
        match fs::read_link(name) {
            Ok(target) => match target.into_os_string().into_string() {
                Ok(s) => Ok(PathFragment::create(&s)),
                Err(_) => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} (symlink target is not valid UTF-8)", name),
                )),
            },
            Err(e) if e.raw_os_error() == Some(EINVAL) => {
                Err(NotASymlinkException::new_error(path))
            }
            Err(e) => Err(posix_error(name, e)),
        }
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:154 UnixFileSystem.exists
    fn exists(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.stat_nullable(path, follow_symlinks).is_some()
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:54 UnixFileSystem.getDirectoryEntries
    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>> {
        let name = path.get_path_string();
        let mut result = Vec::new();
        for entry in fs::read_dir(name).map_err(|e| posix_error(name, e))? {
            let entry = entry.map_err(|e| posix_error(name, e))?;
            result.push(entry_name(name, entry)?);
        }
        Ok(result)
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:88 UnixFileSystem.readdir
    fn readdir(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Vec<Dirent>> {
        let name = path.get_path_string();
        let mut result = Vec::new();
        for entry in fs::read_dir(name).map_err(|e| posix_error(name, e))? {
            let entry = entry.map_err(|e| posix_error(name, e))?;
            let file_type = entry.file_type().map_err(|e| posix_error(name, e))?;
            let child = entry_name(name, entry)?;
            // std resolves an unknown d_type with lstat, so "UNKNOWN" here means special file.
            let native_type = if file_type.is_file() {
                DirentType::File
            } else if file_type.is_dir() {
                DirentType::Directory
            } else if file_type.is_symlink() {
                DirentType::Symlink
            } else {
                DirentType::Unknown
            };
            let t = if native_type == DirentType::Unknown
                || (follow_symlinks && native_type == DirentType::Symlink)
            {
                match self.stat_if_found(&path.get_relative_str(&child), follow_symlinks) {
                    Ok(Some(stat)) => stat.get_dirent_type(),
                    Ok(None) => DirentType::Unknown,
                    Err(e) if FileSymlinkLoopException::is(&e) => DirentType::Unknown,
                    Err(e) => return Err(e),
                }
            } else {
                native_type
            };
            result.push(Dirent::new(child, t));
        }
        Ok(result)
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:179 UnixFileSystem.isReadable
    fn is_readable(&self, path: &PathFragment) -> io::Result<bool> {
        Ok((self.stat(path, true)?.get_permissions() & 0o400) != 0)
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:213 UnixFileSystem.setReadable
    fn set_readable(&self, path: &PathFragment, readable: bool) -> io::Result<()> {
        self.modify_permission_bits(path, 0o400, readable)
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:184 UnixFileSystem.isWritable
    fn is_writable(&self, path: &PathFragment) -> io::Result<bool> {
        Ok((self.stat(path, true)?.get_permissions() & 0o200) != 0)
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:218 UnixFileSystem.setWritable
    fn set_writable(&self, path: &PathFragment, writable: bool) -> io::Result<()> {
        self.modify_permission_bits(path, 0o200, writable)
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:189 UnixFileSystem.isExecutable
    fn is_executable(&self, path: &PathFragment) -> io::Result<bool> {
        Ok((self.stat(path, true)?.get_permissions() & 0o100) != 0)
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:223 UnixFileSystem.setExecutable
    fn set_executable(&self, path: &PathFragment, executable: bool) -> io::Result<()> {
        self.modify_permission_bits(path, 0o111, executable)
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:228 UnixFileSystem.chmod
    fn chmod(&self, path: &PathFragment, mode: u32) -> io::Result<()> {
        let name = path.get_path_string();
        fs::set_permissions(name, fs::Permissions::from_mode(mode))
            .map_err(|e| posix_error(name, e))
    }

    // Java: com/google/devtools/build/lib/vfs/DiskBackedFileSystem.java:64 DiskBackedFileSystem.getInputStream
    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        let name = path.get_path_string();
        let f = fs::File::open(name).map_err(|e| posix_error(name, e))?;
        Ok(Box::new(io::BufReader::new(f)))
    }

    // Java: com/google/devtools/build/lib/vfs/DiskBackedFileSystem.java:90 DiskBackedFileSystem.getOutputStream
    fn get_output_stream(
        &self,
        path: &PathFragment,
        append: bool,
        _internal: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        let name = path.get_path_string();
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).mode(0o666);
        if append {
            options.append(true);
        } else {
            options.truncate(true);
        }
        let f = options.open(name).map_err(|e| posix_error(name, e))?;
        Ok(Box::new(f))
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:328 UnixFileSystem.renameTo
    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()> {
        fs::rename(source_path.get_path_string(), target_path.get_path_string()).map_err(|e| {
            posix_error(
                &format!(
                    "{} -> {}",
                    source_path.get_path_string(),
                    target_path.get_path_string()
                ),
                e,
            )
        })
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:411 UnixFileSystem.createFSDependentHardLink
    fn create_fs_dependent_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        // link_common in unix_jni.cc reports the new path.
        fs::hard_link(original_path.get_path_string(), link_path.get_path_string())
            .map_err(|e| posix_error(link_path.get_path_string(), e))
    }
}

fn entry_name(dir: &str, entry: fs::DirEntry) -> io::Result<String> {
    entry.file_name().into_string().map_err(|n| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} (entry name is not valid UTF-8: {:?})", dir, n),
        )
    })
}
