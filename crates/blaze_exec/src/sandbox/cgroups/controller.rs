//! Port of `lib/sandbox/cgroups/controller/`: `Controller` and its cgroups v1 (`Legacy*`) and
//! v2 (`Unified*`) memory and cpu controllers.
//!
//! Paths are `java.nio.file.Path`s of the cgroup file system: platform paths (`PathBuf`).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// `Files.readString(path)`.
fn read_string(path: &Path) -> io::Result<String> {
    let bytes = std::fs::read(path)?;
    // Files.readString decodes UTF-8 and throws MalformedInputException on bad input.
    String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// `Files.writeString(path, s)` (create or truncate).
fn write_string(path: &Path, s: &str) -> io::Result<()> {
    std::fs::write(path, s)
}

/// `Long.parseLong(s.trim())`: a `NumberFormatException` is unchecked in Java and crashes.
fn parse_long_trimmed(s: &str, path: &Path) -> i64 {
    let t = s.trim_matches(|c: char| c <= ' ');
    t.parse::<i64>().unwrap_or_else(|_| {
        panic!(
            "java.lang.NumberFormatException: For input string: \"{t}\" (in {})",
            path.display()
        )
    })
}

/// `File.mkdirs()`: the result is ignored by the callers.
fn mkdirs(path: &Path) {
    let _ = std::fs::create_dir_all(path);
}

/// `File.isDirectory()`.
fn is_directory(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_dir())
}

/// `access(path, mode) == 0` (`File.canWrite`, `File.canRead`).
pub(crate) fn access(path: &Path, mode: libc::c_int) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: a NUL-terminated path.
    unsafe { libc::access(c.as_ptr(), mode) == 0 }
}

/// A cgroup controller.
// Java: com/google/devtools/build/lib/sandbox/cgroups/controller/Controller.java:22 Controller
pub trait Controller: Send + Sync {
    /// `isLegacy()`: whether the controller is a cgroups v1 one.
    fn is_legacy(&self) -> bool;

    /// `exists()`: the controller's directory exists.
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/Controller.java:25 Controller.exists
    fn exists(&self) -> bool {
        is_directory(self.get_path())
    }

    /// `getPath()`.
    fn get_path(&self) -> &Path;
}

/// `Controller.Memory`.
// Java: com/google/devtools/build/lib/sandbox/cgroups/controller/Controller.java:31 Controller.Memory
pub trait MemoryController: Controller {
    fn child(&self, name: &str) -> io::Result<Arc<dyn MemoryController>>;
    fn set_max_bytes(&self, bytes: i64) -> io::Result<()>;
    fn get_max_bytes(&self) -> io::Result<i64>;
    fn get_usage_in_bytes(&self) -> io::Result<i64>;
}

/// `Controller.Cpu`.
// Java: com/google/devtools/build/lib/sandbox/cgroups/controller/Controller.java:41 Controller.Cpu
pub trait CpuController: Controller {
    fn child(&self, name: &str) -> io::Result<Arc<dyn CpuController>>;
    fn set_cpus(&self, cpus: f64) -> io::Result<()>;
    fn get_cpus(&self) -> io::Result<i64>;
}

/// `LegacyController.getChild`: creates the child directory.
// Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v1/LegacyController.java:27 LegacyController.getChild
fn legacy_get_child(path: &Path, name: &str) -> PathBuf {
    let child = path.join(name);
    mkdirs(&child);
    child
}

/// `UnifiedController.getChild`: enables every available controller in the subtree, then
/// creates the child directory.
// Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v2/UnifiedController.java:34 UnifiedController.getChild
fn unified_get_child(path: &Path, name: &str) -> io::Result<PathBuf> {
    let subtree = path.join("cgroup.subtree_control");
    let controllers = path.join("cgroup.controllers");
    if access(&subtree, libc::W_OK) && access(&controllers, libc::R_OK) {
        // new Scanner(controllers): whitespace-separated tokens; CharSink.writeLines(lines, " ")
        // writes each line followed by the separator.
        let available = read_string(&controllers)?;
        let mut content = String::new();
        for c in available.split_ascii_whitespace() {
            content.push('+');
            content.push_str(c);
            content.push(' ');
        }
        write_string(&subtree, &content)?;
    }
    let child = path.join(name);
    mkdirs(&child);
    Ok(child)
}

