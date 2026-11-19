//! Port of
//! `src/test/java/com/google/devtools/build/lib/bazel/rules/genrule/GenRuleCommandSubstitutionTest.java`:
//! the label and "Make"-variable substitutions applied to the genrule `cmd` attribute.

use crate::testing::{BuildViewTestCase, get_files_to_build};
use blaze_actions::actions::action::CommandAction;
use blaze_actions::analysis::actions::spawn_action::SpawnAction;

/// The test case plus the counter of `genrule(command)` (Java's `seq` field).
struct GenRuleCommandSubstitutionTest {
    t: BuildViewTestCase,
    seq: usize,
}

impl GenRuleCommandSubstitutionTest {
    fn new() -> Self {
        GenRuleCommandSubstitutionTest {
            t: BuildViewTestCase::new(),
            seq: 0,
        }
    }

    // Java: GenRuleCommandSubstitutionTest.getGenruleCommand
    fn get_genrule_command(&mut self, genrule: &str) -> String {
        let ct = self
            .t
            .get_configured_target(genrule)
            .unwrap_or_else(|| panic!("no configured target {genrule}"));
        let artifact = get_files_to_build(&ct).to_list()[0].clone();
        let action = self
            .t
            .get_generating_action(&artifact)
            .expect("generating action");
        let spawn_action = action
            .as_any()
            .downcast_ref::<SpawnAction>()
            .expect("ClassCastException: not a SpawnAction");
        spawn_action.get_arguments().unwrap()[2].clone()
    }

    // Java: GenRuleCommandSubstitutionTest.assertExpansionEquals
    fn assert_expansion_equals(&mut self, expected: &str, genrule: &str) {
        let command = self.get_genrule_command(genrule);
        assert_command_equals(expected, &command);
    }

    // Java: GenRuleCommandSubstitutionTest.assertExpansionFails
    fn assert_expansion_fails(&mut self, expected_error_suffix: &str, genrule: &str) {
        self.t.remove_fail_fast_handler(); // we expect errors
        self.t.event_collector.clear();
        self.t.get_configured_target(genrule);
        self.t.assert_contains_event(expected_error_suffix);
    }

    /// Creates a BUILD file defining a genrule called "//test" with no srcs or deps, one output
    /// and the specified command.
    // Java: GenRuleCommandSubstitutionTest.genrule
    fn genrule(&mut self, command: &str) {
        // This is a horrible workaround for b/147306893:
        // somehow, duplicate events (same location, same message)
        // are being suppressed, so we must vary the location of the
        // genrule by inserting a unique number of newlines.
        let newlines = "\n".repeat(self.seq);
        self.seq += 1;
        let cmd_line = format!("        cmd = '{command}')");
        self.t.scratch.overwrite_file(
            "test/BUILD",
            &[
                &newlines,
                "genrule(name = 'test',",
                "        outs = ['out'],",
                &cmd_line,
            ],
        );

        // Since we're probably re-defining "//test":
        self.t.invalidate_packages();
    }

    /// `targetConfig.getMakeVariableDefault("GENDIR")`.
    fn gendir(&mut self) -> String {
        self.t
            .get_target_configuration()
            .get_global_make_environment()
            .get("GENDIR")
            .cloned()
            .expect("GENDIR")
    }
}

/// `SETUP_COMMAND_PATTERN` (`.*/genrule-setup.sh;\s+(?<command>.*)`) with `Matcher.matches()`:
/// the `command` group, or `None` when the whole command does not match. Java's `.` matches
/// anything but a line terminator; the greedy `.*` prefix takes the last occurrence.
fn setup_command_group(command: &str) -> Option<&str> {
    const SETUP: &str = "/genrule-setup.sh;";
    let is_line_terminator = |c: char| c == '\n' || c == '\r';
    let is_java_whitespace = |c: char| matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r');
    for (i, _) in command.rmatch_indices(SETUP) {
        if command[..i].contains(is_line_terminator) {
            continue;
        }
        let after = &command[i + SETUP.len()..];
        if !after.starts_with(is_java_whitespace) {
            continue;
        }
        let group = after.trim_start_matches(is_java_whitespace);
        if group.contains(is_line_terminator) {
            continue;
        }
        return Some(group);
    }
    None
}

