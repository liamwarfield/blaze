//! Port of `src/test/java/com/google/devtools/build/lib/runtime/ProcessWrapperTest.java`.

use std::time::Duration;

use blaze_exec::actions::{ActionInputRef, BasicActionInput, execution_requirements};
use blaze_exec::runtime::ProcessWrapper;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn test_process_wrapper_command_line_builder_builds_without_optional_arguments() {
    let command_arguments = strings(&["echo", "hello, world"]);

    let mut expected_command_line = strings(&["/some/path"]);
    expected_command_line.extend(command_arguments.iter().cloned());

    let process_wrapper = ProcessWrapper::new(
        PathFragment::create("/some/path"),
        ActionInputRef::new(BasicActionInput::from_path("/some/path")),
        /* kill_delay= */ None,
        /* graceful_sigterm= */ false,
    );
    let command_line = process_wrapper
        .command_line_builder(command_arguments)
        .build();

    assert_eq!(command_line, expected_command_line);
}

#[test]
fn test_process_wrapper_command_line_builder_builds_with_optional_arguments() {
    let command_arguments = strings(&["echo", "hello, world"]);

    let timeout = Duration::from_secs(10);
    let kill_delay = Duration::from_secs(2);
    let override_process_wrapper_path = PathFragment::create("/override/process-wrapper");
    let stdout_path = PathFragment::create("/stdout.txt");
    let stderr_path = PathFragment::create("/stderr.txt");
    let statistics_path = PathFragment::create("/stats.out");

    let mut expected_command_line = vec![
        override_process_wrapper_path.get_path_string().to_string(),
        format!("--timeout={}", timeout.as_secs()),
        format!("--kill_delay={}", kill_delay.as_secs()),
        format!("--stdout={stdout_path}"),
        format!("--stderr={stderr_path}"),
        format!("--stats={statistics_path}"),
        "--graceful_sigterm".to_string(),
    ];
    expected_command_line.extend(command_arguments.iter().cloned());

    let process_wrapper = ProcessWrapper::new(
        PathFragment::create("/path/process-wrapper"),
        ActionInputRef::new(BasicActionInput::from_path("/path/process-wrapper")),
        Some(kill_delay.into()),
        /* graceful_sigterm= */ true,
    );

    let command_line = process_wrapper
        .command_line_builder(command_arguments)
        .override_process_wrapper_path(override_process_wrapper_path)
        .set_timeout(timeout)
        .set_stdout_path(stdout_path)
        .set_stderr_path(stderr_path)
        .set_statistics_path(statistics_path)
        .build();

    assert_eq!(command_line, expected_command_line);
}

#[test]
fn test_process_wrapper_command_line_builder_with_execution_info() {
    let command_arguments = strings(&["echo", "hello, world"]);

    let process_wrapper = ProcessWrapper::new(
        PathFragment::create("/some/path"),
        ActionInputRef::new(BasicActionInput::from_path("/some/path")),
        /* kill_delay= */ None,
        /* graceful_sigterm= */ false,
    );
    let mut builder = process_wrapper.command_line_builder(command_arguments.clone());

    let mut expected_without_execution_info = strings(&["/some/path"]);
    expected_without_execution_info.extend(command_arguments.iter().cloned());
    assert_eq!(builder.build(), expected_without_execution_info);

    let mut expected_with_execution_info = strings(&["/some/path", "--graceful_sigterm"]);
    expected_with_execution_info.extend(command_arguments.iter().cloned());
    let mut execution_info = IndexMap::new();
    execution_info.insert(
        execution_requirements::GRACEFUL_TERMINATION.to_string(),
        "1".to_string(),
    );
    builder.add_execution_info(&execution_info);
    assert_eq!(builder.build(), expected_with_execution_info);
}
