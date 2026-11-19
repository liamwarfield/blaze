//! Port of `src/main/cpp/util/file.cc` and `src/main/cpp/util/file_posix.cc`.

use std::sync::OnceLock;

use blaze_util::string_encoding::internal_to_platform;

use super::errors::{errno, get_last_error_string, set_errno};
use super::exit_code;
use super::logging::bazel_die;
use super::path::{Path, dirname, is_root_directory};
use super::strings::{cstr, from_platform_bytes};

const FILE_POSIX_CC: &str = "src/main/cpp/util/file_posix.cc";

fn stat_path(path: &str) -> Option<libc::stat> {
    let c = cstr(path);
    // SAFETY: zeroed stat is a valid out-parameter; c is NUL-terminated.
    unsafe {
        let mut buf: libc::stat = std::mem::zeroed();
        if libc::stat(c.as_ptr(), &mut buf) == -1 {
            None
        } else {
            Some(buf)
        }
    }
}

fn lstat_path(path: &str) -> Option<libc::stat> {
    let c = cstr(path);
    // SAFETY: as above.
    unsafe {
        let mut buf: libc::stat = std::mem::zeroed();
        if libc::lstat(c.as_ptr(), &mut buf) == -1 {
            None
        } else {
            Some(buf)
        }
    }
}

fn is_dir_mode(mode: libc::mode_t) -> bool {
    mode & libc::S_IFMT == libc::S_IFDIR
}

fn is_lnk_mode(mode: libc::mode_t) -> bool {
    mode & libc::S_IFMT == libc::S_IFLNK
}

// Java: src/main/cpp/util/file_posix.cc:51 GetDirectoryStat
fn get_directory_stat(path: &str, mode: libc::mode_t, check_perms: bool) -> bool {
    let Some(filestat) = stat_path(path) else {
        return false;
    };
    if !is_dir_mode(filestat.st_mode) {
        set_errno(libc::ENOTDIR);
        return false;
    }
    if check_perms {
        let Some(linkstat) = lstat_path(path) else {
            return false;
        };
        // SAFETY: geteuid has no preconditions.
        if linkstat.st_uid != unsafe { libc::geteuid() } {
            set_errno(libc::EACCES);
            return false;
        }
        // SAFETY: umask has no preconditions; the mask is restored immediately.
        let mask = unsafe { libc::umask(0o022) };
        unsafe { libc::umask(mask) };
        let mode = mode & !mask;
        if (filestat.st_mode & 0o777) != mode {
            let c = cstr(path);
            // SAFETY: c is NUL-terminated.
            if unsafe { libc::chmod(c.as_ptr(), mode) } == -1 {
                return false;
            }
        }
    }
    true
}

// Java: src/main/cpp/util/file_posix.cc:79 MakeDirectories
fn make_directories_impl(path: &str, mode: libc::mode_t, childmost: bool) -> bool {
    if path.is_empty() || is_root_directory(path) {
        set_errno(libc::EACCES);
        return false;
    }
    let stat_succeeded = get_directory_stat(path, mode, childmost);
    if stat_succeeded {
        return true;
    }
    if errno() == libc::ENOENT {
        let parent = dirname(path);
        if !make_directories_impl(&parent, mode, false) {
            return false;
        }
        let c = cstr(path);
        // SAFETY: c is NUL-terminated.
        if unsafe { libc::mkdir(c.as_ptr(), mode) } == -1 {
            if errno() == libc::EEXIST {
                if childmost {
                    return get_directory_stat(path, mode, childmost);
                } else {
                    return true;
                }
            }
            return false;
        }
        return true;
    }
    stat_succeeded
}

// Java: src/main/cpp/util/file_posix.cc:498 MakeDirectories
pub fn make_directories(path: &str, mode: u32) -> bool {
    make_directories_impl(path, mode as libc::mode_t, true)
}

// Java: src/main/cpp/util/file_posix.cc:502 MakeDirectories
pub fn make_directories_path(path: &Path, mode: u32) -> bool {
    make_directories(&path.as_native_path(), mode)
}

