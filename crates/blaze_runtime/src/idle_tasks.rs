//! The idle tasks the runtime and Bazel's modules register with `env.addIdleTask`, which the
//! server runs between commands (`blaze_server::IdleTaskManager`).
//!
//! PORT: Java keeps each task beside the code it cleans up after (`lib/server`,
//! `lib/runtime/BlazeWorkspace`, `lib/remote/disk`, `lib/bazel/repository/cache`). The
//! `IdleTask` interface is `blaze_server`'s, and the crates that own the collected state
//! (`blaze_exec`, `blaze_remote`, `blaze_repo_fetch`) sit beside it, not above it; the tasks are
//! implemented here, where both are visible, each over the owning crate's ported collector.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use blaze_repo_fetch::file_system_lock::{FileSystemLock, LockMode};
use blaze_server::idle_task::{IdleTask, IdleTaskError, IdleTaskException};
use blaze_shell::thread_interrupt::{self, InterruptedException};
use blaze_vfs::{DirentType, Path, Symlinks};

use crate::commands::info::items::pretty_print_bytes;

/// An idle task that triggers a garbage collection and shrinks interner pools.
///
/// PORT: Blaze has no garbage collector; `System.gc()` is the allocator's release of free memory
/// (ruling D15, `malloc_trim`). Blaze has no `PooledInterner`s to shrink. The `[Idle GC]` log line
/// is not written (Blaze has no server log).
// Java: com/google/devtools/build/lib/server/GcAndInternerShrinkingIdleTask.java:26 GcAndInternerShrinkingIdleTask
pub struct GcAndInternerShrinkingIdleTask {
    state_kept_after_build: bool,
}

impl GcAndInternerShrinkingIdleTask {
    // Java: com/google/devtools/build/lib/server/GcAndInternerShrinkingIdleTask.java:31 GcAndInternerShrinkingIdleTask.<init>
    pub fn new(state_kept_after_build: bool) -> Self {
        GcAndInternerShrinkingIdleTask {
            state_kept_after_build,
        }
    }
}

impl IdleTask for GcAndInternerShrinkingIdleTask {
    // Java: com/google/devtools/build/lib/server/GcAndInternerShrinkingIdleTask.java:36 GcAndInternerShrinkingIdleTask.displayName
    fn display_name(&self) -> String {
        "GC and interner shrinking".to_string()
    }

    // Java: com/google/devtools/build/lib/server/GcAndInternerShrinkingIdleTask.java:41 GcAndInternerShrinkingIdleTask.delay
    fn delay(&self) -> Duration {
        // If state was kept after the build, wait for a few seconds before triggering GC, to
        // avoid unnecessarily slowing down an immediately following incremental build.
        if self.state_kept_after_build {
            Duration::from_secs(10)
        } else {
            Duration::ZERO
        }
    }

    // Java: com/google/devtools/build/lib/server/GcAndInternerShrinkingIdleTask.java:48 GcAndInternerShrinkingIdleTask.run
    fn run(&self) -> Result<(), IdleTaskError> {
        let before = crate::process_memory::get_heap_memory_usage();
        crate::process_memory::release_free_memory();
        // Shrinking interner pools can take multiple seconds for large builds, and is maximally
        // effective for builds that don't keep state. Avoid running it if state was kept, or if
        // the cleanup was interrupted in any case.
        if !thread_interrupt::interrupted() && !self.state_kept_after_build {
            // PooledInterner.shrinkAll(): Blaze has no pooled interners.
        }
        let after = crate::process_memory::get_heap_memory_usage();
        // PORT: skipped, logging ("[Idle GC] used: %s -> %s, committed: %s -> %s").
        let _ = (
            pretty_print_bytes(before.get_used()),
            pretty_print_bytes(after.get_used()),
            pretty_print_bytes(before.get_committed()),
            pretty_print_bytes(after.get_committed()),
        );
        Ok(())
    }
}

/// A garbage collector for stale install bases.
// Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollector.java:39 InstallBaseGarbageCollector
pub struct InstallBaseGarbageCollector {
    root: Path,
    own_install_base: Path,
    max_age: Duration,
}

// Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollector.java:40 InstallBaseGarbageCollector.LOCK_SUFFIX
const LOCK_SUFFIX: &str = ".lock";
// Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollector.java:41 InstallBaseGarbageCollector.DELETED_SUFFIX
const DELETED_SUFFIX: &str = ".deleted";

