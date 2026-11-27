//! Port of `lib/sandbox/CgroupsInfo.java`, `CgroupsInfoV1.java` and `CgroupsInfoV2.java`: the old
//! cgroups implementation (`--noincompatible_use_new_cgroup_implementation`), used for
//! `--experimental_sandbox_memory_limit_mb`, `--experimental_worker_use_cgroups_on_linux` and the
//! hardened worker sandbox when the new implementation is off.
//!
//! The Java class hierarchy (`CgroupsInfoV1`, `CgroupsInfoV2`, `InvalidCgroupsInfo`) is one struct
//! with a [`Kind`]. Paths are `java.io.File`s ([`JavaFile`]). The logging is skipped
//! (`logger.atInfo()` goes to the server's java.log, not the console).

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use blaze_shell::JavaFile;
use regex::Regex;

use super::cgroups::Cgroup;
use crate::worker::shutdown_hooks;

/// The version of Cgroups that is currently being used.
// Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:146 CgroupsInfo.Version
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    V1,
    V2,
}

/// The types of cgroups relevant to Blaze:
///
/// - `Root`: corresponds to the root cgroup where the hierarchy is mounted at; one of
///   "/dev/cgroup/{controller}" or "/sys/fs/cgroup".
/// - `BlazeSpawns`: corresponds the overarching cgroup that contains children `Spawn` cgroups.
/// - `Spawn`: corresponds to the cgroup for a single spawn - this could be a locally executed
///   action or a worker process.
// Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:165 CgroupsInfo.Type
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Root,
    BlazeSpawns,
    Spawn,
}

/// The concrete class of a `CgroupsInfo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `CgroupsInfoV1`.
    V1,
    /// `CgroupsInfoV2`.
    V2,
    /// `InvalidCgroupsInfo`, with `getException().toString()`.
    Invalid { exception: String },
}

/// This class manages cgroups directories for memory-limiting sandboxed processes.
// Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:41 CgroupsInfo
#[derive(Clone, Debug)]
pub struct CgroupsInfo {
    kind: Kind,
    version: Option<Version>,
    type_: Type,
    /// This is the directory where the cgroup is in, any related files pertaining to limits /
    /// resource usage or child cgroups (nested directories) are found here.
    cgroup_dir: Option<JavaFile>,
}

/// A regexp that matches cgroups entries in `/proc/mounts`.
///
/// Group 1 is empty (cgroups v1) or '2' (cgroups v2) Group 2 is the mount point. Group 3 is the
/// options, which for v1 includes which hierarchies are mounted here.
///
/// Java's `\s` and `\S` are ASCII classes and `.` stops at line terminators.
// Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:51 CgroupsInfo.CGROUPS_MOUNT_PATTERN
static CGROUPS_MOUNT_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    let s = r"[ \t\n\x0B\f\r]";
    let ns = r"[^ \t\n\x0B\f\r]";
    Regex::new(&format!(
        r"^cgroup(|2){s}+({ns}*){s}+cgroup2?{s}+({ns}*)[^\n\r\u{{85}}\u{{2028}}\u{{2029}}]*$"
    ))
    .unwrap()
});

// Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:54 CgroupsInfo.PROC_SELF_MOUNTS_PATH
const PROC_SELF_MOUNTS_PATH: &str = "/proc/self/mounts";
// Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:55 CgroupsInfo.PROC_SELF_CGROUP_PATH
const PROC_SELF_CGROUP_PATH: &str = "/proc/self/cgroup";

// Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:57 CgroupsInfo.rootCgroup
static ROOT_CGROUP: LazyLock<CgroupsInfo> =
    LazyLock::new(|| CgroupsInfo::get_root_cgroup(&JavaFile::new(PROC_SELF_MOUNTS_PATH)));

// Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:59 CgroupsInfo.blazeSpawnsCgroupSupplier
static BLAZE_SPAWNS_CGROUP: LazyLock<Arc<CgroupsInfo>> =
    LazyLock::new(|| Arc::new(CgroupsInfo::create_blaze_spawns_cgroup_singleton()));

/// `Files.readLines(file, UTF_8)` (Guava): lines split at `\n`, `\r\n` and `\r`, without a
/// trailing empty line.
fn read_lines(file: &JavaFile) -> io::Result<Vec<String>> {
    let bytes = std::fs::read(os_path(file))?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines = Vec::new();
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
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    Ok(lines)
}

