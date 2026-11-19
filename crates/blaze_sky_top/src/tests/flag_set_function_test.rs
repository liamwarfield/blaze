//! Port of `src/test/java/com/google/devtools/build/lib/skyframe/config/FlagSetFunctionTest.java`.
//!
//! PORT: Java's rule class provider is `TestRuleClassProvider.addStandardRules` plus
//! `DummyTestFragment`; no test reads a `DummyTestFragment` option (they only add to the default
//! options' checksum), so the harness's standard provider is used.
//! PORT: `createBuildOptions(args)` (`SkyframeExecutor.createBuildOptionsForTesting`) is the
//! harness's `use_configuration(args)` then `get_build_options()`, which parse the same way.

use std::sync::Arc;

use blaze_build_config::build_options::BuildOptions;
use blaze_cmdline::Label;
use blaze_events::EventKind;
use blaze_sky_analysis::flag_set_function::{
    ConfigFlagDefinitions, FlagSetKey, FlagSetValue, does_buildable_unit_match_target,
};
use blaze_sky_loading::project_function::BuildableUnit;
use blaze_skyframe::SkyKey;
use indexmap::{IndexMap, IndexSet};

use crate::testing::BuildViewTestCase;

const PROTO_LOAD: &str =
    "load(\"//test:project_proto.scl\", \"buildable_unit_pb2\", \"project_pb2\")";

// Java: FlagSetFunctionTest.setUp
fn set_up() -> BuildViewTestCase {
    let t = BuildViewTestCase::new();
    write_project_scl_definition(&t, "test/project_proto.scl");
    t
}

/// `BuildViewTestCase.writeProjectSclDefinition(dest)`.
// PORT: Java reads `TestConstants.PROJECT_SCL_DEFINITION_PATH` from the runfiles; the file is
// the pinned source's `src/main/protobuf/project/project_proto.scl`.
fn write_project_scl_definition(t: &BuildViewTestCase, dest: &str) {
    let path = std::path::Path::new(&std::env::var_os("HOME").expect("HOME"))
        .join(".cache/blaze/src/bazel-9.3.2/src/main/protobuf/project/project_proto.scl");
    let contents = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e));
    t.scratch.file_bytes(dest, &contents);
}

fn file(t: &BuildViewTestCase, path: &str, text: &str) {
    let lines: Vec<&str> = text.lines().collect();
    t.scratch.file(path, &lines);
}

/// `test/PROJECT.scl` with the protolark load and `body`.
fn proto_project(t: &BuildViewTestCase, body: &str) {
    file(t, "test/PROJECT.scl", &format!("{PROTO_LOAD}\n{body}"));
}

/// `createStringFlag(labelName, defaultValue)`.
// Java: FlagSetFunctionTest.createStringFlag
fn create_string_flag(t: &BuildViewTestCase, label_name: &str, default_value: &str) {
    let flag_dir = &label_name[2..label_name.find(':').unwrap()];
    let flag_name = &label_name[label_name.find(':').unwrap() + 1..];
    file(
        t,
        &format!("{flag_dir}/build_settings.bzl"),
        "string_flag = rule(\n    implementation = lambda ctx: [],\n    build_setting = config.string(flag = True),\n    attrs = {\n        \"scope\": attr.string(default = \"universal\"),\n    },\n)\n",
    );
    file(
        t,
        &format!("{flag_dir}/BUILD"),
        &format!(
            "load(\":build_settings.bzl\", \"string_flag\")\nstring_flag(\n    name = \"{flag_name}\",\n    build_setting_default = \"{default_value}\",\n)\n"
        ),
    );
}

/// The plain string_flag rule of most tests (`test/build_settings.bzl`).
fn plain_string_flag_bzl(t: &BuildViewTestCase) {
    file(
        t,
        "test/build_settings.bzl",
        "string_flag = rule(implementation = lambda ctx: [], build_setting = config.string(flag = True))\n",
    );
}

/// The scoped string_flag rule (`attrs = {"scope": ...}`) at `test/build_settings.bzl`.
fn scoped_string_flag_bzl(t: &BuildViewTestCase) {
    file(
        t,
        "test/build_settings.bzl",
        "string_flag = rule(\n    implementation = lambda ctx: [],\n    build_setting = config.string(flag = True),\n    attrs = {\n        \"scope\": attr.string(default = \"universal\"),\n    },\n)\n",
    );
}

/// `test/BUILD` declaring string flags `names` (default "default").
fn string_flags_build(t: &BuildViewTestCase, names: &[&str]) {
    let mut text = "load(\"//test:build_settings.bzl\", \"string_flag\")\n".to_owned();
    for name in names {
        text.push_str(&format!(
            "string_flag(\n    name = \"{name}\",\n    build_setting_default = \"default\",\n)\n"
        ));
    }
    file(t, "test/BUILD", &text);
}

