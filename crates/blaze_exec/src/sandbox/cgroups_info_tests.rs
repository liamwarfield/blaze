//! Port of src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java.

use super::*;

struct Scratch {
    _tmp: tempfile::TempDir,
    /// We use this pseudo-root to get around not being able to replace absolute paths.
    root: String,
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:42 CgroupsInfoTest.setUp
fn set_up() -> Scratch {
    let tmp = tempfile::tempdir().unwrap();
    let root = format!("{}/fake_root", tmp.path().to_str().unwrap());
    std::fs::create_dir_all(&root).unwrap();
    Scratch { _tmp: tmp, root }
}

impl Scratch {
    /// `scratch.dir(path).createDirectoryAndParents()`.
    fn dir(&self, path: &str) {
        std::fs::create_dir_all(path).unwrap();
    }

    /// `scratch.file(path, lines...)`: the lines, each ended by `\n`.
    fn file(&self, path: &str, lines: &[&str]) -> String {
        std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap()).unwrap();
        let mut content = String::new();
        for line in lines {
            content.push_str(line);
            content.push('\n');
        }
        std::fs::write(path, content).unwrap();
        path.to_string()
    }

    // Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:345 CgroupsInfoTest.createFakeAbsoluteFile
    fn create_fake_absolute_file(&self, file_name: &str, contents: &[&str]) -> String {
        self.file(&format!("{}{file_name}", self.root), contents)
    }
}

/// `Files.readLines(new File(path), UTF_8)`.
fn lines_of(path: &str) -> Vec<String> {
    read_lines(&JavaFile::new(path)).unwrap()
}

