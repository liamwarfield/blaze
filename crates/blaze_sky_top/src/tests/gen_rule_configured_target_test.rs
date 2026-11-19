//! Port of
//! `src/test/java/com/google/devtools/build/lib/bazel/rules/genrule/GenRuleConfiguredTargetTest.java`.
//!
//! PORT: the Java class overrides `createRuleClassProvider` to add
//! `TestRuleClassProvider.MakeVariableTesterRule`; the harness uses Bazel's rule class provider,
//! so the one test using `make_variable_tester` is ignored.

use std::sync::Arc;

use crate::testing::{BuildViewTestCase, base_artifact_names, get_executable, get_files_to_build};
use blaze_actions::actions::action::CommandAction;
use blaze_actions::analysis::actions::spawn_action::SpawnAction;
use blaze_actions::{ActionRef, Artifact, ArtifactRoot};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::rule::artifact_roots::to_actions_root;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_vfs::PathFragment;

/// `TestConstants.GENRULE_SETUP`.
const GENRULE_SETUP: &str = "@bazel_tools//tools/genrule:genrule-setup.sh";
/// `TestConstants.GENRULE_SETUP_PATH`.
const GENRULE_SETUP_PATH: &str = "genrule-setup.sh";

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

// Java: GenRuleConfiguredTargetTest.assertCommandEquals
fn assert_command_equals(expected: &str, command: &str) {
    // Ensure the command after the genrule setup is correct.
    let command = setup_command_group(command).unwrap_or(command);
    assert_eq!(command, expected);
}

/// The subset of `java.util.regex` syntax the patterns of this test use: literals, `\x`
/// escapes, `.`, `.*`, `.{n}` and `(...)?`.
#[derive(Clone, Debug)]
enum RegexToken {
    Literal(char),
    Any,
    AnyStar,
    Optional(Vec<RegexToken>),
}

fn parse_regex(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    in_group: bool,
) -> Vec<RegexToken> {
    let mut tokens = Vec::new();
    while let Some(c) = chars.next() {
        match c {
            '.' => match chars.peek() {
                Some('*') => {
                    chars.next();
                    tokens.push(RegexToken::AnyStar);
                }
                Some('{') => {
                    chars.next();
                    let count: String = chars.by_ref().take_while(|c| *c != '}').collect();
                    let count: usize = count.parse().expect("repetition count");
                    tokens.extend(std::iter::repeat_n(RegexToken::Any, count));
                }
                _ => tokens.push(RegexToken::Any),
            },
            '\\' => tokens.push(RegexToken::Literal(
                chars.next().expect("escaped character"),
            )),
            '(' => {
                let inner = parse_regex(chars, true);
                assert_eq!(chars.next(), Some('?'), "only (...)? groups are supported");
                tokens.push(RegexToken::Optional(inner));
            }
            ')' if in_group => return tokens,
            '*' | '+' | '?' | '[' | ']' | '{' | '}' | '|' | '^' | '$' => {
                panic!("unsupported regex syntax '{c}'")
            }
            _ => tokens.push(RegexToken::Literal(c)),
        }
    }
    assert!(!in_group, "unterminated group");
    tokens
}

fn regex_match_at(tokens: &[RegexToken], s: &[char], pos: usize) -> bool {
    let is_line_terminator = |c: char| c == '\n' || c == '\r';
    let Some((first, rest)) = tokens.split_first() else {
        return true;
    };
    match first {
        RegexToken::Literal(c) => pos < s.len() && s[pos] == *c && regex_match_at(rest, s, pos + 1),
        RegexToken::Any => {
            pos < s.len() && !is_line_terminator(s[pos]) && regex_match_at(rest, s, pos + 1)
        }
        RegexToken::AnyStar => {
            let mut end = pos;
            while end < s.len() && !is_line_terminator(s[end]) {
                end += 1;
            }
            (pos..=end).rev().any(|p| regex_match_at(rest, s, p))
        }
        RegexToken::Optional(inner) => {
            let mut with_group = inner.clone();
            with_group.extend(rest.iter().cloned());
            regex_match_at(&with_group, s, pos) || regex_match_at(rest, s, pos)
        }
    }
}

/// `assertThat(actual).containsMatch(regex)`: `Pattern.compile(regex).matcher(actual).find()`.
fn assert_contains_match(actual: &str, regex: &str) {
    let tokens = parse_regex(&mut regex.chars().peekable(), false);
    let s: Vec<char> = actual.chars().collect();
    assert!(
        (0..=s.len()).any(|i| regex_match_at(&tokens, &s, i)),
        "expected to contain a match for /{regex}/, but was <{actual}>"
    );
}

/// `assertThat(actual).containsExactly(expected...)` for artifacts (any order, same multiplicity).
fn assert_contains_exactly_artifacts(actual: &[Artifact], expected: &[&Artifact]) {
    let mut remaining: Vec<&Artifact> = actual.iter().collect();
    for e in expected {
        match remaining.iter().position(|a| a == e) {
            Some(i) => {
                remaining.remove(i);
            }
            None => panic!("containsExactly: missing {e:?}; actual {actual:?}"),
        }
    }
    assert!(
        remaining.is_empty(),
        "containsExactly: unexpected {remaining:?}; actual {actual:?}"
    );
}