/// `BuildOptions.getDefaultBuildOptionsForFragments(...)`.
fn default_build_options(t: &mut BuildViewTestCase) -> Arc<BuildOptions> {
    t.get_build_options()
}

/// `createBuildOptions(args)`.
fn create_build_options(t: &mut BuildViewTestCase, args: &[&str]) -> Arc<BuildOptions> {
    t.use_configuration(args);
    t.get_build_options()
}

/// `FlagSetValue.Key.create(targets, //test:PROJECT.scl, ...)`.
fn key(
    targets: &[&str],
    scl_config: &str,
    target_options: Arc<BuildOptions>,
    all_option_names: &[&str],
    user_options: &[(&str, &str)],
    enforce_canonical: bool,
) -> FlagSetKey {
    FlagSetKey {
        targets: targets
            .iter()
            .map(|l| Label::parse_canonical(l).unwrap())
            .collect::<IndexSet<_>>(),
        project_file: Label::parse_canonical("//test:PROJECT.scl").unwrap(),
        scl_config: scl_config.to_owned(),
        target_options,
        all_option_names: Arc::new(all_option_names.iter().map(|s| s.to_string()).collect()),
        user_options: Arc::new(
            user_options
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<IndexMap<_, _>>(),
        ),
        config_flag_definitions: Arc::new(ConfigFlagDefinitions::default()),
        enforce_canonical,
    }
}

const TEST_TARGET: &[&str] = &["//test:test_target"];

/// `executeFunction(key)`: the value, or the message of the key's exception.
// Java: FlagSetFunctionTest.executeFunction
fn execute_function(
    t: &mut BuildViewTestCase,
    key: &FlagSetKey,
) -> Result<Arc<FlagSetValue>, String> {
    let result = t.evaluate(&[SkyKey::new(key.clone())], false);
    if result.has_error() {
        return Err(result
            .get_first_error()
            .and_then(|e| e.get_exception())
            .expect("exception")
            .to_string());
    }
    Ok(result.get(key).expect("value"))
}

fn assert_throws(t: &mut BuildViewTestCase, key: &FlagSetKey) -> String {
    match execute_function(t, key) {
        Ok(_) => panic!("expected an exception"),
        Err(message) => message,
    }
}

fn assert_message_contains(message: &str, expected: &str) {
    assert!(
        message.contains(expected),
        "{message:?} does not contain {expected:?}"
    );
}

/// `assertContainsPersistentMessage(value, kind, frequency, message)`.
// Java: FlagSetFunctionTest.assertContainsPersistentMessage
fn assert_contains_persistent_message(
    value: &FlagSetValue,
    kind: EventKind,
    frequency: usize,
    message: &str,
) {
    let mut count = 0;
    for event in value.get_persistent_messages() {
        if event.get_kind() != kind {
            continue;
        }
        count += 1;
        assert_message_contains(&event.get_message(), message);
    }
    assert_eq!(count, frequency);
}

fn options_from_flagset(value: &FlagSetValue) -> Vec<String> {
    value.get_options_from_flagset().iter().cloned().collect()
}

// Java: FlagSetFunctionTest.flagSetsFunction_returns_modified_buildOptions
#[test]
fn flag_sets_function_returns_modified_build_options() {
    let mut t = set_up();
    proto_project(
        &t,
        r#"project = project_pb2.Project.create(
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "test_config",
          flags = ["--platforms=//buildenv/platforms/android:x86"]
      ),
  ],
)"#,
    );
    file(&t, "test/BUILD", "");
    let options = default_build_options(&mut t);
    let key = key(
        TEST_TARGET,
        "test_config",
        options,
        &["platforms"],
        &[],
        true,
    );
    let value = execute_function(&mut t, &key).unwrap();
    assert_eq!(
        options_from_flagset(&value),
        vec!["--platforms=//buildenv/platforms/android:x86"]
    );
}

// Java: FlagSetFunctionTest.noEnforceCanonical_unknown_sclConfig_returns_original_buildOptions
#[test]
fn no_enforce_canonical_unknown_scl_config_returns_original_build_options() {
    let mut t = set_up();
    proto_project(
        &t,
        r#"project = project_pb2.Project.create(
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "test_config",
          flags = ["--platforms=//buildenv/platforms/android:x86"]
      ),
  ],
)"#,
    );
    file(&t, "test/BUILD", "");
    let options = default_build_options(&mut t);
    let key = key(TEST_TARGET, "unknown_config", options, &[], &[], false);
    let value = execute_function(&mut t, &key).unwrap();
    assert!(value.get_options_from_flagset().is_empty());
}