/// What `run` throws: `IOException` or `InterruptedException`.
#[derive(Debug)]
pub enum IoOrInterrupted {
    Io(std::io::Error),
    Interrupted(InterruptedException),
}

impl From<std::io::Error> for IoOrInterrupted {
    fn from(e: std::io::Error) -> Self {
        IoOrInterrupted::Io(e)
    }
}

impl From<IoOrInterrupted> for IdleTaskError {
    fn from(e: IoOrInterrupted) -> Self {
        match e {
            IoOrInterrupted::Io(e) => IdleTaskError::IdleTask(IdleTaskException::new(e)),
            IoOrInterrupted::Interrupted(e) => IdleTaskError::Interrupted(e),
        }
    }
}

/// `System.currentTimeMillis()`.
fn current_time_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl InstallBaseGarbageCollector {
    /// Creates a new garbage collector.
    ///
    /// * `root`: the install user root, i.e., the parent directory of install bases
    /// * `own_install_base`: the current server's install base
    /// * `max_age`: how long an install base must remain unused before it's eligible for
    ///   collection
    // Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollector.java:55 InstallBaseGarbageCollector.<init>
    pub fn new(root: Path, own_install_base: Path, max_age: Duration) -> Self {
        InstallBaseGarbageCollector {
            root,
            own_install_base,
            max_age,
        }
    }

    // Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollector.java:62 InstallBaseGarbageCollector.getRoot
    pub fn get_root(&self) -> &Path {
        &self.root
    }

    // Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollector.java:67 InstallBaseGarbageCollector.getOwnInstallBase
    pub fn get_own_install_base(&self) -> &Path {
        &self.own_install_base
    }

    // Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollector.java:72 InstallBaseGarbageCollector.getMaxAge
    pub fn get_max_age(&self) -> Duration {
        self.max_age
    }

    // Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollector.java:76 InstallBaseGarbageCollector.run
    pub fn run(&self) -> Result<(), IoOrInterrupted> {
        for dirent in self.root.readdir(Symlinks::Follow)? {
            if thread_interrupt::interrupted() {
                return Err(IoOrInterrupted::Interrupted(InterruptedException));
            }
            if dirent.get_type() != DirentType::Directory {
                // Ignore non-directories.
                continue;
            }
            let child = self.root.get_child(dirent.get_name());
            if is_install_base(&child) {
                if child == self.own_install_base {
                    // Don't attempt to collect our own install base.
                    continue;
                }
                self.collect_when_stale(&child)?;
            } else if is_incomplete_deletion(&child) {
                // This install base is either being deleted, or an earlier attempt to delete it
                // was interrupted. Assume the latter and try again, otherwise it will never be
                // deleted. Concurrent attempts are fine because deleteTree treats not found as
                // successful deletion.
                child.delete_tree()?;
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollector.java:101 InstallBaseGarbageCollector.collectWhenStale
    fn collect_when_stale(&self, install_base: &Path) -> std::io::Result<()> {
        let lock_path = get_lock_path(install_base);
        let path_to_delete;
        {
            let lock = match FileSystemLock::try_get(&lock_path, LockMode::EXCLUSIVE) {
                Ok(lock) => lock,
                // PORT: `LockAlreadyHeldException` is the `IOException` of `tryGet` whose message
                // is "failed to acquire <mode> filesystem lock on <path>".
                Err(e) if is_lock_already_held(&e) => {
                    // Looks like this install base is currently in use. Back off.
                    return Ok(());
                }
                Err(e) => return Err(e.into_io_error()),
            };
            let Some(status) = install_base.stat_if_found(Symlinks::Follow)? else {
                // The install base is already gone. Back off.
                // This cannot be a garbage collection by another Bazel server, as it would have
                // taken an exclusive lock, but maybe the user or something else in the system
                // did a cleanup.
                return Ok(());
            };
            let age_millis = current_time_millis() - status.get_last_modified_time();
            if (age_millis as i128) < self.max_age.as_millis() as i128 {
                // The install base was recently used. Back off.
                // If the install base belongs to an older binary that doesn't lock it before
                // use, it's possible to hit a tiny race condition between the older binary
                // checking whether the install base exists and updating its mtime.
                // Unfortunately, this is the best we can do.
                return Ok(());
            }
            // Rename the install base before deleting it.
            // This avoids leaving behind a corrupted install base if the deletion is
            // interrupted, which would be treated as a fatal error by a subsequent invocation
            // and require a manual cleanup. The new name must be unique, because the same
            // install base can be recreated and deleted for a second time after a first deletion
            // attempt is interrupted.
            path_to_delete = get_deleted_path(install_base);
            install_base.rename_to(&path_to_delete)?;
            // Now that the install base has been renamed, we can delete the lock file.
            // This is done early to avoid leaving the lock file behind if the deletion is
            // interrupted. It's still possible to get interrupted in between the rename and
            // delete, but we accept it.
            lock_path.delete()?;
            lock.close();
        }
        // We can now perform the actual deletion.
        path_to_delete.delete_tree()
    }
}

/// Whether `e` is `FileSystemLock.LockAlreadyHeldException`.
fn is_lock_already_held(e: &blaze_repo_fetch::errors::IoException) -> bool {
    e.message
        .as_deref()
        .is_some_and(|m| m.starts_with("failed to acquire "))
}

// Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollector.java:143 InstallBaseGarbageCollector.getLockPath
fn get_lock_path(install_base: &Path) -> Path {
    let parent = install_base
        .get_parent_directory()
        .expect("an install base has a parent");
    parent.get_child(&format!("{}{}", install_base.get_base_name(), LOCK_SUFFIX))
}

// Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollector.java:148 InstallBaseGarbageCollector.getDeletedPath
fn get_deleted_path(install_base: &Path) -> Path {
    let parent = install_base
        .get_parent_directory()
        .expect("an install base has a parent");
    parent.get_child(&format!("{}{}", uuid::Uuid::new_v4(), DELETED_SUFFIX))
}

// Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollector.java:153 InstallBaseGarbageCollector.isInstallBase
fn is_install_base(path: &Path) -> bool {
    let name = path.get_base_name();
    name.chars().count() == 32
        && name
            .chars()
            .all(|c| ('a'..='f').contains(&c) || c.is_ascii_digit())
}

// Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollector.java:159 InstallBaseGarbageCollector.isIncompleteDeletion
fn is_incomplete_deletion(path: &Path) -> bool {
    path.get_base_name().ends_with(DELETED_SUFFIX)
}

/// An idle task to garbage collect stale install bases.
// Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollectorIdleTask.java:22 InstallBaseGarbageCollectorIdleTask
pub struct InstallBaseGarbageCollectorIdleTask {
    gc: InstallBaseGarbageCollector,
}

impl InstallBaseGarbageCollectorIdleTask {
    /// Creates a new `InstallBaseGarbageCollectorIdleTask` according to the options.
    ///
    /// * `install_base`: the server install base
    /// * `max_age`: how long an install base must remain unused to be considered stale
    // Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollectorIdleTask.java:36 InstallBaseGarbageCollectorIdleTask.create
    pub fn create(install_base: &Path, max_age: Duration) -> Self {
        InstallBaseGarbageCollectorIdleTask {
            gc: InstallBaseGarbageCollector::new(
                install_base
                    .get_parent_directory()
                    .expect("an install base has a parent"),
                install_base.clone(),
                max_age,
            ),
        }
    }

    // Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollectorIdleTask.java:47 InstallBaseGarbageCollectorIdleTask.getGarbageCollector
    pub fn get_garbage_collector(&self) -> &InstallBaseGarbageCollector {
        &self.gc
    }
}

impl IdleTask for InstallBaseGarbageCollectorIdleTask {
    // Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollectorIdleTask.java:42 InstallBaseGarbageCollectorIdleTask.displayName
    fn display_name(&self) -> String {
        "Install base garbage collector".to_string()
    }

    // Java: com/google/devtools/build/lib/server/InstallBaseGarbageCollectorIdleTask.java:52 InstallBaseGarbageCollectorIdleTask.run
    fn run(&self) -> Result<(), IdleTaskError> {
        Ok(self.gc.run()?)
    }
}

use blaze_buildtool::buildtool::execution_tool::PERSISTENT_ACTION_CACHE_LOADED;

/// `actionCache = <loaded>` (`BlazeWorkspace.getOrLoadPersistentActionCache`, from
/// `dump --action_cache`; the build's load sets the flag in `blaze_buildtool`).
// Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:387 BlazeWorkspace.getOrLoadPersistentActionCache
pub fn note_persistent_action_cache_loaded() {
    PERSISTENT_ACTION_CACHE_LOADED.store(true, Ordering::SeqCst);
}

/// `actionCache = null` (`BlazeWorkspace.clearCaches`, and `createEnv` under
/// `--nouse_action_cache`).
// Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:371 BlazeWorkspace.clearCaches
pub fn note_persistent_action_cache_dropped() {
    PERSISTENT_ACTION_CACHE_LOADED.store(false, Ordering::SeqCst);
}

/// An `IdleTask` to garbage collect the action cache.
// Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:110 BlazeWorkspace.ActionCacheGarbageCollectorIdleTask
pub struct ActionCacheGarbageCollectorIdleTask {
    delay: Duration,
    threshold: f32,
    max_age: Duration,
    /// `getActionCacheDirectory()`, `getCorruptedActionCacheDirectory()`,
    /// `getActionCacheTmpDirectory()` of the outer `BlazeWorkspace`.
    output_base: Path,
}

impl ActionCacheGarbageCollectorIdleTask {
    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:115 ActionCacheGarbageCollectorIdleTask.<init>
    pub fn new(delay: Duration, threshold: f32, max_age: Duration, output_base: Path) -> Self {
        ActionCacheGarbageCollectorIdleTask {
            delay,
            threshold,
            max_age,
            output_base,
        }
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:131 ActionCacheGarbageCollectorIdleTask.getThreshold
    pub fn get_threshold(&self) -> f32 {
        self.threshold
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:136 ActionCacheGarbageCollectorIdleTask.getMaxAge
    pub fn get_max_age(&self) -> Duration {
        self.max_age
    }
}

impl IdleTask for ActionCacheGarbageCollectorIdleTask {
    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:121 ActionCacheGarbageCollectorIdleTask.displayName
    fn display_name(&self) -> String {
        "Action cache garbage collector".to_string()
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:126 ActionCacheGarbageCollectorIdleTask.delay
    fn delay(&self) -> Duration {
        self.delay
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:141 ActionCacheGarbageCollectorIdleTask.run
    fn run(&self) -> Result<(), IdleTaskError> {
        use blaze_exec::actions::cache::compact_persistent_action_cache::CompactPersistentActionCache;
        use blaze_exec::actions::cache::output_metadata_store::IoOrInterruptedException;
        if !PERSISTENT_ACTION_CACHE_LOADED.load(Ordering::SeqCst) {
            // Do not load the action cache just to garbage collect it.
            return Ok(());
        }
        // PORT: the cache Java holds in memory is the one on disk (see
        // PERSISTENT_ACTION_CACHE_LOADED); initialization errors were reported when the command
        // loaded it.
        let action_cache = CompactPersistentActionCache::create(
            &self.output_base.get_child("action_cache"),
            &self.output_base.get_child("action_cache.bad"),
            &self.output_base.get_child("action_cache.tmp"),
            Arc::new(blaze_events::JavaClock),
            blaze_events::NullEventHandler::INSTANCE,
        )
        .map_err(|e| IdleTaskError::IdleTask(IdleTaskException::new(e)))?;
        // Note that this reads and writes to the field in the outer class.
        match Arc::new(action_cache).trim(self.threshold, self.max_age) {
            Ok(_trimmed) => Ok(()),
            Err(IoOrInterruptedException::Io(e)) => {
                Err(IdleTaskError::IdleTask(IdleTaskException::new(e)))
            }
            Err(IoOrInterruptedException::Interrupted(e)) => Err(IdleTaskError::Interrupted(e)),
        }
    }
}

/// `LocalRepoContentsCache.createGcIdleTask`'s anonymous `IdleTask`.
// Java: com/google/devtools/build/lib/bazel/repository/cache/LocalRepoContentsCache.java:207 LocalRepoContentsCache.createGcIdleTask
pub struct RepoContentsCacheGcIdleTask {
    cache: blaze_repo_fetch::cache::LocalRepoContentsCache,
    max_age: Duration,
    idle_delay: Duration,
}

impl RepoContentsCacheGcIdleTask {
    /// `repoContentsCache.createGcIdleTask(maxAge, idleDelay)` for the cache at `path`.
    pub fn create(path: Path, max_age: Duration, idle_delay: Duration) -> Self {
        let mut cache = blaze_repo_fetch::cache::LocalRepoContentsCache::new();
        // Preconditions.checkState(path != null)
        cache.set_path(Some(path));
        RepoContentsCacheGcIdleTask {
            cache,
            max_age,
            idle_delay,
        }
    }
}

impl IdleTask for RepoContentsCacheGcIdleTask {
    fn display_name(&self) -> String {
        "Repo contents cache garbage collection".to_string()
    }

    fn delay(&self) -> Duration {
        self.idle_delay
    }

    fn run(&self) -> Result<(), IdleTaskError> {
        self.cache
            .run_gc_idle_task(self.max_age)
            .map_err(|e| IdleTaskError::IdleTask(IdleTaskException::new(e.into_io_error())))
    }
}

/// `DiskCacheGarbageCollectorIdleTask` as an `IdleTask` (the type is in `blaze_remote`).
pub struct DiskCacheGcIdleTask(pub blaze_remote::disk::DiskCacheGarbageCollectorIdleTask);

impl IdleTask for DiskCacheGcIdleTask {
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollectorIdleTask.java:83 DiskCacheGarbageCollectorIdleTask.displayName
    fn display_name(&self) -> String {
        self.0.display_name()
    }

    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollectorIdleTask.java:93 DiskCacheGarbageCollectorIdleTask.delay
    fn delay(&self) -> Duration {
        self.0.delay()
    }

    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollectorIdleTask.java:98 DiskCacheGarbageCollectorIdleTask.run
    fn run(&self) -> Result<(), IdleTaskError> {
        self.0
            .run()
            .map_err(|e| IdleTaskError::IdleTask(IdleTaskException::new(e)))
    }
}

#[cfg(test)]
mod tests {
    // Ported from src/test/java/com/google/devtools/build/lib/server/InstallBaseGarbageCollectorTest.java.
    use super::*;
    use blaze_vfs::{DigestHashFunction, FileSystemExt, PathFragment, UnixFileSystem};
    use std::io::{BufRead, Write};

    const OWN_MD5: &str = "012345678901234567890123456789012";
    const OTHER_MD5: &str = "abcdefabcdefabcdefabcdefabcdefab";
    const DAY: Duration = Duration::from_secs(24 * 3600);

    struct Fixture {
        _tmp: tempfile::TempDir,
        root_dir: Path,
        own_install_base: Path,
    }

    // setUp
    fn set_up() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root_dir = fs.get_path(tmp.path().to_str().unwrap());
        let own_install_base = create_subdirectory(&root_dir, OWN_MD5);
        Fixture {
            _tmp: tmp,
            root_dir,
            own_install_base,
        }
    }

    fn write(path: &Path, content: &str) {
        let mut out = path.get_output_stream().unwrap();
        out.write_all(content.as_bytes()).unwrap();
    }

    fn create_subdirectory(root_dir: &Path, name: &str) -> Path {
        let dir = root_dir.get_child(name);
        let file = dir.get_child("file");
        let subdir = dir.get_child("subdir");
        let subfile = subdir.get_child("file");
        dir.create_directory_and_parents().unwrap();
        subdir.create_directory_and_parents().unwrap();
        write(&file, "content");
        write(&subfile, "content");
        dir
    }

    fn set_age(path: &Path, age: Duration) {
        path.set_last_modified_time(current_time_millis() - age.as_millis() as i64)
            .unwrap();
    }

    fn run(f: &Fixture, max_age: Duration) {
        InstallBaseGarbageCollector::new(f.root_dir.clone(), f.own_install_base.clone(), max_age)
            .run()
            .unwrap();
    }

    fn assert_directory_contents(f: &Fixture, expected: &[&str]) {
        let mut actual: Vec<String> = f
            .root_dir
            .get_directory_entries()
            .unwrap()
            .iter()
            .map(|p| p.get_base_name().to_string())
            .collect();
        actual.sort();
        let mut expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        expected.sort();
        assert_eq!(actual, expected);
    }

    /// `ExternalFileSystemLock.getShared`: a POSIX shared lock on `path` held by another process
    /// (python3's `fcntl.lockf`, the record lock `FileChannel.lock` takes), until dropped.
    struct ExternalFileSystemLock(std::process::Child);

    impl ExternalFileSystemLock {
        fn get_shared(path: &Path) -> ExternalFileSystemLock {
            let mut child = std::process::Command::new("python3")
                .args([
                    "-c",
                    "import fcntl,sys\nf=open(sys.argv[1],'a+')\nfcntl.lockf(f,fcntl.LOCK_SH)\nprint('locked',flush=True)\nsys.stdin.read()",
                    path.get_path_string(),
                ])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .expect("python3");
            let mut line = String::new();
            std::io::BufReader::new(child.stdout.as_mut().unwrap())
                .read_line(&mut line)
                .unwrap();
            assert_eq!(line, "locked\n");
            ExternalFileSystemLock(child)
        }
    }

    impl Drop for ExternalFileSystemLock {
        fn drop(&mut self) {
            drop(self.0.stdin.take());
            let _ = self.0.wait();
        }
    }

    #[test]
    fn only_own_install_base_not_collected() {
        let f = set_up();
        run(&f, Duration::ZERO);
        assert_directory_contents(&f, &[OWN_MD5]);
    }

    #[test]
    fn other_install_base_not_stale_and_unlocked_not_collected() {
        let f = set_up();
        let other_install_base = create_subdirectory(&f.root_dir, OTHER_MD5);
        set_age(&other_install_base, DAY);
        run(&f, 2 * DAY);
        let lock = format!("{OTHER_MD5}{LOCK_SUFFIX}");
        assert_directory_contents(&f, &[OWN_MD5, OTHER_MD5, &lock]);
    }

    #[test]
    fn other_install_base_not_stale_and_locked_not_collected() {
        let f = set_up();
        let other_install_base = create_subdirectory(&f.root_dir, OTHER_MD5);
        set_age(&other_install_base, DAY);
        let lock = format!("{OTHER_MD5}{LOCK_SUFFIX}");
        {
            let _lock = ExternalFileSystemLock::get_shared(&f.root_dir.get_child(&lock));
            run(&f, 2 * DAY);
        }
        assert_directory_contents(&f, &[OWN_MD5, OTHER_MD5, &lock]);
    }

    #[test]
    fn other_install_base_stale_and_unlocked_collected() {
        let f = set_up();
        let other_install_base = create_subdirectory(&f.root_dir, OTHER_MD5);
        set_age(&other_install_base, 3 * DAY);
        run(&f, 2 * DAY);
        assert_directory_contents(&f, &[OWN_MD5]);
    }

    #[test]
    fn other_install_base_stale_and_locked_not_collected() {
        let f = set_up();
        let other_install_base = create_subdirectory(&f.root_dir, OTHER_MD5);
        set_age(&other_install_base, 3 * DAY);
        let lock = format!("{OTHER_MD5}{LOCK_SUFFIX}");
        {
            let _lock = ExternalFileSystemLock::get_shared(&f.root_dir.get_child(&lock));
            run(&f, 2 * DAY);
        }
        assert_directory_contents(&f, &[OWN_MD5, OTHER_MD5, &lock]);
    }

    #[test]
    fn incomplete_deletion_collected() {
        let f = set_up();
        let incomplete_deletion =
            create_subdirectory(&f.root_dir, &format!("{OTHER_MD5}{DELETED_SUFFIX}"));
        set_age(&incomplete_deletion, 2 * DAY);
        run(&f, DAY);
        assert_directory_contents(&f, &[OWN_MD5]);
    }

    #[test]
    fn other_files_and_directories_not_collected() {
        let f = set_up();
        let other_file = f.root_dir.get_child("file");
        write(&other_file, "content");
        set_age(&other_file, 2 * DAY);
        let other_dir = f.root_dir.get_child("dir");
        other_dir.create_directory_and_parents().unwrap();
        set_age(&other_dir, 2 * DAY);
        let other_symlink = f.root_dir.get_child("symlink");
        other_symlink
            .create_symbolic_link_to_fragment(&PathFragment::create(OWN_MD5))
            .unwrap();
        run(&f, DAY);
        assert_directory_contents(&f, &[OWN_MD5, "file", "dir", "symlink"]);
    }

    /// The collector of `InstallBaseGarbageCollectorIdleTask.create` (the part of
    /// `BlazeRuntimeTest.addsInstallBaseGcIdleTaskWhenEnabled` that reads the collector).
    #[test]
    fn install_base_gc_idle_task_create() {
        let f = set_up();
        let task = InstallBaseGarbageCollectorIdleTask::create(&f.own_install_base, 365 * DAY);
        assert_eq!(task.delay(), Duration::ZERO);
        assert_eq!(task.get_garbage_collector().get_root(), &f.root_dir);
        assert_eq!(
            task.get_garbage_collector().get_own_install_base(),
            &f.own_install_base
        );
        assert_eq!(task.get_garbage_collector().get_max_age(), 365 * DAY);
    }

    /// `BlazeRuntimeTest.addsActionCacheGcIdleTaskWhenEnabled`: threshold and max age.
    #[test]
    fn action_cache_gc_idle_task_fields() {
        let f = set_up();
        let task = ActionCacheGarbageCollectorIdleTask::new(
            Duration::from_secs(300),
            10 as f32 / 100.0f32,
            7 * DAY,
            f.root_dir.clone(),
        );
        assert_eq!(task.delay(), Duration::from_secs(300));
        assert_eq!(task.get_threshold(), 0.1f32);
        assert_eq!(task.get_max_age(), 7 * DAY);
    }
}
