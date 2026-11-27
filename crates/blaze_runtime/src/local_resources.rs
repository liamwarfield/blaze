//! `LocalHostCapacity` and `LocalHostResource`: the local host's resource capacity (RAM and CPU),
//! detected the way the JVM does on Linux, including cgroup (container) limits.
//!
//! PORT: Java's `ResourceSet` is not ported in this crate; [`LocalResourceSet`] carries the two
//! values the runtime reads (`getMemoryMb`, `getCpuUsage`). `blaze_optdefs` has an older copy of
//! this detection (`common::local_host_capacity`) for its converters; this one follows the JDK's
//! cgroup logic more closely (it walks the cgroup hierarchy, as JDK 25 does).

use std::sync::OnceLock;

use parking_lot::Mutex;

/// The parts of a `ResourceSet` that describe the local host.
// Java: com/google/devtools/build/lib/actions/ResourceSet.java:42 ResourceSet
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalResourceSet {
    memory_mb: f64,
    cpu_usage: f64,
    local_test_count: i32,
}

impl LocalResourceSet {
    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:59 ResourceSet.create
    pub fn create(memory_mb: f64, cpu_usage: f64, local_test_count: i32) -> LocalResourceSet {
        LocalResourceSet {
            memory_mb,
            cpu_usage,
            local_test_count,
        }
    }

    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:106 ResourceSet.getMemoryMb
    pub fn get_memory_mb(&self) -> f64 {
        self.memory_mb
    }

    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:110 ResourceSet.getCpuUsage
    pub fn get_cpu_usage(&self) -> f64 {
        self.cpu_usage
    }

    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:128 ResourceSet.getLocalTestCount
    pub fn get_local_test_count(&self) -> i32 {
        self.local_test_count
    }
}

/// This class estimates the local host's resource capacity.
// Java: com/google/devtools/build/lib/actions/LocalHostCapacity.java:25 LocalHostCapacity
pub struct LocalHostCapacity;

static LOCAL_HOST_CAPACITY: Mutex<Option<LocalResourceSet>> = Mutex::new(None);

impl LocalHostCapacity {
    // Java: com/google/devtools/build/lib/actions/LocalHostCapacity.java:32 LocalHostCapacity.getLocalHostCapacity
    pub fn get_local_host_capacity() -> LocalResourceSet {
        let mut capacity = LOCAL_HOST_CAPACITY.lock();
        if capacity.is_none() {
            *capacity = Some(Self::get_new_local_host_capacity());
        }
        capacity.unwrap()
    }

    // Java: com/google/devtools/build/lib/actions/LocalHostCapacity.java:34 LocalHostCapacity.getNewLocalHostCapacity
    fn get_new_local_host_capacity() -> LocalResourceSet {
        let local_resources = LocalHostResource::get();
        // "Determined local resources: RAM=%dMB, CPU=%.1f" ((int) memoryMb, cpuUsage).
        // PORT: Java's %.1f rounds HALF_UP, Rust's {:.1} half to even; CPU counts are whole
        // numbers, so the two agree.
        blaze_util::server_logger::info(
            "com.google.devtools.build.lib.actions.LocalHostCapacity",
            "getNewLocalHostCapacity",
            &format!(
                "Determined local resources: RAM={}MB, CPU={:.1}",
                local_resources.get_memory_mb() as i32,
                local_resources.get_cpu_usage()
            ),
        );
        local_resources
    }

    /// Sets the local host capacity to hardcoded values.
    ///
    /// `capacity`: the explicit capacity, or `None` to use the machine-specific values again.
    // Java: com/google/devtools/build/lib/actions/LocalHostCapacity.java:53 LocalHostCapacity.setLocalHostCapacity
    pub fn set_local_host_capacity(capacity: Option<LocalResourceSet>) {
        *LOCAL_HOST_CAPACITY.lock() = capacity;
    }
}

/// This class computes the local host's resource capacity.
// Java: com/google/devtools/build/lib/actions/LocalHostResource.java:21 LocalHostResource
struct LocalHostResource;

impl LocalHostResource {
    // Java: com/google/devtools/build/lib/actions/LocalHostResource.java:36 LocalHostResource.get
    fn get() -> LocalResourceSet {
        // Java: com/google/devtools/build/lib/actions/LocalHostResource.java:23 LocalHostResource.DEFAULT_RESOURCES
        static DEFAULT_RESOURCES: OnceLock<LocalResourceSet> = OnceLock::new();
        *DEFAULT_RESOURCES.get_or_init(|| {
            LocalResourceSet::create(
                // Only com.sun.management.OperatingSystemMXBean provides the total physical
                // memory size. This bean is container-aware as of JDK 14.
                get_total_physical_memory_size() as f64 / (1024.0 * 1024.0),
                // As of JDK 11, availableProcessors is aware of cgroups as commonly used by
                // containers.
                available_processors() as f64,
                i32::MAX,
            )
        })
    }
}

/// `com.sun.management.OperatingSystemMXBean.getTotalPhysicalMemorySize()` on Linux (JDK 25,
/// `jdk.management/unix/classes/com/sun/management/internal/OperatingSystemImpl.java`): the
/// container memory limit when there is one and it is lower than the host's memory, else the
/// host's memory (`sysconf(_SC_PHYS_PAGES) * sysconf(_SC_PAGESIZE)`).
fn get_total_physical_memory_size() -> i64 {
    // SAFETY: sysconf has no preconditions.
    let host_total = unsafe {
        libc::sysconf(libc::_SC_PHYS_PAGES) as i64 * libc::sysconf(libc::_SC_PAGESIZE) as i64
    };
    match cgroup_memory_limit() {
        Some(limit) if limit > 0 && limit < host_total => limit,
        _ => host_total,
    }
}