// Java: FlagSetFunctionTest.invalidEnforcementPolicy_fails
#[test]
fn invalid_enforcement_policy_fails() {
    let mut t = set_up();
    proto_project(
        &t,
        r#"project = project_pb2.Project.create(
  enforcement_policy = "invalid",
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "test_config",
          flags = ["--platforms=//buildenv/platforms/android:x86"]
      ),
  ],
)"#,
    );
    file(&t, "test/BUILD", "");
    let options = default_build_options(&mut t);
    let key = key(TEST_TARGET, "test_config", options, &[], &[], true);
    assert_message_contains(
        &assert_throws(&mut t, &key),
        "invalid enforcement_policy 'invalid' in //test:PROJECT.scl",
    );
}

// Java: FlagSetFunctionTest.noEnforceCanonicalConfigs_noConfigsIsNoop
#[test]
fn no_enforce_canonical_configs_no_configs_is_noop() {
    let mut t = set_up();
    file(&t, "test/PROJECT.scl", "");
    file(&t, "test/BUILD", "");
    let options = default_build_options(&mut t);
    let key = key(TEST_TARGET, "random_config_name", options, &[], &[], false);
    let value = execute_function(&mut t, &key).unwrap();
    // Without enforced configs, unknown configs are a no-op.
    assert!(value.get_options_from_flagset().is_empty());
    t.assert_contains_event("Ignoring --scl_config=random_config_name");
}

// Java: FlagSetFunctionTest.noEnforceCanonicalConfigs_sclConfigWarns
#[test]
fn no_enforce_canonical_configs_scl_config_warns() {
    let mut t = set_up();
    proto_project(
        &t,
        r#"project = project_pb2.Project.create(
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "test_config",
          flags = ["--define=bar=bar"]
      ),
  ],
)"#,
    );
    file(&t, "test/BUILD", "");
    let options = default_build_options(&mut t);
    let key = key(TEST_TARGET, "test_config", options, &["define"], &[], false);
    let value = execute_function(&mut t, &key).unwrap();
    assert!(value.get_options_from_flagset().is_empty());
    t.assert_contains_event(
        "Ignoring --scl_config=test_config because --enforce_project_configs is not set",
    );
}

// Java: FlagSetFunctionTest.enforceCanonicalConfigs_noConfigsIsNoop
#[test]
fn enforce_canonical_configs_no_configs_is_noop() {
    let mut t = set_up();
    file(&t, "test/PROJECT.scl", "");
    file(&t, "test/BUILD", "");
    let options = default_build_options(&mut t);
    let key = key(TEST_TARGET, "fake_config", options, &[], &[], false);
    let value = execute_function(&mut t, &key).unwrap();
    assert!(value.get_options_from_flagset().is_empty());
}

// Java: FlagSetFunctionTest.enforceCanonicalConfigsValidConfig
#[test]
fn enforce_canonical_configs_valid_config() {
    let mut t = set_up();
    create_string_flag(&t, "//test:myflag", "default");
    proto_project(
        &t,
        r#"project = project_pb2.Project.create(
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "test_config",
          flags = ["--//test:myflag=test_config_value"],
          is_default = True,
      ),
      buildable_unit_pb2.BuildableUnit.create(
          name = "other_config",
          flags = ["--//test:myflag=other_config_value"],
          description = "user-friendly config description",
      ),
  ],
)"#,
    );
    let options = default_build_options(&mut t);
    let key = key(TEST_TARGET, "other_config", options, &[], &[], true);
    let value = execute_function(&mut t, &key).unwrap();
    assert!(
        value
            .get_options_from_flagset()
            .contains("--//test:myflag=other_config_value")
    );
    assert_contains_persistent_message(
        &value,
        EventKind::Info,
        1,
        "Applying flags from the config 'other_config'",
    );
}

// Java: FlagSetFunctionTest.canonicalConfigs_acceptSpaceDelimitedFlags
#[test]
fn canonical_configs_accept_space_delimited_flags() {
    let mut t = set_up();
    file(&t, "test/BUILD", "");
    proto_project(
        &t,
        r#"project = project_pb2.Project.create(
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "test_config",
          flags = ["--define foo=bar"],
          is_default = True,
      ),
  ],
)"#,
    );
    let options = default_build_options(&mut t);
    let key = key(TEST_TARGET, "test_config", options, &["define"], &[], true);
    let value = execute_function(&mut t, &key).unwrap();
    // space is replaced with =
    assert!(
        value
            .get_options_from_flagset()
            .contains("--define=foo=bar")
    );
    assert_contains_persistent_message(
        &value,
        EventKind::Info,
        1,
        "Applying flags from the config 'test_config'",
    );
}

const STRICT_TWO_CONFIGS: &str = r#"project = project_pb2.Project.create(
  enforcement_policy = "strict",
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "test_config",
          flags = ["--//test:myflag=test_config_value"]
      ),
      buildable_unit_pb2.BuildableUnit.create(
          name = "other_config",
          flags = ["--//test:myflag=other_config_value"]
      ),
  ],
)"#;

fn two_configs_with_policy(policy: &str) -> String {
    STRICT_TWO_CONFIGS.replace("\"strict\"", &format!("\"{policy}\""))
}

