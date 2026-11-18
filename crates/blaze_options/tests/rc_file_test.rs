//! Port of src/test/cpp/rc_file_test.cc.
//!
//! The C++ fixture sets $HOME and chdirs into `cwd`; here both go into an explicit
//! `ClientEnvironment`. Logged warnings (BAZEL_LOG(WARNING), captured from stderr in C++) are
//! read from `OptionProcessor::warnings()`. Startup option *values* (`max_idle_secs`,
//! `io_nice_level`, ...) are parsed by `StartupOptions`, which lives in the client crate; tests
//! check the ordered startup flags and the provenance message derived from them instead, and say
//! so where a value assertion was dropped.

mod rc_support;

use blaze_options::rc::option_processor::CommandLine;
use blaze_options::rc::option_processor::{
    ExitCode, OptionProcessor, RecordingStartupArgsProcessor, StartupFlagRegistry,
};
use blaze_options::rc::rc_file::bazel_version_matches_condition;
use blaze_options::rc::rc_file::{RcFile, make_canonical};
use blaze_options::rc::sem_ver::SemVer;
use rc_support::*;

/// Java: rc_file_test.cc RcFileTest fixture.
struct RcFileTest {
    workspace: String,
    cwd: String,
    home: String,
    binary_dir: String,
    binary_path: String,
    build_label: String,
}

impl RcFileTest {
    fn new(name: &str) -> Self {
        let tmp = fresh_tmpdir(name);
        let workspace = join(&tmp, "workspace");
        let cwd = join(&tmp, "cwd");
        let home = join(&tmp, "home");
        let binary_dir = join(&tmp, "bazeldir");
        for d in [&workspace, &cwd, &home, &binary_dir] {
            mkdirs(d);
        }
        let binary_path = join(&binary_dir, "bazel");
        RcFileTest {
            workspace,
            cwd,
            home,
            binary_dir,
            binary_path,
            build_label: String::new(),
        }
    }

    /// `new OptionProcessor(layout, BazelStartupOptions, "bazel.bazelrc")`.
    fn option_processor(&self) -> OptionProcessor {
        OptionProcessor::new(
            StartupFlagRegistry::bazel(),
            "bazel.bazelrc",
            &self.build_label,
            client_env(&self.cwd, &self.home),
        )
    }

    fn set_up_system_rc_file(&self, contents: &str) -> String {
        let p = join(&self.cwd, "bazel.bazelrc");
        write_file(contents, &p);
        make_canonical(&p)
    }

    fn set_up_workspace_rc_file(&self, contents: &str) -> String {
        let p = join(&self.workspace, ".bazelrc");
        write_file(contents, &p);
        make_canonical(&p)
    }

    fn set_up_home_rc_file(&self, contents: &str) -> String {
        let p = join(&self.home, ".bazelrc");
        write_file(contents, &p);
        make_canonical(&p)
    }

    fn set_up_legacy_master_rc_file_in_workspace(&self, contents: &str) -> String {
        let tools_dir = join(&self.workspace, "tools");
        mkdirs(&tools_dir);
        let p = join(&tools_dir, "bazel.rc");
        write_file(contents, &p);
        p
    }

    fn set_up_legacy_master_rc_file_alongside_binary(&self, contents: &str) -> String {
        let p = join(&self.binary_dir, "bazel.bazelrc");
        write_file(contents, &p);
        p
    }

    fn get_rc_files(
        &self,
        startup_args: &[&str],
    ) -> (OptionProcessor, Result<Vec<RcFile>, (ExitCode, String)>) {
        let mut op = self.option_processor();
        let cmd_line = CommandLine {
            path_to_binary: self.binary_path.clone(),
            startup_args: startup_args.iter().map(|s| s.to_string()).collect(),
            command: "build".to_string(),
            command_args: vec![],
        };
        let r = op.get_rc_files(&self.workspace, &self.cwd, &cmd_line);
        (op, r)
    }

    /// Java: ParseOptionsTest::ParseOptionsAndCheckOutput. Returns the processor for further
    /// checks; `expected_output_regex` is matched against the joined warnings ("WARNING: " lines).
    fn parse_options_and_check_output(
        &self,
        args: &[&str],
        expected_exit_code: ExitCode,
        expected_error_regex: &str,
        expected_output_regex: &str,
    ) -> OptionProcessor {
        let mut op = self.option_processor();
        let mut startup = RecordingStartupArgsProcessor::default();
        let result = op.parse_options(
            args.iter().map(|s| s.to_string()).collect(),
            &self.workspace,
            &self.cwd,
            &mut startup,
        );
        let (exit_code, error) = match result {
            Ok(()) => (ExitCode::Success, String::new()),
            Err((c, e)) => (c, e),
        };
        assert_eq!(expected_exit_code, exit_code, "{}", error);
        assert!(
            contains_regex(&error, expected_error_regex),
            "error: {}",
            error
        );
        let output: String = op
            .warnings()
            .iter()
            .map(|w| format!("WARNING: {}\n", w))
            .collect();
        assert!(
            contains_regex(&output, expected_output_regex),
            "output: {}",
            output
        );
        op
    }

