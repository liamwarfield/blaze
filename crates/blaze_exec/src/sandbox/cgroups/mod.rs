//! Port of `lib/sandbox/Cgroup.java` and `lib/sandbox/cgroups/` (the cgroups implementation of
//! `--incompatible_use_new_cgroup_implementation`, Bazel's default): `VirtualCgroup`,
//! `VirtualCgroupFactory`, `Hierarchy`, `Mount` and the controllers, plus the `CgroupsInfo`
//! message of `cgroups_info.proto` (kept as a plain struct: nothing outside Google reads it).
//!
//! Paths are `java.nio.file.Path`s / `java.io.File`s of the cgroup file system: platform paths.
//! The logging is skipped (`logger.atInfo()` goes to the server's java.log, not the console).

pub mod controller;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use controller::{
    CpuController, LegacyCpu, LegacyMemory, MemoryController, UnifiedCpu, UnifiedMemory, access,
};

/// An interface for interacting with a cgroup.
// Java: com/google/devtools/build/lib/sandbox/Cgroup.java:20 Cgroup
pub trait Cgroup: Send + Sync + std::fmt::Debug {
    fn get_memory_usage_in_kb(&self) -> i32;
    fn exists(&self) -> bool;
    fn add_process(&self, pid: i64) -> io::Result<()>;
    fn paths(&self) -> Vec<PathBuf>;
    fn destroy(&self);
}

/// `CgroupControllerInfo.Version`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CgroupControllerVersion {
    Unknown,
    V1,
    V2,
}

/// `cgroups_info.CgroupControllerInfo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CgroupControllerInfo {
    pub path: String,
    pub is_writable: bool,
    pub version: CgroupControllerVersion,
}

/// `cgroups_info.CgroupsInfo`: all cgroup controllers found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CgroupsInfo {
    pub cgroup_controllers: Vec<CgroupControllerInfo>,
}

/// `Files.readLines(file, UTF_8)`: malformed input is replaced, as Guava's reader does.
fn read_lines(file: &Path) -> io::Result<Vec<String>> {
    let bytes = std::fs::read(file)?;
    let text = String::from_utf8_lossy(&bytes);
    // BufferedReader.readLine: \n, \r or \r\n end a line; text after the last terminator is
    // a last line, an empty one is not returned.
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => lines.push(std::mem::take(&mut current)),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                lines.push(std::mem::take(&mut current));
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    Ok(lines)
}

/// `String.split(",")`: trailing empty strings are removed (an input without a match is
/// returned whole, even when empty).
fn java_split_comma(s: &str) -> Vec<String> {
    if s.is_empty() {
        return vec![String::new()];
    }
    let mut parts: Vec<String> = s.split(',').map(str::to_string).collect();
    while parts.len() > 1 && parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    if parts.len() == 1 && parts[0].is_empty() {
        // ",,,".split(",") is an empty array.
        return Vec::new();
    }
    parts
}

/// A line of `/proc/self/mounts` that mounts a cgroup file system.
// Java: com/google/devtools/build/lib/sandbox/cgroups/Mount.java:29 Mount
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub path: PathBuf,
    pub type_: String,
    pub opts: Vec<String>,
}

impl Mount {
    // Java: com/google/devtools/build/lib/sandbox/cgroups/Mount.java:41 Mount.isV2
    pub fn is_v2(&self) -> bool {
        self.type_ == "cgroup2"
    }

    /// The cgroup mounts of `proc_mounts`, matched by
    /// `^[^\s#]\S*\s+(?<file>\S*)\s+(?<vfstype>cgroup2?)\s+(?<mntops>\S*).*`.
    // Java: com/google/devtools/build/lib/sandbox/cgroups/Mount.java:49 Mount.parse
    pub fn parse(proc_mounts: &Path) -> io::Result<Vec<Mount>> {
        Ok(read_lines(proc_mounts)?
            .iter()
            .filter_map(|line| Mount::parse_line(line))
            .collect())
    }

