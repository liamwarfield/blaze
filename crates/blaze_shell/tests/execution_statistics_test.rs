//! Port of `src/test/java/com/google/devtools/build/lib/shell/ExecutionStatisticsTest.java`.

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use blaze_shell::ExecutionStatistics;
use blaze_shell::execution_statistics::{ProtosExecutionStatistics, ProtosResourceUsage};
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};

/// PORT: Java uses an InMemoryFileSystem (not ported); a unique directory on the real file
/// system serves.
fn working_dir(name: &str) -> Path {
    let fs: FileSystemRef = Arc::new(UnixFileSystem::new(DigestHashFunction::Sha256));
    let dir = std::env::temp_dir().join(format!(
        "blaze_shell_execstats_{}_{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Path::create_from_str(&dir.to_string_lossy(), &fs)
}

fn create_execution_statistics_proto_file(
    dir: &Path,
    execution_statistics_proto: &ProtosExecutionStatistics,
) -> Path {
    let encoded_proto_file = dir.get_relative_str("encoded_action_execution_proto");
    let mut out = encoded_proto_file.get_output_stream().unwrap();
    out.write_all(&execution_statistics_proto.to_bytes())
        .unwrap();
    out.flush().unwrap();
    encoded_proto_file
}

#[test]
fn test_no_resource_usage_when_no_resource_usage_proto() {
    let dir = working_dir("none");
    let proto_filename =
        create_execution_statistics_proto_file(&dir, &ProtosExecutionStatistics::default());
    let resource_usage = ExecutionStatistics::get_resource_usage(&proto_filename).unwrap();
    assert!(resource_usage.is_none());
}

#[test]
fn test_statitics_provided_from_proto_filename() {
    let rigged_user_execution_time = Duration::from_secs(42) + Duration::from_nanos(19790000);
    let rigged_system_execution_time = Duration::from_secs(33) + Duration::from_nanos(290000);

    let resource_usage_proto = ProtosResourceUsage {
        utime_sec: rigged_user_execution_time.as_secs() as i64,
        utime_usec: rigged_user_execution_time.subsec_micros() as i64,
        stime_sec: rigged_system_execution_time.as_secs() as i64,
        stime_usec: rigged_system_execution_time.subsec_micros() as i64,
        maxrss: 1,
        ixrss: 2,
        idrss: 3,
        isrss: 4,
        minflt: 5,
        majflt: 6,
        nswap: 7,
        inblock: 8,
        oublock: 9,
        msgsnd: 10,
        msgrcv: 11,
        nsignals: 12,
        nvcsw: 13,
        nivcsw: 14,
    };
    let execution_statistics_proto = ProtosExecutionStatistics {
        resource_usage: Some(resource_usage_proto),
    };
    let dir = working_dir("some");
    let proto_filename = create_execution_statistics_proto_file(&dir, &execution_statistics_proto);

    let resource_usage = ExecutionStatistics::get_resource_usage(&proto_filename)
        .unwrap()
        .expect("resource usage present");

    assert_eq!(
        resource_usage.get_user_execution_time(),
        rigged_user_execution_time
    );
    assert_eq!(
        resource_usage.get_system_execution_time(),
        rigged_system_execution_time
    );
    assert_eq!(resource_usage.get_maximum_resident_set_size(), 1);
    assert_eq!(resource_usage.get_integral_shared_memory_size(), 2);
    assert_eq!(resource_usage.get_integral_unshared_data_size(), 3);
    assert_eq!(resource_usage.get_integral_unshared_stack_size(), 4);
    assert_eq!(resource_usage.get_page_reclaims(), 5);
    assert_eq!(resource_usage.get_page_faults(), 6);
    assert_eq!(resource_usage.get_swaps(), 7);
    assert_eq!(resource_usage.get_block_input_operations(), 8);
    assert_eq!(resource_usage.get_block_output_operations(), 9);
    assert_eq!(resource_usage.get_ipc_messages_sent(), 10);
    assert_eq!(resource_usage.get_ipc_messages_received(), 11);
    assert_eq!(resource_usage.get_signals_received(), 12);
    assert_eq!(resource_usage.get_voluntary_context_switches(), 13);
    assert_eq!(resource_usage.get_involuntary_context_switches(), 14);
}
