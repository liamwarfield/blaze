//! Port of `com.google.devtools.build.lib.unix.UnixFileSystem` using `std` instead of JNI.
//!
//! Error messages follow the native layer (`unix_jni.cc` `PostException`): `<path> (<strerror>)`.
//!
//! Paths, directory entry names and symlink targets are Bazel's internal strings (one char per
//! byte): each system call gets the path's bytes ([`os`], Java
//! `StringEncoding.internalToPlatform`), and names that come back are decoded one char per byte
//! ([`from_os`], `StringEncoding.platformToInternal`). A file name that is not UTF-8 is therefore
//! an ordinary name, as in Bazel.
//! Not ported: the Profiler (except the `VFS_MD5` and `VFS_XATTR` tasks of `getDigest` and
//! `getxattr`), `Blocker`.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
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

/// The platform string of an internal path string: one byte per char.
///
/// A path string that is not internal (a char above U+00FF: "not well-formed but must be
/// tolerated") gets `?` for each UTF-16 unit above 0xFF, as the native layer writes it.
// Java: (native) src/main/native/latin1_jni_path.cc GetStringLatin1Chars
// PORT: the Java side reports such a path once (`NativePosixFiles.logBadPath`, a non-fatal bug
// report); not ported.
fn os(path: &str) -> std::borrow::Cow<'_, std::ffi::OsStr> {
    use std::borrow::Cow;
    use std::ffi::{OsStr, OsString};
    use std::os::unix::ffi::OsStringExt;
    if path.is_ascii() {
        return Cow::Borrowed(OsStr::new(path));
    }
    let mut bytes = Vec::with_capacity(path.len());
    for c in path.chars() {
        if (c as u32) <= 0xff {
            bytes.push(c as u8);
        } else {
            bytes.extend(std::iter::repeat_n(b'?', c.len_utf16()));
        }
    }
    Cow::Owned(OsString::from_vec(bytes))
}

/// The internal string of a platform string (a directory entry, a symlink target).
// Java: com/google/devtools/build/lib/util/StringEncoding.java:115 StringEncoding.platformToInternal
fn from_os(name: &std::ffi::OsStr) -> String {
    blaze_util::string_encoding::platform_to_internal(name).into_owned()
}

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
    // The native layer throws `FileSymlinkLoopException` for ELOOP.
    if e.raw_os_error() == Some(crate::file_system::ELOOP) {
        return FileSymlinkLoopException::with_message(format!("{} ({})", message, strerror));
    }
    io::Error::new(e.kind(), format!("{} ({})", message, strerror))
}

/// `NativePosixFiles.getxattr` / `lgetxattr` (`getxattr_common` with a 4096-byte buffer):
/// `Ok(None)` when the attribute does not exist (ENODATA); the raw OS error otherwise.
// Java: (native) src/main/native/unix_jni.cc getxattr_common
fn native_getxattr(path: &str, name: &str, follow_symlinks: bool) -> io::Result<Option<Vec<u8>>> {
    let invalid = || io::Error::from_raw_os_error(libc::EINVAL);
    let c_path = std::ffi::CString::new(os(path).as_bytes()).map_err(|_| invalid())?;
    let c_name = std::ffi::CString::new(os(name).as_bytes()).map_err(|_| invalid())?;
    let mut value = [0u8; 4096];
    loop {
        // SAFETY: both strings are NUL-terminated and `value` is a writable buffer of the given
        // length.
        let size = unsafe {
            if follow_symlinks {
                libc::getxattr(
                    c_path.as_ptr(),
                    c_name.as_ptr(),
                    value.as_mut_ptr().cast(),
                    value.len(),
                )
            } else {
                libc::lgetxattr(
                    c_path.as_ptr(),
                    c_name.as_ptr(),
                    value.as_mut_ptr().cast(),
                    value.len(),
                )
            }
        };
        if size >= 0 {
            return Ok(Some(value[..size as usize].to_vec()));
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            // RESTARTABLE
            Some(libc::EINTR) => continue,
            // portable_getxattr: attr_not_found = (errno == ENODATA)
            Some(libc::ENODATA) => return Ok(None),
            _ => return Err(e),
        }
    }
}

/// The real Unix file system (Java `UnixFileSystem`).
// Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:43 UnixFileSystem
#[derive(Debug)]
pub struct UnixFileSystem {
    digest_function: DigestHashFunction,
    /// `hashAttributeName` (`--unix_digest_hash_attribute_name`; empty: none).
    hash_attribute_name: String,
}

impl UnixFileSystem {
    /// Creates a Unix file system with the given digest function (no hash xattr).
    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:48 UnixFileSystem.<init>
    pub fn new(digest_function: DigestHashFunction) -> UnixFileSystem {
        UnixFileSystem::with_hash_attribute_name(digest_function, String::new())
    }

    /// `new UnixFileSystem(hashFunction, hashAttributeName)`: the digest of a file may come from
    /// its extended attribute `hash_attribute_name` (see [`FileSystem::get_fast_digest`]).
    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:48 UnixFileSystem.<init>
    pub fn with_hash_attribute_name(
        digest_function: DigestHashFunction,
        hash_attribute_name: String,
    ) -> UnixFileSystem {
        UnixFileSystem {
            digest_function,
            hash_attribute_name,
        }
    }

