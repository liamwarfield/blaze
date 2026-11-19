//! Port of
//! `src/test/java/com/google/devtools/build/lib/analysis/config/BuildConfigurationValueTest.java`
//! (a `ConfigurationTestCase`: configurations are created through the configuration
//! SkyFunctions of the mock analysis environment, `blaze_sky_top::testing`).

use std::sync::Arc;

use blaze_analysis::config::build_option_details::BuildOptionDetails;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::build_options::StarlarkOptionValue;
use blaze_cmdline::Label;
use blaze_cmdline::repository_name::MAIN;
use blaze_optdefs::fragment::compilation_mode::CompilationMode;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_options::Options;
use blaze_options::definition::OptionsBase;
use blaze_sky_top::testing::ConfigurationTestCase;
use blaze_sky_top::testing::analysis_mock::{PIII_PLATFORM_LABEL, PLATFORM_LABEL};
use indexmap::IndexMap;

#[test]
fn test_basics() {
    // `if (analysisMock.isThisBazel()) return;`: Bazel's AnalysisMock is "this Bazel", so the Java
    // test returns before any assertion.
}

#[test]
fn test_platform_suffix() {
    // `if (analysisMock.isThisBazel()) return;`: Bazel's AnalysisMock is "this Bazel", so the Java
    // test returns before any assertion.
}

#[test]
fn test_environment() {
    // `if (analysisMock.isThisBazel()) return;`: Bazel's AnalysisMock is "this Bazel", so the Java
    // test returns before any assertion.
}

#[test]
fn test_caching() {
    let a = Options::<CoreOptions>::get_defaults();
    let b = Options::<CoreOptions>::get_defaults();
    let a: &dyn OptionsBase = &a;
    let b: &dyn OptionsBase = &b;
    // The String representations of the CoreOptions must be equal even if these are
    // different objects, if they were created with the same options (no options in this case).
    assert_eq!(b.java_to_string(), a.java_to_string());
    assert_eq!(b.cache_key(), a.cache_key());
}

/// Truth's `String.matches(regex)`: the whole string matches.
fn assert_matches(actual: &str, regex: &str) {
    let re = regex::Regex::new(&format!("^(?:{regex})$")).expect("regex");
    assert!(re.is_match(actual), "<{actual}> does not match <{regex}>");
}

fn output_root(config: &BuildConfigurationValue) -> String {
    config
        .get_output_directory(&MAIN)
        .get_root_path()
        .get_path_string()
        .to_string()
}

