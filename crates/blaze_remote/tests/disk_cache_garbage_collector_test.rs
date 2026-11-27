// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `lib/remote/disk/DiskCacheGarbageCollectorTest.java`.
//!
//! PORT: the Rust collector is sequential and takes no executor service (see the `// PORT:` note
//! on `DiskCacheGarbageCollector`), so the test has none either.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use blaze_remote::disk::disk_cache_garbage_collector::{
    CollectionPolicy, CollectionStats, DiskCacheGarbageCollector,
};
use blaze_repo_fetch::file_system_lock::{FileSystemLock, LockMode};
use blaze_vfs::{DigestHashFunction, FileSystemExt, Path, UnixFileSystem};

/// Set in the child process that holds the lock for `fails_when_lock_is_already_held`; the value
/// is the path of the lock file.
const LOCK_HOLDER_ENV: &str = "BLAZE_REMOTE_DISK_CACHE_GC_TEST_LOCK_HOLDER";

/// What the lock holder process prints on stdout once it holds the lock.
const LOCK_HELD_MARKER: &str = "BLAZE_REMOTE_DISK_CACHE_GC_TEST_LOCK_HELD";

/// The lock holder process exits by itself after this time, whatever its parent does.
const LOCK_HOLDER_LIFETIME: Duration = Duration::from_secs(30);

/// How long the test waits for the lock holder process to take the lock.
const LOCK_HOLDER_START_TIMEOUT: Duration = Duration::from_secs(20);

/// How long the test waits for the lock holder process to exit before it kills it.
const LOCK_HOLDER_EXIT_TIMEOUT: Duration = Duration::from_secs(5);

// Java: DiskCacheGarbageCollectorTest.Entry
struct Entry {
    path: &'static str,
    size: i64,
    mtime: SystemTime,
}

impl Entry {
    fn of(path: &'static str, size: i64, mtime: SystemTime) -> Entry {
        Entry { path, size, mtime }
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root_dir: Path,
}

impl Fixture {
    // Java: DiskCacheGarbageCollectorTest.setUp
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root_dir = fs.get_path(tmp.path().to_str().unwrap());
        Fixture {
            _tmp: tmp,
            root_dir,
        }
    }

    // Java: DiskCacheGarbageCollectorTest.assertFilesExist
    fn assert_files_exist(&self, relative_paths: &[&str]) {
        for relative_path in relative_paths {
            let path = self.root_dir.get_relative_str(relative_path);
            assert!(path.exists(), "expected {relative_path} to exist");
        }
    }

    // Java: DiskCacheGarbageCollectorTest.assertFilesDoNotExist
    fn assert_files_do_not_exist(&self, relative_paths: &[&str]) {
        for relative_path in relative_paths {
            let path = self.root_dir.get_relative_str(relative_path);
            assert!(!path.exists(), "expected {relative_path} to not exist");
        }
    }

    // Java: DiskCacheGarbageCollectorTest.runGarbageCollector
    fn run_garbage_collector(
        &self,
        max_size_bytes: Option<i64>,
        max_age: Option<Duration>,
    ) -> std::io::Result<CollectionStats> {
        let gc = DiskCacheGarbageCollector::new(
            self.root_dir.clone(),
            CollectionPolicy::new(max_size_bytes, max_age),
        );
        let result_stats = gc.run()?;
        Ok(CollectionStats::new(
            result_stats.total_entries,
            result_stats.total_bytes,
            result_stats.deleted_entries,
            result_stats.deleted_bytes,
            result_stats.concurrent_update,
            Duration::ZERO,
        ))
    }

    // Java: DiskCacheGarbageCollectorTest.writeFiles
    fn write_files(&self, entries: &[Entry]) {
        for entry in entries {
            self.write_file(entry.path, entry.size, entry.mtime);
        }
    }

    // Java: DiskCacheGarbageCollectorTest.writeFile
    fn write_file(&self, relative_path: &str, size: i64, mtime: SystemTime) {
        let path = self.root_dir.get_relative_str(relative_path);
        path.get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        {
            let mut out = path.get_output_stream().unwrap();
            out.write_all(&vec![0u8; size as usize]).unwrap();
            out.flush().unwrap();
        }
        // `mtime.toEpochMilli()`
        let epoch_milli = mtime.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        path.set_last_modified_time(epoch_milli).unwrap();
    }
}