fn dir_path(cgroup: &CgroupsInfo) -> String {
    cgroup.get_cgroup_dir().unwrap().get_path().to_string()
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:47 CgroupsInfoTest.testGetRootCgroup_v1
#[test]
fn test_get_root_cgroup_v1() {
    let s = set_up();
    let path_string = s.create_fake_absolute_file(
        "/proc/self/mounts",
        &[
            "sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0",
            "cgroup /dev/cgroup/cpu cgroup rw,cpu,cpuacct 0 0",
            "cgroup /dev/cgroup/io cgroup rw,io 0 0",
            "cgroup /dev/cgroup/job cgroup rw,job 0 0",
            "cgroup /dev/cgroup/memory cgroup rw,memory,hugetlb 0 0",
            "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0",
        ],
    );

    let cgroup = CgroupsInfo::get_root_cgroup(&JavaFile::new(&path_string));

    assert_eq!(cgroup.get_type(), Type::Root);
    assert_eq!(cgroup.get_version(), Some(Version::V1));
    assert_eq!(dir_path(&cgroup), "/dev/cgroup/memory");
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:66 CgroupsInfoTest.testGetRootCgroup_v2
#[test]
fn test_get_root_cgroup_v2() {
    let s = set_up();
    let path_string = s.create_fake_absolute_file(
        "/proc/self/mounts",
        &[
            "sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0",
            "cgroup2 /sys/fs/cgroup cgroup2 rw,memory_recursiveprot 0 0",
            "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0",
        ],
    );

    let cgroup = CgroupsInfo::get_root_cgroup(&JavaFile::new(&path_string));

    assert_eq!(cgroup.get_type(), Type::Root);
    assert_eq!(cgroup.get_version(), Some(Version::V2));
    assert_eq!(dir_path(&cgroup), "/sys/fs/cgroup");
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:82 CgroupsInfoTest.testGetRootCgroup_mixed_v1_has_memory
#[test]
fn test_get_root_cgroup_mixed_v1_has_memory() {
    let s = set_up();
    let path_string = s.create_fake_absolute_file(
        "/proc/self/mounts",
        &[
            "sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0",
            "cgroup2 /sys/fs/cgroup cgroup2 rw,memory_recursiveprot 0 0",
            "cgroup /dev/cgroup/job cgroup rw,job 0 0",
            "cgroup /dev/cgroup/memory cgroup rw,memory,hugetlb 0 0",
            "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0",
        ],
    );

    let cgroup = CgroupsInfo::get_root_cgroup(&JavaFile::new(&path_string));

    assert_eq!(cgroup.get_type(), Type::Root);
    assert_eq!(cgroup.get_version(), Some(Version::V1));
    assert_eq!(dir_path(&cgroup), "/dev/cgroup/memory");
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:100 CgroupsInfoTest.testGetRootCgroup_mixed_v2_has_memory
#[test]
fn test_get_root_cgroup_mixed_v2_has_memory() {
    let s = set_up();
    let path_string = s.create_fake_absolute_file(
        "/proc/self/mount",
        &[
            "sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0",
            "cgroup2 /sys/fs/cgroup cgroup2 rw,memory_recursiveprot 0 0",
            "cgroup /dev/cgroup/job cgroup rw,job 0 0",
            "cgroup /dev/cgroup/io cgroup rw,io 0 0",
            "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0",
        ],
    );

    let cgroup = CgroupsInfo::get_root_cgroup(&JavaFile::new(&path_string));

    assert_eq!(cgroup.get_type(), Type::Root);
    assert_eq!(cgroup.get_version(), Some(Version::V2));
    assert_eq!(dir_path(&cgroup), "/sys/fs/cgroup");
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:118 CgroupsInfoTest.testCreateBlazeSpawnsCgroup_v1
#[test]
fn test_create_blaze_spawns_cgroup_v1() {
    let s = set_up();
    let mount_path = format!("{}/dev/cgroup/memory", s.root);
    let root_cgroup = CgroupsInfo::new_v1(Type::Root, Some(JavaFile::new(&mount_path)));
    let proc_self_cgroup_path = s.create_fake_absolute_file(
        "/proc/self/cgroup",
        &[
            "8:net:/netdir/action-6",
            "7:memory,hugetlb:/memdir/action-6",
            "6:job:/jobdir/action-16",
            "5:io:/iodir/action-1",
        ],
    );
    s.dir(&format!("{}/dev/cgroup/memory/memdir/action-6", s.root));
    let blaze_spawns_path = format!(
        "{}/dev/cgroup/memory/memdir/action-6/blaze_{}_spawns",
        s.root,
        std::process::id()
    );
    s.dir(&blaze_spawns_path);

    let cgroup = root_cgroup.create_blaze_spawns_cgroup(&proc_self_cgroup_path);

    assert!(cgroup.get_cgroup_dir().unwrap().exists());
    assert_eq!(dir_path(&cgroup), blaze_spawns_path);
    assert_eq!(cgroup.get_type(), Type::BlazeSpawns);
    assert_eq!(cgroup.get_version(), Some(Version::V1));
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:145 CgroupsInfoTest.testCreateBlazeSpawnsCgroup_v2
#[test]
fn test_create_blaze_spawns_cgroup_v2() {
    let s = set_up();
    let mount_path = format!("{}/sys/fs/cgroup", s.root);
    let root_cgroup = CgroupsInfo::new_v2(Type::Root, Some(JavaFile::new(&mount_path)));
    let proc_self_cgroup_path =
        s.create_fake_absolute_file("/proc/self/cgroup", &["0::/user.slice/session.scope"]);
    // In v2, the blaze spawns cgroup is created one step up from where the blaze process is
    // defined in the /proc/self/cgroup file (defined above). Specifically, here it is in the
    // ".../user.slice".
    let blaze_spawns_path = format!(
        "{mount_path}/user.slice/blaze_{}_spawns.slice",
        std::process::id()
    );
    // Even though the blaze spawn's cgroup directory is meant to be created in the method call,
    // we create it here so that we can prepare the controller files that are expected beforehand.
    s.dir(&blaze_spawns_path);
    // Since this controllers file is missing `pids`, we expect that to be written to it.
    s.file(
        &format!("{blaze_spawns_path}/cgroup.controllers"),
        &["memory pids"],
    );
    s.file(
        &format!("{blaze_spawns_path}/cgroup.subtree_control"),
        &["memory"],
    );

    let cgroup = root_cgroup.create_blaze_spawns_cgroup(&proc_self_cgroup_path);

    assert_eq!(cgroup.get_version(), Some(Version::V2));
    assert!(cgroup.get_cgroup_dir().unwrap().exists());
    assert_eq!(dir_path(&cgroup), blaze_spawns_path);
    assert_eq!(cgroup.get_type(), Type::BlazeSpawns);
    assert_eq!(cgroup.get_version(), Some(Version::V2));
    // This is not what an actual cgroups v2 file would contain, but it's what we expect to write
    // to it to enable subtree control.
    assert_eq!(
        lines_of(&format!("{blaze_spawns_path}/cgroup.subtree_control")),
        vec!["+memory +pids"]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:179 CgroupsInfoTest.testCreateIndividualSpawnCgroup_withLimit_v1
#[test]
fn test_create_individual_spawn_cgroup_with_limit_v1() {
    let s = set_up();
    let blaze_spawns_path = format!(
        "{}/dev/cgroup/memory/memdir/action-6/blaze_1234_spawns",
        s.root
    );
    s.dir(&blaze_spawns_path);
    let blaze_spawns_cgroup =
        CgroupsInfo::new_v1(Type::BlazeSpawns, Some(JavaFile::new(&blaze_spawns_path)));

    let cgroup = blaze_spawns_cgroup.create_individual_spawn_cgroup("spawn_1", 100);

    assert!(cgroup.exists());
    assert_eq!(dir_path(&cgroup), format!("{blaze_spawns_path}/spawn_1"));
    assert_eq!(cgroup.get_type(), Type::Spawn);
    assert_eq!(cgroup.get_version(), Some(Version::V1));
    assert_eq!(
        lines_of(&format!(
            "{blaze_spawns_path}/spawn_1/memory.limit_in_bytes"
        )),
        vec!["104857600"]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:197 CgroupsInfoTest.testCreateIndividualSpawnCgroup_noLimit_v1
#[test]
fn test_create_individual_spawn_cgroup_no_limit_v1() {
    let s = set_up();
    let blaze_spawns_path = format!(
        "{}/dev/cgroup/memory/memdir/action-6/blaze_1234_spawns",
        s.root
    );
    s.dir(&blaze_spawns_path);
    let blaze_spawns_cgroup =
        CgroupsInfo::new_v1(Type::BlazeSpawns, Some(JavaFile::new(&blaze_spawns_path)));

    let cgroup = blaze_spawns_cgroup.create_individual_spawn_cgroup("spawn_1", 0);

    assert!(cgroup.exists());
    assert_eq!(dir_path(&cgroup), format!("{blaze_spawns_path}/spawn_1"));
    assert_eq!(cgroup.get_type(), Type::Spawn);
    assert_eq!(cgroup.get_version(), Some(Version::V1));
    // In reality, cgroups should still create this file automatically, but since we don't have
    // that in our tests, the memory limits file should not have been created since it isn't
    // written to.
    assert!(
        !JavaFile::new(&format!(
            "{blaze_spawns_path}/spawn_1/memory.limit_in_bytes"
        ))
        .exists()
    );
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:217 CgroupsInfoTest.testCreateIndividualSpawnCgroup_withLimit_v2
#[test]
fn test_create_individual_spawn_cgroup_with_limit_v2() {
    let s = set_up();
    let blaze_spawns_path = format!(
        "{}/sys/fs/cgroup/user.slice/blaze_1234_spawns.slice",
        s.root
    );
    s.dir(&blaze_spawns_path);
    let blaze_spawns_cgroup =
        CgroupsInfo::new_v2(Type::BlazeSpawns, Some(JavaFile::new(&blaze_spawns_path)));

    let cgroup = blaze_spawns_cgroup.create_individual_spawn_cgroup("spawn_1", 100);

    assert!(cgroup.exists());
    assert_eq!(
        dir_path(&cgroup),
        format!("{blaze_spawns_path}/spawn_1.scope")
    );
    assert_eq!(cgroup.get_type(), Type::Spawn);
    assert_eq!(cgroup.get_version(), Some(Version::V2));
    assert_eq!(
        lines_of(&format!(
            "{blaze_spawns_path}/spawn_1.scope/memory.oom.group"
        )),
        vec!["1"]
    );
    assert_eq!(
        lines_of(&format!("{blaze_spawns_path}/spawn_1.scope/memory.max")),
        vec!["104857600"]
    );
    assert_eq!(
        lines_of(&format!(
            "{blaze_spawns_path}/spawn_1.scope/memory.swap.max"
        )),
        vec!["0"]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:240 CgroupsInfoTest.testCreateIndividualSpawnCgroup_noLimit_v2
#[test]
fn test_create_individual_spawn_cgroup_no_limit_v2() {
    let s = set_up();
    let blaze_spawns_path = format!(
        "{}/sys/fs/cgroup/user.slice/blaze_1234_spawns.slice",
        s.root
    );
    s.dir(&blaze_spawns_path);
    let blaze_spawns_cgroup =
        CgroupsInfo::new_v2(Type::BlazeSpawns, Some(JavaFile::new(&blaze_spawns_path)));

    let cgroup = blaze_spawns_cgroup.create_individual_spawn_cgroup("spawn_1", 0);

    assert!(cgroup.exists());
    assert_eq!(
        dir_path(&cgroup),
        format!("{blaze_spawns_path}/spawn_1.scope")
    );
    assert_eq!(cgroup.get_type(), Type::Spawn);
    assert_eq!(cgroup.get_version(), Some(Version::V2));
    // In reality, cgroups should still create this file automatically, but since we don't have
    // that in our tests, the memory limits files should not have been created since they aren't
    // written to.
    for f in ["memory.oom.group", "memory.max", "memory.swap.max"] {
        assert!(!JavaFile::new(&format!("{blaze_spawns_path}/spawn_1.scope/{f}")).exists());
    }
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:262 CgroupsInfoTest.testGetMemoryUsageInKb_v1
#[test]
fn test_get_memory_usage_in_kb_v1() {
    let s = set_up();
    let cgroup_path = format!("{}/dev/cgroup/memory/memdir/action-1", s.root);
    s.dir(&cgroup_path);
    let cgroups_info = CgroupsInfo::new_v1(Type::Spawn, Some(JavaFile::new(&cgroup_path)));

    // It should return 0 if the /path/to/cgroup/memory.usage_in_bytes does not exist.
    assert_eq!(cgroups_info.get_memory_usage_in_kb(), 0);

    s.file(
        &format!("{cgroup_path}/memory.usage_in_bytes"),
        &["1024000"],
    );

    assert_eq!(cgroups_info.get_memory_usage_in_kb(), 1000);
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:277 CgroupsInfoTest.testGetMemoryUsageInKb_v2
#[test]
fn test_get_memory_usage_in_kb_v2() {
    let s = set_up();
    let cgroup_path = format!("{}/sys/fs/cgroup/memdir/action-1", s.root);
    s.dir(&cgroup_path);
    let cgroups_info = CgroupsInfo::new_v2(Type::Spawn, Some(JavaFile::new(&cgroup_path)));

    // It should return 0 if the /path/to/cgroup/memory.current does not exist.
    assert_eq!(cgroups_info.get_memory_usage_in_kb(), 0);

    s.file(&format!("{cgroup_path}/memory.current"), &["1024000"]);
    // Divided by 1024.

    assert_eq!(cgroups_info.get_memory_usage_in_kb(), 1000);
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:293 CgroupsInfoTest.testAddProcess_v1
#[test]
fn test_add_process_v1() {
    let s = set_up();
    let cgroup_path = format!("{}/dev/cgroup/memory/memdir/action-1", s.root);
    s.dir(&cgroup_path);
    let cgroups_info = CgroupsInfo::new_v1(Type::Spawn, Some(JavaFile::new(&cgroup_path)));

    cgroups_info.add_process(1234).unwrap();

    assert_eq!(
        lines_of(&format!("{}/cgroup.procs", dir_path(&cgroups_info))),
        vec!["1234"]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:306 CgroupsInfoTest.testAddProcess_v2
#[test]
fn test_add_process_v2() {
    let s = set_up();
    let cgroup_path = format!("{}/sys/fs/cgroup/memdir/action-1", s.root);
    s.dir(&cgroup_path);
    let cgroups_info = CgroupsInfo::new_v2(Type::Spawn, Some(JavaFile::new(&cgroup_path)));

    cgroups_info.add_process(1234).unwrap();

    assert_eq!(
        lines_of(&format!("{}/cgroup.procs", dir_path(&cgroups_info))),
        vec!["1234"]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:319 CgroupsInfoTest.testGetRootCgroup_returnsInvalidCgroup_whenMountNotFound
#[test]
fn test_get_root_cgroup_returns_invalid_cgroup_when_mount_not_found() {
    let s = set_up();
    let path_string = s.create_fake_absolute_file("/proc/self/mounts", &[""]);

    let cgroup = CgroupsInfo::get_root_cgroup(&JavaFile::new(&path_string));

    assert!(matches!(cgroup.get_kind(), Kind::Invalid { .. }));
    assert_eq!(cgroup.get_type(), Type::Root);
}

// Java: src/test/java/com/google/devtools/build/lib/sandbox/CgroupsInfoTest.java:329 CgroupsInfoTest.testCreateCgroupFromInvalidCgroup_returnsInvalidCgroup
#[test]
fn test_create_cgroup_from_invalid_cgroup_returns_invalid_cgroup() {
    let error_message = "Some error message";
    let invalid_root_cgroup = CgroupsInfo::invalid(Type::Root, Some(Version::V1), error_message);
    let invalid_blaze_spawns_cgroup =
        CgroupsInfo::invalid(Type::BlazeSpawns, Some(Version::V2), error_message);

    let created_blaze_spawns_cgroup = invalid_root_cgroup.create_blaze_spawns_cgroup("");
    let created_spawn_cgroup =
        invalid_blaze_spawns_cgroup.create_individual_spawn_cgroup("spawn_1", 1);

    assert!(matches!(
        created_blaze_spawns_cgroup.get_kind(),
        Kind::Invalid { .. }
    ));
    // Should still have the same version as the parent cgroup that attempted to create it.
    assert_eq!(created_blaze_spawns_cgroup.get_version(), Some(Version::V1));
    assert_eq!(created_blaze_spawns_cgroup.get_type(), Type::BlazeSpawns);

    assert!(matches!(
        created_spawn_cgroup.get_kind(),
        Kind::Invalid { .. }
    ));
    // Should still have the same version as the parent cgroup that attempted to create it.
    assert_eq!(created_spawn_cgroup.get_version(), Some(Version::V2));
    assert_eq!(created_spawn_cgroup.get_type(), Type::Spawn);
}
