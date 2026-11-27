//! Port of `lib/remote/disk/DiskCacheGarbageCollectorIdleTask.java`.
//!
//! PORT: the `IdleTask` interface lives in `blaze_server`, which this crate does not depend on;
//! `blaze_runtime` (`idle_tasks.rs`) implements it for this type and registers it with
//! `env.addIdleTask`.

use std::time::Duration;

use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_vfs::Path;

use super::disk_cache_garbage_collector::{CollectionPolicy, DiskCacheGarbageCollector, GcError};

/// An idle task to collect garbage in a disk cache.
// Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollectorIdleTask.java:33 DiskCacheGarbageCollectorIdleTask
pub struct DiskCacheGarbageCollectorIdleTask {
    delay: Duration,
    gc: DiskCacheGarbageCollector,
}

impl DiskCacheGarbageCollectorIdleTask {
    /// Creates a new `DiskCacheGarbageCollectorIdleTask` according to the options, or `None` if
    /// garbage collection is disabled.
    ///
    /// PORT: Java's collector schedules its I/O onto a shared `disk-cache-gc-%d` executor; the
    /// ported collector does its I/O on the calling thread.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollectorIdleTask.java:58 DiskCacheGarbageCollectorIdleTask.create
    pub fn create(
        remote_options: &RemoteOptions,
        working_directory: &Path,
    ) -> Option<DiskCacheGarbageCollectorIdleTask> {
        let disk_cache = remote_options.disk_cache.as_ref()?;
        if disk_cache.get().is_empty() {
            return None;
        }
        let mut max_size_bytes = None;
        if remote_options.disk_cache_gc_max_size > 0 {
            max_size_bytes = Some(remote_options.disk_cache_gc_max_size);
        }
        let mut max_age = None;
        if !remote_options.disk_cache_gc_max_age.is_zero() {
            max_age = Some(remote_options.disk_cache_gc_max_age);
        }
        let delay = remote_options.disk_cache_gc_idle_delay;
        if max_size_bytes.is_none() && max_age.is_none() {
            return None;
        }
        let policy = CollectionPolicy::new(max_size_bytes, max_age);
        let gc = DiskCacheGarbageCollector::new(
            working_directory.get_relative(disk_cache.get()),
            policy,
        );
        Some(DiskCacheGarbageCollectorIdleTask { delay, gc })
    }

    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollectorIdleTask.java:83 DiskCacheGarbageCollectorIdleTask.displayName
    pub fn display_name(&self) -> String {
        "Disk cache garbage collector".to_string()
    }

    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollectorIdleTask.java:88 DiskCacheGarbageCollectorIdleTask.getGarbageCollector
    pub fn get_garbage_collector(&self) -> &DiskCacheGarbageCollector {
        &self.gc
    }

    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollectorIdleTask.java:93 DiskCacheGarbageCollectorIdleTask.delay
    pub fn delay(&self) -> Duration {
        self.delay
    }

    /// `run()`: the collection; an `Err` is the `IOException` Java wraps in an
    /// `IdleTaskException`, or the `InterruptedException` it lets through.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollectorIdleTask.java:98 DiskCacheGarbageCollectorIdleTask.run
    pub fn run(&self) -> Result<(), GcError> {
        let stats = self.gc.run()?;
        blaze_util::server_logger::info(
            "com.google.devtools.build.lib.remote.disk.DiskCacheGarbageCollectorIdleTask",
            "run",
            &stats.display_string(),
        );
        Ok(())
    }
}
