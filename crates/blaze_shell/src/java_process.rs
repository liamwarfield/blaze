//! The parts of `java.lang.ProcessBuilder` and `java.lang.Process` (JDK 21, Linux) that
//! `JavaSubprocessFactory` uses, with the JDK's error texts and its PATH search semantics.
//!
//! The JDK child (`childproc.c`) changes directory, closes every inherited descriptor above 2,
//! and then searches the *parent's* `PATH` (not the child's) with `JDK_execvpe`, falling back to
//! `/bin/sh` for `ENOEXEC`. This module reproduces that with `posix_spawn` (see [`ExecPlan`]) so
//! that the same programs are found and the same `errno` is reported.

use std::ffi::{CString, c_char};
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::java_file::JavaFile;
use crate::thread_interrupt::{self, InterruptedException};

/// `strerror(errnum)` from the C library.
pub fn strerror(errnum: i32) -> String {
    let mut buf = [0 as c_char; 256];
    // SAFETY: buf is valid for buf.len() bytes; the XSI strerror_r NUL-terminates on success.
    let rc = unsafe { libc::strerror_r(errnum, buf.as_mut_ptr(), buf.len()) };
    if rc != 0 {
        return format!("Unknown error {errnum}");
    }
    // SAFETY: NUL-terminated by strerror_r.
    unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

/// The message Java would carry in an `IOException` for this error (`getMessage()`), or `None`
/// when Java's would be null.
pub fn java_io_message(e: &io::Error) -> Option<String> {
    if let Some(errno) = e.raw_os_error() {
        // Java's native I/O reports strerror(errno) as the message.
        return Some(strerror(errno));
    }
    let s = e.to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// `Throwable.toString()` of an `IOException` carrying `e`: `java.io.IOException[: message]`.
pub fn java_io_exception_to_string(e: &io::Error) -> String {
    match java_io_message(e) {
        Some(m) => format!("java.io.IOException: {m}"),
        None => "java.io.IOException".to_string(),
    }
}

/// Port of `java.lang.ProcessBuilder.Redirect` (the kinds Bazel uses).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Redirect {
    /// `Redirect.PIPE`.
    Pipe,
    /// `Redirect.to(file)`: truncate.
    To(JavaFile),
    /// `Redirect.appendTo(file)`.
    AppendTo(JavaFile),
}

/// Port of `java.lang.ProcessBuilder` (Linux, the subset Bazel uses).
#[derive(Clone, Debug)]
pub struct JavaProcessBuilder {
    command: Vec<String>,
    environment: IndexMap<String, String>,
    redirect_output: Redirect,
    redirect_error: Redirect,
    redirect_error_stream: bool,
    directory: Option<JavaFile>,
}

impl JavaProcessBuilder {
    /// `new ProcessBuilder()` followed by `environment().clear()`.
    pub fn new() -> JavaProcessBuilder {
        JavaProcessBuilder {
            command: Vec::new(),
            environment: IndexMap::new(),
            redirect_output: Redirect::Pipe,
            redirect_error: Redirect::Pipe,
            redirect_error_stream: false,
            directory: None,
        }
    }

    /// `ProcessBuilder.command(List)`.
    pub fn command(&mut self, command: Vec<String>) -> &mut Self {
        self.command = command;
        self
    }

    /// `environment().put(key, value)`, with `ProcessEnvironment`'s validation.
    // Java: java/lang/ProcessEnvironment.java StringEnvironment.put / validateVariable /
    // validateValue (JDK 21)
    pub fn environment_put(&mut self, key: &str, value: &str) -> &mut Self {
        if key.contains('=') || key.contains('\0') {
            panic!("IllegalArgumentException: Invalid environment variable name: \"{key}\"");
        }
        if value.contains('\0') {
            panic!("IllegalArgumentException: Invalid environment variable value: \"{value}\"");
        }
        self.environment.insert(key.to_string(), value.to_string());
        self
    }

    pub fn redirect_output(&mut self, r: Redirect) -> &mut Self {
        self.redirect_output = r;
        self
    }

    pub fn redirect_error(&mut self, r: Redirect) -> &mut Self {
        self.redirect_error = r;
        self
    }

    pub fn redirect_error_stream(&mut self, b: bool) -> &mut Self {
        self.redirect_error_stream = b;
        self
    }

    /// `ProcessBuilder.directory(File)`.
    pub fn directory(&mut self, dir: Option<JavaFile>) -> &mut Self {
        self.directory = dir;
        self
    }

    /// `ProcessBuilder.start()`.
    // Java: java/lang/ProcessBuilder.java start (JDK 21)
    pub fn start(&self) -> io::Result<JavaProcess> {
        if self.command.is_empty() {
            panic!("IndexOutOfBoundsException: Index 0 out of bounds for length 0");
        }
        for arg in &self.command {
            if arg.contains('\0') {
                return Err(io::Error::other("invalid null character in command"));
            }
        }
        let prog = &self.command[0];
        let dir = self.directory.as_ref().map(|d| d.to_string());
        match self.start_impl() {
            Ok(p) => Ok(p),
            Err(detail) => {
                // It's much easier for us to create a high-quality error message than the
                // low-level C code which found the problem.
                Err(io::Error::other(format!(
                    "Cannot run program \"{}\"{}: {}",
                    prog,
                    match &dir {
                        None => String::new(),
                        Some(d) => format!(" (in directory \"{d}\")"),
                    },
                    detail
                )))
            }
        }
    }

    // Java: java/lang/ProcessImpl.java start / forkAndExec (JDK 21, Linux). Returns the
    // low-level exception message on failure.
    fn start_impl(&self) -> Result<JavaProcess, String> {
        // stdin: always a pipe (Bazel never redirects it).
        let (stdin_read, stdin_write) = pipe().map_err(|e| java_native_error(&e))?;
        let stdin_read = OwnedFd::from(stdin_read);

        // stdout
        let (stdout_read, stdout_write): (Option<File>, OwnedFd) = match &self.redirect_output {
            Redirect::Pipe => {
                let (r, w) = pipe().map_err(|e| java_native_error(&e))?;
                (Some(r), w)
            }
            Redirect::To(f) => (None, open_redirect(f, false)?),
            Redirect::AppendTo(f) => (None, open_redirect(f, true)?),
        };
        // stderr
        let (stderr_read, stderr_write): (Option<File>, OwnedFd) = if self.redirect_error_stream {
            (
                None,
                stdout_write
                    .try_clone()
                    .map_err(|e| java_native_error(&e))?,
            )
        } else {
            match &self.redirect_error {
                Redirect::Pipe => {
                    let (r, w) = pipe().map_err(|e| java_native_error(&e))?;
                    (Some(r), w)
                }
                Redirect::To(f) => (None, open_redirect(f, false)?),
                Redirect::AppendTo(f) => (None, open_redirect(f, true)?),
            }
        };

        let exec = ExecPlan::new(&self.command, &self.environment);
        let dir = self
            .directory
            .as_ref()
            .map(|d| cstring(d.get_path().as_bytes()));
        let spawned = exec.spawn(
            dir.as_deref(),
            [
                stdin_read.as_raw_fd(),
                stdout_write.as_raw_fd(),
                stderr_write.as_raw_fd(),
            ],
        );
        // The parent's copies of the child's ends are closed.
        drop((stdin_read, stdout_write, stderr_write));
        let pid = spawned.map_err(|e| java_native_error(&e))?;

        // SAFETY: plain syscall; the result is checked.
        let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        let pidfd = if pidfd >= 0 {
            // SAFETY: fresh descriptor we own.
            Some(unsafe { OwnedFd::from_raw_fd(pidfd as i32) })
        } else {
            None
        };
        Ok(JavaProcess {
            pid: pid as u32,
            pidfd,
            state: Mutex::new(ProcState {
                reaped: false,
                exit_code: None,
            }),
            stdin: Mutex::new(Some(File::from(stdin_write))),
            stdout: Mutex::new(Some(match stdout_read {
                Some(f) => Box::new(f),
                None => Box::new(io::empty()),
            })),
            stderr: Mutex::new(Some(match stderr_read {
                Some(f) => Box::new(f),
                None => Box::new(io::empty()),
            })),
        })
    }
}

/// Children are started in a new session (see [`set_start_children_in_new_session`]).
static NEW_SESSION: AtomicBool = AtomicBool::new(false);

/// Starts every later child in a session of its own (`setsid`), without a controlling terminal.
///
/// PORT: Blaze only. Outside `--batch` Bazel's server is a daemon: the client starts it with
/// `ExecuteDaemon` (`src/main/tools/daemonize.cc`: `setsid`), so the processes the server starts
/// have no controlling terminal and do not receive the terminal's signals; a Ctrl-C reaches only
/// the client, which cancels the command. Blaze runs that server in the client's process, so the
/// in-process server calls this to give its children the session Bazel's server would have. In
/// `--batch` mode the JVM is the client's process (`execv`) and its children share its terminal,
/// as here by default.
pub fn set_start_children_in_new_session(new_session: bool) {
    NEW_SESSION.store(new_session, Ordering::SeqCst);
}

/// Message of the JDK's native `throwIOException`: `error=<errno>, <strerror>`.
fn java_native_error(e: &io::Error) -> String {
    match e.raw_os_error() {
        Some(errno) => format!("error={}, {}", errno, strerror(errno)),
        None => e.to_string(),
    }
}

fn pipe() -> io::Result<(File, OwnedFd)> {
    // Both ends are close-on-exec; the child gets its end through a `dup2` file action.
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: fds is a valid 2-element array.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both descriptors are fresh and owned here.
    unsafe { Ok((File::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))) }
}

