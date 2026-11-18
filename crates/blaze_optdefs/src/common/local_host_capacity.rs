//! Port of `com/google/devtools/build/lib/actions/LocalHostCapacity.java` and
//! `LocalHostResource.java`: the local machine's CPU and RAM, as used by `ResourceConverter`.

use std::cell::Cell;
use std::sync::OnceLock;

/// The two `ResourceSet` fields `ResourceConverter` reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalHostResources {
    /// `ResourceSet.getMemoryMb()`.
    pub memory_mb: f64,
    /// `ResourceSet.getCpuUsage()`.
    pub cpu_usage: f64,
}

// Java: com/google/devtools/build/lib/actions/LocalHostCapacity.java:34 LocalHostCapacity.getLocalHostCapacity
/// The local host's resource capacity, computed once.
pub fn get_local_host_capacity() -> LocalHostResources {
    if let Some(capacity) = OVERRIDE.with(Cell::get) {
        return capacity;
    }
    static CAPACITY: OnceLock<LocalHostResources> = OnceLock::new();
    *CAPACITY.get_or_init(local_host_resource_get)
}

thread_local! {
    static OVERRIDE: Cell<Option<LocalHostResources>> = const { Cell::new(None) };
}

// Java: com/google/devtools/build/lib/actions/LocalHostCapacity.java:53 LocalHostCapacity.setLocalHostCapacity
/// Sets the local host capacity to hardcoded values (`None`: use the machine-specific values
/// again).
// PORT: Java's override is a process-wide static; it is per thread here so that tests running in
// parallel do not see each other's values.
pub fn set_local_host_capacity(capacity: Option<LocalHostResources>) {
    OVERRIDE.with(|o| o.set(capacity));
}

// Java: com/google/devtools/build/lib/actions/LocalHostResource.java:25 LocalHostResource.DEFAULT_RESOURCES
fn local_host_resource_get() -> LocalHostResources {
    LocalHostResources {
        memory_mb: total_physical_memory_bytes() as f64 / (1024.0 * 1024.0),
        // Java: Runtime.availableProcessors(), which is cgroup-aware like available_parallelism.
        cpu_usage: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1) as f64,
    }
}

/// `OperatingSystemMXBean.getTotalPhysicalMemorySize()`: host RAM, or the cgroup memory limit
/// when one is set and lower (the JDK's container support).
// PORT: the JDK reads cgroup v1 and v2; only cgroup v2 (`memory.max`) is read here.
fn total_physical_memory_bytes() -> u64 {
    let host = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                let rest = l.strip_prefix("MemTotal:")?;
                let kb: u64 = rest.trim().trim_end_matches("kB").trim().parse().ok()?;
                Some(kb * 1024)
            })
        })
        .unwrap_or_else(|| blaze_util::unported!("LocalHostResource without /proc/meminfo"));
    let limit = std::fs::read_to_string("/sys/fs/cgroup/memory.max")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());
    match limit {
        Some(l) if l < host => l,
        _ => host,
    }
}

/// Runs `f` with the local host capacity set to `(memory_mb, cpu_usage)` (Java tests call
/// `LocalHostCapacity.setLocalHostCapacity` before and `setLocalHostCapacity(null)` after).
#[cfg(test)]
pub(crate) fn with_local_host_capacity<R>(capacity: (f64, f64), f: impl FnOnce() -> R) -> R {
    set_local_host_capacity(Some(LocalHostResources {
        memory_mb: capacity.0,
        cpu_usage: capacity.1,
    }));
    let r = f();
    set_local_host_capacity(None);
    r
}