/// `FileConfiguredTarget.getArtifact()`.
fn file_artifact(ct: &ConfiguredTarget) -> Artifact {
    match ct {
        ConfiguredTarget::InputFile(t) => t.get_artifact().clone(),
        ConfiguredTarget::OutputFile(t) => t.get_artifact().clone(),
        _ => panic!("ClassCastException: not a FileConfiguredTarget"),
    }
}

/// `(SpawnAction) action`.
fn as_spawn_action(action: &ActionRef) -> &SpawnAction {
    action
        .as_any()
        .downcast_ref::<SpawnAction>()
        .expect("ClassCastException: not a SpawnAction")
}

// Java: GenRuleConfiguredTargetTest.createFiles
fn create_files(t: &BuildViewTestCase) {
    t.scratch.file(
        "hello/BUILD",
        &[r#"genrule(
    name = "z",
    outs = ["x/y"],
    cmd = "echo hi > $(@D)/y",
)

genrule(
    name = "w",
    outs = [
        "a/b",
        "c/d",
    ],
    cmd = "echo hi | tee $(@D)/a/b $(@D)/c/d",
)
"#],
    );
}

// Java: GenRuleConfiguredTargetTest.getCommand
/// Returns the expansion of 'cmd' for the specified genrule.
fn get_command(t: &mut BuildViewTestCase, label: &str) -> String {
    let action = get_spawn_action(t, label);
    as_spawn_action(&action).get_arguments().unwrap()[2].clone()
}

// Java: GenRuleConfiguredTargetTest.getSpawnAction
/// Returns the SpawnAction for the specified genrule.
fn get_spawn_action(t: &mut BuildViewTestCase, label: &str) -> ActionRef {
    let ct = t
        .get_configured_target(label)
        .unwrap_or_else(|| panic!("no configured target {label}"));
    let out = get_files_to_build(&ct).to_list()[0].clone();
    let action = t.get_generating_action(&out).expect("generating action");
    as_spawn_action(&action);
    action
}

// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1455 BuildViewTestCase.getDerivedArtifact
/// The output of `owner`'s actions with the given root-relative path and root.
///
/// PORT: Java falls back to `ArtifactFactory.getDerivedArtifact` (a new artifact) when no
/// action has such an output; that artifact could not equal a file target's artifact in these
/// tests, so the fallback is `None`.
fn get_derived_artifact(
    t: &mut BuildViewTestCase,
    root_relative_path: &PathFragment,
    root: &ArtifactRoot,
    owner: &ConfiguredTarget,
) -> Option<Artifact> {
    // Java: ConfiguredTargetKey.fromConfiguredTarget(owner), then getExistingValue.
    let key = owner
        .unwrap_if_merged()
        .get_lookup_key()
        .0
        .as_any()
        .downcast_ref::<blaze_analysis::configured_target_key::ConfiguredTargetKey>()
        .expect("ClassCastException: not a ConfiguredTargetKey")
        .clone();
    let graph = t.skyframe_executor().get_walkable_graph();
    let value = crate::skyframe_executor::get_configured_target_value(
        &*graph,
        &blaze_skyframe::SkyKey::new(key),
    )?;
    for action in value.get_actions() {
        for output in action.get_outputs() {
            if output.get_root_relative_path() == *root_relative_path && output.get_root() == root {
                return Some(output);
            }
        }
    }
    None
}

// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1540 BuildViewTestCase.getBinArtifact
/// PORT: `getRuleContext(owner).getBinDirectory()` is the owner's configuration's bin directory
/// for the rule's repository (`RuleContext.getBinDirectory`); the harness has no
/// `getRuleContext`.
fn get_bin_artifact(
    t: &mut BuildViewTestCase,
    package_relative_path: &str,
    owner: &ConfiguredTarget,
) -> Option<Artifact> {
    let config: Arc<BuildConfigurationValue> =
        t.get_configuration(owner).expect("owner configuration");
    let label = owner.get_label();
    let root = to_actions_root(&config.get_bin_directory(label.get_repository()));
    let path = label
        .get_package_fragment()
        .get_relative_str(package_relative_path);
    get_derived_artifact(t, &path, &root, owner)
}

// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1615 BuildViewTestCase.getGenfilesArtifact
// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1632 BuildViewTestCase.getGenfilesArtifact
// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1671 BuildViewTestCase.getGenfilesArtifact
fn get_genfiles_artifact(
    t: &mut BuildViewTestCase,
    package_relative_path: &str,
    owner: &ConfiguredTarget,
) -> Option<Artifact> {
    let config = t.get_configuration(owner).expect("owner configuration");
    let root =
        to_actions_root(&config.get_genfiles_directory(&blaze_cmdline::repository_name::MAIN));
    let path = owner
        .get_label()
        .get_package_fragment()
        .get_relative_str(package_relative_path);
    get_derived_artifact(t, &path, &root, owner)
}