fn label(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

fn str_value(s: &str) -> StarlarkOptionValue {
    StarlarkOptionValue::Str(s.to_string())
}

/// Truth's `containsExactly` on a map: the same entries, in any order.
fn assert_starlark_options_exactly(
    actual: &IndexMap<Label, StarlarkOptionValue>,
    expected: &[(&str, &str)],
) {
    let expected: IndexMap<Label, StarlarkOptionValue> = expected
        .iter()
        .map(|(k, v)| (label(k), str_value(v)))
        .collect();
    assert_eq!(
        actual.len(),
        expected.len(),
        "{actual:?} is not exactly {expected:?}"
    );
    for (k, v) in &expected {
        assert_eq!(
            actual.get(k),
            Some(v),
            "{actual:?} is not exactly {expected:?}"
        );
    }
}

#[test]
fn test_target_environment() {
    let mut t = ConfigurationTestCase::new();
    // PORT: BuildConfigurationValue.getTargetEnvironments is not ported; Java returns
    // `options.targetEnvironments` (CoreOptions).
    let target_environments = |config: &Arc<BuildConfigurationValue>| -> Vec<Label> {
        config
            .get_options()
            .get::<CoreOptions>()
            .expect("CoreOptions")
            .target_environments
            .iter()
            .map(|l| l.0.clone())
            .collect()
    };
    let one_env_config = t.create(&["--target_environment=//foo"]);
    assert_eq!(target_environments(&one_env_config), vec![label("//foo")]);

    let two_envs_config = t.create(&["--target_environment=//foo", "--target_environment=//bar"]);
    let mut two = target_environments(&two_envs_config);
    two.sort();
    let mut expected = vec![label("//foo"), label("//bar")];
    expected.sort();
    assert_eq!(two, expected);

    let no_envs_config = t.create(&[]);
    assert!(target_environments(&no_envs_config).is_empty());
}

#[test]
fn test_global_make_variable_override() {
    let mut t = ConfigurationTestCase::new();
    assert_eq!(
        t.create(&[])
            .get_make_environment()
            .get("COMPILATION_MODE")
            .map(String::as_str),
        Some("fastbuild")
    );
    let config = t.create(&["--define", "COMPILATION_MODE=fluttershy"]);
    assert_eq!(
        config
            .get_make_environment()
            .get("COMPILATION_MODE")
            .map(String::as_str),
        Some("fluttershy")
    );
}

#[test]
fn test_target_cpu_from_cpu_flag() {
    let mut t = ConfigurationTestCase::new();
    let platforms = format!("--platforms={PLATFORM_LABEL}");
    let config = t.create(&[
        "--noincompatible_target_cpu_from_platform",
        "--cpu=piii",
        &platforms,
    ]);
    assert_eq!(
        config
            .get_make_environment()
            .get("TARGET_CPU")
            .map(String::as_str),
        Some("piii")
    );
}

#[test]
fn test_target_cpu_from_platform() {
    let mut t = ConfigurationTestCase::new();
    let platforms = format!("--platforms={PLATFORM_LABEL}");
    let config = t.create(&[
        "--cpu=piii",
        &platforms,
        "--incompatible_target_cpu_from_platform",
    ]);
    assert_eq!(
        config
            .get_make_environment()
            .get("TARGET_CPU")
            .map(String::as_str),
        Some("x86_64")
    );
}

#[test]
fn test_target_cpu_from_platform_with_cpu_mapping() {
    let mut t = ConfigurationTestCase::new();
    let platforms = format!("--platforms={PLATFORM_LABEL}");
    let mapping = format!("--experimental_override_platform_cpu_name={PLATFORM_LABEL}=new_cpu");
    let config = t.create(&[
        "--cpu=piii",
        &platforms,
        "--incompatible_target_cpu_from_platform",
        &mapping,
    ]);
    assert_eq!(
        config
            .get_make_environment()
            .get("TARGET_CPU")
            .map(String::as_str),
        Some("new_cpu")
    );
}

#[test]
fn test_target_cpu_from_platform_multiple_cpu_mappings_last_one_wins() {
    let mut t = ConfigurationTestCase::new();
    let platforms = format!("--platforms={PLATFORM_LABEL}");
    let mapping1 = format!("--experimental_override_platform_cpu_name={PLATFORM_LABEL}=new_cpu_1");
    let mapping2 = format!("--experimental_override_platform_cpu_name={PLATFORM_LABEL}=new_cpu_2");
    let config = t.create(&[
        "--cpu=piii",
        &platforms,
        "--incompatible_target_cpu_from_platform",
        &mapping1,
        &mapping2,
    ]);
    assert_eq!(
        config
            .get_make_environment()
            .get("TARGET_CPU")
            .map(String::as_str),
        Some("new_cpu_2")
    );
}

#[test]
fn test_exec_config_target_cpu_from_platform() {
    let mut t = ConfigurationTestCase::new();
    let host_platform = format!("--host_platform={PIII_PLATFORM_LABEL}");
    let config = t.create_exec(&[
        "--cpu=x86_64",
        &host_platform,
        "--incompatible_target_cpu_from_platform",
    ]);
    assert_eq!(
        config
            .get_make_environment()
            .get("TARGET_CPU")
            .map(String::as_str),
        Some("x86_32")
    );
}

#[test]
fn test_exec_config_target_cpu_from_platform_with_cpu_mapping() {
    let mut t = ConfigurationTestCase::new();
    let host_platform = format!("--host_platform={PIII_PLATFORM_LABEL}");
    let mapping =
        format!("--experimental_override_platform_cpu_name={PIII_PLATFORM_LABEL}=new_cpu");
    let config = t.create_exec(&[
        "--cpu=x86_64",
        &host_platform,
        "--incompatible_target_cpu_from_platform",
        &mapping,
    ]);
    assert_eq!(
        config
            .get_make_environment()
            .get("TARGET_CPU")
            .map(String::as_str),
        Some("new_cpu")
    );
}

#[test]
fn test_get_build_option_details() {
    let mut t = ConfigurationTestCase::new();
    // Java: BuildConfigurationValue.getBuildOptionDetails() is
    // BuildOptionDetails.forOptions(buildOptions), ported in blaze_analysis.
    let details = |config: Arc<BuildConfigurationValue>| {
        BuildOptionDetails::for_options(config.get_options())
    };
    // Directly defined options:
    let d = details(t.create(&["-c", "dbg"]));
    assert_eq!(
        d.get_option_value("compilation_mode")
            .and_then(|v| v.downcast_ref::<CompilationMode>().cloned()),
        Some(CompilationMode::DBG)
    );
    let d = details(t.create(&["-c", "opt"]));
    assert_eq!(
        d.get_option_value("compilation_mode")
            .and_then(|v| v.downcast_ref::<CompilationMode>().cloned()),
        Some(CompilationMode::OPT)
    );

    // Options defined in a fragment:
    let d = details(t.create(&["--force_pic"]));
    assert_eq!(
        d.get_option_value("force_pic")
            .and_then(|v| v.downcast_ref::<bool>().cloned()),
        Some(true)
    );
    let d = details(t.create(&["--noforce_pic"]));
    assert_eq!(
        d.get_option_value("force_pic")
            .and_then(|v| v.downcast_ref::<bool>().cloned()),
        Some(false)
    );

    // Legitimately null option:
    let d = details(t.create(&[]));
    assert!(d.get_option_value("test_filter").is_none());
}

#[test]
#[ignore = "needs a caching FragmentFactory (the Rust FragmentFactory does not cache fragments, so J2ObjcConfiguration instances are never shared)"]
fn test_config_fragments_are_shareable_across_configurations() {}

#[test]
fn test_command_line_variables() {
    let mut t = ConfigurationTestCase::new();
    let config = t.create(&[
        "--define",
        "a=b/c:d",
        "--define",
        "b=FOO",
        "--define",
        "DEFUN=Nope",
    ]);
    let vars = config.get_command_line_build_variables();
    assert_eq!(vars.get("a").map(String::as_str), Some("b/c:d"));
    assert_eq!(vars.get("b").map(String::as_str), Some("FOO"));
    assert_eq!(vars.get("DEFUN").map(String::as_str), Some("Nope"));
}

// Regression test for bug #2518997:
// "--define in blazerc overrides --define from command line"
#[test]
fn test_command_line_variables_override() {
    let mut t = ConfigurationTestCase::new();
    let config = t.create(&["--define", "a=b", "--define", "a=c"]);
    assert_eq!(
        config
            .get_command_line_build_variables()
            .get("a")
            .map(String::as_str),
        Some("c")
    );
}

#[test]
fn test_normalization_defines_with_different_names() {
    let mut t = ConfigurationTestCase::new();
    let config = t.create(&["--define", "a=1", "--define", "b=2"]);
    let options = config
        .get_options()
        .get::<CoreOptions>()
        .expect("CoreOptions");
    let map: IndexMap<String, String> = options
        .command_line_build_variables
        .iter()
        .cloned()
        .collect();
    assert_eq!(map.len(), 2);
    assert_eq!(map.get("a").map(String::as_str), Some("1"));
    assert_eq!(map.get("b").map(String::as_str), Some("2"));
}

#[test]
fn test_normalization_defines_with_same_name() {
    let mut t = ConfigurationTestCase::new();
    let config = t.create(&["--define", "a=1", "--define", "a=2"]);
    let options = config
        .get_options()
        .get::<CoreOptions>()
        .expect("CoreOptions");
    let map: IndexMap<String, String> = options
        .command_line_build_variables
        .iter()
        .cloned()
        .collect();
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("a").map(String::as_str), Some("2"));
    assert_eq!(*config, *t.create(&["--define", "a=2"]));
}

