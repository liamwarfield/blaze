//! Port of `src/main/cpp/blaze_util.cc`, `blaze_util_posix.cc`, `blaze_util_linux.cc`,
//! `get_self_path_linux.cc` and `option_processor_unix.cc` (the Linux paths the client uses).
//!
//! The module is not called `blaze_util` to avoid clashing with the `blaze_util` crate.

use std::ffi::{CStr, CString};

use crate::util::errors::{errno, get_last_error_string, strerror};
use crate::util::exit_code;
use crate::util::file::{get_cwd, make_canonical, make_directories_path, read_file};
use crate::util::logging::{self, LogLevel, LoggingDetail, bazel_die, bazel_log};
use crate::util::md5::Md5Digest;
use crate::util::numbers::strto32;
use crate::util::path::{Path, dirname, join_path, make_absolute};

const BLAZE_UTIL_H: &str = "src/main/cpp/blaze_util.h";
const BLAZE_UTIL_POSIX_CC: &str = "src/main/cpp/blaze_util_posix.cc";
const BLAZE_UTIL_LINUX_CC: &str = "src/main/cpp/blaze_util_linux.cc";
const GET_SELF_PATH_LINUX_CC: &str = "src/main/cpp/get_self_path_linux.cc";

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// blaze_util.cc / blaze_util.h

// Java: src/main/cpp/blaze_util.cc:46 kServerPidFile
pub const K_SERVER_PID_FILE: &str = "server.pid.txt";

// Java: src/main/cpp/blaze_util.cc:48 kPostShutdownGracePeriodSeconds
pub const K_POST_SHUTDOWN_GRACE_PERIOD_SECONDS: u32 = 60;

// Java: src/main/cpp/blaze_util.cc:50 kPostKillGracePeriodSeconds
pub const K_POST_KILL_GRACE_PERIOD_SECONDS: u32 = 10;

// Java: src/main/cpp/blaze_util.cc:139 AbsolutePathFromFlag
// PORT: blaze_options::rc::option_processor::ClientEnvironment::absolute_path_from_flag is the
// same function over an injected environment; this one reads the process state like the C++.
pub fn absolute_path_from_flag(value: &str) -> String {
    if value.is_empty() {
        get_cwd()
    } else if let Some(rest) = value.strip_prefix('~') {
        join_path(&get_home_dir(), rest)
    } else {
        make_absolute(value)
    }
}

// Java: src/main/cpp/blaze_util.cc:224 SetDebugLog
pub fn set_debug_log(detail: LoggingDetail) {
    if detail == LoggingDetail::Debug {
        logging::set_logging_detail(LoggingDetail::Debug, Some(Box::new(std::io::stderr())));
    } else {
        logging::set_logging_detail(detail, None);
        // PORT: absl log threshold handling skipped (no absl in Blaze).
    }
}

// Java: src/main/cpp/blaze_util.cc:250 IsRunningWithinTest
pub fn is_running_within_test() -> bool {
    exists_env("TEST_TMPDIR")
}

// Java: src/main/cpp/blaze_util.cc:252 GetOOMFilePath
pub fn get_oom_file_path(output_base: &Path) -> Path {
    output_base.get_relative("blaze_oomed")
}

// Java: src/main/cpp/blaze_util.cc:256 GetAbruptExitFilePath
pub fn get_abrupt_exit_file_path(output_base: &Path) -> Path {
    output_base.get_relative("exit_code_to_use_on_abrupt_exit")
}

// Java: src/main/cpp/blaze_util.h:111 EnvVarAction
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvVarAction {
    Unset,
    Set,
}

// Java: src/main/cpp/blaze_util.h:114 EnvVarValue
/// What WithEnvVar should do with an environment variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvVarValue {
    /// What should be done with the given variable.
    pub action: EnvVarAction,
    /// The value of the variable; ignored if action == UNSET.
    pub value: String,
}

impl EnvVarValue {
    pub fn new(action: EnvVarAction, value: impl Into<String>) -> Self {
        EnvVarValue {
            action,
            value: value.into(),
        }
    }
}

// Java: src/main/cpp/blaze_util.h:142 DurationMillis
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DurationMillis {
    pub millis: u64,
}

impl DurationMillis {
    // Java: src/main/cpp/blaze_util.h:148 DurationMillis::DurationMillis
    pub fn new(start: u64, end: u64) -> Self {
        DurationMillis {
            millis: Self::compute_duration(start, end),
        }
    }

    // Java: src/main/cpp/blaze_util.h:151 DurationMillis::operator+=
    pub fn add_assign(&mut self, other: &DurationMillis) {
        self.millis += other.millis;
    }