// Java: GenRuleConfiguredTargetTest.testToolchainOverridesJavabase
#[test]
#[ignore = "needs TestRuleClassProvider's test rules (make_variable_tester, MakeVariableTesterRule)"]
fn test_toolchain_overrides_javabase() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[r#"genrule(
    name = "gr",
    srcs = [],
    outs = ["out"],
    cmd = "JAVABASE=$(JAVABASE)",
    toolchains = [":v"],
)

make_variable_tester(
    name = "v",
    variables = {"JAVABASE": "REPLACED"},
)
"#],
    );

    let cmd = get_command(&mut t, "//a:gr");
    assert!(cmd.ends_with("JAVABASE=REPLACED"), "{cmd}");
}

// Java: GenRuleConfiguredTargetTest.testD
#[test]
fn test_d() {
    let mut t = BuildViewTestCase::new();
    create_files(&t);
    let z = t.get_configured_target("//hello:z").unwrap();
    let y = get_files_to_build(&z).get_singleton().clone();
    assert_eq!(
        y.get_root_relative_path(),
        PathFragment::create("hello/x/y")
    );
}

// Java: GenRuleConfiguredTargetTest.testDMultiOutput
#[test]
fn test_d_multi_output() {
    let mut t = BuildViewTestCase::new();
    create_files(&t);
    let z = t.get_configured_target("//hello:w").unwrap();
    let files = get_files_to_build(&z).to_list();
    assert_eq!(files.len(), 2);
    assert_eq!(
        files[0].get_root_relative_path(),
        PathFragment::create("hello/a/b")
    );
    assert_eq!(
        files[1].get_root_relative_path(),
        PathFragment::create("hello/c/d")
    );
}

// Java: GenRuleConfiguredTargetTest.testOutsWithSameNameAsRule
#[test]
fn test_outs_with_same_name_as_rule() {
    let mut t = BuildViewTestCase::new();
    // The error was demoted to a warning.
    // Re-enable after June 1 2008 when we make it an error again.
    t.check_warning(
        "genrule2",
        "hello_world",
        "target 'hello_world' is both a rule and a file;",
        &[
            "genrule(name = 'hello_world',",
            "srcs = ['ignore_me.txt'],",
            "outs = ['message.txt', 'hello_world'],",
            "cmd  = 'echo \"Hello, world.\" >$(location message.txt)')",
        ],
    );
}

// Java: GenRuleConfiguredTargetTest.testFilesToBuildIsOuts
#[test]
fn test_files_to_build_is_outs() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "genrule1/BUILD",
        &[r#"genrule(
    name = "hello_world",
    srcs = ["ignore_me.txt"],
    outs = ["message.txt"],
    cmd = 'echo "Hello, world." >$(location message.txt)',
)
"#],
    );
    let message_artifact = file_artifact(&t.get_file_configured_target("//genrule1:message.txt"));
    let hello_world = t.get_configured_target("//genrule1:hello_world").unwrap();
    assert_contains_exactly_artifacts(
        &get_files_to_build(&hello_world).to_list(),
        &[&message_artifact],
    );
}

// Java: GenRuleConfiguredTargetTest.testActionIsShellCommand
#[test]
fn test_action_is_shell_command() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "genrule1/BUILD",
        &[r#"genrule(
    name = "hello_world",
    srcs = ["ignore_me.txt"],
    outs = ["message.txt"],
    cmd = 'echo "Hello, world." >$(location message.txt)',
)
"#],
    );

    let message_artifact = file_artifact(&t.get_file_configured_target("//genrule1:message.txt"));
    let shell_action = t
        .get_generating_action(&message_artifact)
        .expect("shellAction is not null");
    let spawn_action = as_spawn_action(&shell_action);

    let ignore_me_artifact =
        file_artifact(&t.get_file_configured_target("//genrule1:ignore_me.txt"));
    let genrule_setup_artifact = file_artifact(&t.get_file_configured_target(GENRULE_SETUP));

    assert_contains_exactly_artifacts(
        &shell_action.get_inputs().to_list(),
        &[&ignore_me_artifact, &genrule_setup_artifact],
    );
    assert_contains_exactly_artifacts(&shell_action.get_outputs(), &[&message_artifact]);

    let expected = format!(
        "echo \"Hello, world.\" >{}",
        message_artifact.get_exec_path_string()
    );
    let target_config = t.get_target_configuration();
    let arguments = spawn_action.get_arguments().unwrap();
    assert_eq!(
        arguments[0],
        blaze_analysis::rule::sh_toolchain::get_path_for_host(&target_config).get_path_string()
    );
    assert_eq!(arguments[1], "-c");
    assert_command_equals(&expected, &arguments[2]);
}