// This is really a test of option parsing, not command-line variable
// semantics.
#[test]
fn test_command_line_variables_with_funny_characters() {
    let mut t = ConfigurationTestCase::new();
    let config = t.create(&[
        "--define",
        "foo=#foo",
        "--define",
        "comma=a,b",
        "--define",
        "space=foo bar",
        "--define",
        "thing=a \"quoted\" thing",
        "--define",
        "qspace=a\\ quoted\\ space",
        "--define",
        "#a=pounda",
    ]);
    let vars = config.get_command_line_build_variables();
    assert_eq!(vars.get("foo").map(String::as_str), Some("#foo"));
    assert_eq!(vars.get("comma").map(String::as_str), Some("a,b"));
    assert_eq!(vars.get("space").map(String::as_str), Some("foo bar"));
    assert_eq!(
        vars.get("thing").map(String::as_str),
        Some("a \"quoted\" thing")
    );
    assert_eq!(
        vars.get("qspace").map(String::as_str),
        Some("a\\ quoted\\ space")
    );
    assert_eq!(vars.get("#a").map(String::as_str), Some("pounda"));
}

#[test]
fn test_exec_define_is_allowed_by_default() {
    let mut t = ConfigurationTestCase::new();
    let cfg = t.create_exec(&["--define=foo=bar"]);
    assert_eq!(
        cfg.get_command_line_build_variables()
            .get("foo")
            .map(String::as_str),
        Some("bar")
    );
}