/// `Files.asCharSink(file, UTF_8).write(s)`: creates or truncates the file.
fn write(file: &JavaFile, s: &str) -> io::Result<()> {
    std::fs::write(os_path(file), s.as_bytes())
}

/// The path as the JVM passes it to the system.
fn os_path(file: &JavaFile) -> PathBuf {
    PathBuf::from(blaze_util::string_encoding::internal_to_platform(file.get_path()).into_owned())
}

/// `File.isDirectory()`.
fn is_directory(file: &JavaFile) -> bool {
    std::fs::metadata(os_path(file)).is_ok_and(|m| m.is_dir())
}

/// `File.canWrite()`: `access(path, W_OK)`.
fn can_write(file: &JavaFile) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(os_path(file).as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: a valid NUL-terminated path.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// `File.mkdirs()` (the result is ignored by every caller).
fn mkdirs(file: &JavaFile) {
    let _ = std::fs::create_dir_all(os_path(file));
}

/// `File.deleteOnExit()`.
fn delete_on_exit(file: &JavaFile) {
    shutdown_hooks::delete_on_exit(os_path(file));
}

/// `Splitter.on(":").limit(3).splitToList(s).get(2)`, failing as `get(2)` does.
fn third_part(s: &str) -> Result<String, String> {
    let parts: Vec<&str> = s.splitn(3, ':').collect();
    parts.get(2).map(|p| p.to_string()).ok_or_else(|| {
        format!(
            "java.lang.IndexOutOfBoundsException: Index 2 out of bounds for length {}",
            parts.len()
        )
    })
}

/// `Exception.toString()` of an `IOException` from reading or writing `file`.
fn io_exception(file: &JavaFile, e: &io::Error) -> String {
    // The JDK's FileInputStream / FileOutputStream report a failed open as
    // `FileNotFoundException: <path> (<strerror>)`; the reads and writes of these small files
    // fail, if at all, at the open.
    match e.raw_os_error() {
        Some(errno) => format!(
            "java.io.FileNotFoundException: {} ({})",
            file.get_path(),
            strerror(errno)
        ),
        None => format!("java.io.IOException: {e}"),
    }
}

/// `strerror(errno)`: the message the JDK puts in an I/O exception.
fn strerror(errno: i32) -> String {
    // SAFETY: strerror returns a pointer to a NUL-terminated message for any errno.
    unsafe { std::ffi::CStr::from_ptr(libc::strerror(errno)) }
        .to_string_lossy()
        .into_owned()
}

impl CgroupsInfo {
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:179 CgroupsInfo.<init>
    fn new(
        kind: Kind,
        type_: Type,
        version: Option<Version>,
        cgroup_dir: Option<JavaFile>,
    ) -> Self {
        // Valid: "Successfully found / created %s (%s) cgroup at %s" (logged).
        CgroupsInfo {
            kind,
            version,
            type_,
            cgroup_dir,
        }
    }

    /// `new CgroupsInfoV1(type, cgroupDir)`.
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfoV1.java:32 CgroupsInfoV1.<init>
    pub fn new_v1(type_: Type, cgroup_dir: Option<JavaFile>) -> CgroupsInfo {
        CgroupsInfo::new(Kind::V1, type_, Some(Version::V1), cgroup_dir)
    }

    /// `new CgroupsInfoV2(type, cgroupDir)`.
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfoV2.java:33 CgroupsInfoV2.<init>
    pub fn new_v2(type_: Type, cgroup_dir: Option<JavaFile>) -> CgroupsInfo {
        CgroupsInfo::new(Kind::V2, type_, Some(Version::V2), cgroup_dir)
    }

    /// `new InvalidCgroupsInfo(type, version, errorMessage)`: the exception is an
    /// `IllegalStateException` with the message.
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:262 CgroupsInfo.InvalidCgroupsInfo.<init>
    pub fn invalid(type_: Type, version: Option<Version>, error_message: &str) -> CgroupsInfo {
        CgroupsInfo::invalid_with_exception(
            type_,
            version,
            format!("java.lang.IllegalStateException: {error_message}"),
        )
    }

    /// `new InvalidCgroupsInfo(type, version, exception)`; `exception` is its `toString()`.
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:268 CgroupsInfo.InvalidCgroupsInfo.<init>
    pub fn invalid_with_exception(
        type_: Type,
        version: Option<Version>,
        exception: String,
    ) -> CgroupsInfo {
        // "Unable to create cgroup." with the exception (logged).
        CgroupsInfo::new(Kind::Invalid { exception }, type_, version, None)
    }

    /// Returns whether the local machine supports cgroups.
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:63 CgroupsInfo.isSupported
    pub fn is_supported() -> bool {
        // OS.getCurrent() == OS.LINUX: Blaze runs on Linux only.
        CgroupsInfo::get_blaze_spawns_cgroup().can_write()
    }

    /// Returns an instance of the root cgroup of the hierarchy, an invalid one if invalid.
    ///
    /// For v1, we only care about the memory hierarchy.
    ///
    /// `proc_mounts_file`: the /proc/self/mounts file.
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:74 CgroupsInfo.getRootCgroup
    pub fn get_root_cgroup(proc_mounts_file: &JavaFile) -> CgroupsInfo {
        // OS.getCurrent() != OS.LINUX: Blaze runs on Linux only.
        let proc_mounts_contents = match read_lines(proc_mounts_file) {
            Ok(c) => c,
            Err(e) => {
                return CgroupsInfo::invalid_with_exception(
                    Type::Root,
                    None,
                    io_exception(proc_mounts_file, &e),
                );
            }
        };
        let mut v1_root_dir: Option<JavaFile> = None;
        let mut v2_root_dir: Option<JavaFile> = None;
        for s in &proc_mounts_contents {
            if let Some(m) = CGROUPS_MOUNT_PATTERN.captures(s) {
                if m[1].is_empty() {
                    // v1
                    if m[3].contains("memory") {
                        // For now, we only care about the memory cgroup
                        v1_root_dir = Some(JavaFile::new(&m[2]));
                    }
                } else {
                    v2_root_dir = Some(JavaFile::new(&m[2]));
                }
            }
        }
        // If we found the memory controller in v1, we use that, just in case we have a hybrid
        // system where some controllers are v1 and some are v2. It would be harder to detect if
        // v2 has the memory controller
        if let Some(v1_root_dir) = v1_root_dir {
            return CgroupsInfo::new_v1(Type::Root, Some(v1_root_dir));
        }
        if let Some(v2_root_dir) = v2_root_dir {
            return CgroupsInfo::new_v2(Type::Root, Some(v2_root_dir));
        }
        CgroupsInfo::invalid(
            Type::Root,
            None,
            &format!(
                "No cgroups mounted in {}: [{}]",
                proc_mounts_file.get_path(),
                proc_mounts_contents.join(", ")
            ),
        )
    }

    /// Returns the singleton `BlazeSpawns` cgroup created under the root cgroup, an invalid one
    /// if invalid.
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:120 CgroupsInfo.getBlazeSpawnsCgroup
    pub fn get_blaze_spawns_cgroup() -> Arc<CgroupsInfo> {
        BLAZE_SPAWNS_CGROUP.clone()
    }

    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:124 CgroupsInfo.createBlazeSpawnsCgroup
    fn create_blaze_spawns_cgroup_singleton() -> CgroupsInfo {
        if !ROOT_CGROUP.exists() {
            return CgroupsInfo::invalid(
                Type::BlazeSpawns,
                ROOT_CGROUP.get_version(),
                "Root cgroup does not exist.",
            );
        }
        ROOT_CGROUP.create_blaze_spawns_cgroup(PROC_SELF_CGROUP_PATH)
    }

    /// Creates a cgroups directory for Blaze to place spawns in.
    ///
    /// This cgroups directory is created at most once per Blaze instance.
    ///
    /// `proc_self_cgroup_path`: path to the `/proc/self/cgroup` file. Returns a CgroupsInfo
    /// representing the created cgroup that Blaze can use for sub-processes (the Blaze process
    /// itself is not moved into this directory). If unable to create, returns an invalid one
    /// containing the exception.
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:142 CgroupsInfo.createBlazeSpawnsCgroup
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfoV1.java:37 CgroupsInfoV1.createBlazeSpawnsCgroup
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfoV2.java:38 CgroupsInfoV2.createBlazeSpawnsCgroup
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:290 CgroupsInfo.InvalidCgroupsInfo.createBlazeSpawnsCgroup
    pub fn create_blaze_spawns_cgroup(&self, proc_self_cgroup_path: &str) -> CgroupsInfo {
        match self.kind {
            Kind::Invalid { .. } => CgroupsInfo::invalid(
                Type::BlazeSpawns,
                self.get_version(),
                "Unable to create BLAZE_SPAWNS cgroup from an invalid cgroup.",
            ),
            Kind::V1 => {
                check_argument(
                    self.type_ == Type::Root,
                    "Should only be creating the Blaze spawns cgroup from the root cgroup.",
                );
                let blaze_process_cgroup_dir = match v1_get_blaze_process_cgroup_dir(
                    self.cgroup_dir.as_ref(),
                    proc_self_cgroup_path,
                ) {
                    Ok(d) => d,
                    Err(e) => {
                        return CgroupsInfo::invalid_with_exception(
                            Type::BlazeSpawns,
                            self.get_version(),
                            e,
                        );
                    }
                };
                let blaze_spawns_dir = JavaFile::new_child(
                    Some(&blaze_process_cgroup_dir),
                    &format!("blaze_{}_spawns", std::process::id()),
                );
                mkdirs(&blaze_spawns_dir);
                delete_on_exit(&blaze_spawns_dir);
                CgroupsInfo::new_v1(Type::BlazeSpawns, Some(blaze_spawns_dir))
            }
            Kind::V2 => {
                check_argument(
                    self.type_ == Type::Root,
                    "Should only be creating the Blaze spawns cgroup from the root cgroup.",
                );
                let result: Result<JavaFile, String> = (|| {
                    let blaze_process_cgroup_dir = v2_get_blaze_process_cgroup_dir(
                        self.cgroup_dir.as_ref(),
                        proc_self_cgroup_path,
                    )?;
                    // In cgroups v2, we need to step back from the leaf node to make a further
                    // hierarchy.
                    let parent = blaze_process_cgroup_dir
                        .get_parent()
                        .map(|p| JavaFile::new(&p));
                    let blaze_spawns_dir = JavaFile::new_child(
                        parent.as_ref(),
                        &format!("blaze_{}_spawns.slice", std::process::id()),
                    );
                    mkdirs(&blaze_spawns_dir);
                    delete_on_exit(&blaze_spawns_dir);
                    set_subtree_controllers(&blaze_spawns_dir)?;
                    Ok(blaze_spawns_dir)
                })();
                match result {
                    Ok(dir) => CgroupsInfo::new_v2(Type::BlazeSpawns, Some(dir)),
                    Err(e) => CgroupsInfo::invalid_with_exception(
                        Type::BlazeSpawns,
                        self.get_version(),
                        e,
                    ),
                }
            }
        }
    }

    /// Returns whether the cgroup at `cgroup_dir` exists.
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:190 CgroupsInfo.exists
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:276 CgroupsInfo.InvalidCgroupsInfo.exists
    pub fn exists(&self) -> bool {
        if matches!(self.kind, Kind::Invalid { .. }) {
            return false;
        }
        self.cgroup_dir
            .as_ref()
            .is_some_and(|d| d.exists() && is_directory(d))
    }

    /// Returns whether Blaze can write to the current cgroup at `cgroup_dir`.
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:195 CgroupsInfo.canWrite
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:281 CgroupsInfo.InvalidCgroupsInfo.canWrite
    pub fn can_write(&self) -> bool {
        if matches!(self.kind, Kind::Invalid { .. }) {
            return false;
        }
        self.exists() && can_write(self.cgroup_dir.as_ref().unwrap())
    }

    /// A cgroups directory for this Blaze instance to put sandboxes in.
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:200 CgroupsInfo.getCgroupDir
    pub fn get_cgroup_dir(&self) -> Option<&JavaFile> {
        self.cgroup_dir.as_ref()
    }

    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:210 CgroupsInfo.getVersion
    pub fn get_version(&self) -> Option<Version> {
        self.version
    }

    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:214 CgroupsInfo.getType
    pub fn get_type(&self) -> Type {
        self.type_
    }

    /// The concrete class (`getClass()`).
    pub fn get_kind(&self) -> &Kind {
        &self.kind
    }

    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:224 CgroupsInfo.getMemoryUsageInKbFromFile
    pub fn get_memory_usage_in_kb_from_file(&self, filename: &str) -> i32 {
        let file = JavaFile::new_child(self.cgroup_dir.as_ref(), filename);
        match read_lines(&file) {
            Ok(lines) => {
                let Some(val) = lines.first() else {
                    panic!(
                        "java.lang.IndexOutOfBoundsException: Index 0 out of bounds for length 0"
                    );
                };
                let value: i64 = val.parse().unwrap_or_else(|_| {
                    panic!("java.lang.NumberFormatException: For input string: \"{val}\"")
                });
                (value / 1024) as i32
            }
            Err(_) => 0,
        }
    }

    /// Creates a cgroups directory for individual spawns (local / workers).
    ///
    /// Has to be called from a `BlazeSpawns` cgroup.
    ///
    /// `dir_name`: the directory name of the spawn's cgroup. `memory_limit_mb`: memory limit in
    /// Mb to set on the cgroup. If 0, no limit is set. Returns an instance of the spawn's cgroup;
    /// if unable to create, returns an invalid one containing the exception.
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:254 CgroupsInfo.createIndividualSpawnCgroup
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfoV1.java:56 CgroupsInfoV1.createIndividualSpawnCgroup
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfoV2.java:61 CgroupsInfoV2.createIndividualSpawnCgroup
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:298 CgroupsInfo.InvalidCgroupsInfo.createIndividualSpawnCgroup
    pub fn create_individual_spawn_cgroup(
        &self,
        dir_name: &str,
        memory_limit_mb: i32,
    ) -> CgroupsInfo {
        if matches!(self.kind, Kind::Invalid { .. }) {
            return CgroupsInfo::invalid(
                Type::Spawn,
                self.get_version(),
                "Unable to create SPAWN cgroup from an invalid cgroup.",
            );
        }
        check_argument(
            self.type_ == Type::BlazeSpawns,
            "Should only be creating the individual spawn's cgroup from the Blaze spawns cgroup.",
        );
        if !self.can_write() {
            return CgroupsInfo::invalid(
                Type::Spawn,
                self.get_version(),
                &format!(
                    "Cgroup {} is invalid, unable to create spawn's cgroup here.",
                    self.cgroup_dir
                        .as_ref()
                        .map_or("null".to_string(), |d| d.get_path().to_string())
                ),
            );
        }
        let v2 = self.kind == Kind::V2;
        let spawn_cgroup_dir = JavaFile::new_child(
            self.cgroup_dir.as_ref(),
            &if v2 {
                format!("{dir_name}.scope")
            } else {
                dir_name.to_string()
            },
        );
        mkdirs(&spawn_cgroup_dir);
        delete_on_exit(&spawn_cgroup_dir);
        let limit = i64::from(memory_limit_mb) * 1024 * 1024;
        let result: Result<(), String> = (|| {
            if memory_limit_mb > 0 {
                let w = |name: &str, s: &str| {
                    let f = JavaFile::new_child(Some(&spawn_cgroup_dir), name);
                    write(&f, s).map_err(|e| io_exception(&f, &e))
                };
                if v2 {
                    // In cgroups v2, we need to propagate the controllers into new subdirs.
                    w("memory.oom.group", "1\n")?;
                    w("memory.max", &limit.to_string())?;
                    // Set swap to 0 so that it doesn't unexpecedly consume more than the memory
                    // limit when swap is enabled.
                    w("memory.swap.max", &0i64.to_string())?;
                } else {
                    w("memory.limit_in_bytes", &limit.to_string())?;
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            return CgroupsInfo::invalid_with_exception(Type::Spawn, self.get_version(), e);
        }
        if v2 {
            CgroupsInfo::new_v2(Type::Spawn, Some(spawn_cgroup_dir))
        } else {
            CgroupsInfo::new_v1(Type::Spawn, Some(spawn_cgroup_dir))
        }
    }

    /// `InvalidCgroupsInfo.getException()`: its `toString()`; `None` for a valid cgroup.
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:285 CgroupsInfo.InvalidCgroupsInfo.getException
    pub fn get_exception(&self) -> Option<&str> {
        match &self.kind {
            Kind::Invalid { exception } => Some(exception),
            _ => None,
        }
    }

    fn cgroup_dir_or_npe(&self) -> &JavaFile {
        self.cgroup_dir
            .as_ref()
            .expect("java.lang.NullPointerException: getCgroupDir() is null")
    }
}

impl Cgroup for CgroupsInfo {
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:219 CgroupsInfo.getMemoryUsageInKb
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfoV1.java:106 CgroupsInfoV1.getMemoryUsageInKb
    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfoV2.java:91 CgroupsInfoV2.getMemoryUsageInKb
    fn get_memory_usage_in_kb(&self) -> i32 {
        match self.kind {
            Kind::V1 => self.get_memory_usage_in_kb_from_file("memory.usage_in_bytes"),
            Kind::V2 => self.get_memory_usage_in_kb_from_file("memory.current"),
            Kind::Invalid { .. } => 0,
        }
    }

    fn exists(&self) -> bool {
        CgroupsInfo::exists(self)
    }

    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:234 CgroupsInfo.addProcess
    fn add_process(&self, pid: i64) -> io::Result<()> {
        let file = JavaFile::new_child(self.cgroup_dir.as_ref(), "cgroup.procs");
        write(&file, &pid.to_string())
    }

    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:205 CgroupsInfo.paths
    fn paths(&self) -> Vec<PathBuf> {
        vec![os_path(self.cgroup_dir_or_npe())]
    }

    // Java: com/google/devtools/build/lib/sandbox/CgroupsInfo.java:239 CgroupsInfo.destroy
    fn destroy(&self) {
        self.cgroup_dir_or_npe().delete();
    }
}

/// `Preconditions.checkArgument(condition, message)`.
fn check_argument(condition: bool, message: &str) {
    if !condition {
        panic!("java.lang.IllegalArgumentException: {message}");
    }
}

/// Returns the path to the cgroup containing the Blaze process.
///
/// The `/proc/self/cgroup` file look like this in v1:
///
/// ```text
/// 8:net:/some/path
/// 7:memory,hugetlb:/some/other/path
/// ...
/// ```
///
/// The error is the `toString()` of the exception (an `IOException` reading the file, or the
/// `IllegalStateException` / `IndexOutOfBoundsException` of a malformed one).
// Java: com/google/devtools/build/lib/sandbox/CgroupsInfoV1.java:93 CgroupsInfoV1.getBlazeProcessCgroupDir
fn v1_get_blaze_process_cgroup_dir(
    mount_point: Option<&JavaFile>,
    proc_self_cgroup_path: &str,
) -> Result<JavaFile, String> {
    let file = JavaFile::new(proc_self_cgroup_path);
    let controllers = read_lines(&file).map_err(|e| io_exception(&file, &e))?;
    let memory_controller = controllers
        .iter()
        .find(|controller| controller.contains("memory"))
        .ok_or_else(|| {
            format!(
                "java.lang.IllegalStateException: Found no memory cgroup entries in '{proc_self_cgroup_path}'"
            )
        })?;
    Ok(JavaFile::new_child(
        mount_point,
        &third_part(memory_controller)?,
    ))
}

/// Returns the path to the cgroup containing the Blaze process.
///
/// In v2, there is only one entry, and it looks something like this:
///
/// ```text
/// 0::/user.slice/user-123.slice/session-1.scope
/// ```
// Java: com/google/devtools/build/lib/sandbox/CgroupsInfoV2.java:110 CgroupsInfoV2.getBlazeProcessCgroupDir
fn v2_get_blaze_process_cgroup_dir(
    mount_point: Option<&JavaFile>,
    proc_self_cgroup_path: &str,
) -> Result<JavaFile, String> {
    let file = JavaFile::new(proc_self_cgroup_path);
    let contents = read_lines(&file).map_err(|e| io_exception(&file, &e))?;
    let Some(first) = contents.first() else {
        return Err(format!(
            "java.lang.IllegalStateException: Found no memory cgroup entries in '{proc_self_cgroup_path}'"
        ));
    };
    Ok(JavaFile::new_child(mount_point, &third_part(first)?))
}

/// Sets the subtree controllers we need. This also checks that the controllers are available.
///
/// Fails if reading or writing the `cgroup.controllers` or `cgroup.subtree_control` file fails,
/// or if the `memory` and `pids` controllers are either not available or cannot be set for
/// subtrees.
// Java: com/google/devtools/build/lib/sandbox/CgroupsInfoV2.java:131 CgroupsInfoV2.setSubtreeControllers
fn set_subtree_controllers(blaze_dir: &JavaFile) -> Result<(), String> {
    let controllers_file = JavaFile::new_child(Some(blaze_dir), "cgroup.controllers");
    let controllers = read_lines(&controllers_file)
        .map_err(|e| io_exception(&controllers_file, &e))?
        .join(" ");
    if !(controllers.contains("memory") && controllers.contains("pids")) {
        return Err(format!(
            "java.lang.IllegalStateException: Required controllers 'memory' and 'pids' not found in {}/cgroup.controllers",
            blaze_dir.get_path()
        ));
    }
    let subtree_file = JavaFile::new_child(Some(blaze_dir), "cgroup.subtree_control");
    let subtree_controllers = read_lines(&subtree_file)
        .map_err(|e| io_exception(&subtree_file, &e))?
        .join(" ");
    if !subtree_controllers.contains("memory") || !subtree_controllers.contains("pids") {
        write(&subtree_file, "+memory +pids\n").map_err(|e| io_exception(&subtree_file, &e))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "cgroups_info_tests.rs"]
mod tests;