// Java: GenRuleCommandSubstitutionTest.assertCommandEquals
fn assert_command_equals(expected: &str, command: &str) {
    // Ensure the command after the genrule setup is correct.
    let command = setup_command_group(command).unwrap_or(command);
    assert_eq!(
        command, expected,
        "Expected command to be \"{expected}\", but found \"{command}\""
    );
}

// Java: GenRuleCommandSubstitutionTest.getBuildFileWithCommand
fn get_build_file_with_command(command: &str) -> String {
    [
        "genrule(name = 'test',".to_string(),
        "        outs = ['out'],".to_string(),
        format!("        cmd = '{command}')"),
    ]
    .join("\n")
}

// Java: GenRuleCommandSubstitutionTest.testLocationSyntaxErrors
#[test]
fn test_location_syntax_errors() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.genrule("$(location )");
    f.assert_expansion_fails(
        "invalid label in $(location) expression: invalid target name '': empty target name",
        "//test",
    );

    f.genrule("$(location foo bar");
    f.assert_expansion_fails("unterminated variable reference", "//test");

    f.genrule("$(location");
    f.assert_expansion_fails("unterminated variable reference", "//test");

    f.genrule("$(locationz");
    f.assert_expansion_fails("unterminated variable reference", "//test");

    f.genrule("$(locationz)");
    f.assert_expansion_fails("$(locationz) not defined", "//test");

    f.genrule("$(locationz )");
    f.assert_expansion_fails("$(locationz) not defined", "//test");

    f.genrule("$(locationz foo )");
    f.assert_expansion_fails("$(locationz) not defined", "//test");
}

// Java: GenRuleCommandSubstitutionTest.testLocationOfLabelThatIsNotAPrerequsite
#[test]
fn test_location_of_label_that_is_not_a_prerequsite() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.scratch.file(
        "test/BUILD",
        &[r#"exports_files(["exists"])

genrule(
    name = "test1",
    outs = ["test1.out"],
    cmd = "$(location :exists)",
)

genrule(
    name = "test2",
    outs = ["test2.out"],
    cmd = "$(location :doesnt_exist)",
)
"#],
    );

    // $(location) of a non-prerequisite fails, even if the target exists:

    f.assert_expansion_fails(
        "label '//test:exists' in $(location) expression is \
         not a declared prerequisite of this rule",
        "//test:test1",
    );

    f.assert_expansion_fails(
        "label '//test:doesnt_exist' in $(location) expression is \
         not a declared prerequisite of this rule",
        "//test:test2",
    );
}

// Java: GenRuleCommandSubstitutionTest.testLocationOfMultiFileLabel
#[test]
fn test_location_of_multi_file_label() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.scratch.file(
        "deuce/BUILD",
        &[r#"genrule(
    name = "deuce",
    outs = [
        "out.1",
        "out.2",
    ],
    cmd = ":",
)
"#],
    );
    f.t.check_error(
        "test",
        "test1",
        "label '//deuce:deuce' in $(location) expression expands to more than one \
         file, please use $(locations //deuce:deuce) instead",
        &[
            "genrule(name = 'test1',",
            "        tools = ['//deuce'],",
            "        outs = ['test1.out'],",
            "        cmd = '$(location //deuce)')",
        ],
    );
}

// Java: GenRuleCommandSubstitutionTest.testUnknownVariable
#[test]
fn test_unknown_variable() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.genrule("$(UNKNOWN)");
    f.assert_expansion_fails("$(UNKNOWN) not defined", "//test");
}

// Java: GenRuleCommandSubstitutionTest.testLocationOfSourceLabel
#[test]
fn test_location_of_source_label() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.scratch.file(
        "test1/BUILD",
        &[r#"genrule(
    name = "test1",
    srcs = ["src"],
    outs = ["out"],
    cmd = "$(location //test1:src)",
)
"#],
    );
    f.assert_expansion_equals("test1/src", "//test1");

    f.t.scratch.file(
        "test2/BUILD",
        &[r#"genrule(
    name = "test2",
    srcs = ["src"],
    outs = ["out"],
    cmd = "$(location src)",
)
"#],
    );
    f.assert_expansion_equals("test2/src", "//test2");

    f.t.scratch.file(
        "test3/BUILD",
        &[r#"genrule(
    name = "test3",
    srcs = ["src"],
    outs = ["out"],
    cmd = "$(location :src)",
)
"#],
    );
    f.assert_expansion_equals("test3/src", "//test3");
}