    fn parse_options(&self, args: &[&str]) -> (OptionProcessor, Result<(), (ExitCode, String)>) {
        let mut op = self.option_processor();
        let mut startup = RecordingStartupArgsProcessor::default();
        let r = op.parse_options(
            args.iter().map(|s| s.to_string()).collect(),
            &self.workspace,
            &self.cwd,
            &mut startup,
        );
        (op, r)
    }
}

fn sources(rcs: &[RcFile]) -> Vec<Vec<String>> {
    rcs.iter()
        .map(|r| r.canonical_source_paths().to_vec())
        .collect()
}

// Java: rc_file_test.cc GetRcFileTest.GetRcFilesLoadsAllDefaultBazelrcs
#[test]
fn get_rc_files_loads_all_default_bazelrcs() {
    let t = RcFileTest::new("loads_all");
    let system_rc = t.set_up_system_rc_file("");
    let workspace_rc = t.set_up_workspace_rc_file("");
    let home_rc = t.set_up_home_rc_file("");
    let (_, r) = t.get_rc_files(&[]);
    let parsed = r.unwrap();
    assert_eq!(
        sources(&parsed),
        vec![vec![system_rc], vec![workspace_rc], vec![home_rc]]
    );
}

// Java: rc_file_test.cc GetRcFileTest.GetRcFilesRespectsNoSystemRc
#[test]
fn get_rc_files_respects_no_system_rc() {
    let t = RcFileTest::new("no_system");
    t.set_up_system_rc_file("");
    let workspace_rc = t.set_up_workspace_rc_file("");
    let home_rc = t.set_up_home_rc_file("");
    let (_, r) = t.get_rc_files(&["--nosystem_rc"]);
    assert_eq!(
        sources(&r.unwrap()),
        vec![vec![workspace_rc], vec![home_rc]]
    );
}

// Java: rc_file_test.cc GetRcFileTest.GetRcFilesRespectsNoWorkspaceRc
#[test]
fn get_rc_files_respects_no_workspace_rc() {
    let t = RcFileTest::new("no_workspace");
    let system_rc = t.set_up_system_rc_file("");
    t.set_up_workspace_rc_file("");
    let home_rc = t.set_up_home_rc_file("");
    let (_, r) = t.get_rc_files(&["--noworkspace_rc"]);
    assert_eq!(sources(&r.unwrap()), vec![vec![system_rc], vec![home_rc]]);
}

// Java: rc_file_test.cc GetRcFileTest.GetRcFilesRespectsNoWorkspaceRcAndNoSystemAndNoHomeRcCombined
#[test]
fn get_rc_files_respects_no_workspace_rc_and_no_system_and_no_home_rc_combined() {
    let t = RcFileTest::new("none");
    t.set_up_system_rc_file("");
    t.set_up_workspace_rc_file("");
    t.set_up_home_rc_file("");
    let (_, r) = t.get_rc_files(&["--noworkspace_rc", "--nosystem_rc", "--nohome_rc"]);
    assert!(r.unwrap().is_empty());
}

// Java: rc_file_test.cc GetRcFileTest.GetRcFilesWarnsAboutIgnoredMasterRcFiles
#[test]
fn get_rc_files_warns_about_ignored_master_rc_files() {
    let t = RcFileTest::new("master");
    let workspace_rc = t.set_up_legacy_master_rc_file_in_workspace("");
    let binary_rc = t.set_up_legacy_master_rc_file_alongside_binary("");
    let (op, r) = t.get_rc_files(&[]);
    assert!(r.is_ok());
    let output = op.warnings().join("\n");
    assert!(output.contains("The following rc files are no longer being read"));
    assert!(output.contains(&workspace_rc));
    assert!(output.contains(&binary_rc));
}

// Java: rc_file_test.cc GetRcFileTest.GetRcFilesReadsCommandLineRc
#[test]
fn get_rc_files_reads_command_line_rc() {
    let t = RcFileTest::new("cmdline_rc");
    let cmdline_rc_path = join(&t.workspace, "mybazelrc");
    write_file("", &cmdline_rc_path);
    let (_, r) = t.get_rc_files(&[&format!("--bazelrc={}", cmdline_rc_path)]);
    let parsed = r.unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].canonical_source_paths().len(), 1);
    assert!(parsed[0].canonical_source_paths()[0].contains("mybazelrc"));
}

// Java: rc_file_test.cc GetRcFileTest.GetRcFilesAcceptsNullCommandLineRc
#[test]
fn get_rc_files_accepts_null_command_line_rc() {
    let t = RcFileTest::new("null_rc");
    let (_, r) = t.get_rc_files(&[
        "--nosystem_rc",
        "--noworkspace_rc",
        "--nohome_rc",
        "--bazelrc=/dev/null",
    ]);
    // /dev/null is not an error, but it does technically count as a file.
    assert_eq!(sources(&r.unwrap()), vec![vec!["/dev/null".to_string()]]);
}

