//! Port of
//! `src/test/java/com/google/devtools/build/lib/analysis/select/ConfiguredAttributeMapperTest.java`:
//! `ConfiguredAttributeMapper` on configured targets.

use std::sync::Arc;

use blaze_analysis::config::config_matching_provider::ConfigMatchingProvider;
use blaze_analysis::config::configured_attribute_mapper::ConfiguredAttributeMapper;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_and_data::ConfiguredTargetAndData;
use blaze_cmdline::Label;
use blaze_optdefs::fragment::compilation_mode::CompilationMode;
use blaze_packages::attribute::attribute_map::AttributeMap;
use blaze_packages::target::Target;
use blaze_packages::types::DEFAULT_CONDITION_KEY;
use blaze_packages::types::attr_value::AttrValue;
use blaze_packages::types::type_::Type;
use indexmap::IndexMap;

use crate::testing::BuildViewTestCase;

fn label(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

/// `ConfiguredTarget.getConfigConditions()` (empty unless the target is a rule or an alias).
fn get_config_conditions(ct: &ConfiguredTarget) -> Arc<IndexMap<Label, ConfigMatchingProvider>> {
    match ct {
        ConfiguredTarget::Rule(t) => t.get_config_conditions().clone(),
        ConfiguredTarget::Alias(t) => t.get_config_conditions().clone(),
        _ => Arc::new(IndexMap::new()),
    }
}

/// `ConfiguredTargetAndData.getAttributeMapperForTesting()`
/// (`BuildViewTestCase.getMapperFromConfiguredTargetAndTarget`).
// Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:335 ConfiguredTargetAndData.getAttributeMapperForTesting
fn get_mapper_from_configured_target_and_target(
    ctad: &ConfiguredTargetAndData,
) -> ConfiguredAttributeMapper {
    let Target::Rule(rule) = ctad.get_target() else {
        panic!(
            "ClassCastException: {:?} is not a Rule",
            ctad.get_target_label()
        );
    };
    ConfiguredAttributeMapper::of(
        rule.clone(),
        get_config_conditions(ctad.get_configured_target()),
        ctad.get_configuration().expect("configuration"),
    )
}

/// Returns a ConfiguredAttributeMapper bound to the given rule with the target configuration.
// Java: ConfiguredAttributeMapperTest.getMapper
fn get_mapper(t: &mut BuildViewTestCase, label: &str) -> ConfiguredAttributeMapper {
    let ctad = t
        .get_configured_target_and_data(label)
        .unwrap_or_else(|| panic!("no configured target {label}"));
    get_mapper_from_configured_target_and_target(&ctad)
}

// Java: ConfiguredAttributeMapperTest.writeConfigRules
fn write_config_rules(t: &BuildViewTestCase) {
    t.scratch.file(
        "conditions/BUILD",
        &[r#"config_setting(
    name = "a",
    values = {"define": "mode=a"},
)

config_setting(
    name = "b",
    values = {"define": "mode=b"},
)
"#],
    );
}

/// `mapper.get(name, Type.STRING)`.
fn get_string(mapper: &ConfiguredAttributeMapper, name: &str) -> Option<String> {
    match mapper.get(name, Type::String) {
        Some(AttrValue::String(s)) => Some(s),
        None => None,
        Some(other) => panic!("ClassCastException: {other:?} is not a String"),
    }
}

/// `mapper.get(name, BuildType.LABEL_LIST)`.
fn get_label_list(mapper: &ConfiguredAttributeMapper, name: &str) -> Vec<Label> {
    match mapper.get(name, Type::LabelList) {
        Some(AttrValue::LabelList(l)) => l,
        other => panic!("ClassCastException: {other:?} is not a label list"),
    }
}

/// `assertThat(labels).containsExactly(expected...)`.
fn assert_contains_exactly_labels(actual: &[Label], expected: &[Label]) {
    let mut a: Vec<String> = actual.iter().map(|l| l.to_string()).collect();
    let mut e: Vec<String> = expected.iter().map(|l| l.to_string()).collect();
    a.sort();
    e.sort();
    assert_eq!(a, e, "containsExactly");
}

/// Tests that [`ConfiguredAttributeMapper::get`] only gets the configuration-appropriate value.
// Java: ConfiguredAttributeMapperTest.testGetAttribute
#[test]
fn test_get_attribute() {
    let mut t = BuildViewTestCase::new();
    write_config_rules(&t);
    t.scratch.file(
        "a/BUILD",
        &[
            "genrule(",
            "    name = 'gen',",
            "    srcs = [],",
            "    outs = ['out'],",
            "    cmd = select({",
            "        '//conditions:a': 'a command',",
            "        '//conditions:b': 'b command',",
            &format!("        '{DEFAULT_CONDITION_KEY}': 'default command',"),
            "    }))",
        ],
    );

    t.use_configuration(&["--define", "mode=a"]);
    assert_eq!(
        get_string(&get_mapper(&mut t, "//a:gen"), "cmd").as_deref(),
        Some("a command")
    );

    t.use_configuration(&["--define", "mode=b"]);
    assert_eq!(
        get_string(&get_mapper(&mut t, "//a:gen"), "cmd").as_deref(),
        Some("b command")
    );

    t.use_configuration(&["--define", "mode=c"]);
    assert_eq!(
        get_string(&get_mapper(&mut t, "//a:gen"), "cmd").as_deref(),
        Some("default command")
    );
}

/// Tests that label visitation only travels down configuration-appropriate paths.
// Java: ConfiguredAttributeMapperTest.testLabelVisitation
#[test]
fn test_label_visitation() {
    let mut t = BuildViewTestCase::new();
    write_config_rules(&t);
    t.scratch.file(
        "a/BUILD",
        &[
            "load('//test_defs:foo_binary.bzl', 'foo_binary')",
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_binary(",
            "    name = 'bin',",
            "    srcs = ['bin.sh'],",
            "    deps = select({",
            "        '//conditions:a': [':adep'],",
            "        '//conditions:b': [':bdep'],",
            &format!("        '{DEFAULT_CONDITION_KEY}': [':defaultdep'],"),
            "    }))",
            "foo_library(",
            "    name = 'adep',",
            "    srcs = ['adep.sh'])",
            "foo_library(",
            "    name = 'bdep',",
            "    srcs = ['bdep.sh'])",
            "foo_library(",
            "    name = 'defaultdep',",
            "    srcs = ['defaultdep.sh'])",
        ],
    );

    let mut visited_labels: Vec<Label> = Vec::new();
    let bin_src = label("//a:bin.sh");

    t.use_configuration(&["--define", "mode=a"]);
    add_relevant_labels(&get_mapper(&mut t, "//a:bin"), &mut visited_labels);
    assert_contains_exactly_labels(&visited_labels, &[bin_src.clone(), label("//a:adep")]);

    visited_labels.clear();
    t.use_configuration(&["--define", "mode=b"]);
    add_relevant_labels(&get_mapper(&mut t, "//a:bin"), &mut visited_labels);
    assert_contains_exactly_labels(&visited_labels, &[bin_src.clone(), label("//a:bdep")]);

    visited_labels.clear();
    t.use_configuration(&["--define", "mode=c"]);
    add_relevant_labels(&get_mapper(&mut t, "//a:bin"), &mut visited_labels);
    assert_contains_exactly_labels(&visited_labels, &[bin_src, label("//a:defaultdep")]);
}

// Java: ConfiguredAttributeMapperTest.addRelevantLabels
fn add_relevant_labels(mapper: &ConfiguredAttributeMapper, visited_labels: &mut Vec<Label>) {
    mapper.visit_all_labels(&mut |_attribute, label| {
        if label
            .get_package_identifier()
            .get_package_fragment()
            .get_path_string()
            == "a"
        {
            visited_labels.push(label.clone());
        }
    });
}

/// Tests that for configurable attributes where the *values* are evaluated in different
/// configurations, the configuration checking still uses the original configuration.
// Java: ConfiguredAttributeMapperTest.testConfigurationTransitions
#[test]
fn test_configuration_transitions() {
    let mut t = BuildViewTestCase::new();
    write_config_rules(&t);
    t.scratch.file(
        "a/BUILD",
        &[
            "load('//test_defs:foo_binary.bzl', 'foo_binary')",
            "genrule(",
            "    name = 'gen',",
            "    srcs = [],",
            "    outs = ['out'],",
            "    cmd = 'nothing',",
            "    tools = select({",
            "        '//conditions:a': [':adep'],",
            "        '//conditions:b': [':bdep'],",
            &format!("        '{DEFAULT_CONDITION_KEY}': [':defaultdep'],"),
            "    }))",
            "foo_binary(",
            "    name = 'adep',",
            "    srcs = ['adep.sh'])",
            "foo_binary(",
            "    name = 'bdep',",
            "    srcs = ['bdep.sh'])",
            "foo_binary(",
            "    name = 'defaultdep',",
            "    srcs = ['defaultdep.sh'])",
        ],
    );
    t.use_configuration(&["--define", "mode=b"]);

    // Target configuration is in dbg mode, so we should match //conditions:b:
    assert_contains_exactly_labels(
        &get_label_list(&get_mapper(&mut t, "//a:gen"), "tools"),
        &[label("//a:bdep")],
    );

    // Verify the "tools" dep uses a different configuration that's not also in "dbg":
    let target = t.get_target("//a:gen").unwrap_or_else(|e| panic!("{e:?}"));
    assert!(
        target
            .get_associated_rule()
            .expect("rule")
            .get_rule_class_object()
            .get_attribute_provider()
            .get_attribute_by_name("tools")
            .get_transition_factory()
            .is_tool()
    );
    assert_eq!(
        t.get_exec_configuration().get_compilation_mode(),
        CompilationMode::OPT
    );
}

// Java: ConfiguredAttributeMapperTest.testConcatenatedSelects
#[test]
fn test_concatenated_selects() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "hello/BUILD",
        &[r#"config_setting(
    name = "a",
    values = {"define": "foo=a"},
)

config_setting(
    name = "b",
    values = {"define": "foo=b"},
)

config_setting(
    name = "c",
    values = {"define": "bar=c"},
)

config_setting(
    name = "d",
    values = {"define": "bar=d"},
)

genrule(
    name = "gen",
    srcs = select({
               ":a": ["a.in"],
               ":b": ["b.in"],
           }) +
           select({
               ":c": ["c.in"],
               ":d": ["d.in"],
           }),
    outs = ["out"],
    cmd = "nothing",
)
"#],
    );
    t.use_configuration(&["--define", "foo=a", "--define", "bar=d"]);
    assert_contains_exactly_labels(
        &get_label_list(&get_mapper(&mut t, "//hello:gen"), "srcs"),
        &[label("//hello:a.in"), label("//hello:d.in")],
    );
}