// Java: src/main/cpp/util/file_posix.cc:126 CreateSiblingTempDir
pub fn create_sibling_temp_dir(other_path: &Path) -> Path {
    let parent = other_path.get_parent();
    if !path_exists_path(&parent) && !make_directories_path(&parent, 0o777) {
        bazel_die(
            exit_code::INTERNAL_ERROR,
            FILE_POSIX_CC,
            131,
            format!(
                "couldn't create '{}': {}",
                parent.as_printable_path(),
                get_last_error_string()
            ),
        );
    }
    let template = format!("{}.tmp.XXXXXX", other_path.as_native_path());
    let mut buf = cstr(&template).into_bytes_with_nul();
    // SAFETY: buf is a writable NUL-terminated template.
    let result = unsafe { libc::mkdtemp(buf.as_mut_ptr() as *mut libc::c_char) };
    if result.is_null() {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            FILE_POSIX_CC,
            138,
            format!(
                "could not create temporary directory under {} to extract install base into ({})",
                parent.as_printable_path(),
                get_last_error_string()
            ),
        );
    }
    buf.pop(); // NUL
    let path = from_platform_bytes(&buf);
    // SAFETY: umask has no preconditions; restored immediately.
    let um = unsafe { libc::umask(0) };
    unsafe { libc::umask(um) };
    let c = cstr(&path);
    // SAFETY: c is NUL-terminated.
    unsafe { libc::chmod(c.as_ptr(), 0o777 & !um) };
    Path::new(&path)
}

// Java: src/main/cpp/util/file_posix.cc:150 RemoveDirRecursively
fn remove_dir_recursively(path: &Path) -> bool {
    let Ok(dir) = std::fs::read_dir(std::path::Path::new(&*internal_to_platform(
        &path.as_native_path(),
    ))) else {
        return false;
    };
    for ent in dir {
        let Ok(ent) = ent else {
            return false;
        };
        let name = from_platform_bytes(ent.file_name().as_encoded_bytes());
        if name == "." || name == ".." {
            continue;
        }
        if !remove_recursively(&path.get_relative(&name)) {
            return false;
        }
    }
    let c = cstr(&path.as_native_path());
    // SAFETY: c is NUL-terminated.
    unsafe { libc::rmdir(c.as_ptr()) == 0 }
}

// Java: src/main/cpp/util/file_posix.cc:177 RemoveRecursively
pub fn remove_recursively(path: &Path) -> bool {
    let Some(stat_buf) = lstat_path(&path.as_native_path()) else {
        return errno() == libc::ENOENT;
    };
    if is_dir_mode(stat_buf.st_mode) && !is_lnk_mode(stat_buf.st_mode) {
        remove_dir_recursively(path)
    } else {
        unlink_path_path(path)
    }
}

// Java: src/main/cpp/util/file_posix.cc:276 ReadFile
/// Reads the file (at most `max_size` bytes if positive). `None` if it cannot be opened or read.
pub fn read_file(filename: &str, max_size: i32) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::open(std::path::Path::new(&*internal_to_platform(filename))).ok()?;
    let mut content = Vec::new();
    if max_size > 0 {
        f.take(max_size as u64).read_to_end(&mut content).ok()?;
    } else {
        f.read_to_end(&mut content).ok()?;
    }
    Some(content)
}

// Java: src/main/cpp/util/file_posix.cc:293 ReadFile
pub fn read_file_path(path: &Path, max_size: i32) -> Option<Vec<u8>> {
    read_file(&path.as_native_path(), max_size)
}

// Java: src/main/cpp/util/file_posix.cc:310 WriteFile
pub fn write_file(data: &[u8], filename: &str, perm: u32) -> bool {
    unlink_path(filename); // We don't care about the success of this.
    let c = cstr(filename);
    // SAFETY: c is NUL-terminated.
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_CREAT | libc::O_WRONLY | libc::O_TRUNC,
            perm as libc::c_uint,
        )
    };
    if fd == -1 {
        return false;
    }
    let mut written = 0usize;
    while written < data.len() {
        let chunk = (data.len() - written).min(i32::MAX as usize);
        // SAFETY: the pointer and length lie within `data`.
        let result =
            unsafe { libc::write(fd, data[written..].as_ptr() as *const libc::c_void, chunk) };
        if result == -1 {
            // SAFETY: fd is open.
            unsafe { libc::close(fd) };
            return false;
        }
        written += result as usize;
    }
    // SAFETY: fd is open.
    unsafe { libc::close(fd) == 0 } // Can fail on NFS.
}