// Java: java/io/FileOutputStream.java <init>(File, boolean); the FileNotFoundException message is
// "<path> (<reason>)".
fn open_redirect(f: &JavaFile, append: bool) -> Result<OwnedFd, String> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).mode(0o666);
    if append {
        options.append(true);
    } else {
        options.truncate(true);
    }
    match options.open(f.get_path()) {
        Ok(file) => Ok(OwnedFd::from(file)),
        Err(e) => Err(format!(
            "{} ({})",
            f.get_path(),
            match e.raw_os_error() {
                Some(errno) => strerror(errno),
                None => e.to_string(),
            }
        )),
    }
}

/// The parent's `PATH` split as the JDK does (`ProcessImpl_md.c` `effectivePathv`): taken once,
/// `":/bin:/usr/bin"` when unset, empty components become `"./"`.
fn parent_pathv() -> &'static [Vec<u8>] {
    static PATHV: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    PATHV.get_or_init(|| {
        use std::os::unix::ffi::OsStrExt;
        let path: Vec<u8> = match std::env::var_os("PATH") {
            Some(p) => p.as_bytes().to_vec(),
            None => b":/bin:/usr/bin".to_vec(),
        };
        path.split(|b| *b == b':')
            .map(|c| {
                if c.is_empty() {
                    b"./".to_vec()
                } else {
                    c.to_vec()
                }
            })
            .collect()
    })
}

