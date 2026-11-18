//! Port of `src/test/java/com/google/devtools/build/lib/actions/SpawnResultTest.java`.

use blaze_exec::actions::{
    ActionInputRef, BasicActionInput, POSIX_TIMEOUT_EXIT_CODE, SpawnResultBuilder, Status,
};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail, spawn::Code};

fn timeout_detail() -> FailureDetail {
    FailureDetail {
        message: String::new(),
        category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
            code: Code::Timeout as i32,
            ..Default::default()
        })),
    }
}

#[test]
fn get_timeout_message() {
    let r = SpawnResultBuilder::new()
        .set_status(Status::Timeout)
        .set_wall_time_in_ms(5 * 1000)
        .set_exit_code(POSIX_TIMEOUT_EXIT_CODE)
        .set_failure_detail(timeout_detail())
        .set_runner_name("test")
        .build();
    assert!(
        r.get_detail_message("", false, false)
            .contains("(failed due to timeout after 5.00 seconds.)")
    );
}

#[test]
fn get_timeout_message_no_time() {
    let r = SpawnResultBuilder::new()
        .set_status(Status::Timeout)
        .set_exit_code(POSIX_TIMEOUT_EXIT_CODE)
        .set_failure_detail(timeout_detail())
        .set_runner_name("test")
        .build();
    assert!(
        r.get_detail_message("", false, false)
            .contains("(failed due to timeout.)")
    );
}

#[test]
fn in_memory_contents() {
    let output = ActionInputRef::new(BasicActionInput::from_path("/foo/bar"));
    let contents = b"hello world".to_vec();

    let r = SpawnResultBuilder::new()
        .set_status(Status::Success)
        .set_exit_code(0)
        .set_runner_name("test")
        .set_in_memory_output(output.clone(), contents.clone())
        .build();

    assert_eq!(r.get_in_memory_output(&output), Some(contents.as_slice()));
    // PORT: getInMemoryOutput(null) has no Rust counterpart (the argument is not nullable).
    assert_eq!(
        r.get_in_memory_output(&ActionInputRef::new(BasicActionInput::from_path(
            "/does/not/exist"
        ))),
        None
    );
}

#[test]
fn default_spawn_metrics_includes_measured_memory_from_resource_usage() {
    let result = SpawnResultBuilder::new()
        .set_status(Status::Success)
        .set_exit_code(0)
        .set_runner_name("test")
        .set_wall_time_in_ms(100)
        .set_memory_in_kb(1000)
        .build();

    assert_eq!(result.get_metrics().measured_memory_peak_bytes, 1000 * 1024);
}