// Java: src/main/cpp/util/file_posix.cc:330 WriteFile
pub fn write_file_path(data: &[u8], path: &Path, perm: u32) -> bool {
    write_file(data, &path.as_native_path(), perm)
}

// Java: src/main/cpp/util/file_posix.cc:345 RenameDirectory
pub const K_RENAME_DIRECTORY_SUCCESS: i32 = 0;
pub const K_RENAME_DIRECTORY_FAILURE_NOT_EMPTY: i32 = 1;
pub const K_RENAME_DIRECTORY_FAILURE_OTHER_ERROR: i32 = 2;

// Java: src/main/cpp/util/file_posix.cc:345 RenameDirectory
pub fn rename_directory(old_path: &Path, new_path: &Path) -> i32 {
    let a = cstr(&old_path.as_native_path());
    let b = cstr(&new_path.as_native_path());
    // SAFETY: both are NUL-terminated.
    if unsafe { libc::rename(a.as_ptr(), b.as_ptr()) } == 0 {
        K_RENAME_DIRECTORY_SUCCESS
    } else {
        let e = errno();
        if e == libc::ENOTEMPTY || e == libc::EEXIST {
            K_RENAME_DIRECTORY_FAILURE_NOT_EMPTY
        } else {
            K_RENAME_DIRECTORY_FAILURE_OTHER_ERROR
        }
    }
}

// Java: src/main/cpp/util/file_posix.cc:359 ReadDirectorySymlink
pub fn read_directory_symlink(symlink: &Path) -> Option<Path> {
    let c = cstr(&symlink.as_native_path());
    let mut buf = vec![0u8; libc::PATH_MAX as usize + 1];
    // SAFETY: buf has PATH_MAX + 1 bytes; at most PATH_MAX are written.
    let len = unsafe {
        libc::readlink(
            c.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_char,
            libc::PATH_MAX as usize,
        )
    };
    if len < 0 {
        return None;
    }
    buf.truncate(len as usize);
    Some(Path::new(&from_platform_bytes(&buf)))
}

// Java: src/main/cpp/util/file_posix.cc:372 UnlinkPath
pub fn unlink_path(file_path: &str) -> bool {
    let c = cstr(file_path);
    // SAFETY: c is NUL-terminated.
    unsafe { libc::unlink(c.as_ptr()) == 0 }
}

// Java: src/main/cpp/util/file_posix.cc:376 UnlinkPath
pub fn unlink_path_path(file_path: &Path) -> bool {
    unlink_path(&file_path.as_native_path())
}

fn access(path: &str, mode: libc::c_int) -> bool {
    let c = cstr(path);
    // SAFETY: c is NUL-terminated.
    unsafe { libc::access(c.as_ptr(), mode) == 0 }
}

// Java: src/main/cpp/util/file_posix.cc:380 PathExists
pub fn path_exists(path: &str) -> bool {
    access(path, libc::F_OK)
}

// Java: src/main/cpp/util/file_posix.cc:384 PathExists
pub fn path_exists_path(path: &Path) -> bool {
    path_exists(&path.as_native_path())
}

// Java: src/main/cpp/util/file_posix.cc:386 MakeCanonical
/// `realpath(path)`, or "" on failure.
pub fn make_canonical(path: &str) -> String {
    let c = cstr(path);
    // SAFETY: realpath with a NULL buffer allocates the result, which we free.
    unsafe {
        let resolved = libc::realpath(c.as_ptr(), std::ptr::null_mut());
        if resolved.is_null() {
            return String::new();
        }
        let ret = from_platform_bytes(std::ffi::CStr::from_ptr(resolved).to_bytes());
        libc::free(resolved as *mut libc::c_void);
        ret
    }
}

// Java: src/main/cpp/util/file_posix.cc:397 CanAccess
fn can_access(path: &str, read: bool, write: bool, exec: bool) -> bool {
    let mut mode = 0;
    if read {
        mode |= libc::R_OK;
    }
    if write {
        mode |= libc::W_OK;
    }
    if exec {
        mode |= libc::X_OK;
    }
    access(path, mode)
}