// Java: DiskCacheGarbageCollectorTest.daysAgo
fn days_ago(days: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60)
}

// Java: DiskCacheGarbageCollectorTest.days
fn days(days: u64) -> Duration {
    Duration::from_secs(days * 24 * 60 * 60)
}

// Java: DiskCacheGarbageCollectorTest.kbytes
fn kbytes(kbytes: i64) -> i64 {
    kbytes * 1024
}

#[test]
fn size_policy_no_collection() {
    let f = Fixture::new();
    f.write_files(&[
        Entry::of("ac/123", kbytes(1), SystemTime::now()),
        Entry::of("cas/456", kbytes(1), SystemTime::now()),
    ]);

    let stats = f.run_garbage_collector(Some(kbytes(2)), None).unwrap();

    assert_eq!(
        stats,
        CollectionStats::new(2, kbytes(2), 0, 0, false, Duration::ZERO)
    );
    f.assert_files_exist(&["ac/123", "cas/456"]);
}

#[test]
fn size_policy_collects_oldest() {
    let f = Fixture::new();
    f.write_files(&[
        Entry::of("ac/123", kbytes(1), days_ago(1)),
        Entry::of("cas/456", kbytes(1), days_ago(2)),
        Entry::of("ac/abc", kbytes(1), days_ago(3)),
        Entry::of("cas/def", kbytes(1), days_ago(4)),
    ]);

    let stats = f.run_garbage_collector(Some(kbytes(2)), None).unwrap();

    assert_eq!(
        stats,
        CollectionStats::new(4, kbytes(4), 2, kbytes(2), false, Duration::ZERO)
    );
    f.assert_files_exist(&["ac/123", "cas/456"]);
    f.assert_files_do_not_exist(&["ac/abc", "cas/def"]);
}

#[test]
fn size_policy_tie_break_by_path() {
    let f = Fixture::new();
    // PORT: Java calls `daysAgo(1)` (that is, `Instant.now()`) once per entry and so depends on
    // the four calls falling into the same millisecond for the tie; one instant is taken here.
    let one_day_ago = days_ago(1);
    f.write_files(&[
        Entry::of("ac/123", kbytes(1), one_day_ago),
        Entry::of("cas/456", kbytes(1), one_day_ago),
        Entry::of("ac/abc", kbytes(1), one_day_ago),
        Entry::of("cas/def", kbytes(1), one_day_ago),
    ]);

    let stats = f.run_garbage_collector(Some(kbytes(2)), None).unwrap();

    assert_eq!(
        stats,
        CollectionStats::new(4, kbytes(4), 2, kbytes(2), false, Duration::ZERO)
    );
    f.assert_files_exist(&["cas/456", "cas/def"]);
    f.assert_files_do_not_exist(&["ac/123", "ac/abc"]);
}

#[test]
fn age_policy_no_collection() {
    let f = Fixture::new();
    f.write_files(&[
        Entry::of("ac/123", kbytes(1), SystemTime::now()),
        Entry::of("cas/456", kbytes(1), SystemTime::now()),
    ]);

    let stats = f.run_garbage_collector(None, Some(days(3))).unwrap();

    assert_eq!(
        stats,
        CollectionStats::new(2, kbytes(2), 0, 0, false, Duration::ZERO)
    );
    f.assert_files_exist(&["ac/123", "cas/456"]);
}

#[test]
fn age_policy_collects_oldest() {
    let f = Fixture::new();
    f.write_files(&[
        Entry::of("ac/123", kbytes(1), days_ago(1)),
        Entry::of("cas/456", kbytes(1), days_ago(2)),
        Entry::of("ac/abc", kbytes(1), days_ago(4)),
        Entry::of("cas/def", kbytes(1), days_ago(5)),
    ]);

    let stats = f.run_garbage_collector(None, Some(days(3))).unwrap();

    assert_eq!(
        stats,
        CollectionStats::new(4, kbytes(4), 2, kbytes(2), false, Duration::ZERO)
    );
    f.assert_files_exist(&["ac/123", "cas/456"]);
    f.assert_files_do_not_exist(&["ac/abc", "cas/def"]);
}

