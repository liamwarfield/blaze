//! The parts of `java.lang.ProcessBuilder` and `java.lang.Process` (JDK 21, Linux) that
//! `JavaSubprocessFactory` uses, with the JDK's error texts and its PATH search semantics.
//!
//! The JDK child (`childproc.c`) changes directory, closes every inherited descriptor above 2,
//! and then searches the *parent's* `PATH` (not the child's) with `JDK_execvpe`, falling back to
//! `/bin/sh` for `ENOEXEC`. This module reproduces that in a `pre_exec` hook so that the same
//! programs are found and the same `errno` is reported.

use std::ffi::{CString, c_char};
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::OnceLock;
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
        let mut cmd = std::process::Command::new(&self.command[0]);
        cmd.stdin(Stdio::piped());

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
        cmd.stdout(Stdio::from(stdout_write));
        cmd.stderr(Stdio::from(stderr_write));
        if let Some(dir) = &self.directory {
            cmd.current_dir(dir.get_path());
        }

        let exec = ExecPlan::new(&self.command, &self.environment);
        // SAFETY: the hook only calls async-signal-safe functions (close_range, execve) on data
        // prepared before the fork.
        unsafe {
            cmd.pre_exec(move || exec.run());
        }
        let spawned = cmd.spawn();
        // Drop the builder so the parent's copies of the write ends are closed.
        drop(cmd);
        let mut child = spawned.map_err(|e| java_native_error(&e))?;

        let pid = child.id();
        // SAFETY: plain syscall; the result is checked.
        let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
        let pidfd = if pidfd >= 0 {
            // SAFETY: fresh descriptor we own.
            Some(unsafe { OwnedFd::from_raw_fd(pidfd as i32) })
        } else {
            None
        };
        let stdin = child.stdin.take();
        Ok(JavaProcess {
            pid,
            pidfd,
            state: Mutex::new(ProcState {
                child: Some(child),
                exit_code: None,
            }),
            stdin: Mutex::new(stdin),
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

/// Message of the JDK's native `throwIOException`: `error=<errno>, <strerror>`.
fn java_native_error(e: &io::Error) -> String {
    match e.raw_os_error() {
        Some(errno) => format!("error={}, {}", errno, strerror(errno)),
        None => e.to_string(),
    }
}

fn pipe() -> io::Result<(File, OwnedFd)> {
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
    // execve_with_shell_fallback (JDK 21). Runs in the forked child; never returns on success.
    fn run(&self) -> io::Result<()> {
        // childproc.c closeDescriptors: nothing above stderr survives the exec. Marking them
        // close-on-exec keeps std's error-reporting pipe working until the exec.
        // SAFETY: plain syscall.
        unsafe {
            libc::syscall(
                libc::SYS_close_range,
                3 as libc::c_uint,
                libc::c_uint::MAX,
                libc::CLOSE_RANGE_CLOEXEC,
            );
        }
        if self.empty_file {
            return Err(io::Error::from_raw_os_error(libc::ENOENT));
        }
        let mut errno = 0;
        let mut sticky_errno = 0;
        for (i, cand) in self.candidates.iter().enumerate() {
            match cand {
                Candidate::TooLong => {
                    errno = libc::ENAMETOOLONG;
                    continue;
                }
                Candidate::Exec { file, .. } => {
                    // SAFETY: NUL-terminated arrays of valid C strings.
                    unsafe {
                        libc::execve(
                            file.as_ptr(),
                            self.argv_ptrs.as_ptr(),
                            self.envp_ptrs.as_ptr(),
                        );
                    }
                    errno = last_errno();
                    if errno == libc::ENOEXEC {
                        // SAFETY: as above.
                        unsafe {
                            libc::execve(
                                c"/bin/sh".as_ptr(),
                                self.sh_argv_ptrs[i].as_ptr(),
                                self.envp_ptrs.as_ptr(),
                            );
                        }
                        errno = last_errno();
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

fn last_errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

struct ProcState {
    // Always Some until the process is dropped while still running (see Drop).
    child: Option<Child>,
    exit_code: Option<i32>,
}

impl ProcState {
    // Java: java/lang/ProcessImpl.java hasExited (set by the reaper; here we reap on demand).
    fn has_exited(&mut self) -> bool {
        if self.exit_code.is_some() {
            return true;
        }
        let Some(child) = self.child.as_mut() else {
            return false;
        };
        if let Ok(Some(status)) = child.try_wait() {
            // Java: ProcessHandleImpl_unix.c waitForProcessExit: WIFEXITED -> WEXITSTATUS,
            // WIFSIGNALED -> 0x80 + WTERMSIG.
            let code = match (status.code(), status.signal()) {
                (Some(code), _) => code,
                (None, Some(sig)) => 0x80 + sig,
                (None, None) => status.into_raw(),
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
    stdin: Mutex<Option<ChildStdin>>,
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
        !self.state.lock().has_exited()
    }

    /// `Process.exitValue()`.
    pub fn exit_value(&self) -> i32 {
        let mut state = self.state.lock();
        if !state.has_exited() {
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
        if !state.has_exited() {
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
                if state.has_exited() {
                    return Ok(state.exit_code.unwrap());
                }
            }
            thread_interrupt::wait_readable(self.pidfd.as_ref().map(|f| f.as_raw_fd()), None)?;
        }
    }

    /// `Process.waitFor(long, TimeUnit.MILLISECONDS)`.
    // Java: java/lang/ProcessImpl.java waitFor(long, TimeUnit) (JDK 21)
    pub fn wait_for_millis(&self, timeout_millis: i64) -> Result<bool, InterruptedException> {
        if self.state.lock().has_exited() {
            return Ok(true);
        }
        if timeout_millis <= 0 {
            return Ok(false);
        }
        let deadline = Instant::now().checked_add(Duration::from_millis(timeout_millis as u64));
        loop {
            let remaining = deadline.map(|d| d.saturating_duration_since(Instant::now()));
            thread_interrupt::wait_readable(self.pidfd.as_ref().map(|f| f.as_raw_fd()), remaining)?;
            if self.state.lock().has_exited() {
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
        let mut state = self.state.lock();
        if !state.has_exited()
            && let Some(mut child) = state.child.take()
        {
            let _ = std::thread::Builder::new()
                .name("process reaper".to_string())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
    }
}