// Java: GenRuleConfiguredTargetTest.testDependentGenrule
#[test]
fn test_dependent_genrule() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "genrule1/BUILD",
        &[r#"genrule(
    name = "hello_world",
    srcs = ["ignore_me.txt"],
    outs = ["message.txt"],
    cmd = 'echo "Hello, world." >$(location message.txt)',
)
"#],
    );
    t.scratch.file(
        "genrule2/BUILD",
        &[r#"genrule(
    name = "goodbye_world",
    srcs = [
        "goodbye.txt",
        "//genrule1:hello_world",
    ],
    outs = ["farewell.txt"],
    cmd = "echo $(SRCS) >$(location farewell.txt)",
)
"#],
    );

    t.get_configured_target("//genrule2:goodbye_world");

    let farewell_artifact = file_artifact(&t.get_file_configured_target("//genrule2:farewell.txt"));
    let goodbye_artifact = file_artifact(&t.get_file_configured_target("//genrule2:goodbye.txt"));
    let message_artifact = file_artifact(&t.get_file_configured_target("//genrule1:message.txt"));
    let genrule_setup_artifact = file_artifact(&t.get_file_configured_target(GENRULE_SETUP));

    let shell_action = t
        .get_generating_action(&farewell_artifact)
        .expect("generating action");
    let spawn_action = as_spawn_action(&shell_action);

    // inputs = { "goodbye.txt", "//genrule1:message.txt" }
    assert_contains_exactly_artifacts(
        &shell_action.get_inputs().to_list(),
        &[
            &goodbye_artifact,
            &message_artifact,
            &genrule_setup_artifact,
        ],
    );

    // outputs = { "farewell.txt" }
    assert_contains_exactly_artifacts(&shell_action.get_outputs(), &[&farewell_artifact]);

    let expected = format!(
        "echo {} {} >{}",
        goodbye_artifact.get_exec_path_string(),
        message_artifact.get_exec_path_string(),
        farewell_artifact.get_exec_path_string()
    );
    assert_command_equals(&expected, &spawn_action.get_arguments().unwrap()[2]);
}

// Java: GenRuleConfiguredTargetTest.testDependenciesViaFiles
/// Ensure that the actions / artifacts created by genrule dependencies allow us to follow the
/// chain of generated files backward.
#[test]
fn test_dependencies_via_files() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "foo/BUILD",
        &[r#"genrule(
    name = "bar",
    srcs = ["bar_in.txt"],
    outs = ["bar_out.txt"],
    cmd = "touch $(OUTS)",
)

genrule(
    name = "baz",
    srcs = ["bar_out.txt"],
    outs = ["baz_out.txt"],
    cmd = "touch $(OUTS)",
)
"#],
    );

    let baz_out_target = t.get_file_configured_target("//foo:baz_out.txt");
    let baz_action = t
        .get_generating_action(&file_artifact(&baz_out_target))
        .expect("generating action");
    let bar_out = baz_action.get_inputs().to_list()[0].clone();
    assert!(
        bar_out
            .get_exec_path()
            .ends_with(&PathFragment::create("foo/bar_out.txt"))
    );
    let bar_action = t
        .get_generating_action(&bar_out)
        .expect("generating action");
    let bar_in = bar_action.get_inputs().to_list()[0].clone();
    assert!(
        bar_in
            .get_exec_path()
            .ends_with(&PathFragment::create("foo/bar_in.txt"))
    );
}

// Java: GenRuleConfiguredTargetTest.testOutputDirExpansion
/// Ensure that variable $(@D) gets expanded correctly in the genrule cmd.
#[test]
fn test_output_dir_expansion() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "foo/BUILD",
        &[r#"genrule(
    name = "bar",
    srcs = ["bar_in.txt"],
    outs = ["bar/bar_out.txt"],
    cmd = "touch $(@D)",
)

genrule(
    name = "baz",
    srcs = ["bar/bar_out.txt"],
    outs = [
        "logs/baz_out.txt",
        "logs/baz.log",
    ],
    cmd = "touch $(@D)",
)
"#],
    );

    t.get_configured_target("//foo:bar");

    let baz_out_target = t.get_file_configured_target("//foo:logs/baz_out.txt");

    let baz_action = t
        .get_generating_action(&file_artifact(&baz_out_target))
        .expect("generating action");
    let baz_arguments = as_spawn_action(&baz_action).get_arguments().unwrap();

    // Make sure the expansion for $(@D) results in the
    // directory of the BUILD file ("foo"), not the common parent
    // directory of the output files ("logs")
    let baz_expected = format!(
        "touch {}",
        file_artifact(&baz_out_target)
            .get_exec_path()
            .get_parent_directory()
            .unwrap()
            .get_parent_directory()
            .unwrap()
            .get_path_string()
    );
    assert_command_equals(&baz_expected, &baz_arguments[2]);
    assert!(baz_arguments[2].ends_with("/foo"), "{}", baz_arguments[2]);

    t.get_configured_target("//foo:bar");

    let bar_out = baz_action.get_inputs().to_list()[0].clone();
    assert!(
        bar_out
            .get_exec_path()
            .ends_with(&PathFragment::create("foo/bar/bar_out.txt"))
    );
    let bar_action = t
        .get_generating_action(&bar_out)
        .expect("generating action");
    let bar_expected = format!(
        "touch {}",
        bar_out
            .get_exec_path()
            .get_parent_directory()
            .unwrap()
            .get_path_string()
    );
    assert_command_equals(
        &bar_expected,
        &as_spawn_action(&bar_action).get_arguments().unwrap()[2],
    );
    assert!(baz_expected != bar_expected);
}