enum Candidate {
    Exec {
        file: CString,
        sh_argv: Vec<CString>,
    },
    TooLong,
}

/// Everything the child needs to run `JDK_execvpe`, prepared in the parent.
///
/// PORT: the JDK on Linux starts children with `posix_spawn` (`jdk.lang.Process.launchMechanism`
/// defaults to `POSIX_SPAWN`; the spawned `jspawnhelper` runs `childproc.c`), which never copies
/// the parent's address space: the kernel's overcommit heuristic does not account the parent's
/// mappings, so a server with hundreds of large thread stacks can still start children (`fork`
/// fails with ENOMEM there). Blaze calls glibc's `posix_spawn` directly (also `CLONE_VM |
/// CLONE_VFORK`) with `childproc.c`'s steps as file actions (redirections, `chdir`, closing every
/// descriptor above 2) and runs `JDK_execvpe`'s search in the parent, one `posix_spawn` per
/// candidate: a candidate that fails to exec leaves no child (glibc reaps it) and reports the
/// `errno` the child's `execve` got, so the same program is found and the same error is
/// reported.
struct ExecPlan {
    search: bool,
    empty_file: bool,
    candidates: Vec<Candidate>,
    _argv: Vec<CString>,
    argv_ptrs: Vec<*const c_char>,
    _envp: Vec<CString>,
    envp_ptrs: Vec<*const c_char>,
    sh_argv_ptrs: Vec<Vec<*const c_char>>,
}