    // Java: src/main/cpp/blaze_util.h:157 DurationMillis::ComputeDuration
    fn compute_duration(start: u64, end: u64) -> u64 {
        if end < start {
            bazel_log(
                LogLevel::Warning,
                BLAZE_UTIL_H,
                159,
                format!("Invalid duration: start={}, end={}", start, end),
            );
            return 0;
        }
        end - start
    }
}

// ---------------------------------------------------------------------------------------------
// blaze_util_posix.cc

// Java: src/main/cpp/blaze_util_posix.cc:255 GetProcessIdAsString
pub fn get_process_id_as_string() -> String {
    std::process::id().to_string()
}

// Java: src/main/cpp/blaze_util_posix.cc:257 GetHomeDir
pub fn get_home_dir() -> String {
    get_path_env("HOME")
}

// Java: src/main/cpp/blaze_util_posix.cc:259 GetJavaBinaryUnderJavabase
pub fn get_java_binary_under_javabase() -> String {
    "bin/java".to_string()
}

// Java: src/main/cpp/blaze_util_posix.cc:261 Which
pub fn which(executable: &str) -> String {
    let path = get_path_env("PATH");
    if path.is_empty() {
        return String::new();
    }
    for piece in path.split(':') {
        let piece = if piece.is_empty() { "." } else { piece };
        let candidate = join_path(piece, executable);
        let c = cstr(&candidate);
        // SAFETY: c is NUL-terminated; file_stat is a valid out-parameter.
        let ok = unsafe {
            let mut file_stat: libc::stat = std::mem::zeroed();
            libc::access(c.as_ptr(), libc::X_OK) == 0
                && libc::stat(c.as_ptr(), &mut file_stat) == 0
                && (file_stat.st_mode & libc::S_IFMT) == libc::S_IFREG
        };
        if ok {
            return candidate;
        }
    }
    String::new()
}

// Java: src/main/cpp/blaze_util_posix.cc:362 kListSeparator
pub const K_LIST_SEPARATOR: char = ':';

// Java: src/main/cpp/blaze_util_posix.cc:364 SymlinkDirectories
pub fn symlink_directories(target: &Path, link: &Path) -> bool {
    let t = cstr(&target.as_native_path());
    let l = cstr(&link.as_native_path());
    // SAFETY: both are NUL-terminated.
    unsafe { libc::symlink(t.as_ptr(), l.as_ptr()) == 0 }
}

// Java: src/main/cpp/blaze_util_posix.cc:509 GetHashedBaseDir
pub fn get_hashed_base_dir(root: &Path, hashable: &str) -> Path {
    let mut digest = Md5Digest::new();
    digest.update(hashable.as_bytes());
    digest.finish();
    root.get_relative(&digest.string())
}

// Java: src/main/cpp/blaze_util_posix.cc:518 CreateSecureDirectory
pub fn create_secure_directory(path: &Path) {
    if !make_directories_path(path, 0o755) {
        let err = get_last_error_string();
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            BLAZE_UTIL_POSIX_CC,
            524,
            format!("mkdir('{}'): {}", path.as_printable_path(), err),
        );
    }

    // The path already exists.
    // Check ownership and mode, and verify that it is a directory.
    let c = cstr(&path.as_native_path());
    // SAFETY: zeroed stat is a valid out-parameter; c is NUL-terminated.
    let mut fileinfo: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::lstat(c.as_ptr(), &mut fileinfo) } < 0 {
        let err = get_last_error_string();
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            BLAZE_UTIL_POSIX_CC,
            533,
            format!("lstat('{}'): {}", path.as_printable_path(), err),
        );
    }

    // SAFETY: geteuid has no preconditions.
    if fileinfo.st_uid != unsafe { libc::geteuid() } {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            BLAZE_UTIL_POSIX_CC,
            538,
            format!("'{}' is not owned by me", path.as_printable_path()),
        );
    }

    if (fileinfo.st_mode & 0o022) != 0 {
        let new_mode = fileinfo.st_mode & !0o022;
        // SAFETY: c is NUL-terminated.
        if unsafe { libc::chmod(c.as_ptr(), new_mode) } < 0 {
            bazel_die(
                exit_code::LOCAL_ENVIRONMENTAL_ERROR,
                BLAZE_UTIL_POSIX_CC,
                545,
                format!(
                    "'{}' has mode {}, chmod to {} failed",
                    path.as_printable_path(),
                    fileinfo.st_mode & 0o7777,
                    new_mode
                ),
            );
        }
    }

    // SAFETY: as above.
    if unsafe { libc::stat(c.as_ptr(), &mut fileinfo) } < 0 {
        let err = get_last_error_string();
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            BLAZE_UTIL_POSIX_CC,
            554,
            format!("stat('{}'): {}", path.as_printable_path(), err),
        );
    }

    if (fileinfo.st_mode & libc::S_IFMT) != libc::S_IFDIR {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            BLAZE_UTIL_POSIX_CC,
            559,
            format!("'{}' is not a directory", path.as_printable_path()),
        );
    }

    exclude_path_from_backup(path);
}