// Java: ConfiguredAttributeMapperTest.testNoneValuesMeansAttributeIsNotExplicitlySet
#[test]
fn test_none_values_means_attribute_is_not_explicitly_set() {
    let mut t = BuildViewTestCase::new();
    write_config_rules(&t);
    t.scratch.file(
        "a/BUILD",
        &[r#"genrule(
    name = "gen",
    srcs = [],
    outs = ["out"],
    cmd = "",
    message = select({
        "//conditions:a": "defined message",
        "//conditions:b": None,
    }),
)
"#],
    );

    t.use_configuration(&["--define", "mode=a"]);
    assert!(get_mapper(&mut t, "//a:gen").is_attribute_value_explicitly_specified("message"));

    t.use_configuration(&["--define", "mode=b"]);
    assert!(!get_mapper(&mut t, "//a:gen").is_attribute_value_explicitly_specified("message"));
}

// Java: ConfiguredAttributeMapperTest.testNoneValuesWithMultipleSelectsAllNone
#[test]
fn test_none_values_with_multiple_selects_all_none() {
    let mut t = BuildViewTestCase::new();
    write_config_rules(&t);
    t.scratch.file(
        "a/BUILD",
        &[r#"genrule(
    name = "gen",
    srcs = [],
    outs = ["out"],
    cmd = "",
    message = select({
        "//conditions:a": "defined message 1",
        "//conditions:b": None,
    }) + select({
        "//conditions:a": "defined message 2",
        "//conditions:b": None,
    }),
)
"#],
    );

    t.use_configuration(&["--define", "mode=a"]);
    assert!(get_mapper(&mut t, "//a:gen").is_attribute_value_explicitly_specified("message"));

    t.use_configuration(&["--define", "mode=b"]);
    assert!(!get_mapper(&mut t, "//a:gen").is_attribute_value_explicitly_specified("message"));
}