// Java: src/main/cpp/util/file_posix.cc:411 CanReadFile
pub fn can_read_file(path: &str) -> bool {
    !is_directory(path) && can_access(path, true, false, false)
}

// Java: src/main/cpp/util/file_posix.cc:415 CanReadFile
pub fn can_read_file_path(path: &Path) -> bool {
    can_read_file(&path.as_native_path())
}

// Java: src/main/cpp/util/file_posix.cc:419 CanExecuteFile
pub fn can_execute_file(path: &str) -> bool {
    !is_directory(path) && can_access(path, false, false, true)
}

// Java: src/main/cpp/util/file_posix.cc:423 CanExecuteFile
pub fn can_execute_file_path(path: &Path) -> bool {
    can_execute_file(&path.as_native_path())
}

// Java: src/main/cpp/util/file_posix.cc:427 CanAccessDirectory
pub fn can_access_directory(path: &str) -> bool {
    is_directory(path) && can_access(path, true, true, true)
}

// Java: src/main/cpp/util/file_posix.cc:431 CanAccessDirectory
pub fn can_access_directory_path(path: &Path) -> bool {
    can_access_directory(&path.as_native_path())
}

// Java: src/main/cpp/util/file_posix.cc:435 IsDirectory
pub fn is_directory(path: &str) -> bool {
    match stat_path(path) {
        Some(buf) => is_dir_mode(buf.st_mode),
        None => false,
    }
}

// Java: src/main/cpp/util/file_posix.cc:440 IsDirectory
pub fn is_directory_path(path: &Path) -> bool {
    is_directory(&path.as_native_path())
}

// Java: src/main/cpp/util/file_posix.cc:434 SyncFile
pub fn sync_file(path: &str) {
    let c = cstr(path);
    // SAFETY: c is NUL-terminated.
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY) };
    if fd < 0 {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            FILE_POSIX_CC,
            438,
            format!(
                "failed to open '{}' for syncing: {}",
                path,
                get_last_error_string()
            ),
        );
    }
    // SAFETY: fd is open.
    if unsafe { libc::fsync(fd) } < 0 {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            FILE_POSIX_CC,
            443,
            format!("failed to sync '{}': {}", path, get_last_error_string()),
        );
    }
    // SAFETY: fd is open.
    unsafe { libc::close(fd) };
}

// Java: src/main/cpp/util/file_posix.cc:449 SyncFile
pub fn sync_file_path(path: &Path) {
    sync_file(&path.as_native_path())
}

// Java: src/main/cpp/util/file_posix.cc:451 GetNow
fn get_now() -> libc::time_t {
    // SAFETY: time(NULL) has no preconditions.
    let result = unsafe { libc::time(std::ptr::null_mut()) };
    if result == -1 {
        bazel_die(
            exit_code::INTERNAL_ERROR,
            FILE_POSIX_CC,
            454,
            format!("time(nullptr) failed: {}", get_last_error_string()),
        );
    }
    result
}

// Java: src/main/cpp/util/file_posix.cc:460 GetYearsInFuture
fn get_years_in_future(years: i64) -> libc::time_t {
    get_now() + 3600 * 24 * 365 * years as libc::time_t
}

// Java: src/main/cpp/util/file_posix.cc:464 SetMtime
fn set_mtime(path: &Path, mtime: libc::time_t) -> bool {
    let c = cstr(&path.as_native_path());
    let times = libc::utimbuf {
        actime: mtime,
        modtime: mtime,
    };
    // SAFETY: c is NUL-terminated and times is a valid utimbuf.
    unsafe { libc::utime(c.as_ptr(), &times) == 0 }
}

// Java: src/main/cpp/util/file_posix.cc:469 kNearFuture
// PORT: C++ computes these in static initializers (before main); here on first use.
fn k_near_future() -> libc::time_t {
    static V: OnceLock<libc::time_t> = OnceLock::new();
    *V.get_or_init(|| get_years_in_future(9))
}

