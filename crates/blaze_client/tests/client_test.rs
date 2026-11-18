//! Locking and server-ownership tests of the client.
//!
//! The pinned tree has no C++ unit test for `AcquireLock` (blaze_util_posix.cc); its behaviour is
//! covered by src/test/shell/integration/client_test.sh, whose locking scenarios are ported here
//! with the Java/C++ names kept (`test_noblock_for_lock_with_batch`, ...). The other tests are
//! Blaze-only: they cover what differs because Blaze runs the server in-process (review
//! 2026-11-18, S1-1 and S2-1).
//!
//! `AcquireLock` and the client's fatal errors exit the process (`BAZEL_DIE`), so every step that
//! may die runs in a child process: this test binary re-executes itself for the same test with
//! `BLAZE_CLIENT_TEST_CHILD_OP` set (see `run_as_child_if_requested`), and the parent checks the
//! child's exit code and output, as the shell tests do.

use std::cell::RefCell;
use std::ffi::CString;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};

use blaze_client::archive_utils::{EmbeddedArchive, LoggingInfo};
use blaze_client::blaze::{BlazeServer, find_live_foreign_server};
use blaze_client::platform::{DurationMillis, LockMode, acquire_lock, release_lock};
use blaze_client::server::{InProcessServer, ServerStartup};
use blaze_client::startup_options::StartupOptions;
use blaze_client::util::exit_code;
use blaze_client::util::file::{change_directory, get_cwd, make_canonical};
use blaze_client::util::path::{Path, join_path};
use blaze_protos::command_server::RunRequest;

// ---------------------------------------------------------------------------------------------
// Child processes

const CHILD_OP: &str = "BLAZE_CLIENT_TEST_CHILD_OP";
const CHILD_ARGS: &str = "BLAZE_CLIENT_TEST_CHILD_ARGS";

/// Lines the child writes to stdout for the parent.
const WAIT_MS: &str = "BLAZE_TEST_WAIT_MS=";
const ARG: &str = "BLAZE_TEST_ARG=";
const OUTPUT_BASE_LOCK_HELD: &str = "BLAZE_TEST_OUTPUT_BASE_LOCK_HELD=";
const INSTALL_BASE_LOCK_HELD: &str = "BLAZE_TEST_INSTALL_BASE_LOCK_HELD=";

/// The install md5 of the test archive (any 32 hex digits).
const INSTALL_MD5: &str = "0123456789abcdef0123456789abcdef";

/// Tests in this file run one at a time. A fork duplicates every descriptor of this process
/// until the child execs, including the lock descriptors (an OFD lock lives as long as any
/// descriptor of its open file description), so a lock released by one test could stay held for
/// a moment by a child another test is spawning.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Writes one line to the real stdout (not through `println!`, which the test harness may
/// capture).
fn emit(line: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{}", line);
    let _ = stdout.flush();
}

/// First statement of every test that spawns children: when this process is such a child, does
/// what the parent asked for and exits.
fn run_as_child_if_requested() {
    let Ok(op) = std::env::var(CHILD_OP) else {
        return;
    };
    let args: Vec<String> = std::env::var(CHILD_ARGS)
        .unwrap()
        .split('\n')
        .map(str::to_string)
        .collect();
    // With --nocapture the test harness has just printed "test <name> ... " without a newline:
    // end that line, so that every line emitted below starts at the beginning of a line.
    emit("");
    match op.as_str() {
        "lock" => child_lock(&args),
        "client" => child_client(&args),
        other => panic!("unknown child op '{}'", other),
    }
}

/// Child op "lock": `AcquireLock(name, path, mode, /* batch_mode= */ false, block)`. Exits 0 with
/// the lock held (process exit releases it), unless `AcquireLock` dies.
fn child_lock(args: &[String]) -> ! {
    let [name, path, mode, block] = args else {
        panic!("lock: bad arguments {:?}", args);
    };
    let mode = match mode.as_str() {
        "shared" => LockMode::Shared,
        "exclusive" => LockMode::Exclusive,
        other => panic!("lock: bad mode '{}'", other),
    };
    let (_lock, wait_time) = acquire_lock(name, &Path::new(path), mode, false, block == "block");
    emit(&format!("{}{}", WAIT_MS, wait_time.millis));
    std::process::exit(0)
}

/// Child op "client": the whole client (`blaze::Main`) in `<dir>/workspace`, with the output
/// user root `<dir>/root` and the output base `<dir>/output_base`, followed by the given
/// startup options and command. The in-process server is a [`ProbeServer`].
fn child_client(args: &[String]) -> ! {
    let dir = &args[0];
    assert!(change_directory(&join_path(dir, "workspace")));
    let mut argv = vec![
        "blaze".to_string(),
        "--ignore_all_rc_files".to_string(),
        format!("--output_user_root={}", join_path(dir, "root")),
        format!("--output_base={}", join_path(dir, "output_base")),
    ];
    argv.extend(args[1..].iter().cloned());
    let server = ProbeServer::new(
        &output_base_lock_path(dir),
        &install_base_lock_path(dir),
        /* emit= */ true,
        0,
    );
    let code = blaze_client::main(argv, &server, &test_archive());
    std::process::exit(code)
}