// Java: FlagSetFunctionTest.enforceCanonicalConfigsExtraNativeFlag_withSclConfig_fails
#[test]
fn enforce_canonical_configs_extra_native_flag_with_scl_config_fails() {
    let mut t = set_up();
    plain_string_flag_bzl(&t);
    string_flags_build(&t, &["myflag"]);
    proto_project(&t, STRICT_TWO_CONFIGS);
    let options = create_build_options(&mut t, &["--define=foo=bar"]);
    let key = key(
        TEST_TARGET,
        "test_config",
        options,
        &[],
        &[("--define=foo=bar", "")],
        true,
    );
    assert_message_contains(&assert_throws(&mut t, &key), "Found ['--define=foo=bar']");
}

// Java: FlagSetFunctionTest.enforceCanonicalConfigsFlag_warnPolicy_passes
#[test]
fn enforce_canonical_configs_flag_warn_policy_passes() {
    let mut t = set_up();
    plain_string_flag_bzl(&t);
    string_flags_build(&t, &["myflag"]);
    proto_project(&t, &two_configs_with_policy("warn"));
    let options = create_build_options(&mut t, &["--define=foo=bar"]);
    let key = key(
        TEST_TARGET,
        "test_config",
        options,
        &[],
        &[("--define=foo=bar", "")],
        true,
    );
    assert_contains_persistent_message(
        &execute_function(&mut t, &key).unwrap(),
        EventKind::Warning,
        1,
        "also sets output-affecting flags in the command line or user bazelrc: ['--define=foo=bar']",
    );
}

// Java: FlagSetFunctionTest.enforceCanonicalConfigsFlag_compatiblePolicy_unrelatedFlag_warns
#[test]
fn enforce_canonical_configs_flag_compatible_policy_unrelated_flag_warns() {
    let mut t = set_up();
    plain_string_flag_bzl(&t);
    string_flags_build(&t, &["myflag"]);
    proto_project(&t, &two_configs_with_policy("compatible"));
    let options = create_build_options(&mut t, &["--define=foo=bar"]);
    let key = key(
        TEST_TARGET,
        "test_config",
        options,
        &[],
        &[("--define=foo=bar", "")],
        true,
    );
    assert_contains_persistent_message(
        &execute_function(&mut t, &key).unwrap(),
        EventKind::Warning,
        1,
        "also sets output-affecting flags in the command line or user bazelrc: ['--define=foo=bar']",
    );
}

// Java: FlagSetFunctionTest.enforceCanonicalConfigs_compatiblePolicy_onlyDifferentValue_fails
#[test]
fn enforce_canonical_configs_compatible_policy_only_different_value_fails() {
    let mut t = set_up();
    plain_string_flag_bzl(&t);
    string_flags_build(&t, &["myflag", "other_flag"]);
    proto_project(
        &t,
        r#"project = project_pb2.Project.create(
  enforcement_policy = "compatible",
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "test_config",
          flags = ["--//test:myflag=test_config_value", "--//test:other_flag=test_config_value"]
      ),
  ],
)"#,
    );
    let options = create_build_options(
        &mut t,
        &[
            "--//test:myflag=other_value",
            "--//test:other_flag=test_config_value",
        ],
    );
    let key = key(
        TEST_TARGET,
        "test_config",
        options,
        &[],
        &[
            ("--//test:myflag=other_value", ""),
            ("--//test:other_flag=test_config_value", ""),
        ],
        true,
    );
    let message = assert_throws(&mut t, &key);
    assert_message_contains(&message, "Found ['--//test:myflag=other_value']");
    assert!(!message.contains("['--//test:other_flag=test_config_value']"));
}

fn assert_old_schema_configs_error(project: &str) {
    let mut t = set_up();
    file(&t, "test/PROJECT.scl", project);
    file(&t, "test/BUILD", "");
    let options = create_build_options(&mut t, &["--define=foo=bar"]);
    let key = key(TEST_TARGET, "test_config", options, &[], &[], true);
    assert_message_contains(
        &assert_throws(&mut t, &key),
        "configs variable must be a map of strings to lists of strings",
    );
}

// Java: FlagSetFunctionTest.oldSchema_enforceCanonicalConfigs_wrongConfigsType
#[test]
fn old_schema_enforce_canonical_configs_wrong_configs_type() {
    assert_old_schema_configs_error("project = {\n  \"configs\": 1,\n}\n");
}

// Java: FlagSetFunctionTest.oldSchema_enforceCanonicalConfigs_wrongConfigsKeyType
#[test]
fn old_schema_enforce_canonical_configs_wrong_configs_key_type() {
    assert_old_schema_configs_error(
        "project = {\n  \"configs\": {\n    123: [\"--compilation_mode=opt\"],\n  },\n}\n",
    );
}

