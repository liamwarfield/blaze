//! Port of src/test/cpp/option_processor_test.cc.
//!
//! Startup option values (`host_jvm_args`, `batch`) are parsed by `StartupOptions` in the client
//! crate; tests that assert them check the recorded startup flags instead and say so.

mod rc_support;

use blaze_options::rc::option_processor::{
    BAZEL_SYSTEM_BAZELRC_PATH, CommandLine, OptionProcessor, RecordingStartupArgsProcessor,
    StartupFlagRegistry, can_read_file, dedupe_blazerc_paths,
};
use rc_support::*;

/// Java: option_processor_test.cc OptionProcessorTest fixture.
struct OptionProcessorTest {
    workspace: String,
    cwd: String,
    option_processor: OptionProcessor,
}

impl OptionProcessorTest {
    fn new(name: &str) -> Self {
        let tmp = fresh_tmpdir(name);
        let workspace = join(&tmp, "testdir");
        mkdirs(&workspace);
        let home = join(&tmp, "home");
        mkdirs(&home);
        // The C++ fixture uses cwd "cwd" and the default system rc path; HOME is the test
        // environment's. Use an empty fake home so no real rc file is read.
        let cwd = "cwd".to_string();
        let system_rc = if std::path::Path::new(BAZEL_SYSTEM_BAZELRC_PATH).exists() {
            // PORT: avoid reading the host's /etc/bazel.bazelrc in tests.
            join(&tmp, "bazel.bazelrc")
        } else {
            BAZEL_SYSTEM_BAZELRC_PATH.to_string()
        };
        let option_processor = OptionProcessor::new(
            StartupFlagRegistry::bazel(),
            &system_rc,
            "",
            client_env(&cwd, &home),
        );
        OptionProcessorTest {
            workspace,
            cwd,
            option_processor,
        }
    }

    fn failed_split_command_line_test(&self, args: &[&str], expected_error: &str) {
        let result = self
            .option_processor
            .split_command_line(args.iter().map(|s| s.to_string()).collect());
        assert_eq!(result, Err(expected_error.to_string()));
    }

    fn successful_split_command_line_test(&self, args: &[&str], expected: CommandLine) {
        let result = self
            .option_processor
            .split_command_line(args.iter().map(|s| s.to_string()).collect())
            .unwrap();
        assert_eq!(expected.path_to_binary, result.path_to_binary);
        assert_eq!(expected.startup_args, result.startup_args);
        assert_eq!(expected.command, result.command);
        assert_eq!(expected.command_args, result.command_args);
    }

    fn parse_options(&mut self, args: &[&str]) -> Vec<String> {
        let mut startup = RecordingStartupArgsProcessor::default();
        let r = self.option_processor.parse_options(
            args.iter().map(|s| s.to_string()).collect(),
            &self.workspace,
            &self.cwd,
            &mut startup,
        );
        if let Err((_, e)) = r {
            panic!("{}", e);
        }
        startup.flags.iter().map(|f| f.value.clone()).collect()
    }

    fn help_arg_is_interpreted_as_a_command(&mut self, arg: &str) {
        self.parse_options(&["bazel", arg]);
        assert_eq!(arg, self.option_processor.get_command());
        assert!(
            self.option_processor
                .get_explicit_command_arguments()
                .is_empty()
        );
    }
}

fn cl(path: &str, startup: &[&str], command: &str, args: &[&str]) -> CommandLine {
    CommandLine {
        path_to_binary: path.to_string(),
        startup_args: startup.iter().map(|s| s.to_string()).collect(),
        command: command.to_string(),
        command_args: args.iter().map(|s| s.to_string()).collect(),
    }
}

// Java: option_processor_test.cc OptionProcessorTest.CanParseOptions
#[test]
fn can_parse_options() {
    let mut t = OptionProcessorTest::new("can_parse_options");
    let flags = t.parse_options(&[
        "bazel",
        "--ignore_all_rc_files",
        "--host_jvm_args=MyParam",
        "--nobatch",
        "command",
        "--flag",
        "//my:target",
        "--flag2=42",
    ]);
    // PORT: host_jvm_args == ["MyParam"] and batch == false need StartupOptions (client crate).
    assert_eq!(
        flags,
        vec![
            "--ignore_all_rc_files",
            "--host_jvm_args=MyParam",
            "--nobatch"
        ]
    );
    assert_eq!("command", t.option_processor.get_command());
    assert_eq!(
        t.option_processor.get_explicit_command_arguments(),
        vec!["--flag", "//my:target", "--flag2=42"]
    );
}

