//! Port of `src/test/java/com/google/devtools/build/lib/actions/CommandLinesTest.java`.

use blaze_actions::InputMetadataProvider;
use blaze_actions::actions::command_line;
use blaze_actions::actions::command_line_limits::CommandLineLimits;
use blaze_actions::actions::command_lines::CommandLines;
use blaze_actions::actions::param_file_info::ParamFileInfo;
use blaze_actions::actions::parameter_file::ParameterFileType;
use blaze_actions::actions::path_mapper::{NOOP, PathMapper};
use blaze_vfs::PathFragment;

const INPUT_METADATA_PROVIDER: Option<&dyn InputMetadataProvider> = None;
const NO_LIMIT: CommandLineLimits = CommandLineLimits::new(10000);

fn exec_path() -> PathFragment {
    PathFragment::create("output.txt")
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn unquoted(use_always: bool) -> ParamFileInfo {
    ParamFileInfo::builder(ParameterFileType::UNQUOTED)
        .set_use_always(use_always)
        .build()
}

#[test]
fn expand_simple_command_line_returns_correct_command_line() {
    let command_lines = CommandLines::builder()
        .add_command_line(command_line::of(strs(&["--foo", "--bar"])))
        .build();

    let expanded = command_lines
        .expand_with_estimate(INPUT_METADATA_PROVIDER, &exec_path(), &NO_LIMIT, &NOOP, 0)
        .unwrap();

    assert_eq!(
        command_lines.all_arguments().unwrap(),
        strs(&["--foo", "--bar"])
    );
    assert_eq!(expanded.arguments(), strs(&["--foo", "--bar"]).as_slice());
    assert!(expanded.get_param_files().is_empty());
}

#[test]
fn expand_command_line_from_arguments_returns_correct_command_line() {
    let command_lines = CommandLines::of_args(strs(&["--foo", "--bar"]));

    let expanded = command_lines
        .expand_with_estimate(INPUT_METADATA_PROVIDER, &exec_path(), &NO_LIMIT, &NOOP, 0)
        .unwrap();

    assert_eq!(
        command_lines.all_arguments().unwrap(),
        strs(&["--foo", "--bar"])
    );
    assert_eq!(expanded.arguments(), strs(&["--foo", "--bar"]).as_slice());
    assert!(expanded.get_param_files().is_empty());
}

#[test]
fn expand_concat_command_lines_returns_concatenated_arguments() {
    let command_lines = CommandLines::concat(
        command_line::of(strs(&["--before"])),
        &CommandLines::of_args(strs(&["--foo", "--bar"])),
    );

    let expanded = command_lines
        .expand_with_estimate(INPUT_METADATA_PROVIDER, &exec_path(), &NO_LIMIT, &NOOP, 0)
        .unwrap();

    assert_eq!(
        command_lines.all_arguments().unwrap(),
        strs(&["--before", "--foo", "--bar"])
    );
    assert_eq!(
        expanded.arguments(),
        strs(&["--before", "--foo", "--bar"]).as_slice()
    );
    assert!(expanded.get_param_files().is_empty());
}

#[test]
fn expand_param_file_use_always_returns_command_line_with_param_file() {
    let command_lines = CommandLines::builder()
        .add_command_line_with_param_file(
            command_line::of(strs(&["--foo", "--bar"])),
            Some(unquoted(true)),
        )
        .build();

    let expanded = command_lines
        .expand_with_estimate(INPUT_METADATA_PROVIDER, &exec_path(), &NO_LIMIT, &NOOP, 0)
        .unwrap();

    assert_eq!(
        command_lines.all_arguments().unwrap(),
        strs(&["--foo", "--bar"])
    );
    assert_eq!(
        expanded.arguments(),
        strs(&["@output.txt-0.params"]).as_slice()
    );
    assert_eq!(expanded.get_param_files().len(), 1);
    assert_eq!(
        expanded.get_param_files()[0].get_arguments(),
        strs(&["--foo", "--bar"]).as_slice()
    );
}

#[test]
fn expand_param_file_command_within_limits_returns_no_param_file() {
    let command_lines = CommandLines::builder()
        .add_command_line_with_param_file(
            command_line::of(strs(&["--foo", "--bar"])),
            Some(unquoted(false)),
        )
        .build();

    // Set max length to longer than command line, no param file needed
    let expanded = command_lines
        .expand_with_estimate(INPUT_METADATA_PROVIDER, &exec_path(), &NO_LIMIT, &NOOP, 0)
        .unwrap();

    assert_eq!(expanded.arguments(), strs(&["--foo", "--bar"]).as_slice());
    assert!(expanded.get_param_files().is_empty());
}

#[test]
fn expand_param_file_command_over_limits_returns_param_file() {
    let command_lines = CommandLines::builder()
        .add_command_line_with_param_file(
            command_line::of(strs(&["--foo", "--bar"])),
            Some(unquoted(false)),
        )
        .build();

    // Set max length to 0, spill to param file is forced
    let expanded = command_lines
        .expand_with_estimate(
            INPUT_METADATA_PROVIDER,
            &exec_path(),
            &CommandLineLimits::new(0),
            &NOOP,
            0,
        )
        .unwrap();

    assert_eq!(
        expanded.arguments(),
        strs(&["@output.txt-0.params"]).as_slice()
    );
    assert_eq!(expanded.get_param_files().len(), 1);
    assert_eq!(
        expanded.get_param_files()[0].get_arguments(),
        strs(&["--foo", "--bar"]).as_slice()
    );
}

#[test]
fn expand_mix_of_command_lines_and_param_files_returns_correct_command_lines() {
    let command_lines = CommandLines::builder()
        .add_command_line(command_line::of(strs(&["a", "b"])))
        .add_command_line_with_param_file(command_line::of(strs(&["c", "d"])), Some(unquoted(true)))
        .add_command_line(command_line::of(strs(&["e", "f"])))
        .add_command_line_with_param_file(command_line::of(strs(&["g", "h"])), Some(unquoted(true)))
        .build();

    let expanded = command_lines
        .expand_with_estimate(INPUT_METADATA_PROVIDER, &exec_path(), &NO_LIMIT, &NOOP, 0)
        .unwrap();

    assert_eq!(
        command_lines.all_arguments().unwrap(),
        strs(&["a", "b", "c", "d", "e", "f", "g", "h"])
    );
    assert_eq!(
        expanded.arguments(),
        strs(&[
            "a",
            "b",
            "@output.txt-0.params",
            "e",
            "f",
            "@output.txt-1.params"
        ])
        .as_slice()
    );
    assert_eq!(expanded.get_param_files().len(), 2);
    assert_eq!(
        expanded.get_param_files()[0].get_arguments(),
        strs(&["c", "d"]).as_slice()
    );
    assert_eq!(
        expanded.get_param_files()[0].get_exec_path_string(),
        "output.txt-0.params"
    );
    assert_eq!(
        expanded.get_param_files()[1].get_arguments(),
        strs(&["g", "h"]).as_slice()
    );
    assert_eq!(
        expanded.get_param_files()[1].get_exec_path_string(),
        "output.txt-1.params"
    );
}

#[test]
fn expand_commands_with_param_files_second_exceeds_limits_returns_param_file_for_second_only() {
    let command_lines = CommandLines::builder()
        .add_command_line_with_param_file(
            command_line::of(strs(&["a", "b"])),
            Some(unquoted(false)),
        )
        .add_command_line_with_param_file(
            command_line::of(strs(&["c", "d"])),
            Some(unquoted(false)),
        )
        .build();

    let expanded = command_lines
        .expand_with_estimate(
            INPUT_METADATA_PROVIDER,
            &exec_path(),
            &CommandLineLimits::new(4),
            &NOOP,
            0,
        )
        .unwrap();

    assert_eq!(
        command_lines.all_arguments().unwrap(),
        strs(&["a", "b", "c", "d"])
    );
    assert_eq!(
        expanded.arguments(),
        strs(&["a", "b", "@output.txt-0.params"]).as_slice()
    );
    assert_eq!(expanded.get_param_files().len(), 1);
    assert_eq!(
        expanded.get_param_files()[0].get_arguments(),
        strs(&["c", "d"]).as_slice()
    );
}

/// Filtering of flag and positional arguments with flagsOnly.
#[test]
fn expand_flags_only_moves_only_dash_dash_prefixed_flags_to_param_file() {
    let command_lines = CommandLines::builder()
        .add_command_line_with_param_file(
            command_line::of(strs(&["--a", "1", "--b=c", "-2"])),
            Some(
                ParamFileInfo::builder(ParameterFileType::UNQUOTED)
                    .set_use_always(true)
                    .set_flags_only(true)
                    .build(),
            ),
        )
        .build();

    let expanded = command_lines
        .expand_with_estimate(
            INPUT_METADATA_PROVIDER,
            &exec_path(),
            &CommandLineLimits::new(4),
            &NOOP,
            0,
        )
        .unwrap();
    assert_eq!(
        command_lines.all_arguments().unwrap(),
        strs(&["--a", "1", "--b=c", "-2"])
    );
    // Java: containsExactly (any order).
    let mut args = expanded.arguments().to_vec();
    args.sort();
    let mut expected = strs(&["1", "-2", "@output.txt-0.params"]);
    expected.sort();
    assert_eq!(args, expected);
    assert_eq!(expanded.get_param_files().len(), 1);
    assert_eq!(
        expanded.get_param_files()[0].get_arguments(),
        strs(&["--a", "--b=c"]).as_slice()
    );
}

/// `execPath -> execPath.startsWith("bazel-out") ? "mapped".getRelative(execPath) : execPath`.
struct TestPathMapper;

impl PathMapper for TestPathMapper {
    fn map(&self, exec_path: &PathFragment) -> PathFragment {
        if exec_path.starts_with(&PathFragment::create("bazel-out")) {
            PathFragment::create("mapped").get_relative(exec_path)
        } else {
            exec_path.clone()
        }
    }
}

#[test]
fn expand_only_executable_arg_processed_for_path_mapping() {
    for num_non_executable_args in [0, 1, 2, 3] {
        for normalized_executable_path in [false, true] {
            for mappable_non_executable_path in [false, true] {
                let mut builder = CommandLines::builder();
                let executable_arg = if normalized_executable_path {
                    "bazel-out/k8-fastbuild/bin/my_binary"
                } else {
                    "bazel-out/some/path/../my_binary"
                };
                let non_executable_arg = if mappable_non_executable_path {
                    "bazel-out/k8-fastbuild/bin/unrelated"
                } else {
                    "hello/../world"
                };
                builder.add_single_argument(executable_arg);
                for _ in 0..num_non_executable_args {
                    builder.add_single_argument(non_executable_arg);
                }
                let command_lines = builder.build();

                let expected_executable_arg = if normalized_executable_path {
                    "mapped/bazel-out/k8-fastbuild/bin/my_binary"
                } else {
                    executable_arg
                };
                let mut expected_args = vec![expected_executable_arg.to_string()];
                for _ in 0..num_non_executable_args {
                    expected_args.push(non_executable_arg.to_string());
                }
                assert_eq!(
                    command_lines.all_arguments_with(&TestPathMapper).unwrap(),
                    expected_args,
                    "num={num_non_executable_args} normalized={normalized_executable_path} \
                     mappable={mappable_non_executable_path}"
                );
            }
        }
    }
}