    fn parse_line(line: &str) -> Option<Mount> {
        // Java's `\s` and `\S` are ASCII classes; `.` stops at the line terminators. With the
        // whole line anchored (`Matcher.matches`), the leftmost-first captures of `regex` are
        // the ones Java's backtracking matcher returns (an empty `file` group included).
        static CGROUPS_MOUNT_PATTERN: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(concat!(
                r"^[^ \t\n\x0B\x0C\r#][^ \t\n\x0B\x0C\r]*[ \t\n\x0B\x0C\r]+",
                r"(?P<file>[^ \t\n\x0B\x0C\r]*)[ \t\n\x0B\x0C\r]+",
                r"(?P<vfstype>cgroup2?)[ \t\n\x0B\x0C\r]+",
                r"(?P<mntops>[^ \t\n\x0B\x0C\r]*)[^\n\r\x{85}\x{2028}\x{2029}]*$",
            ))
            .expect("CGROUPS_MOUNT_PATTERN")
        });
        let m = CGROUPS_MOUNT_PATTERN.captures(line)?;
        Some(Mount {
            path: PathBuf::from(&m["file"]),
            type_: m["vfstype"].to_string(),
            opts: java_split_comma(&m["mntops"]),
        })
    }
}

/// A line of `/proc/self/cgroup`.
// Java: com/google/devtools/build/lib/sandbox/cgroups/Hierarchy.java:30 Hierarchy
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hierarchy {
    pub id: i32,
    pub controllers: Vec<String>,
    pub path: PathBuf,
}

impl Hierarchy {
    // Java: com/google/devtools/build/lib/sandbox/cgroups/Hierarchy.java:38 Hierarchy.isV2
    pub fn is_v2(&self) -> bool {
        self.controllers.len() == 1 && self.controllers[0].is_empty() && self.id == 0
    }

    /// The hierarchies of `proc_cgroup`, matched by `^(?<id>\d+):(?<controllers>[^:]*):(?<file>.+)`.
    // Java: com/google/devtools/build/lib/sandbox/cgroups/Hierarchy.java:49 Hierarchy.parse
    pub fn parse(proc_cgroup: &Path) -> io::Result<Vec<Hierarchy>> {
        let mut hierarchies = Vec::new();
        for line in read_lines(proc_cgroup)? {
            let mut parts = line.splitn(3, ':');
            let (Some(id), Some(controllers), Some(file)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) || file.is_empty() {
                continue;
            }
            let id: i32 = id.parse().unwrap_or_else(|_| {
                panic!("java.lang.NumberFormatException: For input string: \"{id}\"")
            });
            hierarchies.push(Hierarchy {
                id,
                controllers: java_split_comma(controllers),
                path: PathBuf::from(file),
            });
        }
        Ok(hierarchies)
    }
}

/// `mountPath.resolve(Path.of("/").relativize(hierarchyPath))`.
fn resolve_hierarchy(mount_path: &Path, hierarchy_path: &Path) -> PathBuf {
    let relative = hierarchy_path.strip_prefix("/").unwrap_or(hierarchy_path);
    if relative.as_os_str().is_empty() {
        mount_path.to_path_buf()
    } else {
        mount_path.join(relative)
    }
}

/// A cgroup with its memory and cpu controllers (either may be absent).
// Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:47 VirtualCgroup
pub struct VirtualCgroup {
    cpu: Option<Arc<dyn CpuController>>,
    memory: Option<Arc<dyn MemoryController>>,
    paths: IndexSet<PathBuf>,
    cgroups_info: Option<CgroupsInfo>,
    children: Mutex<Vec<Arc<VirtualCgroup>>>,
}

impl std::fmt::Debug for VirtualCgroup {
    // AutoValue's toString lists the properties.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualCgroup")
            .field(
                "cpu",
                &self.cpu.as_ref().map(|c| c.get_path().to_path_buf()),
            )
            .field(
                "memory",
                &self.memory.as_ref().map(|m| m.get_path().to_path_buf()),
            )
            .field("paths", &self.paths)
            .field("cgroupsInfo", &self.cgroups_info)
            .finish()
    }
}

/// `VirtualCgroup.getInstance()`, memoized.
static INSTANCE: LazyLock<Arc<VirtualCgroup>> = LazyLock::new(VirtualCgroup::create_instance);

impl VirtualCgroup {
    /// `VirtualCgroup.NULL`: no controllers, no paths.
    pub fn null() -> Arc<VirtualCgroup> {
        Arc::new(VirtualCgroup::create(None, None, IndexSet::new(), None))
    }