// Java: option_processor_test.cc OptionProcessorTest.CanParseHelpCommandSurroundedByOtherArgs
#[test]
fn can_parse_help_command_surrounded_by_other_args() {
    let mut t = OptionProcessorTest::new("help_surrounded");
    let flags = t.parse_options(&[
        "bazel",
        "--ignore_all_rc_files",
        "--host_jvm_args=MyParam",
        "--nobatch",
        "help",
        "--flag",
        "//my:target",
        "--flag2=42",
    ]);
    // PORT: host_jvm_args/batch values need StartupOptions (client crate).
    assert_eq!(
        flags,
        vec![
            "--ignore_all_rc_files",
            "--host_jvm_args=MyParam",
            "--nobatch"
        ]
    );
    assert_eq!("help", t.option_processor.get_command());
    assert_eq!(
        t.option_processor.get_explicit_command_arguments(),
        vec!["--flag", "//my:target", "--flag2=42"]
    );
}

// Java: option_processor_test.cc OptionProcessorTest.CanParseHelpCommand
#[test]
fn can_parse_help_command() {
    OptionProcessorTest::new("help_cmd").help_arg_is_interpreted_as_a_command("help");
}

// Java: option_processor_test.cc OptionProcessorTest.CanParseHelpShortFlag
#[test]
fn can_parse_help_short_flag() {
    OptionProcessorTest::new("help_short").help_arg_is_interpreted_as_a_command("-h");
}

// Java: option_processor_test.cc OptionProcessorTest.CanParseHelpFlag
#[test]
fn can_parse_help_flag() {
    OptionProcessorTest::new("help_flag").help_arg_is_interpreted_as_a_command("-help");
}

// Java: option_processor_test.cc OptionProcessorTest.CanParseEmptyArgs
#[test]
fn can_parse_empty_args() {
    let mut t = OptionProcessorTest::new("empty_args");
    t.parse_options(&["bazel"]);
    assert_eq!("", t.option_processor.get_command());
    assert!(
        t.option_processor
            .get_explicit_command_arguments()
            .is_empty()
    );
}