    /// The value of the extended attribute `name` of `path`, `None` when it has none or the file
    /// system does not support extended attributes.
    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:372 UnixFileSystem.getxattr
    pub fn getxattr(
        &self,
        path: &PathFragment,
        name: &str,
        follow_symlinks: bool,
    ) -> io::Result<Option<Vec<u8>>> {
        let path_name = path.get_path_string();
        let start_time = blaze_util::profiler::profiler::Profiler::nano_time_maybe();
        let result = native_getxattr(path_name, name, follow_symlinks);
        blaze_util::profiler::profiler::Profiler::instance().log_simple_task(
            start_time,
            blaze_util::profiler::profiler_task::ProfilerTask::VFS_XATTR,
            path_name,
        );
        match result {
            // getxattr() syscall is not supported by the underlying filesystem (it returned
            // ENOTSUP). Per method contract, treat this as ENODATA.
            Err(e) if e.raw_os_error() == Some(libc::ENOTSUP) => Ok(None),
            Err(e) => Err(posix_error(path_name, e)),
            Ok(value) => Ok(value),
        }
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

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:393 UnixFileSystem.getFastDigest
    fn get_fast_digest(&self, path: &PathFragment) -> io::Result<Option<Vec<u8>>> {
        // Attempt to obtain the digest from an extended attribute attached to the file. This is
        // much faster than reading and digesting the file's contents on the fly, especially for
        // large files.
        if self.hash_attribute_name.is_empty() {
            return Ok(None);
        }
        self.getxattr(path, &self.hash_attribute_name, true)
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:400 UnixFileSystem.getDigest
    fn get_digest(&self, path: &PathFragment) -> io::Result<Vec<u8>> {
        let name = path.get_path_string();
        let start_time = blaze_util::profiler::profiler::Profiler::nano_time_maybe();
        // super.getDigest(path)
        // Java: com/google/devtools/build/lib/vfs/FileSystem.java:367 FileSystem.getDigest
        let result = self
            .get_input_stream(path)
            .and_then(|mut input| self.get_digest_function().hash_reader(&mut input));
        blaze_util::profiler::profiler::Profiler::instance().log_simple_task(
            start_time,
            blaze_util::profiler::profiler_task::ProfilerTask::VFS_MD5,
            name,
        );
        result
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
            .create(os(path.get_path_string()))
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
        let result = match fs::remove_file(os(name)) {
            Err(e) if e.raw_os_error() == Some(EISDIR) => fs::remove_dir(os(name)),
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
        // NativePosixFiles.utimensat(path, now, millis): utimensat(AT_FDCWD, path, {t, t}, 0),
        // following symlinks; `now` is UTIME_NOW (the kernel's clock, and only write access is
        // needed, not ownership).
        // Java: (native) src/main/native/unix_jni.cc Java_com_google_devtools_build_lib_unix_NativePosixFiles_utimensat
        let name = path.get_path_string();
        let c_name = std::ffi::CString::new(os(name).as_bytes()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name} (Invalid argument)"),
            )
        })?;
        let t = if new_time == NOW_SENTINEL_TIME {
            libc::timespec {
                tv_sec: 0,
                tv_nsec: libc::UTIME_NOW,
            }
        } else {
            libc::timespec {
                tv_sec: new_time.div_euclid(1000) as libc::time_t,
                tv_nsec: (new_time.rem_euclid(1000) * 1_000_000) as libc::c_long,
            }
        };
        let times = [t, t];
        // SAFETY: `c_name` is a NUL-terminated string and `times` two valid timespecs, both
        // alive for the call.
        let rc = unsafe { libc::utimensat(libc::AT_FDCWD, c_name.as_ptr(), times.as_ptr(), 0) };
        if rc != 0 {
            return Err(posix_error(name, io::Error::last_os_error()));
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:118 UnixFileSystem.stat
    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus> {
        let name = path.get_path_string();
        let m = if follow_symlinks {
            fs::metadata(os(name))
        } else {
            fs::symlink_metadata(os(name))
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
            os(target_fragment.get_safe_path_string()),
            os(link_path.get_path_string()),
        )
        .map_err(|e| posix_error(link_path.get_path_string(), e))
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:311 UnixFileSystem.readSymbolicLink
    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment> {
        let name = path.get_path_string();
        match fs::read_link(os(name)) {
            Ok(target) => Ok(PathFragment::create(&from_os(target.as_os_str()))),
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
        for entry in fs::read_dir(os(name)).map_err(|e| posix_error(name, e))? {
            let entry = entry.map_err(|e| posix_error(name, e))?;
            result.push(from_os(&entry.file_name()));
        }
        Ok(result)
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:88 UnixFileSystem.readdir
    fn readdir(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Vec<Dirent>> {
        let name = path.get_path_string();
        let mut result = Vec::new();
        for entry in fs::read_dir(os(name)).map_err(|e| posix_error(name, e))? {
            let entry = entry.map_err(|e| posix_error(name, e))?;
            let file_type = entry.file_type().map_err(|e| posix_error(name, e))?;
            let child = from_os(&entry.file_name());
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
        fs::set_permissions(os(name), fs::Permissions::from_mode(mode))
            .map_err(|e| posix_error(name, e))
    }

    // Java: com/google/devtools/build/lib/vfs/DiskBackedFileSystem.java:64 DiskBackedFileSystem.getInputStream
    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        let name = path.get_path_string();
        let f = fs::File::open(os(name)).map_err(|e| posix_error(name, e))?;
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
        let f = options.open(os(name)).map_err(|e| posix_error(name, e))?;
        Ok(Box::new(f))
    }

    // Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:328 UnixFileSystem.renameTo
    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()> {
        fs::rename(
            os(source_path.get_path_string()),
            os(target_path.get_path_string()),
        )
        .map_err(|e| {
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
        fs::hard_link(
            os(original_path.get_path_string()),
            os(link_path.get_path_string()),
        )
        .map_err(|e| posix_error(link_path.get_path_string(), e))
    }
}