    /// `VirtualCgroup.create(cpu, memory, paths, cgroupsInfo)`.
    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:221 VirtualCgroup.create
    pub fn create(
        cpu: Option<Arc<dyn CpuController>>,
        memory: Option<Arc<dyn MemoryController>>,
        paths: IndexSet<PathBuf>,
        cgroups_info: Option<CgroupsInfo>,
    ) -> VirtualCgroup {
        VirtualCgroup {
            cpu,
            memory,
            paths,
            cgroups_info,
            children: Mutex::new(Vec::new()),
        }
    }

    pub fn cpu(&self) -> Option<&Arc<dyn CpuController>> {
        self.cpu.as_ref()
    }

    pub fn memory(&self) -> Option<&Arc<dyn MemoryController>> {
        self.memory.as_ref()
    }

    pub fn cgroups_info(&self) -> Option<&CgroupsInfo> {
        self.cgroups_info.as_ref()
    }

    /// The cgroup of this server's spawns: `blaze_<pid>_spawns.slice` beside the server's own
    /// cgroup, deleted at exit.
    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:69 VirtualCgroup.getInstance
    pub fn get_instance() -> Arc<VirtualCgroup> {
        INSTANCE.clone()
    }

    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:73 VirtualCgroup.createInstance
    fn create_instance() -> Arc<VirtualCgroup> {
        let result = (|| -> io::Result<Arc<VirtualCgroup>> {
            let cgroup_root = VirtualCgroup::create_root()?;
            let instance = cgroup_root.create_child_with_info(
                &format!("blaze_{}_spawns.slice", std::process::id()),
                cgroup_root.cgroups_info.clone(),
            )?;
            crate::worker::shutdown_hooks::add_shutdown_hook(Arc::new(
                VirtualCgroup::delete_instance,
            ));
            Ok(instance)
        })();
        // Failed to create root cgroup (logged).
        result.unwrap_or_else(|_| VirtualCgroup::null())
    }

    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:91 VirtualCgroup.deleteInstance
    pub fn delete_instance() {
        VirtualCgroup::get_instance().destroy();
    }

    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:100 VirtualCgroup.createRoot
    pub fn create_root() -> io::Result<VirtualCgroup> {
        VirtualCgroup::create_root_from_files(
            Path::new("/proc/self/mounts"),
            Path::new("/proc/self/cgroup"),
        )
    }

    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:104 VirtualCgroup.createRoot
    pub fn create_root_from_files(
        proc_mounts: &Path,
        proc_cgroup: &Path,
    ) -> io::Result<VirtualCgroup> {
        VirtualCgroup::create_root_from(
            &Mount::parse(proc_mounts)?,
            &Hierarchy::parse(proc_cgroup)?,
        )
    }