// SAFETY: the raw pointers point into CStrings owned by the same struct; they are only read.
unsafe impl Send for ExecPlan {}
// SAFETY: as above.
unsafe impl Sync for ExecPlan {}

fn cstring(s: &[u8]) -> CString {
    CString::new(s).expect("NUL checked by caller")
}

impl ExecPlan {
    fn new(argv: &[String], env: &IndexMap<String, String>) -> ExecPlan {
        let argv_c: Vec<CString> = argv.iter().map(|a| cstring(a.as_bytes())).collect();
        // PORT: the JDK builds envp from a HashMap (unspecified order); we keep insertion order.
        let envp_c: Vec<CString> = env
            .iter()
            .map(|(k, v)| cstring(format!("{k}={v}").as_bytes()))
            .collect();
        let file = argv[0].as_bytes();
        let empty_file = file.is_empty();
        let search = !file.contains(&b'/');
        let mut candidates = Vec::new();
        let make_sh_argv = |f: &CString| -> Vec<CString> {
            let mut v = vec![cstring(b"/bin/sh"), f.clone()];
            v.extend(argv_c[1..].iter().cloned());
            v
        };
        if !empty_file {
            if search {
                let path_max = libc::PATH_MAX as usize;
                for dir in parent_pathv() {
                    if file.len() + dir.len() + 2 >= path_max {
                        candidates.push(Candidate::TooLong);
                        continue;
                    }
                    let mut expanded = dir.clone();
                    if expanded.last() != Some(&b'/') {
                        expanded.push(b'/');
                    }
                    expanded.extend_from_slice(file);
                    let f = cstring(&expanded);
                    let sh_argv = make_sh_argv(&f);
                    candidates.push(Candidate::Exec { file: f, sh_argv });
                }
            } else {
                let f = cstring(file);
                let sh_argv = make_sh_argv(&f);
                candidates.push(Candidate::Exec { file: f, sh_argv });
            }
        }
        let mut argv_ptrs: Vec<*const c_char> = argv_c.iter().map(|c| c.as_ptr()).collect();
        argv_ptrs.push(std::ptr::null());
        let mut envp_ptrs: Vec<*const c_char> = envp_c.iter().map(|c| c.as_ptr()).collect();
        envp_ptrs.push(std::ptr::null());
        let sh_argv_ptrs = candidates
            .iter()
            .map(|c| match c {
                Candidate::Exec { sh_argv, .. } => {
                    let mut v: Vec<*const c_char> = sh_argv.iter().map(|c| c.as_ptr()).collect();
                    v.push(std::ptr::null());
                    v
                }
                Candidate::TooLong => Vec::new(),
            })
            .collect();
        ExecPlan {
            search,
            empty_file,
            candidates,
            _argv: argv_c,
            argv_ptrs,
            _envp: envp_c,
            envp_ptrs,
            sh_argv_ptrs,
        }
    }