// Java: FlagSetFunctionTest.oldSchema_enforceCanonicalConfigs_wrongConfigsValueType
#[test]
fn old_schema_enforce_canonical_configs_wrong_configs_value_type() {
    assert_old_schema_configs_error(
        "project = {\n  \"configs\": {\n    \"test_config\": 123,\n  },\n}\n",
    );
}

// Java: FlagSetFunctionTest.enforceCanonicalConfigsExtraFakeExpandedFlag_withSclConfig_fails
#[test]
#[ignore = "needs DummyTestFragment (its --bar option must be a build option for the user option to count)"]
fn enforce_canonical_configs_extra_fake_expanded_flag_with_scl_config_fails() {
    let mut t = set_up();
    scoped_string_flag_bzl(&t);
    string_flags_build(&t, &["myflag"]);
    proto_project(&t, STRICT_TWO_CONFIGS);
    // this is a fake flag so don't add it here.
    let options = create_build_options(&mut t, &[]);
    let key = key(
        TEST_TARGET,
        "test_config",
        options,
        &[],
        &[("--bar", "--config=foo")],
        true,
    );
    assert_message_contains(&assert_throws(&mut t, &key), "Found ['--config=foo']");
}

const STRICT_ONE_CONFIG: &str = r#"project = project_pb2.Project.create(
  enforcement_policy = "strict",
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "test_config",
          flags = ["--//test:myflag=test_config_value"]
      ),
  ],
)"#;

// Java: FlagSetFunctionTest.enforceCanonicalConfigs_extraFlagThatIsAlsoInConfig_differentValue_fails
#[test]
fn enforce_canonical_configs_extra_flag_that_is_also_in_config_different_value_fails() {
    let mut t = set_up();
    scoped_string_flag_bzl(&t);
    string_flags_build(&t, &["myflag"]);
    proto_project(&t, STRICT_ONE_CONFIG);
    let options = create_build_options(&mut t, &["--//test:myflag=other_value"]);
    let key = key(
        TEST_TARGET,
        "test_config",
        options,
        &[],
        &[("--//test:myflag=other_value", "")],
        true,
    );
    assert_message_contains(
        &assert_throws(&mut t, &key),
        "Found ['--//test:myflag=other_value']",
    );
}

// Java: FlagSetFunctionTest.enforceCanonicalConfigs_passedFlagThatIsInConfig_passes
#[test]
fn enforce_canonical_configs_passed_flag_that_is_in_config_passes() {
    let mut t = set_up();
    scoped_string_flag_bzl(&t);
    string_flags_build(&t, &["myflag"]);
    proto_project(
        &t,
        &STRICT_ONE_CONFIG.replace("  enforcement_policy = \"strict\",\n", ""),
    );
    let options = create_build_options(&mut t, &["--//test:myflag=test_config_value"]);
    let key = key(
        TEST_TARGET,
        "test_config",
        options,
        &[],
        &[("--//test:myflag=test_config_value", "")],
        true,
    );
    execute_function(&mut t, &key).unwrap();
    t.assert_does_not_contain_event("--scl_config must be the only configuration-affecting flag");
}

// Java: FlagSetFunctionTest.enforceCanonicalConfigsExtraStarlarkFlag_fails
#[test]
fn enforce_canonical_configs_extra_starlark_flag_fails() {
    let mut t = set_up();
    scoped_string_flag_bzl(&t);
    string_flags_build(
        &t,
        &["myflag", "starlark_flags_always_affect_configuration"],
    );
    proto_project(&t, STRICT_TWO_CONFIGS);
    let options = create_build_options(
        &mut t,
        &["--//test:starlark_flags_always_affect_configuration=yes_they_do"],
    );
    let key = key(
        TEST_TARGET,
        "test_config",
        options,
        &[],
        &[(
            "--//test:starlark_flags_always_affect_configuration=yes_they_do",
            "",
        )],
        true,
    );
    assert_message_contains(
        &assert_throws(&mut t, &key),
        "Found ['--//test:starlark_flags_always_affect_configuration=yes_they_do']",
    );
}

// Java: FlagSetFunctionTest.enforceCanonicalConfigsExtraTestFlag_passes
#[test]
fn enforce_canonical_configs_extra_test_flag_passes() {
    let mut t = set_up();
    scoped_string_flag_bzl(&t);
    string_flags_build(
        &t,
        &["myflag", "starlark_flags_always_affect_configuration"],
    );
    proto_project(&t, STRICT_TWO_CONFIGS);
    let options = create_build_options(
        &mut t,
        &[
            "--test_filter=foo",
            "--cache_test_results=true",
            "--test_arg=blah",
        ],
    );
    let key = key(
        TEST_TARGET,
        "test_config",
        options,
        &[],
        &[
            ("--test_filter=foo", ""),
            ("--cache_test_results=true", ""),
            ("--test_arg=blah", ""),
        ],
        true,
    );
    execute_function(&mut t, &key).unwrap();
    t.assert_does_not_contain_event("--scl_config must be the only configuration-affecting flag");
}