    /// The root from the mounts and the hierarchies (`ImmutableMap` of controller name to
    /// hierarchy: a controller listed twice is an `IllegalArgumentException`).
    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:109 VirtualCgroup.createRoot
    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:118 VirtualCgroup.createRoot
    pub fn create_root_from(
        mounts: &[Mount],
        hierarchies: &[Hierarchy],
    ) -> io::Result<VirtualCgroup> {
        let mut by_controller: IndexMap<&str, &Hierarchy> = IndexMap::new();
        for h in hierarchies {
            for c in &h.controllers {
                if by_controller.insert(c.as_str(), h).is_some() {
                    panic!(
                        "java.lang.IllegalArgumentException: Multiple entries with same key: {c}"
                    );
                }
            }
        }
        let mut memory: Option<Arc<dyn MemoryController>> = None;
        let mut cpu: Option<Arc<dyn CpuController>> = None;
        let mut paths: IndexSet<PathBuf> = IndexSet::new();
        let mut cgroups_info = CgroupsInfo::default();
        for m in mounts {
            if memory.is_some() && cpu.is_some() {
                break;
            }
            if m.is_v2() {
                let Some(h) = by_controller.get("") else {
                    continue;
                };
                let mut cgroup = resolve_hierarchy(&m.path, &h.path);
                // Found cgroup v2 (logged).
                if cgroup != m.path {
                    // Due to the no internal processes rule, the parent is used instead.
                    cgroup = cgroup.parent().map(Path::to_path_buf).unwrap_or(cgroup);
                }
                let is_cgroup_writable = access(&cgroup, libc::W_OK);
                cgroups_info.cgroup_controllers.push(CgroupControllerInfo {
                    path: cgroup.display().to_string(),
                    is_writable: is_cgroup_writable,
                    version: CgroupControllerVersion::V2,
                });
                if !is_cgroup_writable {
                    // Found non-writable cgroup v2 (logged).
                    continue;
                }
                paths.insert(cgroup.clone());
                // new Scanner(cgroup.controllers): whitespace-separated tokens.
                let controllers = std::fs::read(cgroup.join("cgroup.controllers"))?;
                let controllers = String::from_utf8_lossy(&controllers);
                for c in controllers.split_ascii_whitespace() {
                    match c {
                        "memory" => {
                            if memory.is_none() {
                                memory = Some(Arc::new(UnifiedMemory::new(cgroup.clone())));
                            }
                        }
                        "cpu" => {
                            if cpu.is_none() {
                                cpu = Some(Arc::new(UnifiedCpu::new(cgroup.clone())));
                            }
                        }
                        _ => {} // Ignore all other controllers.
                    }
                }
            } else {
                for opt in &m.opts {
                    let Some(h) = by_controller.get(opt.as_str()) else {
                        continue;
                    };
                    let cgroup = resolve_hierarchy(&m.path, &h.path);
                    let is_cgroup_writable = access(&cgroup, libc::W_OK);
                    cgroups_info.cgroup_controllers.push(CgroupControllerInfo {
                        path: cgroup.display().to_string(),
                        is_writable: is_cgroup_writable,
                        version: CgroupControllerVersion::V1,
                    });
                    if !is_cgroup_writable {
                        // Found non-writable cgroup v1 (logged).
                        continue;
                    }
                    paths.insert(cgroup.clone());
                    match opt.as_str() {
                        "memory" => {
                            if memory.is_none() {
                                memory = Some(Arc::new(LegacyMemory::new(cgroup.clone())));
                            }
                        }
                        "cpu" => {
                            if cpu.is_none() {
                                cpu = Some(Arc::new(LegacyCpu::new(cgroup.clone())));
                            }
                        }
                        _ => {} // Ignore all other controllers.
                    }
                }
            }
        }
        Ok(VirtualCgroup::create(
            cpu,
            memory,
            paths,
            Some(cgroups_info),
        ))
    }

    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:248 VirtualCgroup.createChild
    pub fn create_child(&self, name: &str) -> io::Result<Arc<VirtualCgroup>> {
        self.create_child_with_info(name, None)
    }

    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:252 VirtualCgroup.createChild
    fn create_child_with_info(
        &self,
        name: &str,
        cgroups_info: Option<CgroupsInfo>,
    ) -> io::Result<Arc<VirtualCgroup>> {
        let mut paths = IndexSet::new();
        let memory = match &self.memory {
            Some(m) => {
                let child = m.child(name)?;
                paths.insert(child.get_path().to_path_buf());
                Some(child)
            }
            None => None,
        };
        let cpu = match &self.cpu {
            Some(c) => {
                let child = c.child(name)?;
                paths.insert(child.get_path().to_path_buf());
                Some(child)
            }
            None => None,
        };
        let child = Arc::new(VirtualCgroup::create(cpu, memory, paths, cgroups_info));
        self.children.lock().push(child.clone());
        Ok(child)
    }
}

impl Cgroup for VirtualCgroup {
    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:275 VirtualCgroup.getMemoryUsageInKb
    fn get_memory_usage_in_kb(&self) -> i32 {
        match &self.memory {
            None => 0,
            Some(m) => m.get_usage_in_bytes().map_or(0, |b| (b / 1024) as i32),
        }
    }

    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:284 VirtualCgroup.exists
    fn exists(&self) -> bool {
        self.memory.as_ref().is_some_and(|m| m.exists())
    }

    /// Writes the pid into `cgroup.procs` of each path.
    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:265 VirtualCgroup.addProcess
    fn add_process(&self, pid: i64) -> io::Result<()> {
        for path in &self.paths {
            std::fs::write(path.join("cgroup.procs"), pid.to_string())?;
        }
        Ok(())
    }

    fn paths(&self) -> Vec<PathBuf> {
        self.paths.iter().cloned().collect()
    }