// Java: src/main/cpp/util/file_posix.cc:470 kDistantFuture
fn k_distant_future() -> libc::time_t {
    static V: OnceLock<libc::time_t> = OnceLock::new();
    *V.get_or_init(|| get_years_in_future(10))
}

/// Initializes the static time constants, as C++ static initialization does before `main`.
pub fn init_static_times() {
    k_near_future();
    k_distant_future();
}

// Java: src/main/cpp/util/file_posix.cc:472 IsUntampered
pub fn is_untampered(path: &Path) -> bool {
    let Some(buf) = stat_path(&path.as_native_path()) else {
        return false;
    };
    is_dir_mode(buf.st_mode) || buf.st_mtime > k_near_future()
}

// Java: src/main/cpp/util/file_posix.cc:481 SetMtimeToNow
pub fn set_mtime_to_now(path: &Path) -> bool {
    let now = get_now();
    set_mtime(path, now)
}

// Java: src/main/cpp/util/file_posix.cc:486 SetMtimeToNowIfPossible
pub fn set_mtime_to_now_if_possible(path: &Path) -> bool {
    let mut okay = set_mtime_to_now(path);
    if !okay {
        let e = errno();
        if e == libc::EROFS || e == libc::EPERM {
            okay = true;
        }
    }
    okay
}

// Java: src/main/cpp/util/file_posix.cc:497 SetMtimeToDistantFuture
pub fn set_mtime_to_distant_future(path: &Path) -> bool {
    set_mtime(path, k_distant_future())
}

