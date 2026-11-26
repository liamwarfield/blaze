//! Ports of src/test/java/com/google/devtools/build/lib/sandbox/cgroups/: CpuTest, MemoryTest,
//! HierarchyTest, MountTest, VirtualCgroupFactoryTest and VirtualCgroupTest.

use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use super::controller::{
    CpuController, LegacyCpu, LegacyMemory, MemoryController, UnifiedCpu, UnifiedMemory,
};
use super::*;

/// `FsApparatus.newNative()`: files under a temporary directory; `file(path, lines...)` writes
/// each line followed by a newline.
struct Scratch {
    dir: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Scratch {
        Scratch {
            dir: tempfile::tempdir().unwrap(),
        }
    }
    fn path(&self, p: &str) -> PathBuf {
        self.dir.path().join(p)
    }
    fn file(&self, p: &str, lines: &[&str]) -> PathBuf {
        let path = self.path(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut content = String::new();
        for l in lines {
            content.push_str(l);
            content.push('\n');
        }
        std::fs::write(&path, content).unwrap();
        path
    }
    fn dir(&self, p: &str) -> PathBuf {
        let path = self.path(p);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    fn root(&self) -> String {
        self.dir.path().display().to_string()
    }
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

// CpuTest

#[test]
fn set_cpu_limit_v1() {
    let s = Scratch::new();
    let quota = s.file("cgroup/cpu/cpu.cfs_quota_us", &["-1"]);
    s.file("cgroup/cpu/cpu.cfs_period_us", &["1000"]);
    let cpu = LegacyCpu::new(s.path("cgroup/cpu"));
    cpu.set_cpus(3.0).unwrap();
    assert_eq!(read(&quota), "3000");
}

#[test]
fn get_cpu_limit_v1() {
    let s = Scratch::new();
    s.file("cgroup/cpu/cpu.cfs_quota_us", &["4000"]);
    s.file("cgroup/cpu/cpu.cfs_period_us", &["1000"]);
    assert_eq!(LegacyCpu::new(s.path("cgroup/cpu")).get_cpus().unwrap(), 4);
}

#[test]
fn set_cpu_limit_v2() {
    let s = Scratch::new();
    let limit = s.path("cgroup/cpu/cpu.max");
    s.dir("cgroup/cpu");
    std::fs::write(&limit, "-1 100000").unwrap();
    UnifiedCpu::new(s.path("cgroup/cpu")).set_cpus(5.0).unwrap();
    assert_eq!(read(&limit), "500000 100000");
}

#[test]
fn set_cpu_limit_new_line_v2() {
    let s = Scratch::new();
    let limit = s.file("cgroup/cpu/cpu.max", &["-1 100000"]);
    UnifiedCpu::new(s.path("cgroup/cpu")).set_cpus(5.0).unwrap();
    assert_eq!(read(&limit), "500000 100000");
}

#[test]
fn get_cpu_limit_v2() {
    let s = Scratch::new();
    s.dir("cgroup/cpu");
    std::fs::write(s.path("cgroup/cpu/cpu.max"), "6000 1000").unwrap();
    assert_eq!(UnifiedCpu::new(s.path("cgroup/cpu")).get_cpus().unwrap(), 6);
}

#[test]
fn get_cpu_limit_new_line_v2() {
    let s = Scratch::new();
    s.file("cgroup/cpu/cpu.max", &["6000 1000"]);
    assert_eq!(UnifiedCpu::new(s.path("cgroup/cpu")).get_cpus().unwrap(), 6);
}

#[test]
fn get_cpu_limit_max_v2() {
    let s = Scratch::new();
    s.file("cgroup/cpu/cpu.max", &["max 1000"]);
    assert_eq!(
        UnifiedCpu::new(s.path("cgroup/cpu")).get_cpus().unwrap(),
        std::thread::available_parallelism().unwrap().get() as i64
    );
}

// MemoryTest

#[test]
fn set_memory_limit_v1() {
    let s = Scratch::new();
    let limit = s.file("cgroup/memory/memory.limit_in_bytes", &["0"]);
    LegacyMemory::new(s.path("cgroup/memory"))
        .set_max_bytes(1000)
        .unwrap();
    assert_eq!(read(&limit), "1000");
}

#[test]
fn get_memory_limit_v1() {
    let s = Scratch::new();
    s.file("cgroup/memory/memory.limit_in_bytes", &["100"]);
    assert_eq!(
        LegacyMemory::new(s.path("cgroup/memory"))
            .get_max_bytes()
            .unwrap(),
        100
    );
}

#[test]
fn get_memory_usage_v1() {
    let s = Scratch::new();
    s.file("cgroup/memory/memory.usage_in_bytes", &["2000"]);
    assert_eq!(
        LegacyMemory::new(s.path("cgroup/memory"))
            .get_usage_in_bytes()
            .unwrap(),
        2000
    );
}

#[test]
fn set_memory_limit_v2() {
    let s = Scratch::new();
    let limit = s.file("cgroup/memory/memory.max", &["0"]);
    let swap = s.file("cgroup/memory/memory.swap.max", &["0"]);
    UnifiedMemory::new(s.path("cgroup/memory"))
        .set_max_bytes(1000)
        .unwrap();
    assert_eq!(read(&limit), "1000");
    assert_eq!(read(&swap), "0");
}

#[test]
fn get_memory_limit_v2() {
    let s = Scratch::new();
    s.file("cgroup/memory/memory.max", &["100"]);
    assert_eq!(
        UnifiedMemory::new(s.path("cgroup/memory"))
            .get_max_bytes()
            .unwrap(),
        100
    );
}

#[test]
fn get_memory_usage_v2() {
    let s = Scratch::new();
    s.file("cgroup/memory/memory.current", &["2000"]);
    assert_eq!(
        UnifiedMemory::new(s.path("cgroup/memory"))
            .get_usage_in_bytes()
            .unwrap(),
        2000
    );
}

// HierarchyTest

#[test]
fn hierarchy_test_parse() {
    let s = Scratch::new();
    let f = s.file(
        "proc/self/cgroup",
        &[
            "1:cpu,cpuacct:/user.slice",
            "0::/user.slice/session-1.scope",
        ],
    );
    let h = Hierarchy::parse(&f).unwrap();
    assert_eq!(h.len(), 2);
    assert!(!h[0].is_v2());
    assert!(h[1].is_v2());
    assert_eq!(h[0].controllers, vec!["cpu", "cpuacct"]);
    assert_eq!(h[1].controllers, vec![""]);
    assert_eq!(h[0].path, PathBuf::from("/user.slice"));
    assert_eq!(h[1].path, PathBuf::from("/user.slice/session-1.scope"));
}

// MountTest

#[test]
fn mount_test_parse() {
    let s = Scratch::new();
    let f = s.file(
        "proc/self/mounts",
        &[
            "sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0",
            "cgroup /dev/cgroup/cpu cgroup rw,cpu,cpuacct 0 0",
            "cgroup /dev/cgroup/unified cgroup2 ro 0 0",
            "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0",
        ],
    );
    let m = Mount::parse(&f).unwrap();
    assert_eq!(m.len(), 2);
    assert!(!m[0].is_v2());
    assert!(m[1].is_v2());
    assert!(m[0].opts.contains(&"cpu".to_string()));
    assert_eq!(m[0].path, PathBuf::from("/dev/cgroup/cpu"));
    assert_eq!(m[1].path, PathBuf::from("/dev/cgroup/unified"));
}

// VirtualCgroupFactoryTest

fn factory_root(s: &Scratch) -> Arc<VirtualCgroup> {
    s.dir("cpu/cpu");
    s.dir("mem/mem");
    let mounts = vec![
        Mount {
            path: s.path("cpu"),
            type_: "cgroup".into(),
            opts: vec!["cpu".into()],
        },
        Mount {
            path: s.path("mem"),
            type_: "cgroup".into(),
            opts: vec!["memory".into()],
        },
    ];
    let hierarchies = vec![
        Hierarchy {
            id: 1,
            controllers: vec!["cpu".into()],
            path: PathBuf::from("/cpu"),
        },
        Hierarchy {
            id: 2,
            controllers: vec!["memory".into()],
            path: PathBuf::from("/mem"),
        },
    ];
    let root = VirtualCgroup::create_root_from(&mounts, &hierarchies).unwrap();
    assert!(root.cpu().is_some());
    assert!(root.memory().is_some());
    Arc::new(root)
}

fn limits(entries: &[(&str, f64)]) -> IndexMap<String, f64> {
    entries.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

#[test]
fn test_create_no_limits() {
    let s = Scratch::new();
    let f = VirtualCgroupFactory::new("nolimits", factory_root(&s), limits(&[]), false);
    assert!(f.create(1, &limits(&[])).unwrap().paths().is_empty());
}

#[test]
fn test_force_create_no_limits() {
    let s = Scratch::new();
    let f = VirtualCgroupFactory::new("nolimits", factory_root(&s), limits(&[]), true);
    let vcg = f.create(1, &limits(&[])).unwrap();
    assert!(!vcg.paths().is_empty());
    assert!(vcg.cpu().is_some());
    assert!(vcg.memory().is_some());
}

#[test]
fn test_create_with_default_limits() {
    let s = Scratch::new();
    let f = VirtualCgroupFactory::new(
        "defaults",
        factory_root(&s),
        limits(&[("memory", 100.0)]),
        false,
    );
    let vcg = f.create(1, &limits(&[])).unwrap();
    assert!(!vcg.paths().is_empty());
    assert!(vcg.cpu().is_some());
    assert_eq!(
        vcg.memory().unwrap().get_max_bytes().unwrap(),
        100 * 1024 * 1024
    );
}

#[test]
fn test_create_with_custom_limits() {
    let s = Scratch::new();
    let root = factory_root(&s);
    s.file("cpu/cpu/custom1.scope/cpu.cfs_period_us", &["1000"]);
    let f = VirtualCgroupFactory::new(
        "custom",
        root,
        limits(&[("memory", 100.0), ("cpu", 1.0)]),
        false,
    );
    let vcg = f.create(1, &limits(&[("memory", 200.0)])).unwrap();
    assert!(!vcg.paths().is_empty());
    assert_eq!(vcg.cpu().unwrap().get_cpus().unwrap(), 1);
    assert_eq!(
        vcg.memory().unwrap().get_max_bytes().unwrap(),
        200 * 1024 * 1024
    );
}

#[test]
fn test_create_null() {
    let f = VirtualCgroupFactory::new(
        "null",
        VirtualCgroup::null(),
        limits(&[("memory", 100.0), ("cpu", 1.0)]),
        false,
    );
    let vcg = f.create(1, &limits(&[])).unwrap();
    assert!(vcg.paths().is_empty());
    assert!(vcg.cpu().is_none());
    assert!(vcg.memory().is_none());
}

#[test]
fn test_get() {
    let s = Scratch::new();
    let f = VirtualCgroupFactory::new("get", factory_root(&s), limits(&[("memory", 100.0)]), false);
    let vcg = f.create(1, &limits(&[])).unwrap();
    assert!(Arc::ptr_eq(&f.get(1).unwrap(), &vcg));
}

#[test]
fn test_remove() {
    let s = Scratch::new();
    let f = VirtualCgroupFactory::new("get", factory_root(&s), limits(&[]), true);
    let vcg = f.create(1, &limits(&[])).unwrap();
    assert!(Arc::ptr_eq(&f.remove(1).unwrap(), &vcg));
    for p in vcg.paths() {
        assert!(!p.exists());
    }
}

// VirtualCgroupTest

fn update_v1(s: &Scratch, p: &str) {
    s.file(&format!("{p}/cgroup.procs"), &[]);
}

fn create_v1(s: &Scratch) -> VirtualCgroup {
    let r = s.root();
    let mounts = s.file(
        "proc/self/mounts",
        &[
            "sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0",
            &format!("cgroup {r}/dev/cgroup/memory cgroup rw,memory,hugetlb 0 0"),
            &format!("cgroup {r}/dev/cgroup/cpuset cgroup rw,cpuset 0 0"),
            &format!("cgroup {r}/dev/cgroup/cpu cgroup rw,cpu,cpuacct 0 0"),
            &format!("cgroup {r}/dev/cgroup/blkio cgroup rw,blkio 0 0"),
            "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0",
        ],
    );
    let hierarchies = s.file(
        "proc/self/cgroup",
        &[
            "4:memory,hugetlb:/user.slice",
            "3:cpuset:/",
            "2:cpu,cpuacct:/user.slice",
            "1:blkio:/user.slice",
        ],
    );
    update_v1(s, "dev/cgroup/memory/user.slice");
    update_v1(s, "dev/cgroup/cpuset");
    update_v1(s, "dev/cgroup/cpu/user.slice");
    update_v1(s, "dev/cgroup/blkio/user.slice");
    VirtualCgroup::create_root_from_files(&mounts, &hierarchies).unwrap()
}

fn update_v2(s: &Scratch, p: &str, controllers: &str) {
    s.file(&format!("{p}/cgroup.procs"), &[]);
    s.file(&format!("{p}/cgroup.controllers"), &[controllers]);
    s.file(&format!("{p}/cgroup.subtree_control"), &[]);
}

fn create_v2(s: &Scratch) -> VirtualCgroup {
    let r = s.root();
    let mounts = s.file(
        "proc/self/mounts",
        &[
            "sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0",
            &format!("cgroup {r}/dev/cgroup/unified cgroup2 ro 0 0"),
            "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0",
        ],
    );
    let hierarchies = s.file("proc/self/cgroup", &["0::/user.slice"]);
    update_v2(s, "dev/cgroup/unified", "memory cpu");
    update_v2(s, "dev/cgroup/unified/user.slice", "memory cpu");
    VirtualCgroup::create_root_from_files(&mounts, &hierarchies).unwrap()
}

fn create_hybrid(s: &Scratch) -> VirtualCgroup {
    let r = s.root();
    let mounts = s.file(
        "proc/self/mounts",
        &[
            "sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0",
            &format!("cgroup {r}/dev/cgroup/cpuset cgroup rw,cpuset 0 0"),
            &format!("cgroup {r}/dev/cgroup/cpu cgroup rw,cpu,cpuacct 0 0"),
            &format!("cgroup {r}/dev/cgroup/blkio cgroup rw,blkio 0 0"),
            &format!("cgroup {r}/dev/cgroup/unified cgroup2 ro 0 0"),
            "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0",
        ],
    );
    let hierarchies = s.file(
        "proc/self/cgroup",
        &[
            "3:cpuset:/",
            "2:cpu,cpuacct:/user.slice",
            "1:blkio:/user.slice",
            "0::/user.slice",
        ],
    );
    update_v1(s, "dev/cgroup/cpuset");
    update_v1(s, "dev/cgroup/cpu/user.slice");
    update_v1(s, "dev/cgroup/blkio/user.slice");
    update_v2(s, "dev/cgroup/unified", "memory pids");
    update_v2(s, "dev/cgroup/unified/user.slice", "memory pids");
    VirtualCgroup::create_root_from_files(&mounts, &hierarchies).unwrap()
}

#[test]
fn test_get_root_cgroup_v1() {
    let s = Scratch::new();
    let vcg = create_v1(&s);
    assert!(vcg.cpu().unwrap().is_legacy());
    assert!(vcg.memory().unwrap().is_legacy());
    assert_eq!(
        vcg.cpu().unwrap().get_path(),
        s.path("dev/cgroup/cpu/user.slice")
    );
    assert_eq!(
        vcg.memory().unwrap().get_path(),
        s.path("dev/cgroup/memory/user.slice")
    );
}

#[test]
fn test_get_root_cgroup_v2() {
    let s = Scratch::new();
    let vcg = create_v2(&s);
    assert!(!vcg.cpu().unwrap().is_legacy());
    assert!(!vcg.memory().unwrap().is_legacy());
    assert_eq!(vcg.cpu().unwrap().get_path(), s.path("dev/cgroup/unified"));
    assert_eq!(
        vcg.memory().unwrap().get_path(),
        s.path("dev/cgroup/unified")
    );
}

#[test]
fn test_get_root_cgroup_mixed() {
    let s = Scratch::new();
    let vcg = create_hybrid(&s);
    assert!(vcg.cpu().unwrap().is_legacy());
    assert!(!vcg.memory().unwrap().is_legacy());
    assert_eq!(
        vcg.cpu().unwrap().get_path(),
        s.path("dev/cgroup/cpu/user.slice")
    );
    assert_eq!(
        vcg.memory().unwrap().get_path(),
        s.path("dev/cgroup/unified")
    );
}

#[test]
fn test_create_child() {
    let s = Scratch::new();
    let vcg = create_hybrid(&s);
    let child = vcg.create_child("foo").unwrap();
    assert_eq!(
        child.memory().unwrap().get_path(),
        s.path("dev/cgroup/unified/foo")
    );
    assert_eq!(
        child.cpu().unwrap().get_path(),
        s.path("dev/cgroup/cpu/user.slice/foo")
    );
    let subtree = vcg
        .memory()
        .unwrap()
        .get_path()
        .join("cgroup.subtree_control");
    assert_eq!(read(&subtree), "+memory +pids ");
}

#[test]
fn test_null_cgroup_creates_null_child() {
    let child = VirtualCgroup::null().create_child("foo").unwrap();
    assert!(child.cpu().is_none());
    assert!(child.memory().is_none());
}

#[test]
fn test_add_process_v1() {
    let s = Scratch::new();
    let vcg = create_v1(&s);
    vcg.add_process(1234).unwrap();
    assert_eq!(
        read(&vcg.cpu().unwrap().get_path().join("cgroup.procs")),
        "1234"
    );
    assert_eq!(
        read(&vcg.memory().unwrap().get_path().join("cgroup.procs")),
        "1234"
    );
}

#[test]
fn test_add_process_v2() {
    let s = Scratch::new();
    let vcg = create_v2(&s);
    vcg.add_process(1234).unwrap();
    assert_eq!(
        read(&vcg.cpu().unwrap().get_path().join("cgroup.procs")),
        "1234"
    );
    assert_eq!(
        read(&vcg.memory().unwrap().get_path().join("cgroup.procs")),
        "1234"
    );
}

#[test]
fn test_cgroup_invalid_mounts() {
    let s = Scratch::new();
    let mounts = s.file("proc/self/mounts", &[]);
    let hierarchies = s.file("proc/self/cgroup", &["0::/user.slice"]);
    let vcg = VirtualCgroup::create_root_from_files(&mounts, &hierarchies).unwrap();
    assert!(vcg.cpu().is_none());
    assert!(vcg.memory().is_none());
}

#[test]
fn test_cgroup_invalid_hierarchies() {
    let s = Scratch::new();
    let r = s.root();
    let mounts = s.file(
        "proc/self/mounts",
        &[&format!("cgroup {r}/dev/cgroup/unified cgroup2 ro 0 0")],
    );
    let hierarchies = s.file("proc/self/cgroup", &[]);
    let vcg = VirtualCgroup::create_root_from_files(&mounts, &hierarchies).unwrap();
    assert!(vcg.cpu().is_none());
    assert!(vcg.memory().is_none());
}

#[test]
fn test_cgroup_only_memory() {
    let s = Scratch::new();
    let r = s.root();
    let mounts = s.file(
        "proc/self/mounts",
        &[
            &format!("cgroup {r}/dev/cgroup/memory cgroup rw,memory,hugetlb 0 0"),
            &format!("cgroup {r}/dev/cgroup/cpuset cgroup rw,cpuset 0 0"),
        ],
    );
    let hierarchies = s.file(
        "proc/self/cgroup",
        &["2:cpuset:/user.slice", "1:memory,hugetlb:/user.slice"],
    );
    s.file("dev/cgroup/memory/user.slice/cgroup.procs", &[]);
    s.file("dev/cgroup/cpuset/user.slice/cgroup.procs", &[]);
    let vcg = VirtualCgroup::create_root_from_files(&mounts, &hierarchies).unwrap();
    assert!(vcg.cpu().is_none());
    assert!(vcg.memory().is_some());
}

#[test]
fn test_cgroup_only_cpu() {
    let s = Scratch::new();
    let r = s.root();
    let mounts = s.file(
        "proc/self/mounts",
        &[
            &format!("cgroup {r}/dev/cgroup/cpu cgroup rw,cpu 0 0"),
            &format!("cgroup {r}/dev/cgroup/cpuset cgroup rw,cpuset 0 0"),
        ],
    );
    let hierarchies = s.file(
        "proc/self/cgroup",
        &["2:cpuset:/user.slice", "1:cpu:/user.slice"],
    );
    s.file("dev/cgroup/cpu/user.slice/cgroup.procs", &[]);
    s.file("dev/cgroup/cpuset/user.slice/cgroup.procs", &[]);
    let vcg = VirtualCgroup::create_root_from_files(&mounts, &hierarchies).unwrap();
    assert!(vcg.cpu().is_some());
    assert!(vcg.memory().is_none());
}

fn info(path: PathBuf, version: CgroupControllerVersion) -> CgroupControllerInfo {
    CgroupControllerInfo {
        path: path.display().to_string(),
        is_writable: true,
        version,
    }
}

#[test]
fn valid_cgroup_v1_cgroups_info_returns_correct_info() {
    let s = Scratch::new();
    let vcg = create_v1(&s);
    let v1 = CgroupControllerVersion::V1;
    assert_eq!(
        vcg.cgroups_info().unwrap(),
        &CgroupsInfo {
            cgroup_controllers: vec![
                info(s.path("dev/cgroup/memory/user.slice"), v1),
                info(s.path("dev/cgroup/memory/user.slice"), v1),
                info(s.path("dev/cgroup/cpuset"), v1),
                info(s.path("dev/cgroup/cpu/user.slice"), v1),
                info(s.path("dev/cgroup/cpu/user.slice"), v1),
            ]
        }
    );
}

#[test]
fn valid_cgroup_v2_cgroups_info_returns_correct_info() {
    let s = Scratch::new();
    let vcg = create_v2(&s);
    assert_eq!(
        vcg.cgroups_info().unwrap(),
        &CgroupsInfo {
            cgroup_controllers: vec![info(
                s.path("dev/cgroup/unified"),
                CgroupControllerVersion::V2
            )]
        }
    );
}