/// The cgroups v1 memory controller.
// Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v1/LegacyMemory.java:24 LegacyMemory
pub struct LegacyMemory {
    path: PathBuf,
}

impl LegacyMemory {
    pub fn new(path: PathBuf) -> LegacyMemory {
        LegacyMemory { path }
    }
}

impl Controller for LegacyMemory {
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v1/LegacyController.java:23 LegacyController.isLegacy
    fn is_legacy(&self) -> bool {
        true
    }
    fn get_path(&self) -> &Path {
        &self.path
    }
}

impl MemoryController for LegacyMemory {
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v1/LegacyMemory.java:38 LegacyMemory.child
    fn child(&self, name: &str) -> io::Result<Arc<dyn MemoryController>> {
        Ok(Arc::new(LegacyMemory::new(legacy_get_child(
            &self.path, name,
        ))))
    }
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v1/LegacyMemory.java:43 LegacyMemory.setMaxBytes
    fn set_max_bytes(&self, bytes: i64) -> io::Result<()> {
        write_string(&self.path.join("memory.limit_in_bytes"), &bytes.to_string())
    }
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v1/LegacyMemory.java:48 LegacyMemory.getMaxBytes
    fn get_max_bytes(&self) -> io::Result<i64> {
        let p = self.path.join("memory.limit_in_bytes");
        Ok(parse_long_trimmed(&read_string(&p)?, &p))
    }
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v1/LegacyMemory.java:53 LegacyMemory.getUsageInBytes
    fn get_usage_in_bytes(&self) -> io::Result<i64> {
        let p = self.path.join("memory.usage_in_bytes");
        Ok(parse_long_trimmed(&read_string(&p)?, &p))
    }
}

/// The cgroups v1 cpu controller.
// Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v1/LegacyCpu.java:24 LegacyCpu
pub struct LegacyCpu {
    path: PathBuf,
}

impl LegacyCpu {
    pub fn new(path: PathBuf) -> LegacyCpu {
        LegacyCpu { path }
    }
}

impl Controller for LegacyCpu {
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v1/LegacyController.java:23 LegacyController.isLegacy
    fn is_legacy(&self) -> bool {
        true
    }
    fn get_path(&self) -> &Path {
        &self.path
    }
}

impl CpuController for LegacyCpu {
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v1/LegacyCpu.java:32 LegacyCpu.child
    fn child(&self, name: &str) -> io::Result<Arc<dyn CpuController>> {
        Ok(Arc::new(LegacyCpu::new(legacy_get_child(&self.path, name))))
    }
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v1/LegacyCpu.java:42 LegacyCpu.setCpus
    fn set_cpus(&self, cpus: f64) -> io::Result<()> {
        let p = self.path.join("cpu.cfs_period_us");
        let period = parse_long_trimmed(&read_string(&p)?, &p);
        let quota = java_math_round(cpus * period as f64);
        write_string(&self.path.join("cpu.cfs_quota_us"), &quota.to_string())
    }
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v1/LegacyCpu.java:49 LegacyCpu.getCpus
    fn get_cpus(&self) -> io::Result<i64> {
        let q = self.path.join("cpu.cfs_quota_us");
        let quota = parse_long_trimmed(&read_string(&q)?, &q);
        let p = self.path.join("cpu.cfs_period_us");
        let period = parse_long_trimmed(&read_string(&p)?, &p);
        Ok(java_long_div(quota, period))
    }
}

/// The cgroups v2 memory controller.
// Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v2/UnifiedMemory.java:24 UnifiedMemory
pub struct UnifiedMemory {
    path: PathBuf,
}

impl UnifiedMemory {
    pub fn new(path: PathBuf) -> UnifiedMemory {
        UnifiedMemory { path }
    }
}

impl Controller for UnifiedMemory {
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v2/UnifiedController.java:31 UnifiedController.isLegacy
    fn is_legacy(&self) -> bool {
        false
    }
    fn get_path(&self) -> &Path {
        &self.path
    }
}