    // Java: src/java.base/unix/native/libjava/childproc.c JDK_execvpe /
    // execve_with_shell_fallback (JDK 21). Returns the child's pid, or the `errno` of the last
    // attempt as the JDK child reports it.
    fn spawn(&self, dir: Option<&std::ffi::CStr>, stdio: [libc::c_int; 3]) -> io::Result<i32> {
        if self.empty_file {
            return Err(io::Error::from_raw_os_error(libc::ENOENT));
        }
        let actions = SpawnActions::new(dir, stdio)?;
        let attr = SpawnAttr::new()?;
        let mut errno = 0;
        let mut sticky_errno = 0;
        for (i, cand) in self.candidates.iter().enumerate() {
            match cand {
                Candidate::TooLong => {
                    errno = libc::ENAMETOOLONG;
                    continue;
                }
                Candidate::Exec { file, .. } => {
                    match posix_spawn(file, &actions, &attr, &self.argv_ptrs, &self.envp_ptrs) {
                        Ok(pid) => return Ok(pid),
                        Err(e) => errno = e,
                    }
                    if errno == libc::ENOEXEC {
                        match posix_spawn(
                            c"/bin/sh",
                            &actions,
                            &attr,
                            &self.sh_argv_ptrs[i],
                            &self.envp_ptrs,
                        ) {
                            Ok(pid) => return Ok(pid),
                            Err(e) => errno = e,
                        }
                    }
                }
            }
            if !self.search {
                break;
            }
            // There are 3 responses to various classes of errno: return immediately, continue
            // (especially for ENOENT), or continue with "sticky" errno.
            match errno {
                libc::EACCES => sticky_errno = errno,
                libc::ENOENT
                | libc::ENOTDIR
                | libc::ELOOP
                | libc::ESTALE
                | libc::ENODEV
                | libc::ETIMEDOUT => {}
                _ => return Err(io::Error::from_raw_os_error(errno)),
            }
        }
        if sticky_errno != 0 {
            errno = sticky_errno;
        }
        Err(io::Error::from_raw_os_error(errno))
    }
}

/// `posix_spawn(3)`: the child's pid, or the error number (of a failed file action or exec).
fn posix_spawn(
    file: &std::ffi::CStr,
    actions: &SpawnActions,
    attr: &SpawnAttr,
    argv: &[*const c_char],
    envp: &[*const c_char],
) -> Result<i32, i32> {
    let mut pid: libc::pid_t = 0;
    // SAFETY: all pointers are valid, NUL-terminated as posix_spawn requires, and outlive the
    // call; the actions and attributes are initialized.
    let rc = unsafe {
        libc::posix_spawn(
            &mut pid,
            file.as_ptr(),
            &actions.0,
            &attr.0,
            argv.as_ptr() as *const *mut c_char,
            envp.as_ptr() as *const *mut c_char,
        )
    };
    if rc == 0 { Ok(pid) } else { Err(rc) }
}

/// `childproc.c`'s steps before the exec, as `posix_spawn` file actions: stdin, stdout and
/// stderr from the given descriptors, `chdir` to the working directory, and every other
/// descriptor closed (`closeDescriptors`).
struct SpawnActions(libc::posix_spawn_file_actions_t);

impl SpawnActions {
    fn new(dir: Option<&std::ffi::CStr>, stdio: [libc::c_int; 3]) -> io::Result<SpawnActions> {
        let mut actions = std::mem::MaybeUninit::<libc::posix_spawn_file_actions_t>::uninit();
        // SAFETY: initializes the value.
        check_rc(unsafe { libc::posix_spawn_file_actions_init(actions.as_mut_ptr()) })?;
        // SAFETY: initialized above.
        let mut actions = SpawnActions(unsafe { actions.assume_init() });
        for (target, fd) in stdio.into_iter().enumerate() {
            // SAFETY: initialized actions; plain integers.
            check_rc(unsafe {
                libc::posix_spawn_file_actions_adddup2(&mut actions.0, fd, target as libc::c_int)
            })?;
        }
        if let Some(dir) = dir {
            // SAFETY: initialized actions; NUL-terminated path (copied by glibc).
            check_rc(unsafe {
                libc::posix_spawn_file_actions_addchdir_np(&mut actions.0, dir.as_ptr())
            })?;
        }
        // SAFETY: initialized actions.
        check_rc(unsafe { libc::posix_spawn_file_actions_addclosefrom_np(&mut actions.0, 3) })?;
        Ok(actions)
    }
}

impl Drop for SpawnActions {
    fn drop(&mut self) {
        // SAFETY: initialized in `new`.
        unsafe {
            libc::posix_spawn_file_actions_destroy(&mut self.0);
        }
    }
}