// Java: option_processor_test.cc OptionProcessorTest.CanParseDifferentStartupArgs
#[test]
fn can_parse_different_startup_args() {
    let mut t = OptionProcessorTest::new("different_startup");
    let flags = t.parse_options(&[
        "bazel",
        "--nobatch",
        "--ignore_all_rc_files",
        "--host_jvm_args=MyParam",
        "--host_jvm_args",
        "42",
    ]);
    // PORT: host_jvm_args == ["MyParam", "42"] needs StartupOptions (client crate).
    // SplitCommandLine joins the space-separated unary value.
    assert_eq!(
        flags,
        vec![
            "--nobatch",
            "--ignore_all_rc_files",
            "--host_jvm_args=MyParam",
            "--host_jvm_args=42"
        ]
    );
    assert_eq!("", t.option_processor.get_command());
    assert!(
        t.option_processor
            .get_explicit_command_arguments()
            .is_empty()
    );
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineWithEmptyArgs
#[test]
fn split_command_line_with_empty_args() {
    OptionProcessorTest::new("split_empty")
        .failed_split_command_line_test(&[], "Unable to split command line, args is empty");
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineWithAllParams
#[test]
fn split_command_line_with_all_params() {
    OptionProcessorTest::new("split_all").successful_split_command_line_test(
        &[
            "bazel",
            "--ignore_all_rc_files",
            "build",
            "--bar",
            ":mytarget",
        ],
        cl(
            "bazel",
            &["--ignore_all_rc_files"],
            "build",
            &["--bar", ":mytarget"],
        ),
    );
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineWithAbsolutePathToBinary
#[test]
fn split_command_line_with_absolute_path_to_binary() {
    OptionProcessorTest::new("split_abs").successful_split_command_line_test(
        &["mybazel", "build", ":mytarget"],
        cl("mybazel", &[], "build", &[":mytarget"]),
    );
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineWithUnaryStartupWithEquals
#[test]
fn split_command_line_with_unary_startup_with_equals() {
    OptionProcessorTest::new("split_unary_eq").successful_split_command_line_test(
        &["bazel", "--bazelrc=foo", "build", ":mytarget"],
        cl("bazel", &["--bazelrc=foo"], "build", &[":mytarget"]),
    );
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineWithUnaryStartupWithoutEquals
#[test]
fn split_command_line_with_unary_startup_without_equals() {
    OptionProcessorTest::new("split_unary_noeq").successful_split_command_line_test(
        &["bazel", "--bazelrc", "foo", "build", ":mytarget"],
        cl("bazel", &["--bazelrc=foo"], "build", &[":mytarget"]),
    );
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineWithIncompleteUnaryOption
#[test]
fn split_command_line_with_incomplete_unary_option() {
    OptionProcessorTest::new("split_incomplete").failed_split_command_line_test(
        &["bazel", "--bazelrc"],
        "Startup option '--bazelrc' expects a value.\n\
         Usage: '--bazelrc=somevalue' or '--bazelrc somevalue'.\n  \
         For more info, run 'bazel help startup_options'.",
    );
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineWithMultipleStartup
#[test]
fn split_command_line_with_multiple_startup() {
    OptionProcessorTest::new("split_multi_startup").successful_split_command_line_test(
        &[
            "bazel",
            "--bazelrc",
            "foo",
            "--ignore_all_rc_files",
            "build",
            ":mytarget",
        ],
        cl(
            "bazel",
            &["--bazelrc=foo", "--ignore_all_rc_files"],
            "build",
            &[":mytarget"],
        ),
    );
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineWithNoStartupArgs
#[test]
fn split_command_line_with_no_startup_args() {
    OptionProcessorTest::new("split_no_startup").successful_split_command_line_test(
        &["bazel", "build", ":mytarget"],
        cl("bazel", &[], "build", &[":mytarget"]),
    );
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineWithNoCommandArgs
#[test]
fn split_command_line_with_no_command_args() {
    OptionProcessorTest::new("split_no_cmd_args")
        .successful_split_command_line_test(&["bazel", "build"], cl("bazel", &[], "build", &[]));
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineWithBlazeHelp
#[test]
fn split_command_line_with_blaze_help() {
    let t = OptionProcessorTest::new("split_help");
    t.successful_split_command_line_test(&["bazel", "help"], cl("bazel", &[], "help", &[]));
    t.successful_split_command_line_test(&["bazel", "-h"], cl("bazel", &[], "-h", &[]));
    t.successful_split_command_line_test(&["bazel", "-help"], cl("bazel", &[], "-help", &[]));
    t.successful_split_command_line_test(&["bazel", "--help"], cl("bazel", &[], "--help", &[]));
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineWithBlazeVersion
#[test]
fn split_command_line_with_blaze_version() {
    OptionProcessorTest::new("split_version").successful_split_command_line_test(
        &["bazel", "version"],
        cl("bazel", &[], "version", &[]),
    );
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineWithMultipleCommandArgs
#[test]
fn split_command_line_with_multiple_command_args() {
    OptionProcessorTest::new("split_multi_cmd_args").successful_split_command_line_test(
        &["bazel", "build", "--foo", "-s", ":mytarget"],
        cl("bazel", &[], "build", &["--foo", "-s", ":mytarget"]),
    );
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineFailsWithDashDashInStartupArgs
#[test]
fn split_command_line_fails_with_dash_dash_in_startup_args() {
    OptionProcessorTest::new("split_dashdash_startup").failed_split_command_line_test(
        &["bazel", "--"],
        "Unknown startup option: '--'.\n  For more info, run 'bazel help startup_options'.",
    );
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineWithDashDash
#[test]
fn split_command_line_with_dash_dash() {
    OptionProcessorTest::new("split_dashdash").successful_split_command_line_test(
        &[
            "bazel",
            "--ignore_all_rc_files",
            "build",
            "--b",
            "--",
            ":mytarget",
        ],
        cl(
            "bazel",
            &["--ignore_all_rc_files"],
            "build",
            &["--b", "--", ":mytarget"],
        ),
    );
}

// Java: option_processor_test.cc OptionProcessorTest.TestDedupePathsOmitsInvalidPath
#[test]
fn test_dedupe_paths_omits_invalid_path() {
    let _t = OptionProcessorTest::new("dedupe_invalid");
    assert!(dedupe_blazerc_paths(&["foo".to_string()]).is_empty());
}

// Java: option_processor_test.cc OptionProcessorTest.TestDedupePathsOmitsEmptyPath
#[test]
fn test_dedupe_paths_omits_empty_path() {
    let _t = OptionProcessorTest::new("dedupe_empty");
    assert!(dedupe_blazerc_paths(&[String::new()]).is_empty());
}

// Java: option_processor_test.cc OptionProcessorTest.TestDedupePathsWithDifferentFiles
#[test]
fn test_dedupe_paths_with_different_files() {
    let t = OptionProcessorTest::new("dedupe_different");
    let foo_path = join(&t.workspace, "foo");
    let bar_path = join(&t.workspace, "bar");
    write_file("foo", &foo_path);
    write_file("bar", &bar_path);
    let input = vec![foo_path, bar_path];
    assert_eq!(input, dedupe_blazerc_paths(&input));
}

// Java: option_processor_test.cc OptionProcessorTest.TestDedupePathsWithSameFile
#[test]
fn test_dedupe_paths_with_same_file() {
    let t = OptionProcessorTest::new("dedupe_same");
    let foo_path = join(&t.workspace, "foo");
    write_file("foo", &foo_path);
    let input = vec![foo_path.clone(), foo_path.clone()];
    assert_eq!(vec![foo_path], dedupe_blazerc_paths(&input));
}

// Java: option_processor_test.cc OptionProcessorTest.TestDedupePathsWithRelativePath
#[test]
fn test_dedupe_paths_with_relative_path() {
    let t = OptionProcessorTest::new("dedupe_relative");
    let dir = join(&t.workspace, "dir");
    let foo_path = join(&dir, "foo");
    let relative_foo_path = join(&dir, "../dir/foo");
    mkdirs(&dir);
    write_file("foo", &foo_path);
    let input = vec![foo_path.clone(), relative_foo_path];
    assert_eq!(vec![foo_path], dedupe_blazerc_paths(&input));
}

// Java: option_processor_test.cc OptionProcessorTest.TestDedupePathsWithSymbolicLink
#[test]
fn test_dedupe_paths_with_symbolic_link() {
    let t = OptionProcessorTest::new("dedupe_symlink");
    let foo_path = join(&t.workspace, "foo");
    let sym_foo_path = join(&t.workspace, "sym_foo");
    write_file("foo", &foo_path);
    std::os::unix::fs::symlink(&foo_path, &sym_foo_path).unwrap();
    let input = vec![foo_path.clone(), sym_foo_path];
    assert_eq!(vec![foo_path], dedupe_blazerc_paths(&input));
}

// Java: option_processor_test.cc OptionProcessorTest.SplitCommandLineFailsWithDeprecatedOptionInStartupArgs
#[test]
fn split_command_line_fails_with_deprecated_option_in_startup_args() {
    OptionProcessorTest::new("split_deprecated").failed_split_command_line_test(
        &["bazel", "--nomaster_bazelrc"],
        "Unknown startup option: '--nomaster_bazelrc'.\n  For more info, run 'bazel help startup_options'.",
    );
}

// Java: src/main/cpp/util/file_posix.cc:403 blaze_util::CanReadFile
/// CanReadFile is `!IsDirectory(path) && access(path, R_OK) == 0`: it never opens the file.
/// Opening a FIFO for reading would block until a writer appears (review 2026-11-18, S4).
#[test]
fn can_read_file_uses_access_and_does_not_open_a_fifo() {
    unsafe extern "C" {
        fn mkfifo(pathname: *const std::ffi::c_char, mode: u32) -> std::ffi::c_int;
    }
    let t = OptionProcessorTest::new("can_read_file");
    let fifo = join(&t.workspace, "fifo.bazelrc");
    let c_fifo = std::ffi::CString::new(fifo.clone()).unwrap();
    // SAFETY: c_fifo is NUL-terminated and outlives the call.
    assert_eq!(0, unsafe { mkfifo(c_fifo.as_ptr(), 0o644) });
    // Nobody ever writes to the FIFO: an open(2) for reading would hang this test.
    assert!(can_read_file(&fifo));

    let file = join(&t.workspace, "file.bazelrc");
    write_file("startup --foo", &file);
    assert!(can_read_file(&file));
    // A directory is not a readable file, and neither is a missing path.
    assert!(!can_read_file(&t.workspace));
    assert!(!can_read_file(&join(&t.workspace, "missing.bazelrc")));
}