// Java: src/main/cpp/blaze_util_posix.cc:565 GetEnv
pub fn get_env(name: &str) -> String {
    let c = cstr(name);
    // SAFETY: getenv returns NULL or a NUL-terminated string owned by the environment.
    unsafe {
        let result = libc::getenv(c.as_ptr());
        if result.is_null() {
            String::new()
        } else {
            CStr::from_ptr(result).to_string_lossy().into_owned()
        }
    }
}

// Java: src/main/cpp/blaze_util_posix.cc:570 GetPathEnv
pub fn get_path_env(name: &str) -> String {
    get_env(name)
}

// Java: src/main/cpp/blaze_util_posix.cc:572 ExistsEnv
pub fn exists_env(name: &str) -> bool {
    let c = cstr(name);
    // SAFETY: getenv has no preconditions beyond a NUL-terminated name.
    unsafe { !libc::getenv(c.as_ptr()).is_null() }
}

// Java: src/main/cpp/blaze_util_posix.cc:574 SetEnv
/// # Safety
/// Mutates the process environment: no other thread may read or write it concurrently.
pub unsafe fn set_env(name: &str, value: &str) {
    // SAFETY: forwarded to the caller.
    unsafe { std::env::set_var(name, value) }
}

// Java: src/main/cpp/blaze_util_posix.cc:578 UnsetEnv
/// # Safety
/// Mutates the process environment: no other thread may read or write it concurrently.
pub unsafe fn unset_env(name: &str) {
    // SAFETY: forwarded to the caller.
    unsafe { std::env::remove_var(name) }
}

// Java: src/main/cpp/blaze_util_posix.cc:580 WarnIfStartedFromDesktop
pub fn warn_if_started_from_desktop() -> bool {
    false
}

// Java: src/main/cpp/blaze_util_posix.cc:582 SetupStdStreams
pub fn setup_std_streams() {
    // PORT: setvbuf(stdout/stderr, _IONBF) has no Rust equivalent: Rust's stderr is unbuffered
    // and its stdout is line-buffered; client output is flushed explicitly where it matters.

    // Ensure we have three open fds.  Otherwise we can end up with bizarre things like stdout
    // going to the lock file, etc.
    let dev_null = cstr("/dev/null");
    // SAFETY: fcntl/open on standard descriptors and a NUL-terminated path.
    unsafe {
        if libc::fcntl(libc::STDIN_FILENO, libc::F_GETFL) == -1 {
            libc::open(dev_null.as_ptr(), libc::O_RDONLY);
        }
        if libc::fcntl(libc::STDOUT_FILENO, libc::F_GETFL) == -1 {
            libc::open(dev_null.as_ptr(), libc::O_WRONLY);
        }
        if libc::fcntl(libc::STDERR_FILENO, libc::F_GETFL) == -1 {
            libc::open(dev_null.as_ptr(), libc::O_WRONLY);
        }
    }
}

// Java: src/main/cpp/blaze_util_platform.h:211 LockHandle
pub type LockHandle = libc::c_int;

// Java: src/main/cpp/blaze_util_platform.h:213 LockMode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockMode {
    Shared,
    Exclusive,
}