// Java: rc_file_test.cc ParseOptionsTest.IgnoreAllRcFilesIgnoresAllMasterAndUserRcFiles
#[test]
fn ignore_all_rc_files_ignores_all_master_and_user_rc_files() {
    let t = RcFileTest::new("ignore_all");
    t.set_up_workspace_rc_file("startup --workspacefoo");
    t.set_up_system_rc_file("startup --systemfoo");
    let cmdline_rc_path = join(&t.workspace, "mybazelrc");
    write_file("startup --myfoo", &cmdline_rc_path);
    let (op, r) = t.parse_options(&[&t.binary_path.clone(), "--ignore_all_rc_files", "build"]);
    assert!(r.is_ok());
    assert_eq!(provenance_message(op.rc_startup_flags()), "");
}

// Java: rc_file_test.cc ParseOptionsTest.LaterIgnoreAllRcFilesValueWins
// PORT: ported as `later_ignore_all_rc_files_value_wins` in
// crates/blaze_client/tests/rc_file_test.rs. It needs `StartupOptions` ("Unknown startup
// option" comes from StartupOptions::ProcessArgs), which lives in the client crate, and
// blaze_client depends on blaze_options.

// Java: rc_file_test.cc ParseOptionsTest.IgnoreAllRcFilesIgnoresCommandLineRcFileToo
#[test]
fn ignore_all_rc_files_ignores_command_line_rc_file_too() {
    let t = RcFileTest::new("ignore_cmdline");
    t.set_up_workspace_rc_file("startup --workspacefoo");
    t.set_up_system_rc_file("startup --systemfoo");
    let cmdline_rc_path = join(&t.workspace, "mybazelrc");
    write_file("startup --cmdlinefoo", &cmdline_rc_path);
    let bazelrc = format!("--bazelrc={}", cmdline_rc_path);
    let (op, r) = t.parse_options(&[
        &t.binary_path.clone(),
        "--ignore_all_rc_files",
        &bazelrc,
        "build",
    ]);
    assert!(r.is_ok());
    assert_eq!(provenance_message(op.rc_startup_flags()), "");
}

// Java: rc_file_test.cc ParseOptionsTest.CommandLineBazelrcHasUnknownOption
// PORT: ported as `command_line_bazelrc_has_unknown_option` in
// crates/blaze_client/tests/rc_file_test.rs. It needs `StartupOptions` ("Unknown startup
// option" comes from StartupOptions::ProcessArgs), which lives in the client crate, and
// blaze_client depends on blaze_options.

// Java: rc_file_test.cc ParseOptionsTest.BazelrcHasUnknownOption
// PORT: ported as `bazelrc_has_unknown_option` in
// crates/blaze_client/tests/rc_file_test.rs. It needs `StartupOptions` ("Unknown startup
// option" comes from StartupOptions::ProcessArgs), which lives in the client crate, and
// blaze_client depends on blaze_options.

// Java: rc_file_test.cc ParseOptionsTest.IncorrectWorkspaceBazelrcIgnoredWhenNoWorkspaceRcIsPresent
#[test]
fn incorrect_workspace_bazelrc_ignored_when_no_workspace_rc_is_present() {
    let t = RcFileTest::new("noworkspace_ignored");
    t.set_up_workspace_rc_file("startup --foo");
    let (op, r) = t.parse_options(&[&t.binary_path.clone(), "--noworkspace_rc", "build"]);
    assert!(r.is_ok());
    assert_eq!(provenance_message(op.rc_startup_flags()), "");
}

// Java: rc_file_test.cc ParseOptionsTest.PositiveOptionOverridesNegativeOption
#[test]
fn positive_option_overrides_negative_option() {
    let t = RcFileTest::new("positive_overrides");
    t.set_up_workspace_rc_file("startup --max_idle_secs=123");
    let op = t.parse_options_and_check_output(
        &["bazel", "--noworkspace_rc", "--workspace_rc", "build"],
        ExitCode::Success,
        "",
        "",
    );
    // PORT: EXPECT_EQ(123, max_idle_secs) needs StartupOptions (client crate).
    assert!(matches_regex(
        &provenance_message(op.rc_startup_flags()),
        "INFO: Reading 'startup' options from .*workspace.*bazelrc: --max_idle_secs=123\n"
    ));
}

// Java: rc_file_test.cc ParseOptionsTest.MultipleStartupArgsInMasterBazelrcWorksCorrectly
#[test]
fn multiple_startup_args_in_master_bazelrc_works_correctly() {
    let t = RcFileTest::new("multiple_startup");
    t.set_up_workspace_rc_file("startup --max_idle_secs=42\nstartup --io_nice_level=6");
    let op = t.parse_options_and_check_output(
        &[&t.binary_path.clone(), "build"],
        ExitCode::Success,
        "",
        "",
    );
    // PORT: max_idle_secs == 42 / io_nice_level == 6 need StartupOptions (client crate).
    assert!(matches_regex(
        &provenance_message(op.rc_startup_flags()),
        "INFO: Reading 'startup' options from .*workspace.*bazelrc: --max_idle_secs=42 --io_nice_level=6\n"
    ));
}