/// The child's signal state: an empty signal mask and `SIGPIPE` at its default (Rust ignores
/// it in the parent; the JVM's children get the default disposition), and a new session when
/// [`set_start_children_in_new_session`] asked for one.
struct SpawnAttr(libc::posix_spawnattr_t);

impl SpawnAttr {
    fn new() -> io::Result<SpawnAttr> {
        let mut attr = std::mem::MaybeUninit::<libc::posix_spawnattr_t>::uninit();
        // SAFETY: initializes the value.
        check_rc(unsafe { libc::posix_spawnattr_init(attr.as_mut_ptr()) })?;
        // SAFETY: initialized above.
        let mut attr = SpawnAttr(unsafe { attr.assume_init() });
        // SAFETY: sigset_t values are initialized by sigemptyset before use; attr is valid.
        unsafe {
            let mut empty = std::mem::zeroed::<libc::sigset_t>();
            libc::sigemptyset(&mut empty);
            check_rc(libc::posix_spawnattr_setsigmask(&mut attr.0, &empty))?;
            let mut default = std::mem::zeroed::<libc::sigset_t>();
            libc::sigemptyset(&mut default);
            libc::sigaddset(&mut default, libc::SIGPIPE);
            check_rc(libc::posix_spawnattr_setsigdefault(&mut attr.0, &default))?;
        }
        let mut flags = libc::POSIX_SPAWN_SETSIGMASK | libc::POSIX_SPAWN_SETSIGDEF;
        if NEW_SESSION.load(Ordering::SeqCst) {
            flags |= libc::POSIX_SPAWN_SETSID as libc::c_int;
        }
        // SAFETY: initialized attr.
        check_rc(unsafe { libc::posix_spawnattr_setflags(&mut attr.0, flags as libc::c_short) })?;
        Ok(attr)
    }
}

impl Drop for SpawnAttr {
    fn drop(&mut self) {
        // SAFETY: initialized in `new`.
        unsafe {
            libc::posix_spawnattr_destroy(&mut self.0);
        }
    }
}

fn check_rc(rc: libc::c_int) -> io::Result<()> {
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(rc))
    }
}

struct ProcState {
    /// The child was handed to a reaper thread (see `Drop`); nobody else may wait for it.
    reaped: bool,
    exit_code: Option<i32>,
}

impl ProcState {
    // Java: java/lang/ProcessImpl.java hasExited (set by the reaper; here we reap on demand).
    fn has_exited(&mut self, pid: u32) -> bool {
        if self.exit_code.is_some() {
            return true;
        }
        if self.reaped {
            return false;
        }
        let mut status: libc::c_int = 0;
        // SAFETY: plain syscall on our own, not yet reaped child.
        let rc = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
        if rc == pid as libc::pid_t {
            // Java: ProcessHandleImpl_unix.c waitForProcessExit: WIFEXITED -> WEXITSTATUS,
            // WIFSIGNALED -> 0x80 + WTERMSIG.
            let code = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else if libc::WIFSIGNALED(status) {
                0x80 + libc::WTERMSIG(status)
            } else {
                status
            };
            self.exit_code = Some(code);
            return true;
        }
        false
    }
}

/// Port of `java.lang.Process` (Linux `ProcessImpl`).
pub struct JavaProcess {
    pid: u32,
    pidfd: Option<OwnedFd>,
    state: Mutex<ProcState>,
    stdin: Mutex<Option<File>>,
    stdout: Mutex<Option<Box<dyn Read + Send>>>,
    stderr: Mutex<Option<Box<dyn Read + Send>>>,
}

impl JavaProcess {
    /// `Process.pid()`.
    pub fn pid(&self) -> i64 {
        self.pid as i64
    }

    /// `Process.getOutputStream()`. PORT: ownership moves to the caller on the first call; later
    /// calls return `None`.
    pub fn get_output_stream(&self) -> Option<Box<dyn io::Write + Send>> {
        self.stdin
            .lock()
            .take()
            .map(|s| Box::new(s) as Box<dyn io::Write + Send>)
    }

    /// `Process.getInputStream()` (the child's stdout). PORT: ownership moves on the first call.
    pub fn get_input_stream(&self) -> Option<Box<dyn Read + Send>> {
        self.stdout.lock().take()
    }