// Java: GenRuleConfiguredTargetTest.testRuleDirExpansion
/// Ensure that variable $(RULE_DIR) gets expanded correctly in the genrule cmd.
#[test]
fn test_rule_dir_expansion() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "foo/BUILD",
        &[r#"genrule(
    name = "bar",
    srcs = ["bar_in.txt"],
    outs = ["bar/bar_out.txt"],
    cmd = "touch $(RULEDIR)",
)

genrule(
    name = "baz",
    srcs = ["bar/bar_out.txt"],
    outs = [
        "baz/baz_out.txt",
        "logs/baz.log",
    ],
    cmd = "touch $(RULEDIR)",
)
"#],
    );

    // Make sure the expansion for $(RULE_DIR) results in the directory of the BUILD file ("foo")
    let expected_regex = "touch b.{4}-out.*foo";
    assert_contains_match(&get_command(&mut t, "//foo:bar"), expected_regex);
    assert_contains_match(&get_command(&mut t, "//foo:baz"), expected_regex);
}

// Java: GenRuleConfiguredTargetTest.testMessage
#[test]
fn test_message() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "genrule3/BUILD",
        &[r#"genrule(
    name = "hello_world",
    srcs = ["ignore_me.txt"],
    outs = ["hello.txt"],
    cmd = 'echo "Hello, world." >hello.txt',
)

genrule(
    name = "goodbye_world",
    srcs = ["ignore_me.txt"],
    outs = ["goodbye.txt"],
    cmd = 'echo "Goodbye, world." >goodbye.txt',
    message = "Generating message",
)
"#],
    );
    assert_eq!(
        get_spawn_action(&mut t, "//genrule3:hello_world")
            .get_progress_message(None)
            .as_deref(),
        Some("Executing genrule //genrule3:hello_world")
    );
    assert_eq!(
        get_spawn_action(&mut t, "//genrule3:goodbye_world")
            .get_progress_message(None)
            .as_deref(),
        Some("Generating message //genrule3:goodbye_world")
    );
}

// Java: GenRuleConfiguredTargetTest.testBinaryTargetsExpandToExecutable
/// Ensure that labels from binary targets expand to the executable
#[test]
#[ignore = "needs the mock C++ toolchain (cc_binary from the mock rules_cc)"]
fn test_binary_targets_expand_to_executable() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "genrule3/BUILD",
        &[r#"load("@rules_cc//cc:cc_binary.bzl", "cc_binary")

genrule(
    name = "hello_world",
    srcs = ["ignore_me.txt"],
    outs = ["message.txt"],
    cmd = '$(location :echo) "Hello, world." >message.txt',
    tools = ["echo"],
)

cc_binary(
    name = "echo",
    srcs = ["echo.cc"],
)
"#],
    );
    let regex = "b.{4}-out/.*/bin/genrule3/echo(\\.exe)? \"Hello, world.\" >message.txt";
    assert_contains_match(&get_command(&mut t, "//genrule3:hello_world"), regex);
}

// Java: GenRuleConfiguredTargetTest.testOutputToBindir
#[test]
fn test_output_to_bindir() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "x/BUILD",
        &[r#"genrule(
    name = "bin",
    outs = ["bin.out"],
    cmd = ":",
    output_to_bindir = 1,
)

genrule(
    name = "genfiles",
    outs = ["genfiles.out"],
    cmd = ":",
    output_to_bindir = 0,
)
"#],
    );

    let bin_out = file_artifact(&t.get_file_configured_target("//x:bin.out"));
    let bin_ct = t.get_configured_target("//x:bin").unwrap();
    assert_eq!(Some(bin_out), get_bin_artifact(&mut t, "bin.out", &bin_ct));
    let genfiles_out = file_artifact(&t.get_file_configured_target("//x:genfiles.out"));
    // Java: getGenfilesArtifact(String, String owner) configures the owner.
    let genfiles_ct = t.get_configured_target("//x:genfiles").unwrap();
    assert_eq!(
        Some(genfiles_out),
        get_genfiles_artifact(&mut t, "genfiles.out", &genfiles_ct)
    );
}

// Java: GenRuleConfiguredTargetTest.testMultipleOutputsToBindir
#[test]
fn test_multiple_outputs_to_bindir() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "x/BUILD",
        &[r#"genrule(
    name = "bin",
    outs = [
        "bin_a.out",
        "bin_b.out",
    ],
    cmd = ":",
    output_to_bindir = 1,
)