#[test]
fn size_and_age_policy_no_collection() {
    let f = Fixture::new();
    f.write_files(&[
        Entry::of("ac/123", kbytes(1), SystemTime::now()),
        Entry::of("cas/456", kbytes(1), SystemTime::now()),
    ]);

    let stats = f
        .run_garbage_collector(Some(kbytes(2)), Some(days(1)))
        .unwrap();

    assert_eq!(
        stats,
        CollectionStats::new(2, kbytes(2), 0, 0, false, Duration::ZERO)
    );
    f.assert_files_exist(&["ac/123", "cas/456"]);
}

#[test]
fn size_and_age_policy_size_more_restrictive_than_age_collects_oldest() {
    let f = Fixture::new();
    f.write_files(&[
        Entry::of("ac/123", kbytes(1), days_ago(1)),
        Entry::of("cas/456", kbytes(1), days_ago(2)),
        Entry::of("ac/abc", kbytes(1), days_ago(3)),
        Entry::of("cas/def", kbytes(1), days_ago(4)),
    ]);

    let stats = f
        .run_garbage_collector(Some(kbytes(2)), Some(days(4)))
        .unwrap();

    assert_eq!(
        stats,
        CollectionStats::new(4, kbytes(4), 2, kbytes(2), false, Duration::ZERO)
    );
    f.assert_files_exist(&["ac/123", "cas/456"]);
    f.assert_files_do_not_exist(&["ac/abc", "cas/def"]);
}

#[test]
fn size_and_age_policy_age_more_restrictive_than_size_collects_oldest() {
    let f = Fixture::new();
    f.write_files(&[
        Entry::of("ac/123", kbytes(1), days_ago(1)),
        Entry::of("cas/456", kbytes(1), days_ago(2)),
        Entry::of("ac/abc", kbytes(1), days_ago(3)),
        Entry::of("cas/def", kbytes(1), days_ago(4)),
    ]);
    // PORT: "ac/abc" is exactly as old as the age limit. The collector keeps an entry whose
    // mtime (in milliseconds) is not before `now - maxAge`, so the entry is collected only if
    // the clock has moved on to a later millisecond between `daysAgo(3)` and the collector's
    // `Instant.now()`. The Java test relies on the time its own steps take; this one makes sure
    // of it.
    std::thread::sleep(Duration::from_millis(5));

    let stats = f
        .run_garbage_collector(Some(kbytes(3)), Some(days(3)))
        .unwrap();

    assert_eq!(
        stats,
        CollectionStats::new(4, kbytes(4), 2, kbytes(2), false, Duration::ZERO)
    );
    f.assert_files_exist(&["ac/123", "cas/456"]);
    f.assert_files_do_not_exist(&["ac/abc", "cas/def"]);
}

#[test]
fn ignores_tmp_and_gc_subdirectories() {
    let f = Fixture::new();
    f.write_files(&[
        Entry::of("gc/foo", kbytes(1), days_ago(1)),
        Entry::of("tmp/foo", kbytes(1), days_ago(1)),
    ]);

    let stats = f.run_garbage_collector(Some(1), Some(days(1))).unwrap();

    assert_eq!(
        stats,
        CollectionStats::new(0, 0, 0, 0, false, Duration::ZERO)
    );
    f.assert_files_exist(&["gc/foo", "tmp/foo"]);
}

/// The child process of `fails_when_lock_is_already_held` (Java: the `ExternalFileSystemLock`
/// helper binary): takes a shared lock on `lock_path`, says so on stdout, and holds the lock
/// until its stdin is closed. It exits by itself after [`LOCK_HOLDER_LIFETIME`] at the latest.
fn hold_shared_lock_until_stdin_closes(lock_path: &str) -> ! {
    std::thread::spawn(|| {
        std::thread::sleep(LOCK_HOLDER_LIFETIME);
        std::process::exit(4);
    });
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let lock = match FileSystemLock::get(&fs.get_path(lock_path), LockMode::SHARED) {
        Ok(lock) => lock,
        Err(e) => {
            eprintln!("cannot take the shared lock: {e}");
            std::process::exit(3);
        }
    };
    // The test harness has already written `test <name> ... ` to this line of stdout (it runs
    // the one test on its main thread), so the marker starts a line of its own.
    let mut stdout = std::io::stdout();
    stdout
        .write_all(format!("\n{LOCK_HELD_MARKER}\n").as_bytes())
        .unwrap();
    stdout.flush().unwrap();
    let mut sink = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut sink);
    drop(lock);
    std::process::exit(0);
}