// Java: rc_file_test.cc ParseOptionsTest.CommandLineBazelrcHasPriorityOverDefaultBazelrc
#[test]
fn command_line_bazelrc_has_priority_over_default_bazelrc() {
    let t = RcFileTest::new("cmdline_priority");
    t.set_up_workspace_rc_file("startup --max_idle_secs=42\nstartup --io_nice_level=6");
    let cmdline_rc_path = join(&t.workspace, "mybazelrc");
    write_file("startup --max_idle_secs=123", &cmdline_rc_path);
    let bazelrc = format!("--bazelrc={}", cmdline_rc_path);
    let op =
        t.parse_options_and_check_output(&["bazel", &bazelrc, "build"], ExitCode::Success, "", "");
    // PORT: max_idle_secs == 123 / io_nice_level == 6 need StartupOptions (client crate).
    assert!(matches_regex(
        &provenance_message(op.rc_startup_flags()),
        "INFO: Reading 'startup' options from .*workspace.*bazelrc: --max_idle_secs=42 --io_nice_level=6\n\
         INFO: Reading 'startup' options from .*mybazelrc: --max_idle_secs=123\n"
    ));
}

// Java: rc_file_test.cc ParseOptionsTest.PlatformSpecificBazelrcOptions
#[test]
fn platform_specific_bazelrc_options() {
    let t = RcFileTest::new("platform_specific");
    t.set_up_workspace_rc_file(
        "startup --max_idle_secs=1\n\
         startup:linux --max_idle_secs=2\n\
         startup:macos --max_idle_secs=3\n\
         startup:windows --max_idle_secs=4\n\
         startup:freebsd --max_idle_secs=5\n\
         startup:openbsd --max_idle_secs=6\n",
    );
    let op = t.parse_options_and_check_output(
        &[&t.binary_path.clone(), "build"],
        ExitCode::Success,
        "",
        "",
    );
    // PORT: max_idle_secs == 2 needs StartupOptions (client crate); the flag order shows the
    // linux value is processed last.
    assert!(matches_regex(
        &provenance_message(op.rc_startup_flags()),
        "INFO: Reading 'startup' options from .*workspace.*bazelrc: --max_idle_secs=1 --max_idle_secs=2\n"
    ));
}

/// Java: BlazercImportTest::TestBazelRcImportsMaintainsFlagOrdering
fn test_bazel_rc_imports_maintains_flag_ordering(name: &str, import_type: &str) {
    let t = RcFileTest::new(name);
    let imported_rc_path = join(&t.workspace, "myimportedbazelrc");
    write_file(
        "startup --max_idle_secs=123\nstartup --io_nice_level=4",
        &imported_rc_path,
    );
    t.set_up_workspace_rc_file(&format!(
        "startup --max_idle_secs=42\n{} {}\nstartup --io_nice_level=6",
        import_type, imported_rc_path
    ));
    let op = t.parse_options_and_check_output(&["bazel", "build"], ExitCode::Success, "", "");
    // PORT: max_idle_secs == 123 / io_nice_level == 6 need StartupOptions (client crate).
    assert!(matches_regex(
        &provenance_message(op.rc_startup_flags()),
        "INFO: Reading 'startup' options from .*workspace.*bazelrc: --max_idle_secs=42\n\
         INFO: Reading 'startup' options from .*myimportedbazelrc: --max_idle_secs=123 --io_nice_level=4\n\
         INFO: Reading 'startup' options from .*workspace.*bazelrc: --io_nice_level=6\n"
    ));
}

/// Java: BlazercImportTest::TestThatDoubleImportsCauseAWarning
fn test_that_double_imports_cause_a_warning(name: &str, import_type: &str) {
    let t = RcFileTest::new(name);
    let imported_rc_path = join(&t.workspace, "myimportedbazelrc");
    write_file("", &imported_rc_path);
    t.set_up_workspace_rc_file(&format!(
        "{0} {1}\n{0} {1}\n",
        import_type, imported_rc_path
    ));
    t.parse_options_and_check_output(
        &["bazel", "build"],
        ExitCode::Success,
        "",
        "WARNING: Duplicate rc file: .*myimportedbazelrc is imported multiple times from .*workspace.*bazelrc\n",
    );
}

/// Java: BlazercImportTest::TestThatDoubleImportWithWorkspaceRelativeSyntaxCauseAWarning
fn test_that_double_import_with_workspace_relative_syntax_cause_a_warning(
    name: &str,
    import_type: &str,
) {
    let t = RcFileTest::new(name);
    let imported_rc_path = join(&t.workspace, "myimportedbazelrc");
    write_file("", &imported_rc_path);
    t.set_up_workspace_rc_file(&format!(
        "{0} {1}\n{0} %workspace%/myimportedbazelrc\n",
        import_type, imported_rc_path
    ));
    t.parse_options_and_check_output(
        &["bazel", "build"],
        ExitCode::Success,
        "",
        "WARNING: Duplicate rc file: .*myimportedbazelrc is imported multiple times from .*workspace.*bazelrc\n",
    );
}