genrule(
    name = "genfiles",
    outs = [
        "genfiles_a.out",
        "genfiles_b.out",
    ],
    cmd = ":",
    output_to_bindir = 0,
)
"#],
    );

    let bin_ct = t.get_configured_target("//x:bin").unwrap();
    let gen_ct = t.get_configured_target("//x:genfiles").unwrap();
    let bin_a = file_artifact(&t.get_file_configured_target("//x:bin_a.out"));
    assert_eq!(Some(bin_a), get_bin_artifact(&mut t, "bin_a.out", &bin_ct));
    let bin_b = file_artifact(&t.get_file_configured_target("//x:bin_b.out"));
    assert_eq!(Some(bin_b), get_bin_artifact(&mut t, "bin_b.out", &bin_ct));
    let genfiles_a = file_artifact(&t.get_file_configured_target("//x:genfiles_a.out"));
    assert_eq!(
        Some(genfiles_a),
        get_genfiles_artifact(&mut t, "genfiles_a.out", &gen_ct)
    );
    let genfiles_b = file_artifact(&t.get_file_configured_target("//x:genfiles_b.out"));
    assert_eq!(
        Some(genfiles_b),
        get_genfiles_artifact(&mut t, "genfiles_b.out", &gen_ct)
    );
}

// Java: GenRuleConfiguredTargetTest.testMultipleOutsPreservesOrdering
#[test]
fn test_multiple_outs_preserves_ordering() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "multiple/outs/BUILD",
        &[r#"genrule(
    name = "test",
    outs = [
        "file1.out",
        "file2.out",
    ],
    cmd = "touch $(OUTS)",
)
"#],
    );
    let regex = "touch b.{4}-out/.*/multiple/outs/file1.out \
                 b.{4}-out/.*/multiple/outs/file2.out";
    assert_contains_match(&get_command(&mut t, "//multiple/outs:test"), regex);
}

/// `getDirectPrerequisites(target)` (Java `BuildViewForTesting.getDirectPrerequisitesForTesting`),
/// which the harness does not have.
fn get_direct_prerequisites(
    _t: &mut BuildViewTestCase,
    _target: &ConfiguredTarget,
) -> Vec<ConfiguredTarget> {
    panic!("needs BuildViewForTesting.getDirectPrerequisitesForTesting in the harness")
}

// Java: GenRuleConfiguredTargetTest.testToolsAreExecConfiguration
#[test]
#[ignore = "needs getDirectPrerequisites (BuildViewForTesting.getDirectPrerequisitesForTesting) in the harness"]
fn test_tools_are_exec_configuration() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "config/BUILD",
        &[r#"genrule(
    name = "src",
    outs = ["src.out"],
    cmd = ":",
)

genrule(
    name = "tool",
    outs = ["tool.out"],
    cmd = ":",
)

genrule(
    name = "config",
    srcs = [":src"],
    outs = ["out"],
    cmd = "$(location :tool)",
    tools = [":tool"],
)
"#],
    );

    let parent_target = t.get_configured_target("//config").unwrap();

    let prereqs = get_direct_prerequisites(&mut t, &parent_target);

    let mut found_src = false;
    let mut found_tool = false;
    let mut found_setup = false;
    for prereq in &prereqs {
        let name = prereq.get_label().get_name().to_string();
        match name.as_str() {
            "src" => {
                // assertConfigurationsEqual: the configurations' options are equal.
                let parent_config = t.get_configuration(&parent_target).unwrap();
                let prereq_config = t.get_configuration(prereq).unwrap();
                assert_eq!(parent_config.get_key(), prereq_config.get_key());
                found_src = true;
            }
            "tool" => {
                assert!(t.get_configuration(prereq).unwrap().is_tool_configuration());
                found_tool = true;
            }
            GENRULE_SETUP_PATH => {
                assert!(t.get_configuration(prereq).is_none());
                found_setup = true;
            }
            "host" => {
                // Ignore the dependency on the target platform.
            }
            _ => panic!(
                "unexpected prerequisite {:?} (name: {name})",
                prereq.get_label()
            ),
        }
    }

    assert!(found_src);
    assert!(found_tool);
    assert!(found_setup);
}

// Java: GenRuleConfiguredTargetTest.testLabelsContainingAtDAreExpanded
#[test]
fn test_labels_containing_at_d_are_expanded() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "puck/BUILD",
        &[r#"genrule(
    name = "gen",
    outs = ["out"],
    cmd = "echo $(@D)",
    tools = ["puck"],
)
"#],
    );
    let regex = "echo b.{4}-out/.*/puck";
    assert_contains_match(&get_command(&mut t, "//puck:gen"), regex);
}

// Java: GenRuleConfiguredTargetTest.testGetExecutable
#[test]
fn test_get_executable() {
    let mut t = BuildViewTestCase::new();
    let turtle = t
        .scratch_configured_target(
            "java/com/google/turtle",
            "turtle_bootstrap",
            &[
                "genrule(name = 'turtle_bootstrap',",
                "    srcs = ['Turtle.java'],",
                "    outs = ['turtle'],",
                "    executable = 1,",
                "    cmd = 'touch $(OUTS)')",
            ],
        )
        .unwrap();
    assert_eq!(
        get_executable(&turtle)
            .expect("executable")
            .get_exec_path()
            .get_base_name(),
        "turtle"
    );
}