// Java: ConfiguredAttributeMapperTest.testNoneValueOnDefaultConditionWithNullDefault
#[test]
fn test_none_value_on_default_condition_with_null_default() {
    let mut t = BuildViewTestCase::new();
    write_config_rules(&t);
    t.scratch.file(
        "a/BUILD",
        &[
            "load('@rules_cc//cc:cc_library.bzl', 'cc_library')",
            "cc_library(",
            "    name = 'lib',",
            "    srcs = ['lib.cc'],",
            "    linkstamp = select({",
            "        '//conditions:a': 'notused_linkstamp.cc',",
            &format!("        '{DEFAULT_CONDITION_KEY}': None,"),
            "    }),",
            ")",
        ],
    );

    t.use_configuration(&[]);
    assert!(!get_mapper(&mut t, "//a:lib").is_attribute_value_explicitly_specified("linkstamp"));
    assert!(
        get_mapper(&mut t, "//a:lib")
            .get("linkstamp", Type::Label)
            .is_none()
    );
}

// Java: ConfiguredAttributeMapperTest.testNoneValueOnMandatoryAttribute
#[test]
fn test_none_value_on_mandatory_attribute() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[r#"# Needed to avoid select() being eliminated as trivial.
config_setting(
    name = "config",
    values = {"define": "pi=3"},
)

alias(
    name = "a",
    actual = select({":config": None, "//conditions:default": None}),
)
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//a:a");
    t.assert_contains_event("Mandatory attribute 'actual' resolved to 'None'");
}