#[test]
fn test_exec_define_is_ignored_if_excluded_and_not_allowed() {
    let mut t = ConfigurationTestCase::new();
    let cfg = t.create_exec(&[
        "--define=foo=bar",
        "--experimental_exclude_defines_from_exec_config=true",
    ]);
    assert!(!cfg.get_command_line_build_variables().contains_key("foo"));
}

#[test]
fn test_exec_define_is_propagated_if_allowed_by_flag() {
    let mut t = ConfigurationTestCase::new();
    let cfg = t.create_exec(&[
        "--define=foo=bar",
        "--experimental_exclude_defines_from_exec_config=true",
        "--experimental_propagate_custom_flag=foo",
        "--define=baz=qux",
    ]);
    assert_eq!(
        cfg.get_command_line_build_variables()
            .get("foo")
            .map(String::as_str),
        Some("bar")
    );
    assert_ne!(
        cfg.get_command_line_build_variables()
            .get("baz")
            .map(String::as_str),
        Some("qux")
    );
}

const BOOL_FLAG_RULE_DEFS: &[&str] = &[
    "def _impl(ctx):",
    "    return []",
    "",
    "bool_flag = rule(",
    "    implementation = _impl,",
    "    build_setting = config.bool(flag = True),",
    ")",
];

#[test]
fn test_exec_starlark_flag_is_allowed_by_default() {
    let mut t = ConfigurationTestCase::new();
    t.scratch
        .file("my_starlark_flag/rule_defs.bzl", BOOL_FLAG_RULE_DEFS);
    t.scratch.file(
        "my_starlark_flag/BUILD",
        &[
            "load(\"//my_starlark_flag:rule_defs.bzl\", \"bool_flag\")",
            "",
            "bool_flag(",
            "    name = \"starlark_flag\",",
            "    build_setting_default = False,",
            ")",
        ],
    );
    let cfg = t.create_exec_with_starlark(&[("//my_starlark_flag:starlark_flag", "true")], &[]);
    assert_eq!(
        cfg.get_options()
            .get_starlark_options()
            .get(&label("//my_starlark_flag:starlark_flag")),
        Some(&str_value("true"))
    );
}