// Java: GenRuleConfiguredTargetTest.testGetExecutableForNonExecutableOut
#[test]
fn test_get_executable_for_non_executable_out() {
    let mut t = BuildViewTestCase::new();
    let turtle = t
        .scratch_configured_target(
            "java/com/google/turtle",
            "turtle_bootstrap",
            &[
                "genrule(name = 'turtle_bootstrap',",
                "    srcs = ['Turtle.java'],",
                "    outs = ['debugdata.txt'],",
                "    cmd = 'touch $(OUTS)')",
            ],
        )
        .unwrap();
    assert!(get_executable(&turtle).is_none());
}

// Java: GenRuleConfiguredTargetTest.testGetExecutableForMultipleOuts
#[test]
fn test_get_executable_for_multiple_outs() {
    let mut t = BuildViewTestCase::new();
    let turtle = t
        .scratch_configured_target(
            "java/com/google/turtle",
            "turtle_bootstrap",
            &[
                "genrule(name = 'turtle_bootstrap',",
                "    srcs = ['Turtle.java'],",
                "    outs = ['turtle', 'debugdata.txt'],",
                "    cmd = 'touch $(OUTS)')",
            ],
        )
        .unwrap();
    assert!(get_executable(&turtle).is_none());
}

// Java: GenRuleConfiguredTargetTest.testGetExecutableFailsForMultipleOutputs
#[test]
fn test_get_executable_fails_for_multiple_outputs() {
    let mut t = BuildViewTestCase::new();
    // Multiple output files are invalid when executable=1.
    t.check_error(
        "bad",
        "bad",
        "in executable attribute of genrule rule //bad:bad: \
         if genrules produce executables, they are allowed only one output. \
         If you need the executable=1 argument, then you should split this genrule into \
         genrules producing single outputs",
        &[
            "genrule(name = 'bad',",
            "        outs = [ 'bad_out1', 'bad_out2' ],",
            "        executable = 1,",
            "        cmd = 'touch $(OUTS)')",
        ],
    );
}

// Java: GenRuleConfiguredTargetTest.testEmptyOutsError
#[test]
fn test_empty_outs_error() {
    let mut t = BuildViewTestCase::new();
    t.check_error(
        "x",
        "x",
        "Genrules without outputs don't make sense",
        &["genrule(name = 'x', outs = [], cmd='echo')"],
    );
}

// Java: GenRuleConfiguredTargetTest.testGenruleSetup
#[test]
fn test_genrule_setup() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "foo/BUILD",
        &[r#"genrule(
    name = "foo_sh",
    outs = ["foo.sh"],  # Shell script files are known to be executable.
    cmd = "touch $@",
)
"#],
    );

    let command = get_command(&mut t, "//foo:foo_sh");
    assert!(command.contains(GENRULE_SETUP_PATH), "{command}");
}

// Java: GenRuleConfiguredTargetTest.createStampingTargets
fn create_stamping_targets(t: &BuildViewTestCase) {
    t.scratch.file(
        "u/BUILD",
        &[r#"genrule(
    name = "foo_stamp",
    srcs = [],
    outs = ["uu"],
    cmd = "",
    stamp = 1,
)

genrule(
    name = "foo_nostamp",
    srcs = [],
    outs = ["vv"],
    cmd = "",
    stamp = 0,
)

genrule(
    name = "foo_autostamp",
    srcs = [],
    outs = ["aa"],
    cmd = "",
    stamp = -1,
)

genrule(
    name = "foo_default",
    srcs = [],
    outs = ["xx"],
    cmd = "",
)
"#],
    );
}

/// The base names of the inputs of the action generating `target`'s first file to build.
fn first_output_input_names(t: &mut BuildViewTestCase, target: &ConfiguredTarget) -> Vec<String> {
    let out = get_files_to_build(target).to_list()[0].clone();
    let action = t.get_generating_action(&out).expect("generating action");
    base_artifact_names(action.get_inputs().to_list().iter())
}

// Java: GenRuleConfiguredTargetTest.assertStamped
fn assert_stamped(t: &mut BuildViewTestCase, target: &str) {
    let ct = t.get_configured_target(target).unwrap();
    assert_stamped_ct(t, &ct);
}

// Java: GenRuleConfiguredTargetTest.assertStamped
fn assert_stamped_ct(t: &mut BuildViewTestCase, target: &ConfiguredTarget) {
    let inputs = first_output_input_names(t, target);
    for expected in ["build-info.txt", "build-changelist.txt"] {
        assert!(
            inputs.iter().any(|i| i == expected),
            "containsAtLeast {expected}: {inputs:?}"
        );
    }
}

// Java: GenRuleConfiguredTargetTest.assertNotStamped
fn assert_not_stamped(t: &mut BuildViewTestCase, target: &str) {
    let ct = t.get_configured_target(target).unwrap();
    assert_not_stamped_ct(t, &ct);
}