impl MemoryController for UnifiedMemory {
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v2/UnifiedMemory.java:37 UnifiedMemory.child
    fn child(&self, name: &str) -> io::Result<Arc<dyn MemoryController>> {
        Ok(Arc::new(UnifiedMemory::new(unified_get_child(
            &self.path, name,
        )?)))
    }
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v2/UnifiedMemory.java:42 UnifiedMemory.setMaxBytes
    fn set_max_bytes(&self, bytes: i64) -> io::Result<()> {
        write_string(&self.path.join("memory.max"), &bytes.to_string())?;
        write_string(&self.path.join("memory.swap.max"), "0")
    }
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v2/UnifiedMemory.java:48 UnifiedMemory.getMaxBytes
    fn get_max_bytes(&self) -> io::Result<i64> {
        let p = self.path.join("memory.max");
        Ok(parse_long_trimmed(&read_string(&p)?, &p))
    }
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v2/UnifiedMemory.java:53 UnifiedMemory.getUsageInBytes
    fn get_usage_in_bytes(&self) -> io::Result<i64> {
        let p = self.path.join("memory.current");
        Ok(parse_long_trimmed(&read_string(&p)?, &p))
    }
}

/// The cgroups v2 cpu controller.
// Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v2/UnifiedCpu.java:25 UnifiedCpu
pub struct UnifiedCpu {
    path: PathBuf,
}

impl UnifiedCpu {
    pub fn new(path: PathBuf) -> UnifiedCpu {
        UnifiedCpu { path }
    }
}

impl Controller for UnifiedCpu {
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v2/UnifiedController.java:31 UnifiedController.isLegacy
    fn is_legacy(&self) -> bool {
        false
    }
    fn get_path(&self) -> &Path {
        &self.path
    }
}

impl CpuController for UnifiedCpu {
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v2/UnifiedCpu.java:33 UnifiedCpu.child
    fn child(&self, name: &str) -> io::Result<Arc<dyn CpuController>> {
        Ok(Arc::new(UnifiedCpu::new(unified_get_child(
            &self.path, name,
        )?)))
    }

    /// `cpu.max` is `"<quota> <period>"`: the period is the second token.
    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v2/UnifiedCpu.java:43 UnifiedCpu.setCpus
    fn set_cpus(&self, cpus: f64) -> io::Result<()> {
        let p = self.path.join("cpu.max");
        let content = read_string(&p)?;
        // scanner.skip("\\S+\\s").nextInt()
        let period: i64 = content
            .split_ascii_whitespace()
            .nth(1)
            .and_then(|t| t.parse::<i32>().ok())
            .unwrap_or_else(|| panic!("java.util.InputMismatchException (in {})", p.display()))
            as i64;
        let quota = java_math_round(period as f64 * cpus);
        write_string(&p, &format!("{quota} {period}"))
    }

    // Java: com/google/devtools/build/lib/sandbox/cgroups/controller/v2/UnifiedCpu.java:53 UnifiedCpu.getCpus
    fn get_cpus(&self) -> io::Result<i64> {
        let p = self.path.join("cpu.max");
        let content = read_string(&p)?;
        let mut tokens = content.split_ascii_whitespace();
        let Some(quota) = tokens.next().and_then(|t| t.parse::<i64>().ok()) else {
            // "max": no limit.
            return Ok(std::thread::available_parallelism().map_or(1, |n| n.get() as i64));
        };
        let period: i64 = tokens
            .next()
            .and_then(|t| t.parse().ok())
            .unwrap_or_else(|| panic!("java.util.NoSuchElementException (in {})", p.display()));
        Ok(java_long_div(quota, period))
    }
}

/// `Math.round(double)`: rounds half up; NaN is 0; saturates.
fn java_math_round(a: f64) -> i64 {
    if a.is_nan() {
        return 0;
    }
    (a + 0.5).floor() as i64
}

/// Java's `long / long`: an `ArithmeticException` on a zero divisor (a crash).
fn java_long_div(a: i64, b: i64) -> i64 {
    if b == 0 {
        panic!("java.lang.ArithmeticException: / by zero");
    }
    a.wrapping_div(b)
}