// Java: src/main/cpp/blaze_util_posix.cc:623 TryLock
fn try_lock(fd: libc::c_int, mode: LockMode, name: &str) -> bool {
    // SAFETY: zeroed flock is valid.
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    let l_type = if mode == LockMode::Shared {
        libc::F_RDLCK
    } else {
        libc::F_WRLCK
    };
    lock.l_type = l_type as libc::c_short;
    // Locking is advisory, so any range works as long as it overlaps with the ones requested by
    // other processes.
    lock.l_whence = libc::SEEK_SET as libc::c_short;
    lock.l_start = 0;
    lock.l_len = 1;
    // Prefer OFD locks when available. POSIX locks can be lost "accidentally" due to any close()
    // on the lock file, and are not reliably preserved across execve() on Linux, which we need
    // for --batch mode.
    // SAFETY: fd is open; lock is a valid flock.
    if unsafe { libc::fcntl(fd, libc::F_OFD_SETLK, &lock as *const libc::flock) } == 0 {
        return true;
    }
    let e = errno();
    if e != libc::EINVAL {
        if e != libc::EACCES && e != libc::EAGAIN {
            bazel_die(
                exit_code::LOCAL_ENVIRONMENTAL_ERROR,
                BLAZE_UTIL_POSIX_CC,
                639,
                format!(
                    "fcntl failed for {} lock: {}",
                    name,
                    get_last_error_string()
                ),
            );
        }
        return false;
    }
    // Fall back to POSIX locks on EINVAL.
    // SAFETY: as above.
    if unsafe { libc::fcntl(fd, libc::F_SETLK, &lock as *const libc::flock) } == 0 {
        return true;
    }
    let e = errno();
    if e != libc::EACCES && e != libc::EAGAIN {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            BLAZE_UTIL_POSIX_CC,
            650,
            format!(
                "fctnl failed for {} lock: {}",
                name,
                get_last_error_string()
            ),
        );
    }
    false
}

// Java: src/main/cpp/blaze_util_posix.cc:656 StillExists
fn still_exists(fd: libc::c_int, name: &str) -> bool {
    // SAFETY: zeroed stat is a valid out-parameter; fd is open.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut st) } < 0 {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            BLAZE_UTIL_POSIX_CC,
            659,
            format!(
                "fstat failed for {} lock: {}",
                name,
                get_last_error_string()
            ),
        );
    }
    st.st_nlink > 0
}

// Java: src/main/cpp/blaze_util_posix.cc:665 ReadOwnerInformation
fn read_owner_information(fd: libc::c_int, name: &str) -> Vec<u8> {
    // Assume that 4KB are sufficient to fit the owner information.
    let mut buffer = vec![0u8; 4096];
    // SAFETY: buffer has 4096 bytes; fd is open.
    let mut r = unsafe {
        libc::pread(
            fd,
            buffer.as_mut_ptr() as *mut libc::c_void,
            buffer.len(),
            0,
        )
    };
    if r < 0 {
        bazel_log(
            LogLevel::Warning,
            BLAZE_UTIL_POSIX_CC,
            670,
            format!(
                "pread failed for {} lock: {}",
                name,
                get_last_error_string()
            ),
        );
        r = 0;
    }
    buffer.truncate(r as usize);
    buffer
}

fn fd_write(fd: libc::c_int, s: &[u8]) {
    // SAFETY: s is a valid buffer; fd is open.
    unsafe {
        libc::write(fd, s.as_ptr() as *const libc::c_void, s.len());
    }
}

// Java: src/main/cpp/blaze_util_posix.cc:677 WriteOwnerInformation
fn write_owner_information(fd: libc::c_int) {
    // SAFETY: fd is open.
    unsafe {
        let _ = libc::ftruncate(fd, 0);
        libc::lseek(fd, 0, libc::SEEK_SET);
    }
    // Locking is advisory, so it doesn't matter that this may overflow the locked range.
    fd_write(
        fd,
        format!("pid={}\nowner=client\n", std::process::id()).as_bytes(),
    );
    let cwd = get_cwd();
    fd_write(fd, format!("cwd={}\n", cwd).as_bytes());
    // SAFETY: ttyname returns NULL or a NUL-terminated static buffer.
    let tty = unsafe { libc::ttyname(libc::STDIN_FILENO) };
    if !tty.is_null() {
        // SAFETY: tty is a valid C string.
        let tty = unsafe { CStr::from_ptr(tty) };
        let mut line = b"tty=".to_vec();
        line.extend_from_slice(tty.to_bytes());
        line.push(b'\n');
        fd_write(fd, &line);
    }
}