// Java: GenRuleCommandSubstitutionTest.testLocationOfOutputLabel
#[test]
fn test_location_of_output_label() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    let gendir = f.gendir();
    f.t.scratch.file(
        "test1/BUILD",
        &[r#"genrule(
    name = "test1",
    outs = ["out"],
    cmd = "$(location //test1:out)",
)
"#],
    );
    f.assert_expansion_equals(&format!("{gendir}/test1/out"), "//test1");

    f.t.scratch.file(
        "test2/BUILD",
        &[r#"genrule(
    name = "test2",
    outs = ["out"],
    cmd = "$(location out)",
)
"#],
    );
    f.assert_expansion_equals(&format!("{gendir}/test2/out"), "//test2");

    f.t.scratch.file(
        "test3/BUILD",
        &[r#"genrule(
    name = "test3",
    outs = ["out"],
    cmd = "$(location out)",
)
"#],
    );
    f.assert_expansion_equals(&format!("{gendir}/test3/out"), "//test3");
}

// Java: GenRuleCommandSubstitutionTest.testLocationsSyntaxErrors
#[test]
fn test_locations_syntax_errors() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.genrule("$(locations )");
    f.assert_expansion_fails(
        "invalid label in $(locations) expression: invalid target name '': empty target name",
        "//test",
    );

    f.genrule("$(locations foo bar");
    f.assert_expansion_fails("unterminated variable reference", "//test");

    f.genrule("$(locations");
    f.assert_expansion_fails("unterminated variable reference", "//test");

    f.genrule("$(locationsz");
    f.assert_expansion_fails("unterminated variable reference", "//test");

    f.genrule("$(locationsz)");
    f.assert_expansion_fails("$(locationsz) not defined", "//test");

    f.genrule("$(locationsz )");
    f.assert_expansion_fails("$(locationsz) not defined", "//test");

    f.genrule("$(locationsz foo )");
    f.assert_expansion_fails("$(locationsz) not defined", "//test");
}

// Java: GenRuleCommandSubstitutionTest.testLocationsOfLabelThatIsNotAPrerequsite
#[test]
fn test_locations_of_label_that_is_not_a_prerequsite() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.scratch.file(
        "test/BUILD",
        &[r#"exports_files(["exists"])

genrule(
    name = "test1",
    outs = ["test1.out"],
    cmd = "$(locations :exists)",
)

genrule(
    name = "test2",
    outs = ["test2.out"],
    cmd = "$(locations :doesnt_exist)",
)
"#],
    );

    // $(locations) of a non-prerequisite fails, even if the target exists:

    f.assert_expansion_fails(
        "label '//test:exists' in $(locations) expression is \
         not a declared prerequisite of this rule",
        "//test:test1",
    );

    f.assert_expansion_fails(
        "label '//test:doesnt_exist' in $(locations) expression is \
         not a declared prerequisite of this rule",
        "//test:test2",
    );
}

// Java: GenRuleCommandSubstitutionTest.testLocationsOfMultiFileLabel
#[test]
fn test_locations_of_multi_file_label() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    let gendir = f.gendir();
    f.t.scratch.file(
        "test/BUILD",
        &[r#"genrule(
    name = "x",
    srcs = ["src"],
    outs = [
        "out1",
        "out2",
    ],
    cmd = ":",
)

genrule(
    name = "y",
    srcs = ["x"],
    outs = ["out"],
    cmd = "$(locations x)",
)
"#],
    );

    f.assert_expansion_equals(
        &format!("{gendir}/test/out1 {gendir}/test/out2"),
        "//test:y",
    );
}

// Java: GenRuleCommandSubstitutionTest.testLocationLocationsAndLabel
#[test]
fn test_location_locations_and_label() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    let gendir = f.gendir();
    f.t.scratch.file(
        "test/BUILD",
        &[r#"genrule(
    name = "x",
    srcs = ["src"],
    outs = ["out"],
    cmd = ":",
)

genrule(
    name = "y",
    srcs = ["src"],
    outs = [
        "out1",
        "out2",
    ],
    cmd = ":",
)

genrule(
    name = "r",
    srcs = [
        "x",
        "y",
        "z",
    ],
    outs = ["res"],
    cmd = " _ $(location x) _ $(locations y) _ ",
)
"#],
    );

    let expected = format!("_ {gendir}/test/out _ {gendir}/test/out1 {gendir}/test/out2 _ ");
    f.assert_expansion_equals(&expected, "//test:r");
}