// Java: FlagSetFunctionTest.noEnforceCanonicalConfigs_noSclConfig_extraFlag_passes
#[test]
fn no_enforce_canonical_configs_no_scl_config_extra_flag_passes() {
    let mut t = set_up();
    plain_string_flag_bzl(&t);
    string_flags_build(&t, &["myflag"]);
    proto_project(
        &t,
        &STRICT_TWO_CONFIGS.replace("  enforcement_policy = \"strict\",\n", ""),
    );
    let options = create_build_options(&mut t, &["--define=foo=bar"]);
    let key = key(
        TEST_TARGET,
        "test_config",
        options,
        &[],
        &[("--define=foo=bar", "")],
        false,
    );
    execute_function(&mut t, &key).unwrap();
    t.assert_does_not_contain_event("--scl_config must be the only configuration-affecting flag");
}

// Java: FlagSetFunctionTest.enforceCanonicalConfigsNonExistentConfig_fails
#[test]
fn enforce_canonical_configs_non_existent_config_fails() {
    let mut t = set_up();
    create_string_flag(&t, "//test:myflag", "default");
    proto_project(&t, STRICT_ONE_CONFIG);
    let options = default_build_options(&mut t);
    let key = key(TEST_TARGET, "non_existent_config", options, &[], &[], true);
    assert_message_contains(
        &assert_throws(&mut t, &key),
        "--scl_config=non_existent_config is not a valid configuration for this project",
    );
}

// Java: FlagSetFunctionTest.enforceCanonicalConfigsNoSclConfigFlagNoDefaultConfig
#[test]
fn enforce_canonical_configs_no_scl_config_flag_no_default_config() {
    let mut t = set_up();
    create_string_flag(&t, "//test:myflag", "default");
    proto_project(
        &t,
        &STRICT_TWO_CONFIGS.replace("  enforcement_policy = \"strict\",\n", ""),
    );
    let options = default_build_options(&mut t);
    let key = key(TEST_TARGET, "", options, &[], &[], true);
    assert_message_contains(
        &assert_throws(&mut t, &key),
        "This project's builds must set --scl_config because no default config is defined",
    );
}

fn assert_old_schema_default_config_error(default_config: &str) {
    let mut t = set_up();
    create_string_flag(&t, "//test:myflag", "default");
    file(
        &t,
        "test/PROJECT.scl",
        &format!(
            "project = {{\n  \"configs\": {{\n    \"test_config\": ['--//test:myflag=test_config_value'],\n    \"other_config\": ['--//test:myflag=other_config_value'],\n  }},\n  \"default_config\": {default_config},\n}}\n"
        ),
    );
    let options = default_build_options(&mut t);
    let key = key(TEST_TARGET, "", options, &[], &[], true);
    assert_message_contains(
        &assert_throws(&mut t, &key),
        "default_config must be a string matching a configs variable definition",
    );
}

// Java: FlagSetFunctionTest.oldSchema_enforceCanonicalConfigsNonexistentDefaultConfig
#[test]
fn old_schema_enforce_canonical_configs_nonexistent_default_config() {
    assert_old_schema_default_config_error("\"nonexistent_config\"");
}

// Java: FlagSetFunctionTest.oldSchema_enforceCanonicalConfigs_wrongDefaultConfigType
#[test]
fn old_schema_enforce_canonical_configs_wrong_default_config_type() {
    assert_old_schema_default_config_error("[\"test_config\"]");
}

const DEFAULT_MYFLAG_CONFIG: &str = r#"project = project_pb2.Project.create(
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "test_config",
          flags = ["--//test:myflag=test_config_value"],
          is_default = True,
          description = "user-friendly config description",
      ),
  ],
)"#;

// Java: FlagSetFunctionTest.enforceCanonicalConfigsNoSclConfigFlagValidDefaultConfig
#[test]
fn enforce_canonical_configs_no_scl_config_flag_valid_default_config() {
    let mut t = set_up();
    create_string_flag(&t, "//test:myflag", "default");
    proto_project(&t, DEFAULT_MYFLAG_CONFIG);
    let options = default_build_options(&mut t);
    let key = key(TEST_TARGET, "", options, &[], &[], true);
    let value = execute_function(&mut t, &key).unwrap();
    assert_eq!(
        options_from_flagset(&value),
        vec!["--//test:myflag=test_config_value"]
    );
    assert_contains_persistent_message(
        &value,
        EventKind::Info,
        1,
        "Applying flags from the config 'test_config'",
    );
}