// Java: src/main/cpp/blaze_util_posix.cc:692 AcquireLock
pub fn acquire_lock(
    name: &str,
    path: &Path,
    mode: LockMode,
    batch_mode: bool,
    block: bool,
) -> (LockHandle, DurationMillis) {
    let start_time = get_milliseconds_monotonic();
    let mut multiple_attempts = false;
    let mut owner: Vec<u8> = Vec::new();

    loop {
        let mut flags = libc::O_CREAT
            | if mode == LockMode::Shared {
                libc::O_RDONLY
            } else {
                libc::O_RDWR
            };
        // Keep server from inheriting a useless fd if we are not in batch mode.
        if !batch_mode {
            flags |= libc::O_CLOEXEC;
        }

        let c = cstr(&path.as_native_path());
        // SAFETY: c is NUL-terminated.
        let fd = unsafe { libc::open(c.as_ptr(), flags, 0o644 as libc::c_uint) };
        if fd < 0 {
            bazel_die(
                exit_code::LOCAL_ENVIRONMENTAL_ERROR,
                BLAZE_UTIL_POSIX_CC,
                709,
                format!("open failed for {} lock: {}", name, get_last_error_string()),
            );
        }

        // Attempt to take the lock.
        if try_lock(fd, mode, name) {
            // Check that the lock file was not concurrently deleted.
            if still_exists(fd, name) {
                // If taking an exclusive lock, identify ourselves in the lock file. The contents
                // are printed for human consumption when another client fails to take the lock,
                // but not parsed otherwise.
                if mode == LockMode::Exclusive {
                    write_owner_information(fd);
                }
                // If we succeeded on the first try, report zero wait time to avoid unnecessary
                // noise in the logs. We are interested in how long it took for other commands to
                // complete, not how fast acquiring a lock is.
                let end_time = get_milliseconds_monotonic();
                let wait_time = if multiple_attempts {
                    DurationMillis::new(start_time, end_time)
                } else {
                    DurationMillis::default()
                };
                return (fd, wait_time);
            }
        }

        // Someone else holds the lock. Obtain the identity of the current lock owner and print it
        // out.
        let new_owner = read_owner_information(fd, name);
        if new_owner != owner {
            owner = new_owner;
            bazel_log(
                LogLevel::User,
                BLAZE_UTIL_POSIX_CC,
                739,
                format!(
                    "Another command holds the {} lock: \n{}",
                    name,
                    String::from_utf8_lossy(&owner)
                ),
            );
            if block {
                bazel_log(
                    LogLevel::User,
                    BLAZE_UTIL_POSIX_CC,
                    742,
                    "Waiting for it to complete...",
                );
            }
        }

        if !block {
            bazel_die(
                exit_code::LOCK_HELD_NOBLOCK_FOR_LOCK,
                BLAZE_UTIL_POSIX_CC,
                748,
                format!(
                    "Exiting because the {} lock is held and --noblock_for_lock was given.",
                    name
                ),
            );
        }

        multiple_attempts = true;

        // SAFETY: fd is open.
        unsafe { libc::close(fd) };
        try_sleep(500);
    }
}

// Java: src/main/cpp/blaze_util_posix.cc:759 ReleaseLock
pub fn release_lock(lock_handle: LockHandle) {
    // SAFETY: lock_handle is an fd returned by acquire_lock.
    unsafe { libc::close(lock_handle) };
}

// Java: src/main/cpp/blaze_util_posix.cc:763 KillServerProcess
pub fn kill_server_process(pid: i32, _output_base: &Path) -> bool {
    // PORT: the in-process server never leaves a server process behind; killing a process found
    // through a stale pid file (e.g. one written by a real Bazel server) is not ported.
    ::blaze_util::unported!("KillServerProcess", "pid={}", pid)
}

// Java: src/main/cpp/blaze_util_posix.cc:782 TrySleep
pub fn try_sleep(milliseconds: u32) {
    std::thread::sleep(std::time::Duration::from_millis(milliseconds as u64));
}

// Java: src/main/cpp/blaze_util_posix.cc:789 GetUserName
pub fn get_user_name() -> String {
    let user = get_env("USER");
    if !user.is_empty() {
        return user;
    }
    crate::util::errors::set_errno(0);
    // SAFETY: getpwuid returns NULL or a pointer to a static passwd entry.
    unsafe {
        let pwent = libc::getpwuid(libc::getuid());
        if pwent.is_null() || (*pwent).pw_name.is_null() {
            bazel_die(
                exit_code::LOCAL_ENVIRONMENTAL_ERROR,
                BLAZE_UTIL_POSIX_CC,
                796,
                format!(
                    "$USER is not set, and unable to look up name of current user: {}",
                    get_last_error_string()
                ),
            );
        }
        CStr::from_ptr((*pwent).pw_name)
            .to_string_lossy()
            .into_owned()
    }
}

// Java: src/main/cpp/blaze_util_posix.cc:803 IsEmacsTerminal
pub fn is_emacs_terminal() -> bool {
    let emacs = get_env("EMACS");
    // GNU Emacs <25.1 (and ~all non-GNU emacsen) set EMACS=t, but >=25.1 doesn't do that and
    // instead sets INSIDE_EMACS=<stuff> (where <stuff> can look like e.g. "25.1.1,comint").  So
    // we check both variables for maximum compatibility.
    emacs == "t" || exists_env("INSIDE_EMACS")
}