// Java: ConfiguredAttributeMapperTest.testAliasedConfigSetting
#[test]
fn test_aliased_config_setting() {
    let mut t = BuildViewTestCase::new();
    write_config_rules(&t);
    t.scratch.file(
        "a/BUILD",
        &[r#"alias(
    name = "aliased_a",
    actual = "//conditions:a",
)

genrule(
    name = "gen",
    srcs = [],
    outs = ["out"],
    cmd = "",
    message = select({
        ":aliased_a": "defined message",
        "//conditions:default": None,
    }),
)
"#],
    );
    t.use_configuration(&["--define", "mode=a"]);
    assert_eq!(
        get_string(&get_mapper(&mut t, "//a:gen"), "message").as_deref(),
        Some("defined message")
    );
}

// Java: ConfiguredAttributeMapperTest.noMatchErrorFormat
#[test]
fn no_match_error_format() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[r#"config_setting(
    name = "a",
    values = {"define": "mode=a"},
)

config_setting(
    name = "b",
    values = {"define": "mode=b"},
)

genrule(
    name = "g",
    srcs = [],
    outs = ["out"],
    cmd = "",
    message = select({
        ":a": "not chosen",
        ":b": "not chosen",
    }),
)
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//a:g");
    // Match with a regex pattern because the error message includes the failing target's
    // configuration ID, which can vary among builds.
    //
    // PORT: the Java pattern's `.*` wildcards (which do not match line terminators) separate
    // these literal parts; `Pattern.matcher(...).find()`.
    let parts: &[&str] = &[
        "",
        "configurable attribute \"message\" in //a:g doesn't match this configuration. Would \
         a default condition help?\n\
         \n\
         Conditions checked:\n \
         //a:a\n \
         //a:b\n\
         \n\
         To see a condition's definition, run: bazel query --output=build <condition \
         label>.\n\
         \n\
         This instance of //a:g has configuration identifier ",
        ". To inspect its configuration, run: bazel config ",
        ".\n\
         \n\
         For more help, see \
         https://bazel.build/docs/configurable-attributes\
         #faq-select-choose-condition",
        "",
    ];
    t.assert_contains_event_matching(&parts.join(".*"), |event| find_with_wildcards(event, parts));
}

/// Java `Pattern.compile(parts.join(".*")).matcher(s).find()` where every part is a literal and
/// `.*` matches any run of characters other than `\n`.
fn find_with_wildcards(s: &str, parts: &[&str]) -> bool {
    fn matches_from(s: &str, parts: &[&str]) -> bool {
        // `s` must start with parts[0]; between parts any non-newline run.
        let Some(rest) = s.strip_prefix(parts[0]) else {
            return false;
        };
        if parts.len() == 1 {
            return true;
        }
        let mut i = 0;
        loop {
            if matches_from(&rest[i..], &parts[1..]) {
                return true;
            }
            match rest[i..].chars().next() {
                Some('\n') | None => return false,
                Some(c) => i += c.len_utf8(),
            }
        }
    }
    s.char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(s.len()))
        .any(|i| matches_from(&s[i..], parts))
}