    /// Destroys the children, then removes this cgroup's (empty) directories (`File.delete`,
    /// failures ignored).
    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroup.java:242 VirtualCgroup.destroy
    fn destroy(&self) {
        let children: Vec<Arc<VirtualCgroup>> = self.children.lock().clone();
        for child in children {
            child.destroy();
        }
        for path in &self.paths {
            if path.exists() {
                let _ = std::fs::remove_dir(path);
            }
        }
    }
}

/// Creates the cgroups of one kind of spawn (`sandbox_`, `worker_`) as children of a root.
// Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroupFactory.java:29 VirtualCgroupFactory
pub struct VirtualCgroupFactory {
    name: String,
    default_limits: IndexMap<String, f64>,
    root: Arc<VirtualCgroup>,
    cgroups: Mutex<IndexMap<i32, Arc<VirtualCgroup>>>,
    always_create: bool,
}

impl VirtualCgroupFactory {
    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroupFactory.java:41 VirtualCgroupFactory.<init>
    pub fn new(
        name: &str,
        root: Arc<VirtualCgroup>,
        default_limits: IndexMap<String, f64>,
        always_create: bool,
    ) -> VirtualCgroupFactory {
        VirtualCgroupFactory {
            name: name.to_string(),
            default_limits,
            root,
            cgroups: Mutex::new(IndexMap::new()),
            always_create,
        }
    }

    /// `VirtualCgroupFactory.NOOP`.
    pub fn noop() -> VirtualCgroupFactory {
        VirtualCgroupFactory::new("noop_", VirtualCgroup::null(), IndexMap::new(), false)
    }

    pub fn always_create(&self) -> bool {
        self.always_create
    }

    pub fn default_limits(&self) -> &IndexMap<String, f64> {
        &self.default_limits
    }

    /// The cgroup for spawn or worker `id` with `limits` (`cpu` in cpus, `memory` in MB); NULL
    /// when nothing would be limited and the factory does not always create one.
    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroupFactory.java:57 VirtualCgroupFactory.create
    pub fn create(
        &self,
        id: i32,
        limits: &IndexMap<String, f64>,
    ) -> io::Result<Arc<VirtualCgroup>> {
        if !self.always_create && self.default_limits.is_empty() && limits.is_empty() {
            return Ok(VirtualCgroup::null());
        }
        let cpu_limit = limits
            .get("cpu")
            .copied()
            .unwrap_or_else(|| self.default_limits.get("cpu").copied().unwrap_or(0.0));
        let memory_limit = (limits
            .get("memory")
            .copied()
            .unwrap_or_else(|| self.default_limits.get("memory").copied().unwrap_or(0.0))
            * 1024.0
            * 1024.0) as i64;
        if !self.always_create && cpu_limit == 0.0 && memory_limit == 0 {
            return Ok(VirtualCgroup::null());
        }
        let child_name = format!("{}{}.scope", self.name, id);
        let cgroup = self.root.create_child(&child_name)?;
        // Created child cgroup (logged).
        self.cgroups.lock().insert(id, cgroup.clone());
        if memory_limit > 0
            && let Some(memory) = cgroup.memory()
        {
            memory.set_max_bytes(memory_limit)?;
            // Successfully set / Failed to set the memory limit (logged): getMaxBytes is read.
            let _ = memory.get_max_bytes()?;
        }
        if cpu_limit > 0.0
            && let Some(cpu) = cgroup.cpu()
        {
            cpu.set_cpus(cpu_limit)?;
            let _ = cpu.get_cpus()?;
        }
        Ok(cgroup)
    }

    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroupFactory.java:95 VirtualCgroupFactory.get
    pub fn get(&self, id: i32) -> Option<Arc<VirtualCgroup>> {
        self.cgroups.lock().get(&id).cloned()
    }

    // Java: com/google/devtools/build/lib/sandbox/cgroups/VirtualCgroupFactory.java:100 VirtualCgroupFactory.remove
    pub fn remove(&self, id: i32) -> Option<Arc<VirtualCgroup>> {
        let cgroup = self.cgroups.lock().shift_remove(&id);
        if let Some(c) = &cgroup {
            c.destroy();
        }
        cgroup
    }
}

#[cfg(test)]
mod tests;