// Java: src/main/cpp/util/file_posix.cc:520 GetCwd
pub fn get_cwd() -> String {
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: buf has PATH_MAX bytes.
    let r = unsafe { libc::getcwd(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if r.is_null() {
        bazel_die(
            exit_code::INTERNAL_ERROR,
            FILE_POSIX_CC,
            523,
            format!("getcwd() failed: {}", get_last_error_string()),
        );
    }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    buf.truncate(len);
    from_platform_bytes(&buf)
}

// Java: src/main/cpp/util/file_posix.cc:529 ChangeDirectory
pub fn change_directory(path: &str) -> bool {
    let c = cstr(path);
    // SAFETY: c is NUL-terminated.
    unsafe { libc::chdir(c.as_ptr()) == 0 }
}

// Java: src/main/cpp/util/file_posix.cc:533 ForEachDirectoryEntry
/// Calls `consume(child_path, is_directory)` for each entry of `path` except `.` and `..`.
pub fn for_each_directory_entry(path: &Path, consume: &mut dyn FnMut(&Path, bool)) {
    let Ok(dir) = std::fs::read_dir(std::path::Path::new(&*internal_to_platform(
        &path.as_native_path(),
    ))) else {
        return;
    };
    for ent in dir.flatten() {
        let name = from_platform_bytes(ent.file_name().as_encoded_bytes());
        if name == "." || name == ".." {
            continue;
        }
        let child_path = path.get_relative(&name);
        // d_type when known (file_type() uses it), else lstat, like the C++.
        let is_directory = match ent.file_type() {
            Ok(t) => t.is_dir(),
            Err(_) => match lstat_path(&child_path.as_native_path()) {
                Some(buf) => is_dir_mode(buf.st_mode),
                None => bazel_die(
                    exit_code::INTERNAL_ERROR,
                    FILE_POSIX_CC,
                    559,
                    format!(
                        "stat failed for filename '{}': {}",
                        child_path.as_printable_path(),
                        get_last_error_string()
                    ),
                ),
            },
        };
        consume(&child_path, is_directory);
    }
}

// Java: src/main/cpp/util/file.cc:89 GetAllFilesUnder
pub fn get_all_files_under(path: &Path, result: &mut Vec<Path>) {
    fn walk(path: &Path, files: &mut Vec<Path>) {
        for_each_directory_entry(path, &mut |child, is_directory| {
            if is_directory {
                walk(child, &mut *files);
            } else {
                files.push(child.clone());
            }
        });
    }
    walk(path, result);
}

// Java: src/main/cpp/util/file_platform.h:107 WriteResult
/// Result of a `WriteToStdOutErr` operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteResult {
    Success,
    OtherError,
    BrokenPipe,
}

unsafe extern "C" {
    /// glibc's `stdout` and `stderr` `FILE*`s.
    #[link_name = "stdout"]
    static mut C_STDOUT: *mut libc::FILE;
    #[link_name = "stderr"]
    static mut C_STDERR: *mut libc::FILE;
}

/// `setvbuf(stdout, nullptr, _IONBF, 0); setvbuf(stderr, nullptr, _IONBF, 0);` (part of
/// `SetupStdStreams`).
///
/// # Safety
/// Must be called before any output through the C streams.
pub unsafe fn setvbuf_unbuffered_std_streams() {
    // SAFETY: forwarded to the caller.
    unsafe {
        libc::setvbuf(C_STDOUT, std::ptr::null_mut(), libc::_IONBF, 0);
        libc::setvbuf(C_STDERR, std::ptr::null_mut(), libc::_IONBF, 0);
    }
}

// Java: src/main/cpp/util/file_posix.cc:331 WriteToStdOutErr
/// Writes `data` to stdout (or stderr) through C stdio (`fwrite`), as the C++ does (unbuffered:
/// `SetupStdStreams`).
pub fn write_to_std_out_err(data: &[u8], to_stdout: bool) -> WriteResult {
    // SAFETY: the stdio streams are valid for the life of the process; fwrite is thread-safe.
    let r = unsafe {
        let stream = if to_stdout { C_STDOUT } else { C_STDERR };
        libc::fwrite(data.as_ptr().cast(), 1, data.len(), stream)
    };
    if r == data.len() {
        WriteResult::Success
    } else if crate::util::errors::errno() == libc::EPIPE {
        // The SIGPIPE the C++ client's handler records at this write (SIGPIPE is ignored in
        // this process; see signal_handler::install).
        crate::signal_handler::note_broken_pipe();
        WriteResult::BrokenPipe
    } else {
        WriteResult::OtherError
    }
}

// Java: src/main/cpp/util/file_platform.h:69 ReadFileResult
const READ_FILE_RESULT_SUCCESS: i32 = 0;
const READ_FILE_RESULT_OTHER_ERROR: i32 = 1;
const READ_FILE_RESULT_INTERRUPTED: i32 = 2;
const READ_FILE_RESULT_AGAIN: i32 = 3;

// Java: src/main/cpp/util/file_posix.cc:249 ReadFromHandle
fn read_from_handle(fd: libc::c_int, data: &mut [u8], error: &mut i32) -> isize {
    // SAFETY: data is a valid writable buffer of data.len() bytes.
    let result = unsafe { libc::read(fd, data.as_mut_ptr().cast(), data.len()) };
    if result >= 0 {
        *error = READ_FILE_RESULT_SUCCESS;
    } else if errno() == libc::EINTR {
        *error = READ_FILE_RESULT_INTERRUPTED;
    } else if errno() == libc::EAGAIN {
        *error = READ_FILE_RESULT_AGAIN;
    } else {
        *error = READ_FILE_RESULT_OTHER_ERROR;
    }
    result
}

// Java: src/main/cpp/util/file.cc:32 ReadFrom
/// Reads `handle` until end of file (or `max_size` bytes if positive). Returns the content, or
/// `None` on a read error.
pub fn read_from(handle: libc::c_int, max_size: i32) -> Option<Vec<u8>> {
    const K_READ_SIZE: usize = 4096; // read 4K chunks
    let mut max_size = max_size;
    let mut content = Vec::new();
    let mut buf = [0u8; K_READ_SIZE];
    // OPT:  This loop generates one spurious read on regular files.
    let mut error = 0;
    loop {
        let size = if max_size > 0 {
            std::cmp::min(max_size as usize, K_READ_SIZE)
        } else {
            K_READ_SIZE
        };
        let r = read_from_handle(handle, &mut buf[..size], &mut error);
        if r == 0 {
            break;
        }
        if r < 0 {
            if error == READ_FILE_RESULT_INTERRUPTED || error == READ_FILE_RESULT_AGAIN {
                continue;
            }
            return None;
        }
        content.extend_from_slice(&buf[..r as usize]);
        if max_size > 0 {
            if max_size as isize > r {
                max_size -= r as i32;
            } else {
                break;
            }
        }
    }
    Some(content)
}