#[test]
fn test_exec_starlark_flag_is_ignored_if_excluded_and_not_allowed() {
    let mut t = ConfigurationTestCase::new();
    t.scratch
        .file("my_starlark_flag/rule_defs.bzl", BOOL_FLAG_RULE_DEFS);
    t.scratch.file(
        "my_starlark_flag/BUILD",
        &[
            "load(\"//my_starlark_flag:rule_defs.bzl\", \"bool_flag\")",
            "",
            "bool_flag(",
            "    name = \"starlark_flag\",",
            "    build_setting_default = False,",
            ")",
        ],
    );
    let cfg = t.create_exec_with_starlark(
        &[("//my_starlark_flag:starlark_flag", "true")],
        &["--experimental_exclude_starlark_flags_from_exec_config=true"],
    );
    assert_eq!(
        cfg.get_options()
            .get_starlark_options()
            .get(&label("//my_starlark_flag:starlark_flag")),
        None
    );
}

#[test]
fn test_exec_starlark_flag_is_propagated_if_allowed_by_flag() {
    let mut t = ConfigurationTestCase::new();
    t.scratch
        .file("my_starlark_flag/rule_defs.bzl", BOOL_FLAG_RULE_DEFS);
    t.scratch.file(
        "my_starlark_flag/BUILD",
        &[
            "load(\"//my_starlark_flag:rule_defs.bzl\", \"bool_flag\")",
            "",
            "bool_flag(",
            "    name = \"starlark_flag\",",
            "    build_setting_default = False,",
            ")",
            "",
            "bool_flag(",
            "    name = \"other_starlark_flag\",",
            "    build_setting_default = False,",
            ")",
        ],
    );
    let cfg = t.create_exec_with_starlark(
        &[
            ("//my_starlark_flag:starlark_flag", "true"),
            ("//my_starlark_flag:other_starlark_flag", "true"),
        ],
        &[
            "--experimental_exclude_starlark_flags_from_exec_config=true",
            // Verify that labels are parsed rather than compared as strings by specifying the
            // label in non-canonical form.
            "--experimental_propagate_custom_flag=@//my_starlark_flag:starlark_flag",
        ],
    );
    assert_eq!(
        cfg.get_options()
            .get_starlark_options()
            .get(&label("//my_starlark_flag:starlark_flag")),
        Some(&str_value("true"))
    );
    assert_eq!(
        cfg.get_options()
            .get_starlark_options()
            .get(&label("//my_starlark_flag:other_starlark_flag")),
        None
    );
}

#[test]
fn test_exec_starlark_flag_is_propagated_by_target_pattern() {
    let mut t = ConfigurationTestCase::new();
    t.scratch.file("my_starlark_flag/BUILD", &[]);
    t.scratch.file(
        "my_starlark_flag/rule_defs.bzl",
        &[
            "bool_flag = rule(",
            "    implementation = lambda ctx: [],",
            "    build_setting = config.bool(flag = True),",
            ")",
        ],
    );
    t.scratch.file(
        "flags_to_propagate/BUILD",
        &[
            "load(\"//my_starlark_flag:rule_defs.bzl\", \"bool_flag\")",
            "bool_flag(",
            "    name = \"include_me\",",
            "    build_setting_default = False,",
            ")",
        ],
    );
    t.scratch.file(
        "flags_to_reset/BUILD",
        &[
            "load(\"//my_starlark_flag:rule_defs.bzl\", \"bool_flag\")",
            "bool_flag(",
            "    name = \"exclude_me\",",
            "    build_setting_default = False,",
            ")",
        ],
    );

    let cfg = t.create_exec_with_starlark(
        &[
            ("//flags_to_propagate:include_me", "true"),
            ("//flags_to_reset:exclude_me", "true"),
        ],
        &[
            "--experimental_exclude_starlark_flags_from_exec_config=true",
            "--experimental_propagate_custom_flag=//flags_to_propagate/...",
        ],
    );

    assert_eq!(
        cfg.get_options()
            .get_starlark_options()
            .get(&label("//flags_to_propagate:include_me")),
        Some(&str_value("true"))
    );
    assert_eq!(
        cfg.get_options()
            .get_starlark_options()
            .get(&label("//flags_to_reset:exclude_me")),
        None
    );
}

const STRING_FLAG_RULE_DEFS: &[&str] = &[
    "string_flag = rule(",
    "    implementation = lambda ctx: [],",
    "    build_setting = config.string(flag = True),",
    "    attrs = {\"scope\": attr.string(values = [\"target\", \"universal\", \"project\"])},",
    ")",
];