    /// `Process.getErrorStream()`. PORT: ownership moves on the first call.
    pub fn get_error_stream(&self) -> Option<Box<dyn Read + Send>> {
        self.stderr.lock().take()
    }

    /// `Process.isAlive()`.
    pub fn is_alive(&self) -> bool {
        !self.state.lock().has_exited(self.pid)
    }

    /// `Process.exitValue()`.
    pub fn exit_value(&self) -> i32 {
        let mut state = self.state.lock();
        if !state.has_exited(self.pid) {
            panic!("IllegalThreadStateException: process hasn't exited");
        }
        state.exit_code.unwrap()
    }

    // Java: java/lang/ProcessImpl.java destroy(boolean) (JDK 21, Linux)
    fn destroy_impl(&self, force: bool) {
        // There is a risk that pid will be recycled, causing us to kill the wrong process! So we
        // only terminate processes that appear to still be running. Reaping happens under the
        // same lock, so the pid cannot be recycled meanwhile.
        let mut state = self.state.lock();
        if !state.has_exited(self.pid) {
            // SAFETY: plain syscall on a pid we have not reaped.
            unsafe {
                libc::kill(
                    self.pid as libc::pid_t,
                    if force { libc::SIGKILL } else { libc::SIGTERM },
                );
            }
        }
        // PORT: the JDK also closes the parent's ends of stdin/stdout/stderr here. Ownership of
        // those streams has moved to their readers/writers already, so they are not closed; the
        // readers see EOF when the process dies, as Java's blocked readers do.
    }

    /// `Process.destroy()`: SIGTERM.
    pub fn destroy(&self) {
        self.destroy_impl(false);
    }

    /// `Process.destroyForcibly()`: SIGKILL.
    pub fn destroy_forcibly(&self) {
        self.destroy_impl(true);
    }

    /// `Process.waitFor()`.
    // Java: java/lang/ProcessImpl.java waitFor (JDK 21)
    pub fn wait_for(&self) -> Result<i32, InterruptedException> {
        loop {
            {
                let mut state = self.state.lock();
                if state.has_exited(self.pid) {
                    return Ok(state.exit_code.unwrap());
                }
            }
            thread_interrupt::wait_readable(self.pidfd.as_ref().map(|f| f.as_raw_fd()), None)?;
        }
    }

    /// `Process.waitFor(long, TimeUnit.MILLISECONDS)`.
    // Java: java/lang/ProcessImpl.java waitFor(long, TimeUnit) (JDK 21)
    pub fn wait_for_millis(&self, timeout_millis: i64) -> Result<bool, InterruptedException> {
        if self.state.lock().has_exited(self.pid) {
            return Ok(true);
        }
        if timeout_millis <= 0 {
            return Ok(false);
        }
        let deadline = Instant::now().checked_add(Duration::from_millis(timeout_millis as u64));
        loop {
            let remaining = deadline.map(|d| d.saturating_duration_since(Instant::now()));
            thread_interrupt::wait_readable(self.pidfd.as_ref().map(|f| f.as_raw_fd()), remaining)?;
            if self.state.lock().has_exited(self.pid) {
                return Ok(true);
            }
            if let Some(d) = deadline
                && Instant::now() >= d
            {
                return Ok(false);
            }
        }
    }
}

impl Drop for JavaProcess {
    fn drop(&mut self) {
        // The JDK's process reaper thread reaps every child whether or not anyone waits for it.
        // Do the same for a child that is still running when its Process goes away.
        let pid = self.pid;
        let mut state = self.state.lock();
        if !state.has_exited(pid) && !state.reaped {
            state.reaped = true;
            let _ = std::thread::Builder::new()
                .name("process reaper".to_string())
                .spawn(move || {
                    let mut status: libc::c_int = 0;
                    // SAFETY: blocking wait for our own child, which nobody else waits for.
                    unsafe {
                        libc::waitpid(pid as libc::pid_t, &mut status, 0);
                    }
                });
        }
    }
}