/// Java: BlazercImportTest::TestThatDoubleImportWithExcessPathSyntaxCauseAWarning
fn test_that_double_import_with_excess_path_syntax_cause_a_warning(name: &str, import_type: &str) {
    let t = RcFileTest::new(name);
    let imported_rc_path = join(&t.workspace, "myimportedbazelrc");
    write_file("", &imported_rc_path);
    t.set_up_workspace_rc_file(&format!(
        "{0} {1}\n{0} %workspace%///.//myimportedbazelrc\n",
        import_type, imported_rc_path
    ));
    t.parse_options_and_check_output(
        &["bazel", "build"],
        ExitCode::Success,
        "",
        "WARNING: Duplicate rc file: .*myimportedbazelrc is imported multiple times from .*workspace.*bazelrc\n",
    );
}

/// Java: BlazercImportTest::TestThatDeepDoubleImportCausesAWarning
fn test_that_deep_double_import_causes_a_warning(name: &str, import_type: &str) {
    let t = RcFileTest::new(name);
    let dual_imported_rc_path = join(&t.workspace, "dual_imported.bazelrc");
    write_file("", &dual_imported_rc_path);
    let intermediate_import_1 = join(&t.workspace, "intermediate_import_1");
    write_file(
        &format!("{} {}", import_type, dual_imported_rc_path),
        &intermediate_import_1,
    );
    let intermediate_import_2 = join(&t.workspace, "intermediate_import_2");
    write_file(
        &format!("{} {}", import_type, dual_imported_rc_path),
        &intermediate_import_2,
    );
    t.set_up_workspace_rc_file(&format!(
        "{0} {1}\n{0} {2}\n",
        import_type, intermediate_import_1, intermediate_import_2
    ));
    t.parse_options_and_check_output(
        &["bazel", "build"],
        ExitCode::Success,
        "",
        "WARNING: Duplicate rc file: .*dual_imported.bazelrc is imported multiple times from .*workspace.*bazelrc\n",
    );
}

/// Java: BlazercImportTest::TestThatImportingAFileAndPassingItInCausesAWarning
fn test_that_importing_a_file_and_passing_it_in_causes_a_warning(name: &str, import_type: &str) {
    let t = RcFileTest::new(name);
    let imported_rc_path = join(&t.workspace, "myimportedbazelrc");
    write_file("", &imported_rc_path);
    t.set_up_workspace_rc_file(&format!("{} {}", import_type, imported_rc_path));
    let bazelrc = format!("--bazelrc={}", imported_rc_path);
    t.parse_options_and_check_output(
        &["bazel", &bazelrc, "build"],
        ExitCode::Success,
        "",
        "WARNING: Duplicate rc file: .*myimportedbazelrc is read multiple times, it is a standard rc file location but must have been unnecessarily imported earlier.\n",
    );
}

// Java: rc_file_test.cc BlazercImportTest.BazelRcImportFailsForMissingFile
#[test]
fn bazel_rc_import_fails_for_missing_file() {
    let t = RcFileTest::new("import_missing");
    let missing = join(&t.workspace, "myimportedbazelrc");
    t.set_up_workspace_rc_file(&format!("import {}", missing));
    t.parse_options_and_check_output(
        &["bazel", "build"],
        ExitCode::InternalError,
        "Unexpected error reading config file '.*myimportedbazelrc'",
        "",
    );
}

// Java: rc_file_test.cc BlazercImportTest.BazelRcTryImportDoesNotFailForMissingFile
#[test]
fn bazel_rc_try_import_does_not_fail_for_missing_file() {
    let t = RcFileTest::new("try_import_missing");
    let missing = join(&t.workspace, "tryimported.bazelrc");
    t.set_up_workspace_rc_file(&format!("try-import {}", missing));
    t.parse_options_and_check_output(&["bazel", "build"], ExitCode::Success, "", "");
}

// Java: rc_file_test.cc BlazercImportTest.BazelRcTryImportDoesNotFailForUnreadableFile
#[test]
fn bazel_rc_try_import_does_not_fail_for_unreadable_file() {
    let t = RcFileTest::new("try_import_unreadable");
    let unreadable_rc_path = join(&t.workspace, "tryimported.bazelrc");
    write_file_mode("startup --max_idle_secs=123", &unreadable_rc_path, 0o222);
    t.set_up_workspace_rc_file(&format!("try-import {}", unreadable_rc_path));
    t.parse_options_and_check_output(&["bazel", "build"], ExitCode::Success, "", "");
}