fn starlark_flag_exec_scopes(propagate_by_default: bool) {
    let mut t = ConfigurationTestCase::new();
    t.scratch.file("my_starlark_flag/BUILD", &[]);
    t.scratch
        .file("my_starlark_flag/rule_defs.bzl", STRING_FLAG_RULE_DEFS);
    t.scratch.file(
        "test/BUILD",
        &[
            "load(\"//my_starlark_flag:rule_defs.bzl\", \"string_flag\")",
            "string_flag(",
            "    name = \"default_scope\",",
            "    build_setting_default = \"default\",",
            ")",
            "string_flag(",
            "    name = \"target_scope\",",
            "    build_setting_default = \"default\",",
            "    scope = \"target\",",
            ")",
            "string_flag(",
            "    name = \"universal_scope\",",
            "    build_setting_default = \"default\",",
            "    scope = \"universal\",",
            ")",
            "string_flag(",
            "    name = \"project_scope\",",
            "    build_setting_default = \"default\",",
            "    scope = \"project\",",
            ")",
        ],
    );

    let exclude = format!(
        "--experimental_exclude_starlark_flags_from_exec_config={}",
        if propagate_by_default {
            "false"
        } else {
            "true"
        }
    );
    let target_options = Arc::new(t.parse_build_options_with_starlark(
        &[
            ("//test:default_scope", "custom"),
            ("//test:target_scope", "custom"),
            ("//test:universal_scope", "custom"),
            ("//test:project_scope", "custom"),
        ],
        &[&exclude],
    ));

    let exec_options = t.exec_options(&target_options);

    if propagate_by_default {
        assert_starlark_options_exactly(
            exec_options.get_starlark_options(),
            &[
                // Universal survives all transitions, of course.
                ("//test:universal_scope", "custom"),
                // This is just asserting that project scope survives the exec transition. If we
                // had changed project scope, this flag wouldn't be present.
                ("//test:project_scope", "custom"),
                ("//test:default_scope", "custom"),
            ],
        );
    } else {
        assert_starlark_options_exactly(
            exec_options.get_starlark_options(),
            &[
                // Universal survives all transitions, of course.
                ("//test:universal_scope", "custom"),
                // This is just asserting that project scope survives the exec transition. If we
                // had changed project scope, this flag wouldn't be present.
                ("//test:project_scope", "custom"),
            ],
        );
    }
}

// Java: starlarkFlagExecScopes(@TestParameter boolean propagateByDefault)
#[test]
fn starlark_flag_exec_scopes_propagate_by_default_false() {
    starlark_flag_exec_scopes(false);
}

#[test]
fn starlark_flag_exec_scopes_propagate_by_default_true() {
    starlark_flag_exec_scopes(true);
}

#[test]
fn starlark_flag_exec_scopes_take_precedence_over_custom_overrides() {
    let mut t = ConfigurationTestCase::new();
    t.scratch.file("my_starlark_flag/BUILD", &[]);
    t.scratch
        .file("my_starlark_flag/rule_defs.bzl", STRING_FLAG_RULE_DEFS);
    t.scratch.file(
        "test/BUILD",
        &[
            "load(\"//my_starlark_flag:rule_defs.bzl\", \"string_flag\")",
            "string_flag(",
            "    name = \"default_scope\",",
            "    build_setting_default = \"default\",",
            ")",
            "string_flag(",
            "    name = \"target_scope\",",
            "    build_setting_default = \"default\",",
            "    scope = \"target\",",
            ")",
        ],
    );

    let exec_config = t.create_exec_with_starlark(
        &[
            ("//test:default_scope", "custom"),
            ("//test:target_scope", "custom"),
        ],
        &[
            "--experimental_exclude_starlark_flags_from_exec_config=true",
            "--experimental_propagate_custom_flag=//test:target_scope",
            "--experimental_propagate_custom_flag=//test:default_scope",
        ],
    );

    assert_starlark_options_exactly(
        exec_config.get_options().get_starlark_options(),
        &[("//test:default_scope", "custom")],
    );
}