// Java: src/main/cpp/blaze_util_posix.cc:812 IsStandardTerminal
pub fn is_standard_terminal() -> bool {
    let term = get_env("TERM");
    let is_emacs = is_emacs_terminal();

    // Emacs 22+ terminal emulation uses 'eterm-color' as its terminfo name and, more
    // importantly, supports color in terminals.
    if is_emacs && term == "eterm-color" {
        return true;
    }
    if term.is_empty()
        || term == "dumb"
        || term == "emacs"
        || term == "xterm-mono"
        || term == "symbolics"
        || term == "9term"
        || is_emacs
    {
        return false;
    }
    // SAFETY: isatty has no preconditions.
    unsafe { libc::isatty(libc::STDOUT_FILENO) != 0 && libc::isatty(libc::STDERR_FILENO) != 0 }
}

// Java: src/main/cpp/blaze_util_posix.cc:831 GetTerminalColumns
pub fn get_terminal_columns() -> i32 {
    // SAFETY: zeroed winsize is a valid out-parameter.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::ioctl(
            libc::STDOUT_FILENO,
            libc::TIOCGWINSZ,
            &mut ws as *mut libc::winsize,
        )
    } != -1
    {
        return ws.ws_col as i32;
    }
    let columns_env = get_env("COLUMNS");
    if !columns_env.is_empty() {
        let (columns, at_end) = strto32(&columns_env, 10);
        if at_end {
            // $COLUMNS is a valid number
            return columns;
        }
    }
    80 // default if not a terminal.
}

// Java: src/main/cpp/blaze_util_posix.cc:855 UnlimitResource
/// Raises a resource limit to the maximum allowed value. Best-effort: failures are warnings.
fn unlimit_resource(resource: libc::__rlimit_resource_t, allow_infinity: bool) -> bool {
    // SAFETY: zeroed rlimit is a valid out-parameter.
    let mut rl: libc::rlimit = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrlimit(resource, &mut rl) } == -1 {
        bazel_log(
            LogLevel::Warning,
            BLAZE_UTIL_POSIX_CC,
            858,
            format!(
                "failed to get resource limit {}: {}",
                resource,
                strerror(errno())
            ),
        );
        return false;
    }

    if rl.rlim_cur == rl.rlim_max {
        // Nothing to do. Return early to prevent triggering any warnings caused by the code
        // below. This way, we will only show warnings the first time the Blaze server is started
        // and not on each command invocation.
        return true;
    }

    let explicit_limit = get_explicit_system_limit(resource as i32);
    if explicit_limit <= 0 {
        // If not implemented (-1) or on an error (0), do nothing and try to increase the soft
        // limit to the hard one. This might fail, but it's good to try anyway.
        rl.rlim_cur = rl.rlim_max;
    } else if (rl.rlim_max == libc::RLIM_INFINITY && !allow_infinity)
        || rl.rlim_max > explicit_limit as libc::rlim_t
    {
        rl.rlim_cur = explicit_limit as libc::rlim_t;
    } else {
        rl.rlim_cur = rl.rlim_max;
    }

    // SAFETY: rl is a valid rlimit.
    if unsafe { libc::setrlimit(resource, &rl) } == -1 {
        bazel_log(
            LogLevel::Warning,
            BLAZE_UTIL_POSIX_CC,
            886,
            format!(
                "failed to raise resource limit {} to {}: {}",
                resource,
                rl.rlim_cur as i64,
                strerror(errno())
            ),
        );
        return false;
    }
    true
}

// Java: src/main/cpp/blaze_util_posix.cc:895 UnlimitResources
pub fn unlimit_resources() -> bool {
    let mut success = true;
    success &= unlimit_resource(libc::RLIMIT_NOFILE, false);
    success &= unlimit_resource(libc::RLIMIT_NPROC, false);
    success
}

// Java: src/main/cpp/blaze_util_posix.cc:902 UnlimitCoredumps
pub fn unlimit_coredumps() -> bool {
    unlimit_resource(libc::RLIMIT_CORE, true)
}

// ---------------------------------------------------------------------------------------------
// blaze_util_linux.cc