/// A command that re-executes this test binary for `test_name` only, as a child with `op`.
fn child_command(test_name: &str, op: &str, args: &[&str]) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([test_name, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_OP, op)
        .env(CHILD_ARGS, args.join("\n"))
        // $TEST_TMPDIR changes the client's defaults (IsRunningWithinTest).
        .env_remove("TEST_TMPDIR")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

struct ChildOutput {
    pid: u32,
    /// The exit code (`None` if the child was killed by a signal).
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl ChildOutput {
    /// The values of the stdout lines that start with `prefix`.
    fn values(&self, prefix: &str) -> Vec<String> {
        self.stdout
            .lines()
            .filter_map(|line| line.strip_prefix(prefix))
            .map(str::to_string)
            .collect()
    }

    fn describe(&self) -> String {
        format!(
            "exit code {:?}\n--- stdout\n{}\n--- stderr\n{}",
            self.code, self.stdout, self.stderr
        )
    }
}

fn run_child(test_name: &str, op: &str, args: &[&str]) -> ChildOutput {
    let child = child_command(test_name, op, args).spawn().unwrap();
    let pid = child.id();
    let output = child.wait_with_output().unwrap();
    ChildOutput {
        pid,
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Runs the client in a child process (child op "client").
fn run_client(test_name: &str, dir: &str, client_args: &[&str]) -> ChildOutput {
    let mut args = vec![dir];
    args.extend_from_slice(client_args);
    run_child(test_name, "client", &args)
}

// ---------------------------------------------------------------------------------------------
// Fixtures

/// The in-process server of the tests: records the arguments it is run with and whether the
/// client still holds its locks while the server runs.
struct ProbeServer {
    output_base_lock: String,
    install_base_lock: String,
    emit: bool,
    exit_code: i32,
    lines: RefCell<Vec<String>>,
}

impl ProbeServer {
    fn new(output_base_lock: &str, install_base_lock: &str, emit: bool, exit_code: i32) -> Self {
        ProbeServer {
            output_base_lock: output_base_lock.to_string(),
            install_base_lock: install_base_lock.to_string(),
            emit,
            exit_code,
            lines: RefCell::new(Vec::new()),
        }
    }

    fn report(&self, args: Vec<String>) -> i32 {
        let mut lines: Vec<String> = args.iter().map(|arg| format!("{}{}", ARG, arg)).collect();
        lines.push(format!(
            "{}{}",
            OUTPUT_BASE_LOCK_HELD,
            is_locked(&self.output_base_lock)
        ));
        lines.push(format!(
            "{}{}",
            INSTALL_BASE_LOCK_HELD,
            is_locked(&self.install_base_lock)
        ));
        if self.emit {
            for line in &lines {
                emit(line);
            }
        }
        *self.lines.borrow_mut() = lines;
        self.exit_code
    }
}

impl InProcessServer for ProbeServer {
    fn batch_main(&self, _startup: &ServerStartup, args: Vec<String>) -> i32 {
        self.report(args)
    }

    fn run_command(&self, _startup: &ServerStartup, request: RunRequest) -> i32 {
        self.report(
            request
                .arg
                .iter()
                .map(|arg| String::from_utf8_lossy(arg).into_owned())
                .collect(),
        )
    }

    fn canceller(&self) -> std::sync::Arc<dyn Fn() + Send + Sync> {
        std::sync::Arc::new(|| {})
    }
}

/// Whether some other open file description holds a lock on the byte range `AcquireLock` uses
/// (TryLock: the first byte). Probes with a non-blocking exclusive OFD lock on a descriptor of
/// its own, which conflicts with shared and exclusive locks alike, also within one process.
fn is_locked(path: &str) -> bool {
    let c_path = CString::new(path).unwrap();
    // SAFETY: c_path is NUL-terminated.
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    assert!(
        fd >= 0,
        "open({}): {}",
        path,
        std::io::Error::last_os_error()
    );
    // SAFETY: zeroed flock is valid (l_pid must be 0 for OFD locks).
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = libc::F_WRLCK as libc::c_short;
    lock.l_whence = libc::SEEK_SET as libc::c_short;
    lock.l_start = 0;
    lock.l_len = 1;
    // SAFETY: fd is open; lock is a valid flock.
    let result = unsafe { libc::fcntl(fd, libc::F_OFD_SETLK, &lock as *const libc::flock) };
    let error = std::io::Error::last_os_error();
    // Closing the descriptor drops the probe's lock if it got one.
    // SAFETY: fd is open.
    unsafe { libc::close(fd) };
    if result == 0 {
        return false;
    }
    let errno = error.raw_os_error();
    assert!(
        errno == Some(libc::EAGAIN) || errno == Some(libc::EACCES),
        "fcntl(F_OFD_SETLK) on {}: {}",
        path,
        error
    );
    true
}

/// The embedded install archive of the tests: an executable `embedded_tools/jdk/bin/java`
/// (GetJvm's sanity check needs one) and `install_base_key`.
fn test_archive() -> EmbeddedArchive {
    let members: Vec<(&str, Vec<u8>)> = vec![
        ("embedded_tools/jdk/bin/java", b"#!/bin/sh\n".to_vec()),
        ("install_base_key", INSTALL_MD5.as_bytes().to_vec()),
    ];
    let encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
    let mut builder = tar::Builder::new(encoder);
    for (path, data) in &members {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        builder.append_data(&mut header, *path, &data[..]).unwrap();
    }
    let data = builder.into_inner().unwrap().finish().unwrap();
    EmbeddedArchive::new(
        data,
        members.iter().map(|(path, _)| path.to_string()).collect(),
        INSTALL_MD5,
        "test-label",
        "A-server.jar",
    )
}

fn output_base_path(dir: &str) -> String {
    join_path(dir, "output_base")
}

fn output_base_lock_path(dir: &str) -> String {
    join_path(&output_base_path(dir), "lock")
}

fn install_base_path(dir: &str) -> String {
    join_path(&join_path(dir, "root/install"), INSTALL_MD5)
}

fn install_base_lock_path(dir: &str) -> String {
    format!("{}.lock", install_base_path(dir))
}

/// A directory for one client: `workspace` (with a MODULE.bazel), the output user root `root`
/// and the output base `output_base`.
struct ClientDir {
    _temp: tempfile::TempDir,
    dir: String,
}

impl ClientDir {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let dir = make_canonical(temp.path().to_str().unwrap());
        assert!(!dir.is_empty());
        for sub in ["workspace", "root", "output_base"] {
            std::fs::create_dir(join_path(&dir, sub)).unwrap();
        }
        std::fs::write(join_path(&dir, "workspace/MODULE.bazel"), "").unwrap();
        ClientDir { _temp: temp, dir }
    }

    fn output_base(&self) -> String {
        output_base_path(&self.dir)
    }

    fn output_base_lock(&self) -> String {
        output_base_lock_path(&self.dir)
    }

    fn server_file(&self, name: &str) -> String {
        join_path(&join_path(&self.output_base(), "server"), name)
    }

    /// Writes `<output_base>/server/<name>`.
    fn write_server_file(&self, name: &str, contents: &str) {
        std::fs::create_dir_all(join_path(&self.output_base(), "server")).unwrap();
        std::fs::write(self.server_file(name), contents).unwrap();
    }
}

/// A live process that is not a Blaze client: stands in for a real Bazel server.
struct ForeignProcess(std::process::Child);

impl ForeignProcess {
    fn spawn() -> Self {
        ForeignProcess(
            Command::new("sleep")
                .arg("600")
                .stdin(Stdio::null())
                .spawn()
                .expect("spawn sleep"),
        )
    }

    fn pid(&self) -> i32 {
        self.0.id() as i32
    }

    /// The process's start time as the client records it in server/server.starttime: field 22
    /// of /proc/<pid>/stat (blaze_util_linux.cc GetStartTime).
    fn start_time(&self) -> String {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", self.pid())).unwrap();
        stat.split(' ').nth(21).unwrap().to_string()
    }
}

impl Drop for ForeignProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn read_to_string(path: &str) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// The pid of a process that has exited and been reaped.
fn dead_pid() -> i32 {
    let mut child = Command::new("true")
        .stdin(Stdio::null())
        .spawn()
        .expect("spawn true");
    let pid = child.id() as i32;
    child.wait().unwrap();
    pid
}

/// Everything observable under `dir`: each entry's path, mode, modification time and contents
/// (or link target), in path order.
fn snapshot(dir: &str) -> Vec<String> {
    fn visit(path: &std::path::Path, out: &mut Vec<String>) {
        let metadata = std::fs::symlink_metadata(path).unwrap();
        let what = if metadata.file_type().is_symlink() {
            format!("link to {:?}", std::fs::read_link(path).unwrap())
        } else if metadata.is_dir() {
            "directory".to_string()
        } else {
            format!(
                "file {:?}",
                String::from_utf8_lossy(&std::fs::read(path).unwrap())
            )
        };
        out.push(format!(
            "{:?} mode={:o} mtime={:?} {}",
            path,
            metadata.permissions().mode(),
            metadata.modified().unwrap(),
            what
        ));
        if metadata.is_dir() {
            let mut children: Vec<std::path::PathBuf> = std::fs::read_dir(path)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            children.sort();
            for child in children {
                visit(&child, out);
            }
        }
    }
    let mut out = Vec::new();
    visit(std::path::Path::new(dir), &mut out);
    out
}

// ---------------------------------------------------------------------------------------------
// AcquireLock (blaze_util_posix.cc)

/// An exclusive lock identifies its owner in the lock file: "pid=%d\nowner=client\n",
/// "cwd=%s\n" and, with a terminal on stdin, "tty=%s\n" (WriteOwnerInformation).
#[test]
fn acquire_lock_writes_owner_information() {
    let _serial = serial();
    let temp = tempfile::tempdir().unwrap();
    let lock_path = join_path(temp.path().to_str().unwrap(), "lock");

    let (lock, wait_time) = acquire_lock(
        "output base",
        &Path::new(&lock_path),
        LockMode::Exclusive,
        false,
        true,
    );
    // "If we succeeded on the first try, report zero wait time".
    assert_eq!(DurationMillis::default(), wait_time);
    assert!(is_locked(&lock_path));

    let contents = read_to_string(&lock_path);
    let expected = format!(
        "pid={}\nowner=client\ncwd={}\n",
        std::process::id(),
        get_cwd()
    );
    let Some(tty_line) = contents.strip_prefix(&expected) else {
        panic!("lock file contents: {:?}", contents);
    };
    assert!(
        tty_line.is_empty()
            || (tty_line.starts_with("tty=")
                && tty_line.ends_with('\n')
                && tty_line.matches('\n').count() == 1),
        "lock file contents: {:?}",
        contents
    );

    release_lock(lock);
    assert!(!is_locked(&lock_path));
}

/// With `block == false`, a second acquisition of a held exclusive lock exits with
/// LOCK_HELD_NOBLOCK_FOR_LOCK (9) after naming the owner, and does not wait.
#[test]
fn acquire_lock_noblock_fails_while_the_lock_is_held() {
    run_as_child_if_requested();
    let _serial = serial();
    let temp = tempfile::tempdir().unwrap();
    let lock_path = join_path(temp.path().to_str().unwrap(), "lock");

    let (lock, _) = acquire_lock(
        "output base",
        &Path::new(&lock_path),
        LockMode::Exclusive,
        false,
        true,
    );
    let owner = read_to_string(&lock_path);
    assert!(owner.starts_with(&format!("pid={}\nowner=client\n", std::process::id())));

    let child = run_child(
        "acquire_lock_noblock_fails_while_the_lock_is_held",
        "lock",
        &["output base", &lock_path, "exclusive", "noblock"],
    );
    assert_eq!(
        Some(exit_code::LOCK_HELD_NOBLOCK_FOR_LOCK),
        child.code,
        "{}",
        child.describe()
    );
    assert_eq!(Some(9), child.code);
    // BAZEL_LOG(USER) << "Another command holds the " << name << " lock: \n" << owner;
    assert!(
        child.stderr.contains(&format!(
            "Another command holds the output base lock: \n{}",
            owner
        )),
        "{}",
        child.describe()
    );
    // BAZEL_DIE(LOCK_HELD_NOBLOCK_FOR_LOCK)
    assert!(
        child.stderr.contains(
            "FATAL: Exiting because the output base lock is held and --noblock_for_lock was \
             given.\n"
        ),
        "{}",
        child.describe()
    );
    assert!(
        !child.stderr.contains("Waiting for it to complete..."),
        "{}",
        child.describe()
    );
    // The failed attempt did not take over the lock file.
    assert_eq!(owner, read_to_string(&lock_path));
    assert!(is_locked(&lock_path));

    release_lock(lock);
}

/// Once the first handle is released, a second acquisition with `block == false` succeeds at
/// once and becomes the owner named in the lock file.
#[test]
fn acquire_lock_succeeds_after_release() {
    run_as_child_if_requested();
    let _serial = serial();
    let temp = tempfile::tempdir().unwrap();
    let lock_path = join_path(temp.path().to_str().unwrap(), "lock");

    let (lock, _) = acquire_lock(
        "output base",
        &Path::new(&lock_path),
        LockMode::Exclusive,
        false,
        true,
    );
    release_lock(lock);
    assert!(!is_locked(&lock_path));

    let child = run_child(
        "acquire_lock_succeeds_after_release",
        "lock",
        &["output base", &lock_path, "exclusive", "noblock"],
    );
    assert_eq!(Some(0), child.code, "{}", child.describe());
    assert_eq!(vec!["0".to_string()], child.values(WAIT_MS));
    assert!(
        !child.stderr.contains("Another command holds"),
        "{}",
        child.describe()
    );
    assert!(read_to_string(&lock_path).starts_with(&format!("pid={}\nowner=client\n", child.pid)));
    // The child's lock went away with the child.
    assert!(!is_locked(&lock_path));
}

/// With `block == true`, a second acquisition names the owner, prints "Waiting for it to
/// complete..." once, retries until the lock is released and reports the time it waited.
#[test]
fn acquire_lock_block_waits_for_release() {
    run_as_child_if_requested();
    let _serial = serial();
    let temp = tempfile::tempdir().unwrap();
    let lock_path = join_path(temp.path().to_str().unwrap(), "lock");

    let (lock, _) = acquire_lock(
        "output base",
        &Path::new(&lock_path),
        LockMode::Exclusive,
        false,
        true,
    );
    let owner = read_to_string(&lock_path);

    let mut child = child_command(
        "acquire_lock_block_waits_for_release",
        "lock",
        &["output base", &lock_path, "exclusive", "block"],
    )
    .spawn()
    .unwrap();
    let child_pid = child.id();

    // Hold the lock until the child says that it is waiting.
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut seen = String::new();
    loop {
        let mut line = String::new();
        let read = stderr.read_line(&mut line).unwrap();
        assert!(read > 0, "the child exited without waiting: {}", seen);
        seen.push_str(&line);
        if line.contains("Waiting for it to complete...") {
            break;
        }
    }
    assert!(is_locked(&lock_path));
    release_lock(lock);

    stderr.read_to_string(&mut seen).unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(Some(0), output.status.code(), "{}\n{}", stdout, seen);
    assert!(
        seen.contains(&format!(
            "Another command holds the output base lock: \n{}",
            owner
        )),
        "{}",
        seen
    );
    // The owner did not change while the child waited, so both messages are printed once.
    assert_eq!(1, seen.matches("Another command holds").count(), "{}", seen);
    assert_eq!(
        1,
        seen.matches("Waiting for it to complete...").count(),
        "{}",
        seen
    );
    // TrySleep(500) between attempts: the reported wait time is not zero.
    let wait_ms: u64 = stdout
        .lines()
        .find_map(|line| line.strip_prefix(WAIT_MS))
        .unwrap_or_else(|| panic!("no wait time in: {}", stdout))
        .parse()
        .unwrap();
    assert!(wait_ms >= 500, "waited {} ms", wait_ms);
    assert!(read_to_string(&lock_path).starts_with(&format!("pid={}\nowner=client\n", child_pid)));
}

/// The install base lock is shared ("two simultaneous commands may run against the same install
/// base"): shared locks coexist, an exclusive one is refused while a shared one is held, and a
/// shared lock writes no owner information.
#[test]
fn shared_locks_do_not_exclude_each_other() {
    run_as_child_if_requested();
    let _serial = serial();
    let temp = tempfile::tempdir().unwrap();
    let lock_path = join_path(temp.path().to_str().unwrap(), "install.lock");

    let (lock, _) = acquire_lock(
        "install base",
        &Path::new(&lock_path),
        LockMode::Shared,
        false,
        true,
    );
    assert_eq!("", read_to_string(&lock_path));

    let shared = run_child(
        "shared_locks_do_not_exclude_each_other",
        "lock",
        &["install base", &lock_path, "shared", "noblock"],
    );
    assert_eq!(Some(0), shared.code, "{}", shared.describe());
    assert_eq!("", read_to_string(&lock_path));

    let exclusive = run_child(
        "shared_locks_do_not_exclude_each_other",
        "lock",
        &["install base", &lock_path, "exclusive", "noblock"],
    );
    assert_eq!(
        Some(exit_code::LOCK_HELD_NOBLOCK_FOR_LOCK),
        exclusive.code,
        "{}",
        exclusive.describe()
    );
    // blaze_util_posix.cc:735-740: the owner is printed only when it differs from the previous
    // one, which starts empty. A shared holder writes no owner information, so the lock file is
    // empty and "Another command holds the install base lock" is not printed at all.
    assert!(
        !exclusive.stderr.contains("Another command holds"),
        "{}",
        exclusive.describe()
    );
    assert!(
        exclusive.stderr.contains(
            "FATAL: Exiting because the install base lock is held and --noblock_for_lock was \
             given.\n"
        ),
        "{}",
        exclusive.describe()
    );

    release_lock(lock);
}

// ---------------------------------------------------------------------------------------------
// The client holds its locks for the whole in-process command (review 2026-11-18, S1-1)

/// `BlazeServer::Communicate` must not release the client locks before the in-process server
/// runs the command: they are the only locks between two Blaze processes.
#[test]
fn communicate_keeps_the_client_locks_for_the_whole_command() {
    let _serial = serial();
    let dir = ClientDir::new();
    let mut startup_options = StartupOptions::bazel();
    startup_options.output_base = Path::new(&dir.output_base());
    startup_options.install_base = Path::new(&install_base_path(&dir.dir));
    let output_base_lock = dir.output_base_lock();
    let install_base_lock = install_base_lock_path(&dir.dir);

    let mut blaze_server = BlazeServer::new(&startup_options);
    assert_eq!(DurationMillis::default(), blaze_server.acquire_locks());
    assert!(is_locked(&output_base_lock));
    assert!(is_locked(&install_base_lock));
    blaze_server.connect_in_process();

    let server = ProbeServer::new(&output_base_lock, &install_base_lock, false, 42);
    let startup = ServerStartup {
        server_exe_args: vec!["bazel(workspace)".to_string(), "--foo".to_string()],
        first_startup_arg: 1,
    };
    let code = blaze_server.communicate(
        "info",
        &["release".to_string()],
        "",
        &[],
        &LoggingInfo::new("/path/to/blaze", 0),
        DurationMillis::default(),
        None,
        None,
        &server,
        &startup,
    );
    assert_eq!(42, code);
    assert_eq!(
        vec![
            format!("{}info", ARG),
            format!("{}--startup_time=0", ARG),
            format!("{}--binary_path=/path/to/blaze", ARG),
            format!("{}release", ARG),
            format!("{}true", OUTPUT_BASE_LOCK_HELD),
            format!("{}true", INSTALL_BASE_LOCK_HELD),
        ],
        *server.lines.borrow()
    );
    // Nothing releases them after the command either: they go away with the process.
    assert!(is_locked(&output_base_lock));
    assert!(is_locked(&install_base_lock));
}

/// The same through `blaze::Main` without `--batch` (RunClientServerMode): the server runs with
/// both locks held, and they are free again once the client process has exited.
#[test]
fn client_holds_locks_while_the_in_process_server_runs() {
    run_as_child_if_requested();
    let _serial = serial();
    let dir = ClientDir::new();

    let client = run_client(
        "client_holds_locks_while_the_in_process_server_runs",
        &dir.dir,
        &["info"],
    );
    assert_eq!(Some(0), client.code, "{}", client.describe());
    assert_eq!(
        vec!["true".to_string()],
        client.values(OUTPUT_BASE_LOCK_HELD),
        "{}",
        client.describe()
    );
    assert_eq!(
        vec!["true".to_string()],
        client.values(INSTALL_BASE_LOCK_HELD),
        "{}",
        client.describe()
    );
    assert_eq!(Some("info"), client.values(ARG).first().map(String::as_str));
    assert!(
        !client.stderr.contains("Released the client-side locks"),
        "{}",
        client.describe()
    );
    // The lock file names the client that ran the command.
    assert!(
        read_to_string(&dir.output_base_lock())
            .starts_with(&format!("pid={}\nowner=client\n", client.pid))
    );
    // Released on process exit.
    assert!(!is_locked(&dir.output_base_lock()));
    assert!(!is_locked(&install_base_lock_path(&dir.dir)));
}

/// The same with `--batch` (RunBatchMode), which always kept the locks.
#[test]
fn client_holds_locks_in_batch_mode() {
    run_as_child_if_requested();
    let _serial = serial();
    let dir = ClientDir::new();

    let client = run_client(
        "client_holds_locks_in_batch_mode",
        &dir.dir,
        &["--batch", "info"],
    );
    assert_eq!(Some(0), client.code, "{}", client.describe());
    assert_eq!(
        vec!["true".to_string()],
        client.values(OUTPUT_BASE_LOCK_HELD),
        "{}",
        client.describe()
    );
    assert_eq!(
        vec!["true".to_string()],
        client.values(INSTALL_BASE_LOCK_HELD),
        "{}",
        client.describe()
    );
    // main(String[] args): the startup options starting with --batch, then the command.
    assert_eq!(
        Some("--batch"),
        client.values(ARG).first().map(String::as_str)
    );
    assert!(client.values(ARG).contains(&"info".to_string()));
    assert!(!is_locked(&dir.output_base_lock()));
}

// Java: src/test/shell/integration/client_test.sh:531 test_noblock_for_lock_with_batch
/// The first command is stuck holding the output base lock (the shell test blocks a
/// `bazel --batch build` on a FIFO; here this process holds the lock as that client would).
/// A second `--batch --noblock_for_lock` command exits with 9 and names the lock's owner.
#[test]
fn test_noblock_for_lock_with_batch() {
    run_as_child_if_requested();
    let _serial = serial();
    let dir = ClientDir::new();
    let (lock, _) = acquire_lock(
        "output base",
        &Path::new(&dir.output_base_lock()),
        LockMode::Exclusive,
        false,
        true,
    );
    let client_pid = std::process::id();

    let client = run_client(
        "test_noblock_for_lock_with_batch",
        &dir.dir,
        &["--batch", "--noblock_for_lock", "info"],
    );

    // assert_equals 9 "$exit_code" # LOCK_HELD_NOBLOCK_FOR_LOCK
    assert_eq!(Some(9), client.code, "{}", client.describe());
    // expect_log "Another command holds the output base lock"
    assert!(
        client
            .stderr
            .contains("Another command holds the output base lock"),
        "{}",
        client.describe()
    );
    // expect_log "pid=$client_pid"
    assert!(
        client.stderr.contains(&format!("pid={}", client_pid)),
        "{}",
        client.describe()
    );
    // expect_log "Exiting because the output base lock is held and --noblock_for_lock was given"
    assert!(
        client.stderr.contains(
            "Exiting because the output base lock is held and --noblock_for_lock was given"
        ),
        "{}",
        client.describe()
    );
    assert!(client.values(ARG).is_empty(), "{}", client.describe());

    release_lock(lock);
}

/// The scenario of `test_noblock_for_lock_reuse_server` / `test_noblock_for_lock_new_server`
/// without `--batch`, as Blaze behaves with its in-process server: the first command holds the
/// client's output base lock for as long as it runs, so the second one fails in `AcquireLock`
/// with exit code 9 (the same code Bazel's server reports) and the client's wording, the one
/// Bazel prints in `--batch` mode.
#[test]
fn noblock_for_lock_without_batch_fails_on_the_client_lock() {
    run_as_child_if_requested();
    let _serial = serial();
    let dir = ClientDir::new();
    let (lock, _) = acquire_lock(
        "output base",
        &Path::new(&dir.output_base_lock()),
        LockMode::Exclusive,
        false,
        true,
    );
    let owner = read_to_string(&dir.output_base_lock());

    let client = run_client(
        "noblock_for_lock_without_batch_fails_on_the_client_lock",
        &dir.dir,
        &["--noblock_for_lock", "info"],
    );
    assert_eq!(
        Some(exit_code::LOCK_HELD_NOBLOCK_FOR_LOCK),
        client.code,
        "{}",
        client.describe()
    );
    assert!(
        client.stderr.contains(&format!(
            "Another command holds the output base lock: \n{}",
            owner
        )),
        "{}",
        client.describe()
    );
    assert!(
        client.stderr.contains(
            "FATAL: Exiting because the output base lock is held and --noblock_for_lock was \
             given.\n"
        ),
        "{}",
        client.describe()
    );
    assert!(client.values(ARG).is_empty(), "{}", client.describe());
    // The refused client left the lock file to its owner.
    assert_eq!(owner, read_to_string(&dir.output_base_lock()));

    release_lock(lock);
}

// Java: src/test/shell/integration/client_test.sh:465 test_noblock_for_lock_reuse_server
#[test]
#[ignore = "needs a long-lived server (W8): expects the server's \"Another command (pid=...) is running. Exiting immediately.\""]
fn test_noblock_for_lock_reuse_server() {}

// Java: src/test/shell/integration/client_test.sh:498 test_noblock_for_lock_new_server
#[test]
#[ignore = "needs a long-lived server (W8): expects KillRunningServer's \"Exiting because the lock is held and --noblock_for_lock was given\""]
fn test_noblock_for_lock_new_server() {}

// Java: src/test/shell/integration/client_test.sh:286 test_multiple_commands_same_output_base
#[test]
#[ignore = "needs genrule builds through the blaze binary and a long-lived server (W8): expects \"(pid=...) ... on the (client|server)...\" wait messages"]
fn test_multiple_commands_same_output_base() {}

// ---------------------------------------------------------------------------------------------
// A live server of another tool in the output base (review 2026-11-18, S2-1)

#[test]
fn find_live_foreign_server_without_pid_file() {
    let _serial = serial();
    let dir = ClientDir::new();
    let output_base = Path::new(&dir.output_base());
    // No server directory at all.
    assert_eq!(None, find_live_foreign_server(&output_base));
    // A server directory without server.pid.txt.
    dir.write_server_file("cmdline", "bazel(workspace)\0--batch");
    assert_eq!(None, find_live_foreign_server(&output_base));
}

#[test]
fn find_live_foreign_server_with_stale_pid_file() {
    let _serial = serial();
    let dir = ClientDir::new();
    let output_base = Path::new(&dir.output_base());

    // The process is gone (VerifyServerProcess: "Process died meantime, all is good").
    dir.write_server_file("server.pid.txt", &dead_pid().to_string());
    assert_eq!(None, find_live_foreign_server(&output_base));

    // Not a pid (GetServerPid returns -1).
    dir.write_server_file("server.pid.txt", "not a pid");
    assert_eq!(None, find_live_foreign_server(&output_base));
    dir.write_server_file("server.pid.txt", "");
    assert_eq!(None, find_live_foreign_server(&output_base));

    // The pid belongs to another process now: its start time is not the recorded one.
    let other = ForeignProcess::spawn();
    dir.write_server_file("server.pid.txt", &other.pid().to_string());
    dir.write_server_file("server.starttime", &format!("{}0", other.start_time()));
    assert_eq!(None, find_live_foreign_server(&output_base));

    // This process is not a foreign server, whatever the pid file says.
    std::fs::remove_file(dir.server_file("server.starttime")).unwrap();
    dir.write_server_file("server.pid.txt", &std::process::id().to_string());
    assert_eq!(None, find_live_foreign_server(&output_base));
}

#[test]
fn find_live_foreign_server_with_live_server() {
    let _serial = serial();
    let dir = ClientDir::new();
    let output_base = Path::new(&dir.output_base());
    let server = ForeignProcess::spawn();

    // "If start time file got deleted, but PID file didn't, assume that this is an old Blaze
    // process that doesn't know how to write start time files yet."
    dir.write_server_file("server.pid.txt", &server.pid().to_string());
    assert_eq!(Some(server.pid()), find_live_foreign_server(&output_base));

    // The pid and the recorded start time both match.
    dir.write_server_file("server.starttime", &server.start_time());
    assert_eq!(Some(server.pid()), find_live_foreign_server(&output_base));
}

/// With a live server of another tool in the output base, every command refuses to run with
/// LOCAL_ENVIRONMENTAL_ERROR (36) before it changes anything under the output base: `install`
/// is not repointed, `server/command_port` and the other server files stay, no lock file is
/// written, and nothing panics.
#[test]
fn client_refuses_output_base_of_live_foreign_server() {
    run_as_child_if_requested();
    let _serial = serial();
    let dir = ClientDir::new();
    let server = ForeignProcess::spawn();

    // What a running Bazel server's output base contains, as far as the client cares.
    let bazel_install = join_path(&dir.dir, "bazel_install");
    std::fs::create_dir(&bazel_install).unwrap();
    std::os::unix::fs::symlink(
        bazel_install.as_str(),
        join_path(&dir.output_base(), "install"),
    )
    .unwrap();
    dir.write_server_file("server.pid.txt", &server.pid().to_string());
    dir.write_server_file("server.starttime", &server.start_time());
    dir.write_server_file("command_port", "[::1]:12345");
    dir.write_server_file("cmdline", "bazel(workspace)\0--max_idle_secs=10800");
    dir.write_server_file("request_cookie", "cookie");
    let before = snapshot(&dir.output_base());

    for client_args in [
        &["info"][..],
        &["shutdown"][..],
        &["--batch", "info"][..],
        &["--noblock_for_lock", "build", "//..."][..],
    ] {
        let client = run_client(
            "client_refuses_output_base_of_live_foreign_server",
            &dir.dir,
            client_args,
        );
        assert_eq!(
            Some(exit_code::LOCAL_ENVIRONMENTAL_ERROR),
            client.code,
            "{:?}: {}",
            client_args,
            client.describe()
        );
        assert_eq!(Some(36), client.code);
        assert!(
            client.stderr.contains(&format!(
                "FATAL: blaze: a running Bazel server (pid={}) owns the output base '{}'. Blaze \
                 has no server protocol and will not kill or modify another tool's server. Run \
                 'bazel shutdown' in this workspace, or run Blaze with a different \
                 --output_user_root or --output_base.\n",
                server.pid(),
                dir.output_base()
            )),
            "{:?}: {}",
            client_args,
            client.describe()
        );
        assert!(
            !client.stderr.contains("panicked") && !client.stderr.contains("not ported yet"),
            "{:?}: {}",
            client_args,
            client.describe()
        );
        // The server never ran, and nothing under the output base was touched.
        assert!(client.values(ARG).is_empty(), "{}", client.describe());
        assert_eq!(before, snapshot(&dir.output_base()), "{:?}", client_args);
    }

    // The server is still alive.
    assert_eq!(
        Some(server.pid()),
        find_live_foreign_server(&Path::new(&dir.output_base()))
    );
}

/// A stale pid file (its process is dead) does not stop the client: the command runs, with the
/// restart reason the C++ records for that case (PID_FILE_BUT_NO_SERVER).
#[test]
fn client_ignores_stale_server_pid_file() {
    run_as_child_if_requested();
    let _serial = serial();
    let dir = ClientDir::new();
    dir.write_server_file("server.pid.txt", &dead_pid().to_string());
    dir.write_server_file("command_port", "[::1]:12345");

    let client = run_client("client_ignores_stale_server_pid_file", &dir.dir, &["info"]);
    assert_eq!(Some(0), client.code, "{}", client.describe());
    let args = client.values(ARG);
    assert_eq!(Some("info"), args.first().map(String::as_str));
    assert!(
        args.contains(&"--restart_reason=pid_file_but_no_server".to_string()),
        "{}",
        client.describe()
    );
    // StartServerAndConnect ran as before: the old command_port is gone, `install` points at
    // this client's install base and server/cmdline is this client's.
    assert!(!std::path::Path::new(&dir.server_file("command_port")).exists());
    assert_eq!(
        std::path::PathBuf::from(install_base_path(&dir.dir)),
        std::fs::read_link(join_path(&dir.output_base(), "install")).unwrap()
    );
    assert!(
        std::fs::read(dir.server_file("cmdline"))
            .unwrap()
            .starts_with(b"bazel(workspace)\0")
    );
}

/// Without a pid file the client starts its in-process server as before (restart reason
/// NO_DAEMON).
#[test]
fn client_starts_without_server_pid_file() {
    run_as_child_if_requested();
    let _serial = serial();
    let dir = ClientDir::new();

    let client = run_client("client_starts_without_server_pid_file", &dir.dir, &["info"]);
    assert_eq!(Some(0), client.code, "{}", client.describe());
    let args = client.values(ARG);
    assert_eq!(Some("info"), args.first().map(String::as_str));
    assert!(
        args.contains(&"--restart_reason=no_daemon".to_string()),
        "{}",
        client.describe()
    );
    assert!(
        client
            .stderr
            .contains("Starting local Bazel server (test-label) and connecting to it...\n"),
        "{}",
        client.describe()
    );
    assert_eq!(
        std::path::PathBuf::from(install_base_path(&dir.dir)),
        std::fs::read_link(join_path(&dir.output_base(), "install")).unwrap()
    );
}

// ---------------------------------------------------------------------------------------------
// JVM trust store options (review 2026-11-18, S3 "TLS trust")

/// `--host_jvm_args=-Djavax.net.ssl.trustStore=...` is the documented way to give Bazel's
/// downloader extra certificate authorities. Blaze has no JVM and must not ignore it silently:
/// the client exits with BAD_ARGV (2) before it touches the output base, naming the property
/// but not its value.
#[test]
fn client_rejects_host_jvm_args_trust_store() {
    run_as_child_if_requested();
    let _serial = serial();
    let dir = ClientDir::new();
    let before = snapshot(&dir.output_base());

    let client = run_client(
        "client_rejects_host_jvm_args_trust_store",
        &dir.dir,
        &[
            "--host_jvm_args=-Xmx2g",
            "--host_jvm_args=-Djavax.net.ssl.trustStorePassword=hunter2",
            "info",
        ],
    );
    assert_eq!(
        Some(exit_code::BAD_ARGV),
        client.code,
        "{}",
        client.describe()
    );
    assert!(
        client.stderr.contains(
            "FATAL: blaze: not ported yet: \
             --host_jvm_args=-Djavax.net.ssl.trustStorePassword: the JVM trust store and key \
             store options (-Djavax.net.ssl.trustStore*, -Djavax.net.ssl.keyStore*) are not \
             ported. Blaze has no JVM and would silently ignore the option.\n"
        ),
        "{}",
        client.describe()
    );
    assert!(!client.stderr.contains("hunter2"), "{}", client.describe());
    assert!(client.values(ARG).is_empty(), "{}", client.describe());
    assert_eq!(before, snapshot(&dir.output_base()));

    // Other JVM arguments are accepted (and ignored), as before.
    let client = run_client(
        "client_rejects_host_jvm_args_trust_store",
        &dir.dir,
        &["--host_jvm_args=-Xmx2g", "info"],
    );
    assert_eq!(Some(0), client.code, "{}", client.describe());
    assert_eq!(Some("info"), client.values(ARG).first().map(String::as_str));
}