fn label_flag_exec_scopes(propagate_by_default: bool) {
    let mut t = ConfigurationTestCase::new();
    t.scratch.file(
        "test/BUILD",
        &[
            "label_flag(",
            "    name = \"default_scope\",",
            "    build_setting_default = \"//foo\",",
            ")",
            "label_flag(",
            "    name = \"target_scope\",",
            "    build_setting_default = \"//foo\",",
            "    scope = \"target\",",
            ")",
            "label_flag(",
            "    name = \"universal_scope\",",
            "    build_setting_default = \"//foo\",",
            "    scope = \"universal\",",
            ")",
        ],
    );

    let exclude = format!(
        "--experimental_exclude_starlark_flags_from_exec_config={}",
        if propagate_by_default {
            "false"
        } else {
            "true"
        }
    );
    let exec_config = t.create_exec_with_starlark(
        &[
            ("//test:default_scope", "//custom"),
            ("//test:target_scope", "//custom"),
            ("//test:universal_scope", "//custom"),
        ],
        &[&exclude],
    );

    if propagate_by_default {
        assert_starlark_options_exactly(
            exec_config.get_options().get_starlark_options(),
            &[
                ("//test:universal_scope", "//custom"),
                ("//test:default_scope", "//custom"),
            ],
        );
    } else {
        assert_starlark_options_exactly(
            exec_config.get_options().get_starlark_options(),
            &[("//test:universal_scope", "//custom")],
        );
    }
}

// Java: labelFlagExecScopes(@TestParameter boolean propagateByDefault)
#[test]
fn label_flag_exec_scopes_propagate_by_default_false() {
    label_flag_exec_scopes(false);
}

#[test]
fn label_flag_exec_scopes_propagate_by_default_true() {
    label_flag_exec_scopes(true);
}

#[test]
fn test_host_compilation_mode_default() {
    let mut t = ConfigurationTestCase::new();
    let cfg = t.create_exec(&[]);
    assert_eq!(cfg.get_compilation_mode(), CompilationMode::OPT);
}

#[test]
fn test_host_compilation_mode_non_default() {
    let mut t = ConfigurationTestCase::new();
    let cfg = t.create_exec(&["--host_compilation_mode=dbg"]);
    assert_eq!(cfg.get_compilation_mode(), CompilationMode::DBG);
}

#[test]
fn test_incompatible_merge_genfiles_directory() {
    let mut t = ConfigurationTestCase::new();
    let target = t.create(&["--incompatible_merge_genfiles_directory"]);
    let exec = t.create_exec(&["--incompatible_merge_genfiles_directory"]);
    assert_eq!(
        target.get_genfiles_directory(&MAIN),
        target.get_bin_directory(&MAIN)
    );
    assert_eq!(
        exec.get_genfiles_directory(&MAIN),
        exec.get_bin_directory(&MAIN)
    );
}

#[test]
#[ignore = "needs serialization (SerializationTester and the BuildConfigurationValue codec): serialization is skipped"]
fn test_codec() {}

#[test]
#[ignore = "needs serialization (SerializationTester and the BuildConfigurationKey codec): serialization is skipped"]
fn test_key_codec() {}

#[test]
fn test_platform_in_output_dir_legacy_default_platform() {
    let mut t = ConfigurationTestCase::new();
    let config = t.create(&[
        "--experimental_platform_in_output_dir",
        "--experimental_use_platforms_in_output_dir_legacy_heuristic",
        "--cpu=k8",
    ]);
    assert_matches(&output_root(&config), ".*/[^/]+-out/k8-fastbuild");
}

#[test]
fn test_platform_in_output_dir_legacy_with_platform() {
    let mut t = ConfigurationTestCase::new();
    t.scratch
        .file("platform/BUILD", &["platform(name = 'alpha')"]);
    let config = t.create(&[
        "--experimental_platform_in_output_dir",
        "--experimental_use_platforms_in_output_dir_legacy_heuristic",
        "--platforms=//platform:alpha",
    ]);
    assert_matches(&output_root(&config), ".*/[^/]+-out/alpha-fastbuild");
}