// Java: rc_file_test.cc BlazercImportTest.BazelRcImportDoesNotFallBackToLiteralPlaceholder
#[test]
fn bazel_rc_import_does_not_fall_back_to_literal_placeholder() {
    let t = RcFileTest::new("literal_placeholder");
    let literal = join(&t.cwd, "%workspace%/tryimported.bazelrc");
    mkdirs(&join(&t.cwd, "%workspace%"));
    write_file("import syntax error", &literal);
    t.set_up_workspace_rc_file("import %workspace%/tryimported.bazelrc");
    t.parse_options_and_check_output(
        &["bazel", "build"],
        ExitCode::BadArgv,
        "Nonexistent path in import declaration in config file.*'import %workspace%/tryimported.bazelrc'",
        "",
    );
}

// Java: rc_file_test.cc BlazercImportTest.BazelRcTryImportDoesNotFallBackToLiteralPlaceholder
#[test]
fn bazel_rc_try_import_does_not_fall_back_to_literal_placeholder() {
    let t = RcFileTest::new("try_literal_placeholder");
    let literal = join(&t.cwd, "%workspace%/tryimported.bazelrc");
    mkdirs(&join(&t.cwd, "%workspace%"));
    write_file("import syntax error", &literal);
    t.set_up_workspace_rc_file("try-import %workspace%/tryimported.bazelrc");
    t.parse_options_and_check_output(&["bazel", "build"], ExitCode::Success, "", "");
}

fn try_import_version_test(name: &str, build_label: &str) -> Vec<String> {
    let mut t = RcFileTest::new(name);
    let imported_rc_path = join(&t.workspace, "myimportedbazelrc");
    write_file("startup --max_idle_secs=123\n", &imported_rc_path);
    t.build_label = build_label.to_string();
    t.set_up_workspace_rc_file(&format!(
        "startup --max_idle_secs=42\ntry-import-if-bazel-version >=8.4.2 {}\n",
        imported_rc_path
    ));
    let op = t.parse_options_and_check_output(&["bazel", "build"], ExitCode::Success, "", "");
    op.rc_startup_flags()
        .iter()
        .map(|f| f.value.clone())
        .collect()
}

// Java: rc_file_test.cc BlazercImportTest.SuccessfulTryImportIfBazelVersion
#[test]
fn successful_try_import_if_bazel_version() {
    // PORT: the C++ checks max_idle_secs == 123 (StartupOptions); the import's flag comes last.
    assert_eq!(
        try_import_version_test("try_version_ok", "8.4.2"),
        vec!["--max_idle_secs=42", "--max_idle_secs=123"]
    );
}

// Java: rc_file_test.cc BlazercImportTest.TryImportVersionSkippedIfBuildVersionInvalid
#[test]
fn try_import_version_skipped_if_build_version_invalid() {
    // On dev builds, no_version is the build label. The import is not processed.
    assert_eq!(
        try_import_version_test("try_version_invalid", "no_version"),
        vec!["--max_idle_secs=42"]
    );
}

// Java: rc_file_test.cc BlazercImportTest.TryImportVersionSkippedIfComparisonFails
#[test]
fn try_import_version_skipped_if_comparison_fails() {
    assert_eq!(
        try_import_version_test("try_version_old", "5.4.0"),
        vec!["--max_idle_secs=42"]
    );
}

// Java: rc_file_test.cc BlazercImportTest.TryImportVersionSkippedIfFileDoesNotExist
#[test]
fn try_import_version_skipped_if_file_does_not_exist() {
    let mut t = RcFileTest::new("try_version_missing");
    t.build_label = "8.4.2".to_string();
    t.set_up_workspace_rc_file(
        "startup --max_idle_secs=42\ntry-import-if-bazel-version >=8.4.2 non-existent/file.rc\n",
    );
    t.parse_options_and_check_output(&["bazel", "build"], ExitCode::Success, "", "");
}

// Java: rc_file_test.cc BlazercImportTest.TryImportVersionInvalidCondition
#[test]
fn try_import_version_invalid_condition() {
    let mut t = RcFileTest::new("try_version_bad_cond");
    t.build_label = "8.4.2".to_string();
    t.set_up_workspace_rc_file("try-import-if-bazel-version ^8.4.2 /unused/path\n");
    t.parse_options_and_check_output(
        &["bazel", "build"],
        ExitCode::BadArgv,
        "Invalid version condition in config file.*Condition '\\^8.4.2'",
        "",
    );
}

// Java: rc_file_test.cc BlazercImportTest.TryImportVersionInvalidConditionSemanticVersion
#[test]
fn try_import_version_invalid_condition_semantic_version() {
    let mut t = RcFileTest::new("try_version_bad_semver");
    t.build_label = "8.4.2".to_string();
    t.set_up_workspace_rc_file("try-import-if-bazel-version ==no_version /unused/path\n");
    t.parse_options_and_check_output(
        &["bazel", "build"],
        ExitCode::BadArgv,
        "Invalid import declaration in config file.*Could not parse version 'no_version' as a valid semantic version",
        "",
    );
}

// Java: rc_file_test.cc BlazercImportTest.BazelRcImportsMaintainsFlagOrdering
#[test]
fn bazel_rc_imports_maintains_flag_ordering() {
    test_bazel_rc_imports_maintains_flag_ordering("import_ordering", "import");
}