/// `Runtime.availableProcessors()` on Linux (HotSpot `os::active_processor_count` with container
/// support): the number of CPUs in this process's affinity mask, lowered to
/// `ceil(quota / period)` of the lowest cgroup v2 `cpu.max` in the hierarchy.
pub fn available_processors() -> i32 {
    // SAFETY: cpu_set_t is plain data; sched_getaffinity writes at most size_of::<cpu_set_t>().
    let host_cpus = unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        if libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &mut set) == 0 {
            libc::CPU_COUNT(&set)
        } else {
            libc::sysconf(libc::_SC_NPROCESSORS_ONLN) as i32
        }
    };
    match cgroup_cpu_quota_count() {
        Some(quota_count) if quota_count < host_cpus => quota_count.max(1),
        _ => host_cpus,
    }
}

/// The mount point of the unified (v2) hierarchy and the directory of this process's cgroup in
/// it, or `None` when there is no cgroup v2 hierarchy. Panics (unported) on a cgroup v1 memory or cpu controller, which
/// the JDK also reads.
fn cgroup_v2_directory() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let mut v2_path = None;
    for line in cgroup.lines() {
        let mut parts = line.splitn(3, ':');
        let (Some(id), Some(controllers), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if id == "0" && controllers.is_empty() {
            v2_path = Some(path.to_string());
        } else if controllers.split(',').any(|c| c == "memory" || c == "cpu") {
            blaze_util::unported!(
                "CgroupV1Subsystem",
                "cgroup v1 memory/cpu controllers are not supported: {}",
                line
            );
        }
    }
    let cgroup_path = v2_path?;
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    for line in mountinfo.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        if right.split(' ').next() != Some("cgroup2") {
            continue;
        }
        let fields: Vec<&str> = left.split(' ').collect();
        if fields.len() < 5 {
            continue;
        }
        let (root, mount_point) = (fields[3], fields[4]);
        let relative = if root == "/" {
            cgroup_path.as_str()
        } else if cgroup_path == root {
            "/"
        } else if let Some(rest) = cgroup_path.strip_prefix(root) {
            rest
        } else {
            continue;
        };
        let leaf = format!(
            "{}{}",
            mount_point,
            if relative == "/" { "" } else { relative }
        );
        return Some((
            std::path::PathBuf::from(mount_point),
            std::path::PathBuf::from(leaf),
        ));
    }
    None
}

/// The directories from this process's cgroup up to (excluding) the hierarchy root, leaf first.
/// HotSpot (JDK 23+, JDK-8322420) looks for the lowest CPU limit along this path.
fn cgroup_v2_hierarchy() -> Vec<std::path::PathBuf> {
    let Some((mount_point, leaf)) = cgroup_v2_directory() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut dir = Some(leaf.as_path());
    while let Some(d) = dir {
        // The hierarchy root has no memory.max / cpu.max.
        if d == mount_point || !d.starts_with(&mount_point) {
            break;
        }
        result.push(d.to_path_buf());
        dir = d.parent();
    }
    result
}

/// The `memory.max` of this process's own cgroup v2 directory, in bytes (`None`: unlimited).
///
/// The JDK's `OperatingSystemMXBean.getTotalPhysicalMemorySize()` (which
/// `LocalHostResource` reads) takes the container limit from `jdk.internal.platform`'s cgroup
/// metrics, which read `memory.max` of the process's own cgroup only; limits of parent cgroups
/// are not considered (unlike HotSpot's processor count, which `available_processors` ports).
/// PORT: established on the oracle (JDK 25.0.2 embedded in Bazel 9.3.2): with
/// `app-t3code-agents.slice/memory.max` = 25 GiB and `max` in the process's own scope it reports
/// the host's memory (`RAM=31225MB`); under `systemd-run -p MemoryMax=12G` (the limit on the
/// process's own cgroup) it reports `RAM=12288MB`.
fn cgroup_memory_limit() -> Option<i64> {
    let (_, leaf) = cgroup_v2_directory()?;
    let text = std::fs::read_to_string(leaf.join("memory.max")).ok()?;
    let text = text.trim();
    if text == "max" {
        return None;
    }
    text.parse::<i64>().ok()
}

/// `ceil(quota / period)` of the lowest `cpu.max` quota in this process's cgroup v2 hierarchy
/// (`None`: unlimited).
fn cgroup_cpu_quota_count() -> Option<i32> {
    let mut lowest: Option<i32> = None;
    for dir in cgroup_v2_hierarchy() {
        let Ok(text) = std::fs::read_to_string(dir.join("cpu.max")) else {
            continue;
        };
        let mut parts = text.split_whitespace();
        let (Some(quota), Some(period)) = (parts.next(), parts.next()) else {
            continue;
        };
        if quota == "max" {
            continue;
        }
        let (Ok(quota), Ok(period)) = (quota.parse::<f32>(), period.parse::<f32>()) else {
            continue;
        };
        if period <= 0.0 {
            continue;
        }
        let count = (quota / period).ceil() as i32;
        lowest = Some(lowest.map_or(count, |l| l.min(count)));
    }
    lowest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_local_host_capacity_overrides_and_resets() {
        let fake = LocalResourceSet::create(1024.0, 2.0, i32::MAX);
        LocalHostCapacity::set_local_host_capacity(Some(fake));
        assert_eq!(LocalHostCapacity::get_local_host_capacity(), fake);
        LocalHostCapacity::set_local_host_capacity(None);
        let real = LocalHostCapacity::get_local_host_capacity();
        assert!(real.get_memory_mb() > 0.0);
        assert!(real.get_cpu_usage() >= 1.0);
        assert_eq!(real.get_local_test_count(), i32::MAX);
    }
}