// Java: GenRuleCommandSubstitutionTest.testLocationsOfSourceLabel
#[test]
fn test_locations_of_source_label() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.scratch.file(
        "test1/BUILD",
        &[r#"genrule(
    name = "test1",
    srcs = ["src"],
    outs = ["out"],
    cmd = "$(locations //test1:src)",
)
"#],
    );
    f.assert_expansion_equals("test1/src", "//test1");

    f.t.scratch.file(
        "test2/BUILD",
        &[r#"genrule(
    name = "test2",
    srcs = ["src"],
    outs = ["out"],
    cmd = "$(locations src)",
)
"#],
    );
    f.assert_expansion_equals("test2/src", "//test2");

    f.t.scratch.file(
        "test3/BUILD",
        &[r#"genrule(
    name = "test3",
    srcs = ["src"],
    outs = ["out"],
    cmd = "$(location :src)",
)
"#],
    );
    f.assert_expansion_equals("test3/src", "//test3");
}

// Java: GenRuleCommandSubstitutionTest.testLocationsOfOutputLabel
#[test]
fn test_locations_of_output_label() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    let gendir = f.gendir();
    f.t.scratch.file(
        "test1/BUILD",
        &[r#"genrule(
    name = "test1",
    outs = ["out"],
    cmd = "$(locations //test1:out)",
)
"#],
    );
    f.assert_expansion_equals(&format!("{gendir}/test1/out"), "//test1");

    f.t.scratch.file(
        "test2/BUILD",
        &[r#"genrule(
    name = "test2",
    outs = ["out"],
    cmd = "$(locations out)",
)
"#],
    );
    f.assert_expansion_equals(&format!("{gendir}/test2/out"), "//test2");

    f.t.scratch.file(
        "test3/BUILD",
        &[r#"genrule(
    name = "test3",
    outs = ["out"],
    cmd = "$(locations out)",
)
"#],
    );
    f.assert_expansion_equals(&format!("{gendir}/test3/out"), "//test3");
}

// Java: GenRuleCommandSubstitutionTest.testOuts
#[test]
fn test_outs() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    let expected = format!("{}/test/out", f.gendir());
    f.t.scratch.file(
        "test/BUILD",
        &[r#"genrule(
    name = "test",
    outs = ["out"],
    cmd = "$(OUTS) # $@",
)
"#],
    );
    f.assert_expansion_equals(&format!("{expected} # {expected}"), "//test");
}

// Java: GenRuleCommandSubstitutionTest.testSrcs
#[test]
fn test_srcs() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    let expected = "test/src";

    f.t.scratch.file(
        "test/BUILD",
        &[r#"genrule(
    name = "test",
    srcs = ["src"],
    outs = ["out"],
    cmd = "$(SRCS) # $<",
)
"#],
    );
    f.assert_expansion_equals(&format!("{expected} # {expected}"), "//test");
}

// Java: GenRuleCommandSubstitutionTest.testDollarDollar
#[test]
fn test_dollar_dollar() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.scratch.file(
        "test/BUILD",
        &[r#"genrule(
    name = "test",
    outs = ["out"],
    cmd = "$$DOLLAR",
)
"#],
    );
    f.assert_expansion_equals("$DOLLAR", "//test");
}

// Java: GenRuleCommandSubstitutionTest.testDollarLessThanWithZeroInputs
#[test]
fn test_dollar_less_than_with_zero_inputs() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.scratch.file(
        "test/BUILD",
        &[r#"genrule(
    name = "test",
    outs = ["out"],
    cmd = "$<",
)
"#],
    );
    f.assert_expansion_fails("variable '$<' : no input file", "//test");
}

// Java: GenRuleCommandSubstitutionTest.testDollarLessThanWithMultipleInputs
#[test]
fn test_dollar_less_than_with_multiple_inputs() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.scratch.file(
        "test/BUILD",
        &[r#"genrule(
    name = "test",
    srcs = [
        "src1",
        "src2",
    ],
    outs = ["out"],
    cmd = "$<",
)
"#],
    );
    f.assert_expansion_fails("variable '$<' : more than one input file", "//test");
}