// Java: rc_file_test.cc BlazercImportTest.BazelRcTryImportsMaintainsFlagOrdering
#[test]
fn bazel_rc_try_imports_maintains_flag_ordering() {
    test_bazel_rc_imports_maintains_flag_ordering("try_import_ordering", "try-import");
}

// Java: rc_file_test.cc BlazercImportTest.DoubleImportsCauseAWarning
#[test]
fn double_imports_cause_a_warning() {
    test_that_double_imports_cause_a_warning("double_import", "import");
}

// Java: rc_file_test.cc BlazercImportTest.DoubleTryImportsCauseAWarning
#[test]
fn double_try_imports_cause_a_warning() {
    test_that_double_imports_cause_a_warning("double_try_import", "try-import");
}

// Java: rc_file_test.cc BlazercImportTest.DoubleImportWithWorkspaceRelativeSyntaxCauseAWarning
#[test]
fn double_import_with_workspace_relative_syntax_cause_a_warning() {
    test_that_double_import_with_workspace_relative_syntax_cause_a_warning(
        "double_ws_import",
        "import",
    );
}

// Java: rc_file_test.cc BlazercImportTest.DoubleTryImportWithWorkspaceRelativeSyntaxCauseAWarning
#[test]
fn double_try_import_with_workspace_relative_syntax_cause_a_warning() {
    test_that_double_import_with_workspace_relative_syntax_cause_a_warning(
        "double_ws_try_import",
        "try-import",
    );
}

// Java: rc_file_test.cc BlazercImportTest.DoubleImportWithExcessPathSyntaxCauseAWarning
#[test]
fn double_import_with_excess_path_syntax_cause_a_warning() {
    test_that_double_import_with_excess_path_syntax_cause_a_warning(
        "double_excess_import",
        "import",
    );
}

// Java: rc_file_test.cc BlazercImportTest.DoubleTryImportWithExcessPathSyntaxCauseAWarning
#[test]
fn double_try_import_with_excess_path_syntax_cause_a_warning() {
    test_that_double_import_with_excess_path_syntax_cause_a_warning(
        "double_excess_try_import",
        "try-import",
    );
}

// Java: rc_file_test.cc BlazercImportTest.DoubleImportWithEnclosingDirectorySyntaxCauseAWarning
#[test]
fn double_import_with_enclosing_directory_syntax_cause_a_warning() {
    let t = RcFileTest::new("double_enclosing");
    let imported_rc_path = join(&t.workspace, "myimportedbazelrc");
    write_file("", &imported_rc_path);
    mkdirs(&join(&t.workspace, "extra"));
    t.set_up_workspace_rc_file(&format!(
        "import {}\nimport %workspace%/extra/../myimportedbazelrc\n",
        imported_rc_path
    ));
    t.parse_options_and_check_output(
        &["bazel", "build"],
        ExitCode::Success,
        "",
        "WARNING: Duplicate rc file: .*myimportedbazelrc is imported multiple times from .*workspace.*bazelrc\n",
    );
}

// Java: rc_file_test.cc BlazercImportTest.DeepDoubleImportCausesAWarning
#[test]
fn deep_double_import_causes_a_warning() {
    test_that_deep_double_import_causes_a_warning("deep_double_import", "import");
}

// Java: rc_file_test.cc BlazercImportTest.DeepDoubleTryImportCausesAWarning
#[test]
fn deep_double_try_import_causes_a_warning() {
    test_that_deep_double_import_causes_a_warning("deep_double_try_import", "try-import");
}

// Java: rc_file_test.cc BlazercImportTest.ImportingAFileAndPassingItInCausesAWarning
#[test]
fn importing_a_file_and_passing_it_in_causes_a_warning() {
    test_that_importing_a_file_and_passing_it_in_causes_a_warning("import_and_pass", "import");
}

// Java: rc_file_test.cc BlazercImportTest.TryImportingAFileAndPassingItInCausesAWarning
#[test]
fn try_importing_a_file_and_passing_it_in_causes_a_warning() {
    test_that_importing_a_file_and_passing_it_in_causes_a_warning(
        "try_import_and_pass",
        "try-import",
    );
}

// Java: rc_file_test.cc ParseOptionsTest.ImportingAPreviouslyLoadedStandardRcCausesAWarning
#[test]
fn importing_a_previously_loaded_standard_rc_causes_a_warning() {
    let t = RcFileTest::new("import_prev_standard");
    let system_rc = t.set_up_system_rc_file("");
    t.set_up_workspace_rc_file(&format!("import {}", system_rc));
    t.parse_options_and_check_output(
        &["bazel", "build"],
        ExitCode::Success,
        "",
        "WARNING: Duplicate rc file: .*bazel.bazelrc is read multiple times, most recently imported from .*workspace.*bazelrc\n",
    );
}

