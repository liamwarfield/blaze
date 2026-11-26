//! The process figures that stand in for the JVM's memory and garbage collector beans (ruling
//! D15, docs/decisions.md 2026-11-26).
//!
//! Bazel's server reads `ManagementFactory.getMemoryMXBean().getHeapMemoryUsage()`,
//! `Runtime.getRuntime().totalMemory()` and the `GarbageCollectorMXBean`s. Blaze has no JVM, so
//! it reports the nearest figure of its own process in Bazel's format:
//!
//! | Java | Blaze |
//! | --- | --- |
//! | `MemoryUsage.getUsed()` | resident memory (`VmRSS`); after `System.gc()`, resident memory after the allocator returned its free memory (`malloc_trim`) |
//! | `MemoryUsage.getCommitted()`, `Runtime.totalMemory()` | resident memory (`VmRSS`) |
//! | `MemoryUsage.getMax()` | physical memory |
//! | peak heap after a full GC | peak resident memory (`VmHWM`) |
//! | GC count and time | 0 |
//!
//! The values differ from Bazel's by nature; comparison tools treat these keys as volatile.
//! Every heap figure derives from resident memory (D15 as amended on 2026-11-26), so its unit is
//! MB as on a JVM.

/// `java.lang.management.MemoryUsage`: the heap figures `InfoItemUtils.getMemoryUsage` returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryUsage {
    used: i64,
    committed: i64,
    max: i64,
}

impl MemoryUsage {
    pub fn get_used(&self) -> i64 {
        self.used
    }

    pub fn get_committed(&self) -> i64 {
        self.committed
    }

    pub fn get_max(&self) -> i64 {
        self.max
    }
}

/// `ManagementFactory.getMemoryMXBean().getHeapMemoryUsage()` (D15 figures).
pub fn get_heap_memory_usage() -> MemoryUsage {
    let committed = resident_bytes();
    MemoryUsage {
        used: committed,
        committed,
        max: physical_memory_bytes(),
    }
}

/// `Runtime.getRuntime().totalMemory()` (D15: resident memory).
pub fn total_memory() -> i64 {
    resident_bytes()
}

/// The peak heap use of the process (D15: peak resident memory, `VmHWM`).
pub fn peak_resident_bytes() -> Option<i64> {
    proc_self_status_kb("VmHWM:").map(|kb| kb * 1024)
}

/// Returns freed memory of the allocator to the system (`malloc_trim(0)`). D15 allows it after
/// each server command; it changes no output.
pub fn release_free_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // SAFETY: malloc_trim only releases free memory of the allocator.
        unsafe {
            libc::malloc_trim(0);
        }
    }
}

/// Resident memory of this process (`VmRSS` of `/proc/self/status`).
fn resident_bytes() -> i64 {
    proc_self_status_kb("VmRSS:")
        .map(|kb| kb * 1024)
        .unwrap_or(0)
}

/// Physical memory: `sysconf(_SC_PHYS_PAGES) * sysconf(_SC_PAGESIZE)`.
fn physical_memory_bytes() -> i64 {
    // SAFETY: sysconf has no preconditions.
    let (pages, page_size) = unsafe {
        (
            libc::sysconf(libc::_SC_PHYS_PAGES),
            libc::sysconf(libc::_SC_PAGESIZE),
        )
    };
    if pages <= 0 || page_size <= 0 {
        return 0;
    }
    (pages as i64).wrapping_mul(page_size as i64)
}

/// The value in kB of a `/proc/self/status` line such as `VmRSS:   12345 kB`.
fn proc_self_status_kb(field: &str) -> Option<i64> {
    let status = std::fs::read("/proc/self/status").ok()?;
    parse_status_kb(&status, field)
}

fn parse_status_kb(status: &[u8], field: &str) -> Option<i64> {
    for line in status.split(|b| *b == b'\n') {
        if let Some(rest) = line.strip_prefix(field.as_bytes()) {
            let digits: Vec<u8> = rest
                .iter()
                .copied()
                .skip_while(|b| b.is_ascii_whitespace())
                .take_while(|b| b.is_ascii_digit())
                .collect();
            return std::str::from_utf8(&digits).ok()?.parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_lines() {
        let status = b"Name:\tblaze\nVmHWM:\t  204800 kB\nVmRSS:\t   10240 kB\n";
        assert_eq!(parse_status_kb(status, "VmRSS:"), Some(10240));
        assert_eq!(parse_status_kb(status, "VmHWM:"), Some(204800));
        assert_eq!(parse_status_kb(status, "VmSwap:"), None);
    }

    #[test]
    fn figures_of_this_process_are_positive() {
        let usage = get_heap_memory_usage();
        assert!(usage.get_used() > 0);
        assert!(usage.get_committed() > 0);
        assert!(usage.get_max() >= usage.get_committed());
        // Resident memory first: the peak read after it is at least as large.
        let total = total_memory();
        assert!(peak_resident_bytes().unwrap() >= total);
    }
}