// Java: GenRuleCommandSubstitutionTest.testDollarAtWithMultipleOutputs
#[test]
fn test_dollar_at_with_multiple_outputs() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.scratch.file(
        "test/BUILD",
        &[r#"genrule(
    name = "test",
    outs = [
        "out.1",
        "out.2",
    ],
    cmd = "$@",
)
"#],
    );
    f.assert_expansion_fails("variable '$@' : more than one output file", "//test");
}

// Java: GenRuleCommandSubstitutionTest.testDollarAtWithZeroOutputs
#[test]
fn test_dollar_at_with_zero_outputs() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.scratch.file(
        "test/BUILD",
        &[r#"genrule(
    name = "test",
    srcs = [
        "src1",
        "src2",
    ],
    outs = [],
    cmd = "$@",
)
"#],
    );
    f.assert_expansion_fails("Genrules without outputs don't make sense", "//test");
}

// Java: GenRuleCommandSubstitutionTest.testShellVariables
#[test]
fn test_shell_variables() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.genrule("for file in a b c;do echo $$file;done");
    f.assert_expansion_equals("for file in a b c;do echo $file;done", "//test");
    f.t.assert_no_events();

    f.genrule("$${file%:.*8}");
    f.assert_expansion_equals("${file%:.*8}", "//test");
    f.t.assert_no_events();

    f.genrule("$$(basename file)");
    f.assert_expansion_equals("$(basename file)", "//test");
    f.t.assert_no_events();

    f.genrule("$(basename file)");
    f.assert_expansion_fails("$(basename) not defined", "//test");
    f.t.assert_contains_event("$(basename) not defined");
}

// Java: GenRuleCommandSubstitutionTest.heuristicLabelExpansion_singletonFilegroupInTools_expandsToFile
#[test]
fn heuristic_label_expansion_singleton_filegroup_in_tools_expands_to_file() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.scratch.file(
        "foo/BUILD",
        &[r#"filegroup(
    name = "fg",
    srcs = ["fg1.txt"],
)

genrule(
    name = "gen",
    outs = ["gen.out"],
    cmd = "cp :fg $@",
    heuristic_label_expansion = True,
    tools = [":fg"],
)
"#],
    );

    let command = f.get_genrule_command("//foo:gen");
    assert!(command.contains("foo/fg1.txt"), "{command}");
}

// Java: GenRuleCommandSubstitutionTest.heuristicLabelExpansion_emptyFilegroupInTools_fails
#[test]
fn heuristic_label_expansion_empty_filegroup_in_tools_fails() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.scratch.file(
        "foo/BUILD",
        &[r#"filegroup(
    name = "fg",
    srcs = [],
)

genrule(
    name = "gen",
    outs = ["gen.out"],
    cmd = "cp :fg $@",
    heuristic_label_expansion = True,
    tools = [":fg"],
)
"#],
    );

    f.assert_expansion_fails("expands to 0 files", "//foo:gen");
}

// Java: GenRuleCommandSubstitutionTest.heuristicLabelExpansion_multiFilegroupInTools_fails
#[test]
fn heuristic_label_expansion_multi_filegroup_in_tools_fails() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.scratch.file(
        "foo/BUILD",
        &[r#"filegroup(
    name = "fg",
    srcs = [
        "fg1.txt",
        "fg2.txt",
    ],
)

genrule(
    name = "gen",
    outs = ["gen.out"],
    cmd = "cp :fg $@",
    heuristic_label_expansion = True,
    tools = [":fg"],
)
"#],
    );

    f.assert_expansion_fails("expands to 2 files", "//foo:gen");
}

// Java: GenRuleCommandSubstitutionTest.testDollarFileFails
#[test]
fn test_dollar_file_fails() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.check_error(
        "test",
        "test",
        "'$file' syntax is not supported; use '$(file)' ",
        &[&get_build_file_with_command(
            "for file in a b c;do echo $file;done",
        )],
    );
}

// Java: GenRuleCommandSubstitutionTest.testDollarFile2Fails
#[test]
fn test_dollar_file2_fails() {
    let mut f = GenRuleCommandSubstitutionTest::new();
    f.t.check_error(
        "test",
        "test",
        "'${file%:.*8}' syntax is not supported; use '$(file%:.*8)' ",
        &[&get_build_file_with_command("${file%:.*8}")],
    );
}