// Java: rc_file_test.cc ParseOptionsTest.ImportingStandardRcBeforeItIsLoadedCausesAWarning
#[test]
fn importing_standard_rc_before_it_is_loaded_causes_a_warning() {
    let t = RcFileTest::new("import_standard_before");
    let workspace_rc = t.set_up_workspace_rc_file("");
    t.set_up_system_rc_file(&format!("import {}", workspace_rc));
    t.parse_options_and_check_output(
        &["bazel", "build"],
        ExitCode::Success,
        "",
        "WARNING: Duplicate rc file: .*workspace.*bazelrc is read multiple times, it is a standard rc file location but must have been unnecessarily imported earlier.\n",
    );
}

fn matches(build_label: &str, op: &str, v: &str) -> Result<bool, String> {
    bazel_version_matches_condition(&SemVer::parse(build_label).unwrap(), op, v)
}

// Java: rc_file_test.cc BazelVersionMatchesCondition_Tilde.VersionFullSemanticVersion
#[test]
fn version_full_semantic_version() {
    // ~1.2.3 => >=1.2.3 <1.3.0
    assert_eq!(matches("1.2.4", "~", "1.2.3"), Ok(true));
    assert_eq!(matches("1.2.3-prerelease", "~", "1.2.3"), Ok(false));
    assert_eq!(matches("1.2.2", "~", "1.2.3"), Ok(false));
    assert_eq!(matches("1.3.0", "~", "1.2.3"), Ok(false));
}

// Java: rc_file_test.cc BazelVersionMatchesCondition_Tilde.VersionMajorMinorVersion
#[test]
fn version_major_minor_version() {
    // ~1.2 => >=1.2.0 <1.3.0
    assert_eq!(matches("1.2.4", "~", "1.2"), Ok(true));
    assert_eq!(matches("1.2.0", "~", "1.2"), Ok(true));
    assert_eq!(matches("1.1.9", "~", "1.2"), Ok(false));
    assert_eq!(matches("1.3.0", "~", "1.2"), Ok(false));
}

// Java: rc_file_test.cc BazelVersionMatchesCondition_Tilde.VersionMajorVersion
#[test]
fn version_major_version() {
    // ~1 => >=1.0.0 <2.0.0
    assert_eq!(matches("1.0.0", "~", "1"), Ok(true));
    assert_eq!(matches("1.2.4", "~", "1"), Ok(true));
    assert_eq!(matches("1.9.9", "~", "1"), Ok(true));
    assert_eq!(matches("2.0.0", "~", "1"), Ok(false));
}

// Java: rc_file_test.cc BazelVersionMatchesCondition_Tilde.VersionPrerelease
#[test]
fn version_prerelease() {
    // ~1.2.3-beta.2 => >=1.2.3-beta.2 <1.3.0
    let v = "1.2.3-beta.2";
    assert_eq!(matches("1.2.3-beta.2", "~", v), Ok(true));
    assert_eq!(matches("1.2.3-beta.3", "~", v), Ok(true));
    assert_eq!(matches("1.2.3-alpha.42", "~", v), Ok(false));
    assert_eq!(matches("1.3.0", "~", v), Ok(false));
}

// Java: rc_file_test.cc BazelVersionMatchesCondition_Tilde.VersionInvalid
#[test]
fn version_invalid() {
    let e = matches("1.2.3-beta.2", "~", "v1.2.3").unwrap_err();
    assert!(e.contains("Could not parse the tilde range version"));
}

// Java: rc_file_test.cc BazelVersionMatchesCondition.AllComparisonOperators
#[test]
fn all_comparison_operators() {
    let b = "8.4.2";
    assert_eq!(matches(b, ">", "8.4.0"), Ok(true));
    assert_eq!(matches(b, ">", "8.5.0"), Ok(false));
    assert_eq!(matches(b, ">=", "8.4.2"), Ok(true));
    assert_eq!(matches(b, ">=", "8.5.0"), Ok(false));
    assert_eq!(matches(b, "<", "8.5.0"), Ok(true));
    assert_eq!(matches(b, "<", "1.0.0"), Ok(false));
    assert_eq!(matches(b, "<=", "8.4.2"), Ok(true));
    assert_eq!(matches(b, "<=", "8.4.1"), Ok(false));
    assert_eq!(matches(b, "==", "8.4.2"), Ok(true));
    assert_eq!(matches(b, "==", "8.4.2-prerelease"), Ok(false));
    assert_eq!(matches(b, "!=", "8.4.2-prerelease"), Ok(true));
    assert_eq!(matches(b, "!=", "8.4.2"), Ok(false));
}

// Java: rc_file_test.cc BazelVersionMatchesCondition.InvalidCompareVersion
#[test]
fn invalid_compare_version() {
    let e = matches("8.4.2", ">", "v8.4.2").unwrap_err();
    assert!(e.contains("Could not parse version 'v8.4.2' as a valid semantic version."));
}

// Java: src/test/cpp/rc_file_test.cc BlazercImportTest.BazelRcTryImportDoesNotFailForInvalidPosixPathOnWindows
#[test]
#[ignore = "Windows-only test (PORT: skipped, windows only)"]
fn bazel_rc_try_import_does_not_fail_for_invalid_posix_path_on_windows() {}