// Java: GenRuleConfiguredTargetTest.assertNotStamped
fn assert_not_stamped_ct(t: &mut BuildViewTestCase, target: &ConfiguredTarget) {
    let inputs = first_output_input_names(t, target);
    assert!(!inputs.iter().any(|i| i == "build-info.txt"), "{inputs:?}");
    assert!(
        !inputs.iter().any(|i| i == "build-changelist.txt"),
        "{inputs:?}"
    );
}

// Java: GenRuleConfiguredTargetTest.testStampingWithNoStamp
#[test]
fn test_stamping_with_no_stamp() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--nostamp"]);
    create_stamping_targets(&t);
    assert_stamped(&mut t, "//u:foo_stamp");
    let ct = t.get_exec_configured_target("//u:foo_stamp").unwrap();
    assert_stamped_ct(&mut t, &ct);
    assert_not_stamped(&mut t, "//u:foo_nostamp");
    let ct = t.get_exec_configured_target("//u:foo_nostamp").unwrap();
    assert_not_stamped_ct(&mut t, &ct);
    assert_not_stamped(&mut t, "//u:foo_autostamp");
    let ct = t.get_exec_configured_target("//u:foo_autostamp").unwrap();
    assert_not_stamped_ct(&mut t, &ct);
    assert_not_stamped(&mut t, "//u:foo_default");
}

// Java: GenRuleConfiguredTargetTest.testStampingWithStamp
#[test]
fn test_stamping_with_stamp() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--stamp"]);
    create_stamping_targets(&t);
    assert_stamped(&mut t, "//u:foo_stamp");
    let ct = t.get_exec_configured_target("//u:foo_stamp").unwrap();
    assert_stamped_ct(&mut t, &ct);
    assert_not_stamped(&mut t, "//u:foo_nostamp");
    let ct = t.get_exec_configured_target("//u:foo_nostamp").unwrap();
    assert_not_stamped_ct(&mut t, &ct);
    assert_stamped(&mut t, "//u:foo_autostamp");
    let ct = t.get_exec_configured_target("//u:foo_autostamp").unwrap();
    assert_not_stamped_ct(&mut t, &ct);
    assert_not_stamped(&mut t, "//u:foo_default");
}

// Java: GenRuleConfiguredTargetTest.testRequiresDarwin
#[test]
fn test_requires_darwin() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "foo/BUILD",
        &["genrule(name='darwin', srcs=[], outs=['macout'], cmd='', tags=['requires-darwin'])"],
    );

    let action = get_spawn_action(&mut t, "//foo:darwin");
    assert!(
        action.get_execution_info().contains_key("requires-darwin"),
        "{:?}",
        action.get_execution_info()
    );
    // requires-darwin causes /bin/bash to be hard-coded, see CommandHelper.shellPath().
    assert_eq!(
        as_spawn_action(&action)
            .get_command_filename()
            .unwrap()
            .as_deref(),
        Some("/bin/bash")
    );
}

// Java: GenRuleConfiguredTargetTest.testJarError
#[test]
fn test_jar_error() {
    let mut t = BuildViewTestCase::new();
    t.check_error(
        "foo",
        "grj",
        "in cmd attribute of genrule rule //foo:grj: $(JAR) not defined",
        &["genrule(name='grj',      srcs = [],      outs=['grj'],      cmd='$(JAR) foo bar')"],
    );
}

// Java: GenRuleConfiguredTargetTest.testDuplicateLocalFlags
/// Regression test for b/15589451.
#[test]
fn test_duplicate_local_flags() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "foo/BUILD",
        &["genrule(name='g',      srcs = [],      outs = ['grj'],      cmd ='echo g',      local = 1,      tags = ['local'])"],
    );
    t.get_configured_target("//foo:g");
    t.assert_no_events();
}

// Java: GenRuleConfiguredTargetTest.testToolsHaveExecOutputDir
#[test]
fn test_tools_have_exec_output_dir() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "config/BUILD",
        &[r#"genrule(
    name = "src",
    outs = ["src.out"],
    cmd = ":",
)

genrule(
    name = "tool",
    outs = ["tool.out"],
    cmd = ":",
)

genrule(
    name = "config",
    srcs = [":src"],
    outs = ["out"],
    cmd = "$(location :tool)",
    tools = [":tool"],
)
"#],
    );

    let parent_target = t.get_configured_target("//config").unwrap();

    // Cannot use getDirectPrerequisites, as this re-configures that target incorrectly.
    let out = get_files_to_build(&parent_target).to_list()[0].clone();
    let action = t.get_generating_action(&out).expect("generating action");
    assert_eq!(action.get_tools().to_list().len(), 1);
    let tool = action.get_tools().get_singleton().clone();
    // This is the output dir fragment for the execution transition.
    assert!(
        tool.get_exec_path_string().contains("-exec"),
        "{}",
        tool.get_exec_path_string()
    );
}