#[test]
fn test_platform_in_output_dir_default_platform() {
    let mut t = ConfigurationTestCase::new();
    let config = t.create(&[
        "--experimental_platform_in_output_dir",
        "--noexperimental_use_platforms_in_output_dir_legacy_heuristic",
        "--cpu=k8",
    ]);
    // See tests of these flags with platform_mappings for more realistic results.
    assert_matches(
        &output_root(&config),
        r".*/[^/]+-out/platform-\w*-fastbuild",
    );
}

#[test]
fn test_platform_in_output_dir_with_platform() {
    let mut t = ConfigurationTestCase::new();
    t.scratch
        .file("platform/BUILD", &["platform(name = 'alpha')"]);
    let config = t.create(&[
        "--experimental_platform_in_output_dir",
        "--noexperimental_use_platforms_in_output_dir_legacy_heuristic",
        "--platforms=//platform:alpha",
    ]);
    assert_matches(
        &output_root(&config),
        r".*/[^/]+-out/platform-\w*-fastbuild",
    );
}

#[test]
fn test_platform_in_output_dir_with_platform_and_matching_override() {
    let mut t = ConfigurationTestCase::new();
    t.scratch
        .file("platform/BUILD", &["platform(name = 'alpha')"]);
    let config = t.create(&[
        "--experimental_platform_in_output_dir",
        "--noexperimental_use_platforms_in_output_dir_legacy_heuristic",
        "--experimental_override_name_platform_in_output_dir=//platform:alpha=alpha",
        "--platforms=//platform:alpha",
    ]);
    assert_matches(&output_root(&config), ".*/[^/]+-out/alpha-fastbuild");
}

#[test]
fn test_platform_in_output_dir_with_platform_and_non_matching_override() {
    let mut t = ConfigurationTestCase::new();
    t.scratch
        .file("platform/BUILD", &["platform(name = 'alpha')"]);
    let config = t.create(&[
        "--experimental_platform_in_output_dir",
        "--noexperimental_use_platforms_in_output_dir_legacy_heuristic",
        "--experimental_override_name_platform_in_output_dir=//platform:beta=beta",
        "--platforms=//platform:alpha",
    ]);
    assert_matches(
        &output_root(&config),
        r".*/[^/]+-out/platform-\w*-fastbuild",
    );
}

/// Guava `EqualsTester`: every value equals itself and the members of its group, and no member
/// of another group.
fn equals_tester(groups: &[Vec<BuildConfigurationValue>]) {
    for (i, group) in groups.iter().enumerate() {
        for (j, other_group) in groups.iter().enumerate() {
            for a in group {
                for b in other_group {
                    if i == j {
                        assert!(a == b, "{a:?} should equal {b:?} (group {i})");
                    } else {
                        assert!(
                            a != b,
                            "{a:?} (group {i}) should not equal {b:?} (group {j})"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn test_configuration_equality() {
    let mut t = ConfigurationTestCase::new();
    // Note that, in practice, test_arg should not be used as a no-op argument; however,
    // these configurations are never trimmed nor even used to build targets so not an issue.
    let mut raw = |arg: &str, mnemonic: &str, sibling: bool| {
        let options = t.parse_build_options(&[arg]);
        t.create_raw(options, mnemonic, sibling)
    };
    let groups = vec![
        vec![
            raw("--test_arg=1a", "k8", false),
            raw("--test_arg=1a", "k8", false),
        ],
        // Different BuildOptions means non-equal
        vec![raw("--test_arg=1b", "k8", false)],
        // Different --experimental_sibling_repository_layout means non-equal
        vec![raw("--test_arg=2", "k8", true)],
        vec![raw("--test_arg=2", "k8", false)],
        // Different transitionDirectoryNameFragment means non-equal
        vec![raw("--test_arg=3", "k8", false)],
        vec![raw("--test_arg=3", "arm", false)],
        vec![raw("--test_arg=3", "risc", false)],
    ];
    equals_tester(&groups);
}