// Java: src/main/cpp/blaze_util_linux.cc:46 GetCacheDir
/// `${XDG_CACHE_HOME}/bazel`, a.k.a. `~/.cache/bazel` by default (which is the fallback when
/// XDG_CACHE_HOME is not set).
pub fn get_cache_dir() -> String {
    let mut xdg_cache_home = get_path_env("XDG_CACHE_HOME");
    if xdg_cache_home.is_empty() {
        let mut home = get_home_dir(); // via $HOME env variable
        if home.is_empty() {
            // Fall back to home dir from password database
            // SAFETY: getpwuid_r with a caller-provided buffer of 2048 bytes.
            unsafe {
                let mut buf = vec![0 as libc::c_char; 2048];
                let mut pwbuf: libc::passwd = std::mem::zeroed();
                let mut pw: *mut libc::passwd = std::ptr::null_mut();
                let uid = libc::getuid();
                let r = libc::getpwuid_r(uid, &mut pwbuf, buf.as_mut_ptr(), 2048, &mut pw);
                if r == 0 && !pw.is_null() {
                    home = CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned();
                } else {
                    return "/tmp".to_string();
                }
            }
        }
        xdg_cache_home = join_path(&home, ".cache");
    }
    join_path(&xdg_cache_home, "bazel")
}

// Java: linux/magic.h NFS_SUPER_MAGIC
const NFS_SUPER_MAGIC: i64 = 0x6969;

// Java: src/main/cpp/blaze_util_linux.cc:70 WarnFilesystemType
pub fn warn_filesystem_type(output_base: &Path) {
    let c = cstr(&output_base.as_native_path());
    // SAFETY: zeroed statfs is a valid out-parameter; c is NUL-terminated.
    let mut buf: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut buf) } < 0 {
        bazel_log(
            LogLevel::Warning,
            BLAZE_UTIL_LINUX_CC,
            73,
            format!(
                "couldn't get file system type information for '{}': {}",
                output_base.as_printable_path(),
                strerror(errno())
            ),
        );
        return;
    }

    if buf.f_type as i64 == NFS_SUPER_MAGIC {
        bazel_log(
            LogLevel::Warning,
            BLAZE_UTIL_LINUX_CC,
            80,
            format!(
                "Output base '{}' is on NFS. This may lead to surprising failures and \
                 undetermined behavior.",
                output_base.as_printable_path()
            ),
        );
    }
}

// Java: src/main/cpp/blaze_util_linux.cc:86 GetMillisecondsMonotonic
pub fn get_milliseconds_monotonic() -> u64 {
    // SAFETY: zeroed timespec is a valid out-parameter.
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } != 0 {
        bazel_die(
            exit_code::INTERNAL_ERROR,
            BLAZE_UTIL_LINUX_CC,
            89,
            format!("error calling clock_gettime: {}", get_last_error_string()),
        );
    }
    (ts.tv_sec as u64) * 1000 + (ts.tv_nsec as u64 / 1_000_000)
}

// Java: src/main/cpp/util/port.h IOPRIO_WHO_PROCESS / IOPRIO_CLASS_BE / IOPRIO_PRIO_VALUE
const IOPRIO_WHO_PROCESS: libc::c_int = 1;
const IOPRIO_CLASS_BE: libc::c_int = 2;
const IOPRIO_CLASS_SHIFT: libc::c_int = 13;

// Java: src/main/cpp/blaze_util_linux.cc:95 SetScheduling
pub fn set_scheduling(batch_cpu_scheduling: bool, io_nice_level: i32) {
    if batch_cpu_scheduling {
        // SAFETY: zeroed sched_param is valid; pid 0 is the calling process.
        let param: libc::sched_param = unsafe { std::mem::zeroed() };
        if unsafe { libc::sched_setscheduler(0, libc::SCHED_BATCH, &param) } != 0 {
            bazel_die(
                exit_code::INTERNAL_ERROR,
                BLAZE_UTIL_LINUX_CC,
                100,
                format!(
                    "sched_setscheduler(SCHED_BATCH) failed: {}",
                    get_last_error_string()
                ),
            );
        }
    }

    if io_nice_level >= 0 {
        // SAFETY: ioprio_set syscall with plain integer arguments.
        let r = unsafe {
            libc::syscall(
                libc::SYS_ioprio_set,
                IOPRIO_WHO_PROCESS,
                libc::getpid(),
                (IOPRIO_CLASS_BE << IOPRIO_CLASS_SHIFT) | io_nice_level,
            )
        };
        if r < 0 {
            bazel_die(
                exit_code::INTERNAL_ERROR,
                BLAZE_UTIL_LINUX_CC,
                109,
                format!(
                    "ioprio_set() with class {} and level {} failed: {}",
                    IOPRIO_CLASS_BE,
                    io_nice_level,
                    get_last_error_string()
                ),
            );
        }
    }
}

// Java: src/main/cpp/blaze_util_linux.cc:129 IsSharedLibrary
pub fn is_shared_library(filename: &str) -> bool {
    filename.ends_with(".so")
}

