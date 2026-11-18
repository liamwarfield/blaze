//! The tests of src/test/cpp/rc_file_test.cc that need `BazelStartupOptions` ("Unknown startup
//! option" comes from `StartupOptions::ProcessArgs`). The rest of rc_file_test.cc is ported in
//! crates/blaze_options/tests/rc_file_test.rs, which cannot use the client crate's
//! `StartupOptions`.
//!
//! As there, the C++ fixture's `$HOME` and working directory go into an explicit
//! `ClientEnvironment` instead of the process state.

use blaze_client::option_processor::startup_options_provenance_message;
use blaze_client::startup_options::StartupOptions;
use blaze_client::util::file::{make_canonical, make_directories, write_file};
use blaze_client::util::path::join_path;
use blaze_options::rc::option_processor::{
    ClientEnvironment, ExitCode, OptionProcessor, StartupFlagRegistry,
};

/// Java: rc_file_test.cc RcFileTest fixture.
struct RcFileTest {
    _temp: tempfile::TempDir,
    workspace: String,
    cwd: String,
    home: String,
    binary_path: String,
}

impl RcFileTest {
    // Java: src/test/cpp/rc_file_test.cc:76 RcFileTest::SetUp
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let tmp = make_canonical(temp.path().to_str().unwrap());
        assert!(!tmp.is_empty());
        let workspace = join_path(&tmp, "workspace");
        let cwd = join_path(&tmp, "cwd");
        let home = join_path(&tmp, "home");
        let binary_dir = join_path(&tmp, "bazeldir");
        for dir in [&workspace, &cwd, &home, &binary_dir] {
            assert!(make_directories(dir, 0o755));
        }
        let binary_path = join_path(&binary_dir, "bazel");
        RcFileTest {
            _temp: temp,
            workspace,
            cwd,
            home,
            binary_path,
        }
    }

    /// `new OptionProcessor(layout, BazelStartupOptions, "bazel.bazelrc")`.
    fn option_processor(&self) -> OptionProcessor {
        OptionProcessor::new(
            StartupFlagRegistry::bazel(),
            "bazel.bazelrc",
            "",
            ClientEnvironment {
                cwd: self.cwd.clone(),
                home: self.home.clone(),
                ..Default::default()
            },
        )
    }

    // Java: src/test/cpp/rc_file_test.cc:119 RcFileTest::SetUpWorkspaceRcFile
    fn set_up_workspace_rc_file(&self, contents: &str) -> String {
        let workspace_user_rc_path = join_path(&self.workspace, ".bazelrc");
        assert!(write_file(
            contents.as_bytes(),
            &workspace_user_rc_path,
            0o755
        ));
        make_canonical(&workspace_user_rc_path)
    }

    /// `option_processor_->ParseOptions(args, workspace_, cwd_, &error)` followed by
    /// `PrintStartupOptionsProvenanceMessage()`: `(exit_code, error, stderr output)`.
    fn parse_options(&self, args: &[&str]) -> (ExitCode, String, String) {
        let mut option_processor = self.option_processor();
        let mut startup_options = StartupOptions::bazel();
        let result = option_processor.parse_options(
            args.iter().map(|s| s.to_string()).collect(),
            &self.workspace,
            &self.cwd,
            &mut startup_options,
        );
        let (exit_code, error) = match result {
            Ok(()) => (ExitCode::Success, String::new()),
            Err((exit_code, error)) => (exit_code, error),
        };
        (
            exit_code,
            error,
            startup_options_provenance_message(&startup_options),
        )
    }
}

/// gmock `MatchesRegex(prefix + ".*" + infix + ".*" + suffix)` (or `prefix + ".*" + suffix` when
/// `infix` is empty) for the single-line patterns of these tests.
fn matches(output: &str, prefix: &str, infix: &str, suffix: &str) -> bool {
    if output.len() < prefix.len() + suffix.len()
        || !output.starts_with(prefix)
        || !output.ends_with(suffix)
    {
        return false;
    }
    let middle = &output[prefix.len()..output.len() - suffix.len()];
    !middle.contains('\n') && middle.contains(infix)
}

// Java: rc_file_test.cc ParseOptionsTest.LaterIgnoreAllRcFilesValueWins
#[test]
fn later_ignore_all_rc_files_value_wins() {
    let t = RcFileTest::new();
    t.set_up_workspace_rc_file("startup --workspacefoo");

    let (exit_code, error, output) = t.parse_options(&[
        &t.binary_path,
        "--ignore_all_rc_files",
        "--noignore_all_rc_files",
        "build",
    ]);
    assert_eq!(ExitCode::BadArgv, exit_code);
    assert_eq!(
        "Unknown startup option: '--workspacefoo'.\n  For more info, run 'bazel help \
         startup_options'.",
        error
    );

    // Check that the startup options' provenance message contains the provenance of the
    // incorrect option.
    assert!(
        matches(
            &output,
            "INFO: Reading 'startup' options from ",
            "workspace",
            "bazelrc: --workspacefoo\n"
        ),
        "{}",
        output
    );
}

// Java: rc_file_test.cc ParseOptionsTest.CommandLineBazelrcHasUnknownOption
#[test]
fn command_line_bazelrc_has_unknown_option() {
    let t = RcFileTest::new();
    let cmdline_rc_path = join_path(&t.workspace, "mybazelrc");
    assert!(write_file(b"startup --foo", &cmdline_rc_path, 0o755));

    let bazelrc = format!("--bazelrc={}", cmdline_rc_path);
    let (exit_code, error, output) = t.parse_options(&[&t.binary_path, &bazelrc, "build"]);
    assert_ne!(ExitCode::Success, exit_code, "{}", error);
    assert_eq!(
        "Unknown startup option: '--foo'.\n  For more info, run 'bazel help startup_options'.",
        error
    );

    // Check that the startup options' provenance message contains the correct information for
    // the incorrect flag, and does not print the command-line provided startup flags.
    assert!(
        matches(
            &output,
            "INFO: Reading 'startup' options from ",
            "",
            "mybazelrc: --foo\n"
        ),
        "{}",
        output
    );
}

// Java: rc_file_test.cc ParseOptionsTest.BazelrcHasUnknownOption
#[test]
fn bazelrc_has_unknown_option() {
    let t = RcFileTest::new();
    t.set_up_workspace_rc_file("startup --foo");

    let (exit_code, error, output) = t.parse_options(&[&t.binary_path, "build"]);
    assert_eq!(ExitCode::BadArgv, exit_code);
    assert_eq!(
        "Unknown startup option: '--foo'.\n  For more info, run 'bazel help startup_options'.",
        error
    );

    // Check that the startup options' provenance message contains nothing for the master
    // bazelrc.
    assert!(
        matches(
            &output,
            "INFO: Reading 'startup' options from ",
            "workspace",
            "bazelrc: --foo\n"
        ),
        "{}",
        output
    );
}