// Java: FlagSetFunctionTest.directlySet_nonBuildOptions_fails
#[test]
fn directly_set_non_build_options_fails() {
    let mut t = set_up();
    create_string_flag(&t, "//test:myflag", "default");
    proto_project(
        &t,
        r#"project = project_pb2.Project.create(
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "test_config",
          flags = ["--bazelrc=foo"],
          is_default = True,
          description = "user-friendly config description",
   ),
  ],
)"#,
    );
    let options = default_build_options(&mut t);
    let key = key(TEST_TARGET, "", options, &["bazelrc"], &[], true);
    assert_message_contains(
        &assert_throws(&mut t, &key),
        "project flags don't support non-output affecting option: --bazelrc",
    );
}

// Java: FlagSetFunctionTest.directlySet_unrecognizedFlag_fails
#[test]
fn directly_set_unrecognized_flag_fails() {
    let mut t = set_up();
    create_string_flag(&t, "//test:myflag", "default");
    proto_project(
        &t,
        r#"project = project_pb2.Project.create(
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "test_config",
          flags = ["--not_a_flag=1"],
          is_default = True,
      ),
  ],
)"#,
    );
    let options = default_build_options(&mut t);
    let key = key(TEST_TARGET, "", options, &[], &[], true);
    assert_message_contains(
        &assert_throws(&mut t, &key),
        "unrecognized option: --not_a_flag",
    );
}

// Java: FlagSetFunctionTest.directlySet_starlarkFlag (@TestParameters)
#[test]
fn directly_set_starlark_flag() {
    for (label, expect_success) in [
        ("//test:myflag", true),
        ("//test:not_a_target", false),
        ("//not_a_package:myflag", false),
        ("//other:not_a_flag", false),
    ] {
        let mut t = set_up();
        create_string_flag(&t, "//test:myflag", "default");
        file(
            &t,
            "other/BUILD",
            "genrule(name = \"not_a_flag\", outs = [\"out\"], cmd = \"echo foo > $@\")\n",
        );
        proto_project(
            &t,
            &format!(
                "project = project_pb2.Project.create(\n  buildable_units = [\n      buildable_unit_pb2.BuildableUnit.create(\n          name = \"test_config\",\n          flags = [\"--{label}=1\"],\n          is_default = True,\n      ),\n  ],\n)"
            ),
        );
        let options = default_build_options(&mut t);
        let key = key(TEST_TARGET, "", options, &[], &[], true);
        if expect_success {
            let value = execute_function(&mut t, &key).unwrap();
            assert_eq!(options_from_flagset(&value), vec![format!("--{label}=1")]);
            assert_contains_persistent_message(
                &value,
                EventKind::Info,
                1,
                "Applying flags from the config 'test_config'",
            );
        } else {
            assert_message_contains(
                &assert_throws(&mut t, &key),
                &format!("unrecognized Starlark flag: --{label}"),
            );
        }
    }
}

// Java: FlagSetFunctionTest.basicFlagsetFunctionalityWithTopLevelProjectSchema
#[test]
fn basic_flagset_functionality_with_top_level_project_schema() {
    let mut t = set_up();
    create_string_flag(&t, "//test:myflag", "default");
    proto_project(&t, DEFAULT_MYFLAG_CONFIG);
    let options = default_build_options(&mut t);
    let key = key(TEST_TARGET, "", options, &[], &[], true);
    let value = execute_function(&mut t, &key).unwrap();
    assert_eq!(
        options_from_flagset(&value),
        vec!["--//test:myflag=test_config_value"]
    );
    assert_contains_persistent_message(
        &value,
        EventKind::Info,
        1,
        "Applying flags from the config 'test_config'",
    );
}

// Java: FlagSetFunctionTest.clearUserDocumentationOfValidConfigs
#[test]
#[ignore = "fails at the pin: the expected text lacks the blank lines FlagSetFunction.supportedConfigsDesc writes after the first line and before \"This policy is defined\" (the oracle prints them: scripts/config/compare case fs-scl-missing)"]
fn clear_user_documentation_of_valid_configs() {
    let mut t = set_up();
    create_string_flag(&t, "//test:myflag", "default");
    proto_project(
        &t,
        r#"project = project_pb2.Project.create(
  enforcement_policy = "strict",
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "debug",
          flags = ["--//test:myflag=debug_value"],
      ),
      buildable_unit_pb2.BuildableUnit.create(
          name = "release",
          flags = ["--//test:myflag=release_value"],
      ),
  ],
)"#,
    );
    let options = default_build_options(&mut t);
    // sclConfig = null (nullToEmpty).
    let key = key(TEST_TARGET, "", options, &[], &[], true);
    assert_message_contains(
        &assert_throws(&mut t, &key),
        "This project's builds must set --scl_config because no default config is defined.\n\
This project supports:\n  \
--scl_config=debug   -> [--//test:myflag=debug_value]\n  \
--scl_config=release -> [--//test:myflag=release_value]\n\
This policy is defined in test/PROJECT.scl.\n",
    );
}

const TWO_TARGETS: &[&str] = &["//test:test_target", "//test:test_target2"];

