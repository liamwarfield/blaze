//! Port of `LinuxSandboxCommandLineBuilderTest.java`.

use std::time::Duration;

use blaze_exec::sandbox::linux_sandbox_command_line_builder::{
    LinuxSandboxCommandLineBuilder, NetworkNamespace,
};
use blaze_vfs::{DigestHashFunction, FileSystemExt, FileSystemRef, PathFragment, UnixFileSystem};
use indexmap::{IndexMap, IndexSet};

fn test_fs() -> FileSystemRef {
    // PORT: Java uses an InMemoryFileSystem; the builder does no I/O, so the Unix one is the same.
    UnixFileSystem::create(DigestHashFunction::Sha256)
}

#[test]
#[should_panic(expected = "exclusive")]
fn test_linux_sandbox_command_line_builder_fake_root_and_fake_username_are_exclusive() {
    let test_fs = test_fs();
    let linux_sandbox_path = test_fs.get_path("/linux-sandbox");
    let command_arguments = vec!["echo".to_string(), "hello, flo".to_string()];

    LinuxSandboxCommandLineBuilder::command_line_builder(linux_sandbox_path)
        .set_use_fake_root(true)
        .set_use_fake_username(true)
        .build_for_command(&command_arguments);
}

#[test]
fn test_linux_sandbox_command_line_builder_builds_without_optional_arguments() {
    let test_fs = test_fs();
    let linux_sandbox_path = test_fs.get_path("/linux-sandbox");

    let command_arguments = vec!["echo".to_string(), "hello, max".to_string()];

    let mut expected_command_line = vec![linux_sandbox_path.get_path_string().to_string()];
    expected_command_line.push("--".to_string());
    expected_command_line.extend(command_arguments.iter().cloned());

    let command_line = LinuxSandboxCommandLineBuilder::command_line_builder(linux_sandbox_path)
        .build_for_command(&command_arguments);

    assert_eq!(command_line, expected_command_line);
}

#[test]
fn test_linux_sandbox_command_line_builder_builds_with_optional_arguments() {
    let test_fs = test_fs();
    let linux_sandbox_path = test_fs.get_path("/linux-sandbox");

    let command_arguments = vec!["echo".to_string(), "hello, tom".to_string()];

    let timeout = Duration::from_secs(10);
    let kill_delay = Duration::from_secs(2);

    let sandbox_debug_path = test_fs.get_path("/debug.out");
    let statistics_path = test_fs.get_path("/stats.out");

    let working_directory = test_fs.get_path("/all-work-and-no-play");
    let stdout_path = test_fs.get_path("/stdout.txt");
    let stderr_path = test_fs.get_path("/stderr.txt");

    // These two flags are exclusive.
    let use_fake_username = true;
    let use_fake_root = false;

    let create_network_namespace = true;
    let use_fake_hostname = true;

    let file_system = test_fs.clone();
    let work_dir = file_system.get_path("/work");
    let concrete_dir = work_dir.get_relative_str("concrete");
    let sandbox_dir = work_dir.get_relative_str("sandbox");

    let bind_mount_source1 = concrete_dir.get_relative_str("bindMountSource1");
    let bind_mount_source2 = concrete_dir.get_relative_str("bindMountSource2");
    let mount_dir = sandbox_dir.get_relative_str("mount");
    let bind_mount_target1 = mount_dir.get_relative_str("bindMountTarget1");
    let bind_mount_target2 = mount_dir.get_relative_str("bindMountTarget2");
    let bind_mount_same_source_and_target = mount_dir.get_relative_str("bindMountSourceAndTarget");

    let writable_dir1 = sandbox_dir.get_relative_str("writable1");
    let writable_dir2 = sandbox_dir.get_relative_str("writable2");

    let tmpfs_dir1: PathFragment = sandbox_dir.as_fragment().get_relative_str("tmpfs1");
    let tmpfs_dir2: PathFragment = sandbox_dir.as_fragment().get_relative_str("tmpfs2");

    let writable_files_and_directories: IndexSet<_> =
        [writable_dir1.clone(), writable_dir2.clone()]
            .into_iter()
            .collect();

    let tmpfs_directories: IndexSet<PathFragment> = [tmpfs_dir1.clone(), tmpfs_dir2.clone()]
        .into_iter()
        .collect();

    // ImmutableSortedMap.naturalOrder()
    let mut sorted = std::collections::BTreeMap::new();
    sorted.insert(
        bind_mount_same_source_and_target.clone(),
        bind_mount_same_source_and_target.clone(),
    );
    sorted.insert(bind_mount_target1.clone(), bind_mount_source1.clone());
    sorted.insert(bind_mount_target2.clone(), bind_mount_source2.clone());
    let bind_mounts: IndexMap<_, _> = sorted.into_iter().collect();

    let cgroups_dir = file_system.get_path("/sys/fs/cgroups/something");

    let mut expected_command_line: Vec<String> =
        vec![linux_sandbox_path.get_path_string().to_string()];
    for s in [
        "-W",
        working_directory.get_path_string(),
        "-T",
        timeout.as_secs().to_string().as_str(),
        "-t",
        kill_delay.as_secs().to_string().as_str(),
        "-l",
        stdout_path.get_path_string(),
        "-L",
        stderr_path.get_path_string(),
        "-w",
        writable_dir1.get_path_string(),
        "-w",
        writable_dir2.get_path_string(),
        "-e",
        tmpfs_dir1.get_path_string(),
        "-e",
        tmpfs_dir2.get_path_string(),
        "-M",
        bind_mount_same_source_and_target.get_path_string(),
        "-M",
        bind_mount_source1.get_path_string(),
        "-m",
        bind_mount_target1.get_path_string(),
        "-M",
        bind_mount_source2.get_path_string(),
        "-m",
        bind_mount_target2.get_path_string(),
        "-S",
        statistics_path.get_path_string(),
        "-H",
        "-N",
        "-U",
        "-D",
        sandbox_debug_path.get_path_string(),
        "-p",
        "-C",
        cgroups_dir.get_path_string(),
        "--",
    ] {
        expected_command_line.push(s.to_string());
    }
    expected_command_line.extend(command_arguments.iter().cloned());

    let command_line = LinuxSandboxCommandLineBuilder::command_line_builder(linux_sandbox_path)
        .set_working_directory(working_directory)
        .set_stdout_path(stdout_path)
        .set_stderr_path(stderr_path)
        .set_timeout(timeout)
        .set_kill_delay(kill_delay)
        .set_writable_files_and_directories(writable_files_and_directories)
        .set_tmpfs_directories(tmpfs_directories)
        .set_bind_mounts(bind_mounts)
        .set_use_fake_hostname(use_fake_hostname)
        .set_create_network_namespace(if create_network_namespace {
            NetworkNamespace::NetnsWithLoopback
        } else {
            NetworkNamespace::NoNetns
        })
        .set_use_fake_root(use_fake_root)
        .set_statistics_path(statistics_path.clone())
        .set_use_fake_username(use_fake_username)
        .set_sandbox_debug_path(sandbox_debug_path.get_path_string().to_string())
        .set_persistent_process(true)
        .set_cgroups_dirs(
            [cgroups_dir.get_path_string().to_string()]
                .into_iter()
                .collect(),
        )
        .build_for_command(&command_arguments);

    assert_eq!(command_line, expected_command_line);
}