/// `ExternalFileSystemLock`: a lock held by another process, released when this is dropped
/// (Java: `close()`).
///
/// PORT: Java starts a helper binary that holds the lock. Here this test binary is started
/// again, for `fails_when_lock_is_already_held` only, with [`LOCK_HOLDER_ENV`] set; the test
/// function then acts as that helper (`hold_shared_lock_until_stdin_closes`).
struct ExternalFileSystemLock {
    lock_holder: Child,
}

impl ExternalFileSystemLock {
    /// `ExternalFileSystemLock.getShared(lockPath)`.
    fn get_shared(lock_path: &Path) -> ExternalFileSystemLock {
        let mut lock_holder = Command::new(std::env::current_exe().unwrap())
            .args([
                "fails_when_lock_is_already_held",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(LOCK_HOLDER_ENV, lock_path.get_path_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let lock_holder_output = BufReader::new(lock_holder.stdout.take().unwrap());
        // From here on, dropping `external_lock` ends the process.
        let external_lock = ExternalFileSystemLock { lock_holder };

        // Reads the output of the process on a thread, so that waiting for the marker can time
        // out. The thread goes on reading (and discarding) until the process closes its stdout.
        let (locked_sender, locked_receiver) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            for line in lock_holder_output.lines() {
                match line {
                    Ok(line) => {
                        if line.contains(LOCK_HELD_MARKER) {
                            let _ = locked_sender.send(());
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        if locked_receiver
            .recv_timeout(LOCK_HOLDER_START_TIMEOUT)
            .is_err()
        {
            // Dropping `external_lock` kills the process.
            panic!("the lock holder process did not take the lock");
        }
        external_lock
    }
}

impl Drop for ExternalFileSystemLock {
    fn drop(&mut self) {
        // Closing its stdin tells the process to release the lock and exit.
        drop(self.lock_holder.stdin.take());
        let deadline = Instant::now() + LOCK_HOLDER_EXIT_TIMEOUT;
        loop {
            match self.lock_holder.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                _ => break,
            }
        }
        let _ = self.lock_holder.kill();
        let _ = self.lock_holder.wait();
    }
}

#[test]
fn fails_when_lock_is_already_held() {
    // In the lock holder process (see `ExternalFileSystemLock`) this test function is the
    // helper: it never returns.
    if let Ok(lock_path) = std::env::var(LOCK_HOLDER_ENV) {
        hold_shared_lock_until_stdin_closes(&lock_path);
    }

    let f = Fixture::new();
    let external_lock = ExternalFileSystemLock::get_shared(&f.root_dir.get_relative_str("gc/lock"));

    let e = f.run_garbage_collector(Some(1), None).unwrap_err();
    assert!(
        e.to_string()
            .contains("failed to acquire exclusive filesystem lock"),
        "{e}"
    );

    drop(external_lock);
}

/// Blaze-only: `IdleTaskManager.busy` interrupts a running collection (Java: the waiting thread's
/// `awaitQuiescence(true)` throws `InterruptedException` and the queued work does not run).
#[test]
fn interrupted_collection_deletes_nothing() {
    let f = Fixture::new();
    f.write_files(&[
        Entry::of("ac/123", kbytes(1), days_ago(1)),
        Entry::of("cas/456", kbytes(1), days_ago(2)),
    ]);
    blaze_shell::thread_interrupt::ThreadInterrupt::current().interrupt();
    let gc =
        DiskCacheGarbageCollector::new(f.root_dir.clone(), CollectionPolicy::new(Some(0), None));
    match gc.run() {
        Err(blaze_remote::disk::GcError::Interrupted(_)) => {}
        other => panic!("expected an interruption, got {other:?}"),
    }
    // InterruptedException clears the interrupt status.
    assert!(!blaze_shell::thread_interrupt::is_current_interrupted());
    f.assert_files_exist(&["ac/123", "cas/456"]);
}