// Java: src/main/cpp/blaze_util_linux.cc:133 GetSystemJavabase
pub fn get_system_javabase() -> String {
    // if JAVA_HOME is defined, then use it as default.
    let javahome = get_path_env("JAVA_HOME");
    if !javahome.is_empty() {
        let javac = join_path(&javahome, "bin/javac");
        let c = cstr(&javac);
        // SAFETY: c is NUL-terminated.
        if unsafe { libc::access(c.as_ptr(), libc::X_OK) } == 0 {
            return javahome;
        }
        bazel_log(
            LogLevel::Warning,
            BLAZE_UTIL_LINUX_CC,
            140,
            "Ignoring JAVA_HOME, because it must point to a JDK, not a JRE.",
        );
    }

    // which javac
    let javac_dir = which("javac");
    if javac_dir.is_empty() {
        return String::new();
    }

    // Resolve all symlinks.
    let javac_dir = make_canonical(&javac_dir);
    if javac_dir.is_empty() {
        return String::new();
    }

    // dirname dirname
    dirname(&dirname(&javac_dir))
}

// Java: src/main/cpp/blaze_util_linux.cc:162 GetStartTime
fn get_start_time(pid: &str) -> Option<String> {
    let statfile = format!("/proc/{}/stat", pid);
    let statline = read_file(&statfile, 0)?;
    let statline = String::from_utf8_lossy(&statline).into_owned();
    let stat_entries: Vec<&str> = statline.split(' ').collect();
    if stat_entries.len() < 22 {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            BLAZE_UTIL_LINUX_CC,
            172,
            format!(
                "Format of stat file at {} is unknown: {}",
                statfile,
                get_last_error_string()
            ),
        );
    }
    // Start time since startup in jiffies. This combined with the PID should be unique.
    Some(stat_entries[21].to_string())
}

// Java: src/main/cpp/blaze_util_linux.cc:212 VerifyServerProcess
/// On Linux we use a combination of PID and start time to identify the server process.
pub fn verify_server_process(pid: i32, output_base: &Path) -> bool {
    let Some(start_time) = get_start_time(&pid.to_string()) else {
        // Cannot read PID file from /proc . Process died meantime, all is good. No stale server
        // is present.
        return false;
    };
    let recorded_start_time = read_file(
        &output_base
            .get_relative("server/server.starttime")
            .as_native_path(),
        0,
    );
    // If start time file got deleted, but PID file didn't, assume that this is an old Blaze
    // process that doesn't know how to write start time files yet.
    match recorded_start_time {
        None => true,
        Some(r) => r == start_time.as_bytes(),
    }
}

// Java: src/main/cpp/blaze_util_linux.cc:229 ExcludePathFromBackup
/// Not supported.
pub fn exclude_path_from_backup(_path: &Path) {}

// Java: src/main/cpp/blaze_util_linux.cc:231 GetExplicitSystemLimit
pub fn get_explicit_system_limit(_resource: i32) -> i32 {
    -1
}

// Java: src/main/cpp/get_self_path_linux.cc:27 GetSelfPath
pub fn get_self_path(_argv0: &str) -> String {
    let mut buffer = vec![0u8; libc::PATH_MAX as usize];
    let path = cstr("/proc/self/exe");
    // SAFETY: buffer has PATH_MAX bytes.
    let mut bytes = unsafe {
        libc::readlink(
            path.as_ptr(),
            buffer.as_mut_ptr() as *mut libc::c_char,
            buffer.len(),
        )
    };
    if bytes == buffer.len() as isize {
        // symlink contents truncated
        bytes = -1;
        crate::util::errors::set_errno(libc::ENAMETOOLONG);
    }
    if bytes == -1 {
        bazel_die(
            exit_code::INTERNAL_ERROR,
            GET_SELF_PATH_LINUX_CC,
            37,
            format!("error reading /proc/self/exe: {}", get_last_error_string()),
        );
    }
    buffer.truncate(bytes as usize);
    String::from_utf8_lossy(&buffer).into_owned()
}

// Java: src/main/cpp/option_processor_unix.cc:31 GetProcessedEnv
pub fn get_processed_env() -> Vec<String> {
    unsafe extern "C" {
        static environ: *const *const libc::c_char;
    }
    let mut processed_env = Vec::new();
    // SAFETY: environ is a NULL-terminated array of NUL-terminated strings; the client is
    // single-threaded here.
    unsafe {
        let mut env = environ;
        while !env.is_null() && !(*env).is_null() {
            processed_env.push(CStr::from_ptr(*env).to_string_lossy().into_owned());
            env = env.add(1);
        }
    }
    processed_env
}