// Java: FlagSetFunctionTest.buildingMultipleTargets_withSameConfig_isAllowed
#[test]
fn building_multiple_targets_with_same_config_is_allowed() {
    let mut t = set_up();
    create_string_flag(&t, "//test:myflag", "default");
    proto_project(
        &t,
        r#"project = project_pb2.Project.create(
  enforcement_policy = "warn",
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "default",
          flags = [],
          is_default = True,
      ),
      buildable_unit_pb2.BuildableUnit.create(
          name = "debug",
          flags = ["--//test:myflag=debug_value"],
      ),
      buildable_unit_pb2.BuildableUnit.create(
          name = "release",
          flags = ["--//test:myflag=release_value"],
      ),
  ],
)"#,
    );
    let options = default_build_options(&mut t);
    let key = key(TWO_TARGETS, "", options, &[], &[], true);
    execute_function(&mut t, &key).unwrap();
    t.assert_does_not_contain_event(
        "Cannot parse options: Building target(s) with different configurations are not supported",
    );
}

fn two_defaults_project(first: &str, second: &str) -> String {
    format!(
        r#"project = project_pb2.Project.create(
  buildable_units = [
      buildable_unit_pb2.BuildableUnit.create(
          name = "foo_default",
          target_patterns = ["//test:test_target"],
          flags = ["--//test:myflag={first}"],
          is_default = True,
      ),
      buildable_unit_pb2.BuildableUnit.create(
          name = "bar_default",
          target_patterns = ["//test:test_target2"],
          flags = ["--//test:myflag={second}"],
          is_default = True,
      ),
  ],
)"#
    )
}

// Java: FlagSetFunctionTest.multipleTargetsWithMismatchingDefaultBuildableUnitsFails
#[test]
fn multiple_targets_with_mismatching_default_buildable_units_fails() {
    let mut t = set_up();
    create_string_flag(&t, "//test:myflag", "default");
    proto_project(
        &t,
        &two_defaults_project("foo_default_value", "bar_default_value"),
    );
    let options = default_build_options(&mut t);
    let key = key(TWO_TARGETS, "", options, &[], &[], true);
    assert_message_contains(
        &assert_throws(&mut t, &key),
        "Building target(s) with different configurations are not supported",
    );
}

// Java: FlagSetFunctionTest.multipleTargetsWithDifferentDefaultsSucceedsIfSameFlags
#[test]
fn multiple_targets_with_different_defaults_succeeds_if_same_flags() {
    let mut t = set_up();
    create_string_flag(&t, "//test:myflag", "default");
    proto_project(
        &t,
        &two_defaults_project("common_default_value", "common_default_value"),
    );
    let options = default_build_options(&mut t);
    let key = key(TWO_TARGETS, "", options, &[], &[], true);
    execute_function(&mut t, &key).unwrap();
    t.assert_no_events();
}

// Java: FlagSetFunctionTest.doesBuildableUnitMatchTarget (TargetPatternProvider)
#[test]
fn does_buildable_unit_match_target_test() {
    let cases: &[(bool, &[&str], &str)] = &[
        // Single pattern
        (true, &["//foo:foo"], "//foo:foo"),
        (false, &["//foo:foo"], "//foo:bar"),
        (true, &["//foo/..."], "//foo:foo"),
        (true, &["//foo/..."], "//foo/bar:bar"),
        (false, &["//foo/..."], "//bar:bar"),
        (false, &["//foo/bar/..."], "//foo:foo"),
        // Multiple patterns
        (true, &["//foo:foo", "//bar:bar"], "//foo:foo"),
        (true, &["//foo:foo", "//bar:bar"], "//bar:bar"),
        (false, &["//foo:foo", "//bar:bar"], "//quux:quux"),
        // Negative patterns
        (false, &["-//foo:foo"], "//foo:foo"),
        (false, &["-//foo/..."], "//foo:foo"),
        (false, &["//foo/...", "-//foo/bar/..."], "//foo/bar:bar"),
        (true, &["//foo/...", "-//foo/bar/..."], "//foo:foo"),
        (
            true,
            &["//foo/...", "-//foo/bar/...", "//foo/bar/baz/..."],
            "//foo/bar/baz",
        ),
        (
            true,
            &["//foo/...", "-//foo/bar/...", "//foo/bar/baz/..."],
            "//foo:foo",
        ),
        (
            false,
            &["//foo/...", "-//foo/bar/...", "//foo/bar/baz/..."],
            "//foo/bar/quux",
        ),
    ];
    for (included, patterns, label) in cases {
        let buildable_unit = BuildableUnit::create(
            "test".to_owned(),
            patterns.iter().map(|p| p.to_string()).collect(),
            "Test Unit".to_owned(),
            vec!["--flag".to_owned()],
            true,
        )
        .unwrap();
        let label = Label::parse_canonical(label).unwrap();
        assert_eq!(
            does_buildable_unit_match_target(&buildable_unit, &label),
            *included,
            "{patterns:?} {label}"
        );
    }
}
