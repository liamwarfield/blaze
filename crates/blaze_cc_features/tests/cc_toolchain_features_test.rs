//! Port of `src/test/java/com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesTest.java`.
//!
//! The Java test builds features by analyzing a `cc_toolchain_config` rule written with rules_cc's
//! `cc_toolchain_config_lib.bzl`; here the same Starlark is evaluated with starlark-rust (see
//! tests/common) and handed to `CcToolchainFeatures::new`.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use blaze_cc_features::actions::{NOOP, PathMapper};
use blaze_cc_features::cc_toolchain_features::ActionConfig;
use blaze_cc_features::cc_toolchain_variables::{
    Sequence, StringValue, VariableObject, VariableValue,
};
use blaze_cc_features::errors::ExpansionException;
use blaze_cc_features::{
    ArtifactCategory, CcToolchainFeatures, CcToolchainVariables, CcToolchainVariablesRef,
    CppActionNames, FeatureConfiguration,
};
use blaze_vfs::PathFragment;
use common::*;
use indexmap::IndexSet;

fn set(names: &[&str]) -> IndexSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

fn get_feature_configuration(
    features: &CcToolchainFeatures,
    requested: &[&str],
) -> Arc<FeatureConfiguration> {
    features.get_feature_configuration(&set(requested)).unwrap()
}

/// Java: `getEnabledFeatures(features, requestedFeatures...)`.
fn get_enabled_features(features: &CcToolchainFeatures, requested: &[&str]) -> BTreeSet<String> {
    let configuration = get_feature_configuration(features, requested);
    let mut enabled_features = BTreeSet::new();
    for feature in features.get_activatable_names() {
        if configuration.is_enabled(&feature) {
            enabled_features.insert(feature);
        }
    }
    enabled_features
}

fn names(v: &[&str]) -> BTreeSet<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Truth's `containsExactly` (any order).
fn assert_contains_exactly(actual: &[String], expected: &[&str]) {
    let mut a: Vec<&str> = actual.iter().map(|s| s.as_str()).collect();
    let mut e: Vec<&str> = expected.to_vec();
    a.sort_unstable();
    e.sort_unstable();
    assert_eq!(a, e, "actual (unsorted): {actual:?}");
}

fn assert_in_order(actual: &[String], expected: &[&str]) {
    let a: Vec<&str> = actual.iter().map(|s| s.as_str()).collect();
    assert_eq!(a, expected);
}

fn bf(content: &[&str]) -> CcToolchainFeatures {
    build_features(content).unwrap_or_else(|e| panic!("buildFeatures failed: {e}"))
}

// Java: getCommandLineForFlagGroups(groups, variables, pathMapper). The outer Err is a build
// (analysis) failure, the inner one an ExpansionException.
fn command_line_for_flag_groups_with(
    groups: &str,
    variables: &CcToolchainVariablesRef,
    path_mapper: &dyn PathMapper,
) -> Result<Result<Vec<String>, ExpansionException>, String> {
    let lines = [
        "features = [",
        "   feature(name = 'no_legacy_features'),",
        "   feature(",
        "    name = 'a',",
        "    flag_sets = [",
        "        flag_set(",
        "            actions = ['c++-compile'],",
        &format!("            flag_groups = [{groups}],"),
        "        ),",
        "    ],",
        ")]",
    ];
    let features = build_features(&lines)?;
    let configuration = get_feature_configuration(&features, &["a"]);
    Ok(configuration.get_command_line_with(
        CppActionNames::CPP_COMPILE,
        variables,
        None,
        path_mapper,
    ))
}

fn get_command_line_for_flag_groups(
    groups: &str,
    variables: &CcToolchainVariablesRef,
) -> Vec<String> {
    command_line_for_flag_groups_with(groups, variables, &NOOP)
        .unwrap()
        .unwrap()
}

fn get_expansion_of_flag_with(
    value: &str,
    variables: &CcToolchainVariablesRef,
    path_mapper: &dyn PathMapper,
) -> Result<Result<String, ExpansionException>, String> {
    Ok(command_line_for_flag_groups_with(
        &format!("flag_group(flags = ['{value}'])"),
        variables,
        path_mapper,
    )?
    .map(|cl| cl[0].clone()))
}

fn get_expansion_of_flag(value: &str, variables: &CcToolchainVariablesRef) -> String {
    get_expansion_of_flag_with(value, variables, &NOOP)
        .unwrap()
        .unwrap()
}

fn get_flag_parsing_error(value: &str) -> String {
    get_expansion_of_flag_with(value, &create_variables(&[]), &NOOP).unwrap_err()
}

fn get_flag_expansion_error(value: &str, variables: &CcToolchainVariablesRef) -> String {
    get_expansion_of_flag_with(value, variables, &NOOP)
        .unwrap()
        .unwrap_err()
        .message
}

fn get_flag_groups_expansion_error(
    flag_groups: &str,
    variables: &CcToolchainVariablesRef,
) -> String {
    command_line_for_flag_groups_with(flag_groups, variables, &NOOP)
        .unwrap()
        .unwrap_err()
        .message
}

#[test]
#[ignore = "needs skyframe serialization (SerializationTester)"]
fn test_feature_configuration_codec() {}

#[test]
fn test_unconditional_feature() {
    assert!(!get_feature_configuration(&bf(&["features = []"]), &["a"]).is_enabled("a"));
    assert!(
        !get_feature_configuration(&bf(&["features = [feature(name = 'a')]"]), &["b"])
            .is_enabled("a")
    );
    assert!(
        get_feature_configuration(&bf(&["features = [feature(name = 'a')]"]), &["a"])
            .is_enabled("a")
    );
}

#[test]
fn test_unsupported_action() {
    let configuration = get_feature_configuration(&bf(&["features = []"]), &[]);
    assert!(
        configuration
            .get_command_line("invalid-action", &create_variables(&[]))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_flag_order_equals_spec_order() {
    let configuration = get_feature_configuration(
        &bf(&[
            "features = [",
            "    feature(",
            "        name = 'a',",
            "        flag_sets = [",
            "            flag_set(",
            "                actions = ['c++-compile'],",
            "                flag_groups = [flag_group(flags = ['-a-c++-compile'])],",
            "            ),",
            "            flag_set(",
            "                actions = ['link'],",
            "                flag_groups = [flag_group(flags = ['-a-c++-compile'])],",
            "            ),",
            "        ],",
            "    ),",
            "    feature(",
            "        name = 'b',",
            "        flag_sets = [",
            "            flag_set(",
            "                actions = ['c++-compile'],",
            "                flag_groups = [flag_group(flags = ['-b-c++-compile'])],",
            "            ),",
            "            flag_set(",
            "                actions = ['link'],",
            "                flag_groups = [flag_group(flags = ['-b-link'])],",
            "            ),",
            "        ],",
            "    ),",
            "]",
        ]),
        &["a", "b"],
    );
    let command_line = configuration
        .get_command_line(CppActionNames::CPP_COMPILE, &create_variables(&[]))
        .unwrap();
    assert_in_order(&command_line, &["-a-c++-compile", "-b-c++-compile"]);
}

#[test]
fn test_env_vars() {
    let configuration = get_feature_configuration(
        &bf(&[
            "features = [",
            "    feature(",
            "        name = 'a',",
            "        env_sets = [",
            "            env_set(",
            "                actions = ['c++-compile'],",
            "                env_entries = [",
            "                    env_entry(key = 'foo', value = 'bar'),",
            "                    env_entry(key = 'cat', value = 'meow'),",
            "                ],",
            "            ),",
            "        ],",
            "        flag_sets = [",
            "            flag_set(",
            "                actions = ['c++-compile'],",
            "                flag_groups = [flag_group(flags = ['-a-c++-compile'])],",
            "            ),",
            "        ],",
            "    ),",
            "    feature(",
            "        name = 'b',",
            "        env_sets = [",
            "            env_set(",
            "                actions = ['c++-compile'],",
            "                env_entries = [env_entry(key = 'dog', value = 'woof')],",
            "            ),",
            "            env_set(",
            "                actions = ['c++-compile'],",
            "                with_features = [with_feature_set(features = ['d'])],",
            "                env_entries = [env_entry(key = 'withFeature', value = 'value1')],",
            "            ),",
            "            env_set(",
            "                actions = ['c++-compile'],",
            "                with_features = [with_feature_set(features = ['e'])],",
            "                env_entries = [env_entry(key = 'withoutFeature', value = 'value2')],",
            "            ),",
            "            env_set(",
            "                actions = ['c++-compile'],",
            "                with_features = [with_feature_set(not_features = ['f'])],",
            "                env_entries = [env_entry(key = 'withNotFeature', value = 'value3')],",
            "            ),",
            "            env_set(",
            "                actions = ['c++-compile'],",
            "                with_features = [with_feature_set(not_features = ['g'])],",
            "                env_entries = [env_entry(key = 'withoutNotFeature', value = 'value4')],",
            "            ),",
            "        ],",
            "    ),",
            "    feature(",
            "        name = 'c',",
            "        env_sets = [",
            "            env_set(",
            "                actions = ['c++-compile'],",
            "                env_entries = [env_entry(key = 'doNotInclude', value = 'doNotIncludePlease')],",
            "            ),",
            "        ],",
            "    ),",
            "    feature(name = 'd'),",
            "    feature(name = 'e'),",
            "    feature(name = 'f'),",
            "    feature(name = 'g'),",
            "]",
        ]),
        &["a", "b", "d", "f"],
    );
    let env = configuration
        .get_environment_variables(CppActionNames::CPP_COMPILE, &create_variables(&[]), &NOOP)
        .unwrap();
    let entries: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(
        entries,
        vec![
            ("foo", "bar"),
            ("cat", "meow"),
            ("dog", "woof"),
            ("withFeature", "value1"),
            ("withoutNotFeature", "value4"),
        ]
    );
    assert!(!env.contains_key("withoutFeature"));
    assert!(!env.contains_key("withNotFeature"));
    assert!(!env.contains_key("doNotInclude"));
}

fn env_feature_with_expand_if_available(value: &str) -> CcToolchainFeatures {
    bf(&[
        "features = [",
        "    feature(",
        "        name = 'a',",
        "        env_sets = [",
        "            env_set(",
        "                actions = ['c++-compile'],",
        "                env_entries = [",
        &format!(
            "                    env_entry(key = 'foo', value = '{value}', expand_if_available = 'v')"
        ),
        "                ],",
        "            ),",
        "        ],",
        "    ),",
        "]",
    ])
}

#[test]
fn test_env_vars_with_missing_variable_is_not_expanded() {
    let configuration =
        get_feature_configuration(&env_feature_with_expand_if_available("bar"), &["a"]);
    let env = configuration
        .get_environment_variables(CppActionNames::CPP_COMPILE, &create_variables(&[]), &NOOP)
        .unwrap();
    assert!(env.get("foo").map(|v| v.as_str()) != Some("bar"));
}

#[test]
fn test_env_vars_with_all_variables_present_are_expanded() {
    let configuration =
        get_feature_configuration(&env_feature_with_expand_if_available("bar"), &["a"]);
    let env = configuration
        .get_environment_variables(
            CppActionNames::CPP_COMPILE,
            &create_variables(&["v", "1"]),
            &NOOP,
        )
        .unwrap();
    let entries: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(entries, vec![("foo", "bar")]);
}

#[test]
fn test_env_vars_with_all_variables_present_are_expanded_with_variable_expansion() {
    let configuration =
        get_feature_configuration(&env_feature_with_expand_if_available("%{v}"), &["a"]);
    let env = configuration
        .get_environment_variables(
            CppActionNames::CPP_COMPILE,
            &create_variables(&["v", "1"]),
            &NOOP,
        )
        .unwrap();
    let entries: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(entries, vec![("foo", "1")]);
}

#[test]
fn test_variable_expansion() {
    let empty = create_variables(&[]);
    assert_eq!(get_expansion_of_flag("%%", &empty), "%");
    assert_eq!(get_expansion_of_flag("%% a %% b %%", &empty), "% a % b %");
    assert_eq!(get_expansion_of_flag("%%{var}", &empty), "%{var}");
    assert_eq!(
        get_expansion_of_flag("%{v}", &create_variables(&["v", "<flag>"])),
        "<flag>"
    );
    assert_eq!(
        get_expansion_of_flag(" %{v1} %{v2} ", &create_variables(&["v1", "1", "v2", "2"])),
        " 1 2 "
    );
    assert!(
        get_flag_parsing_error("%")
            .contains("expected '{' at position 1 while parsing a flag containing '%'")
    );
    assert!(
        get_flag_parsing_error("% ")
            .contains("expected '{' at position 1 while parsing a flag containing '% '")
    );
    assert!(get_flag_parsing_error("%{").contains("expected variable name"));
    assert!(get_flag_parsing_error("%{}").contains("expected variable name"));
    let mut b = CcToolchainVariables::builder();
    b.add_string_sequence_variable("v", Vec::<String>::new());
    assert!(
        get_command_line_for_flag_groups(
            "flag_group(iterate_over = 'v', flags = ['%{v}'])",
            &b.build()
        )
        .is_empty()
    );
    assert!(
        get_flag_expansion_error("%{v}", &create_variables(&[]))
            .contains("Invalid toolchain configuration: Cannot find variable named 'v'")
    );
}

struct TestPathMapper;

impl PathMapper for TestPathMapper {
    fn map(&self, path: &PathFragment) -> PathFragment {
        if path.starts_with(&PathFragment::create("bazel-out")) {
            path.sub_fragment(0, 1)
                .get_relative_str("cfg")
                .get_relative(&path.sub_fragment_from(2))
        } else {
            path.clone()
        }
    }
}

#[test]
fn test_path_expansion() {
    let path_mapper = TestPathMapper;
    assert_eq!(
        get_expansion_of_flag_with(
            "%{path:my/source.c}",
            &CcToolchainVariables::empty(),
            &path_mapper
        )
        .unwrap()
        .unwrap(),
        "my/source.c"
    );
    assert_eq!(
        get_expansion_of_flag_with(
            "%{path:bazel-out/foobar/bin/my/artifact.a}",
            &CcToolchainVariables::empty(),
            &path_mapper
        )
        .unwrap()
        .unwrap(),
        "bazel-out/cfg/bin/my/artifact.a"
    );

    let e = get_flag_parsing_error("%{path:/absolute/path}");
    assert!(
        e.contains(
            "Invalid toolchain configuration: expected relative Unix-style path after 'path:' at \
             position 2 while parsing a flag containing '%{path:/absolute/path}"
        ),
        "{e}"
    );

    let e = get_flag_parsing_error("%{path:}");
    assert!(
        e.contains(
            "Invalid toolchain configuration: expected path after 'path:' at position 2 while \
             parsing a flag containing '%{path:}"
        ),
        "{e}"
    );
}

#[test]
fn test_simple_structure_variable_expansion() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(flags = ['-A%{struct.foo}', '-B%{struct.bar}'])",
            &create_structure_variables(
                "struct",
                StructureBuilder::new()
                    .add_field("foo", "fooValue")
                    .add_field("bar", "barValue"),
            ),
        ),
        &["-AfooValue", "-BbarValue"],
    );
}

#[test]
fn test_nested_structure_variable_expansion() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(flags = ['-A%{struct.foo.bar}'])",
            &create_structure_variables(
                "struct",
                StructureBuilder::new().add_field_builder(
                    "foo",
                    StructureBuilder::new().add_field("bar", "fooBarValue"),
                ),
            ),
        ),
        &["-AfooBarValue"],
    );
}

#[test]
fn test_accessing_structure_as_string_fails() {
    assert_eq!(
        get_flag_groups_expansion_error(
            "flag_group(flags = ['-A%{struct}'])",
            &create_structure_variables(
                "struct",
                StructureBuilder::new()
                    .add_field("foo", "fooValue")
                    .add_field("bar", "barValue"),
            ),
        ),
        "Invalid toolchain configuration: Cannot expand variable 'struct': expected string, \
         found structure"
    );
}

#[test]
fn test_accessing_string_value_as_structure_fails() {
    assert_eq!(
        get_flag_groups_expansion_error(
            "flag_group(flags = ['-A%{stringVar.foo}'])",
            &create_variables(&["stringVar", "stringVarValue"]),
        ),
        "Invalid toolchain configuration: Cannot expand variable 'stringVar.foo': variable \
         'stringVar' is string, expected structure"
    );
}

#[test]
fn test_accessing_sequence_as_structure_fails() {
    assert_eq!(
        get_flag_groups_expansion_error(
            "flag_group(flags = ['-A%{sequence.foo}'])",
            &create_variables(&["sequence", "foo1", "sequence", "foo2"]),
        ),
        "Invalid toolchain configuration: Cannot expand variable 'sequence.foo': variable \
         'sequence' is sequence, expected structure"
    );
}

#[test]
fn test_accessing_missing_structure_field_fails() {
    assert_eq!(
        get_flag_groups_expansion_error(
            "flag_group(flags = ['-A%{struct.missing}'])",
            &create_structure_variables(
                "struct",
                StructureBuilder::new().add_field("bar", "barValue")
            ),
        ),
        "Invalid toolchain configuration: Cannot expand variable 'struct.missing': structure \
         struct doesn't have a field named 'missing'"
    );
}

#[test]
fn test_sequence_of_structures_expansion() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(iterate_over = 'structs', flags = ['-A%{structs.foo}'])",
            &create_structure_sequence_variables(
                "structs",
                vec![
                    StructureBuilder::new()
                        .add_field("foo", "foo1Value")
                        .build(),
                    StructureBuilder::new()
                        .add_field("foo", "foo2Value")
                        .build(),
                ],
            ),
        ),
        &["-Afoo1Value", "-Afoo2Value"],
    );
}

#[test]
fn test_structure_of_sequences_expansion() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(  iterate_over = 'struct.sequences',  flags = ['-A%{struct.sequences.foo}'])",
            &create_structure_variables(
                "struct",
                StructureBuilder::new().add_field_value(
                    "sequences",
                    VariableValue::Sequence(Sequence::new(vec![
                        VariableObject::Value(
                            StructureBuilder::new()
                                .add_field("foo", "foo1Value")
                                .build(),
                        ),
                        VariableObject::Value(
                            StructureBuilder::new()
                                .add_field("foo", "foo2Value")
                                .build(),
                        ),
                    ])),
                ),
            ),
        ),
        &["-Afoo1Value", "-Afoo2Value"],
    );
}

#[test]
fn test_dotted_names_not_always_mean_structures() {
    let mut b = CcToolchainVariables::builder();
    b.add_variable(
        "struct",
        VariableObject::Value(
            StructureBuilder::new()
                .add_field_list("sequence", &["first", "second"])
                .build(),
        ),
    );
    b.add_string_sequence_variable("other_sequence", ["foo", "bar"]);
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(  iterate_over = 'struct.sequence',  flag_groups = [flag_group(    \
             iterate_over = 'other_sequence',    flag_groups = [flag_group(      flags = \
             ['-A%{struct.sequence} -B%{other_sequence}']    )]  )])",
            &b.build(),
        ),
        &[
            "-Afirst -Bfoo",
            "-Afirst -Bbar",
            "-Asecond -Bfoo",
            "-Asecond -Bbar",
        ],
    );
}

fn foo_bar_struct() -> CcToolchainVariablesRef {
    create_structure_variables(
        "struct",
        StructureBuilder::new()
            .add_field("foo", "fooValue")
            .add_field("bar", "barValue"),
    )
}

#[test]
fn test_expand_if_all_available_with_structs_expands_if_present() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(  expand_if_available = 'struct',  flags = ['-A%{struct.foo}', '-B%{struct.bar}'])",
            &foo_bar_struct(),
        ),
        &["-AfooValue", "-BbarValue"],
    );
}

#[test]
fn test_expand_if_all_available_with_structs_doesnt_expand_if_missing() {
    assert!(
        get_command_line_for_flag_groups(
            "flag_group(  expand_if_available = 'nonexistent',  flags = ['-A%{struct.foo}','-B%{struct.bar}'])",
            &foo_bar_struct(),
        )
        .is_empty()
    );
}

#[test]
fn test_expand_if_all_available_with_structs_doesnt_crash_if_missing() {
    assert!(
        get_command_line_for_flag_groups(
            "flag_group(  expand_if_available = 'nonexistent',  flags = ['-A%{nonexistent.foo}','-B%{nonexistent.bar}'])",
            &create_variables(&[]),
        )
        .is_empty()
    );
}

#[test]
fn test_expand_if_all_available_with_struct_field_doesnt_crash_if_missing() {
    assert!(
        get_command_line_for_flag_groups(
            "flag_group(  expand_if_available = 'nonexistent.nonexistant_field',  flags = ['-A%{nonexistent.foo}','-B%{nonexistent.bar}'])",
            &create_variables(&[]),
        )
        .is_empty()
    );
}

#[test]
fn test_expand_if_all_available_with_struct_field_expands_if_present() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(  expand_if_available = 'struct.foo',  flags = ['-A%{struct.foo}','-B%{struct.bar}'])",
            &foo_bar_struct(),
        ),
        &["-AfooValue", "-BbarValue"],
    );
}

#[test]
fn test_expand_if_all_available_with_struct_field_doesnt_expand_if_missing() {
    assert!(
        get_command_line_for_flag_groups(
            "flag_group(  expand_if_available = 'struct.foo',  flags = ['-A%{struct.foo}','-B%{struct.bar}'])",
            &create_structure_variables("struct", StructureBuilder::new().add_field("bar", "barValue")),
        )
        .is_empty()
    );
}

#[test]
fn test_expand_if_all_available_with_struct_field_scopes_right() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(flag_groups = [flag_group(    expand_if_available = 'struct.foo',    \
             flags = ['-A%{struct.foo}']  ),  flag_group(    flags = ['-B%{struct.bar}']  )])",
            &create_structure_variables(
                "struct",
                StructureBuilder::new().add_field("bar", "barValue"),
            ),
        ),
        &["-BbarValue"],
    );
}

#[test]
fn test_expand_if_none_available_expands_if_not_available() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(flag_groups = [flag_group(    expand_if_not_available = 'not_available',    \
             flags = ['-foo']  ),  flag_group(    expand_if_not_available = 'available',    \
             flags = ['-bar']  )])",
            &create_variables(&["available", "available"]),
        ),
        &["-foo"],
    );
}

#[test]
fn test_expand_if_none_available_doesnt_expand_if_there_is_one_of_many_available() {
    assert!(
        get_command_line_for_flag_groups(
            "flag_group(  expand_if_not_available = 'not_available',  flag_groups = [flag_group(    \
             expand_if_not_available = 'available',    flags = ['-foo']  )])",
            &create_variables(&["available", "available"]),
        )
        .is_empty()
    );
}

#[test]
fn test_expand_if_true_doesnt_expand_if_missing() {
    assert!(
        get_command_line_for_flag_groups(
            "flag_group(  expand_if_true = 'missing',  flags = ['-A%{missing}']),flag_group(  \
             expand_if_false = 'missing',  flags = ['-B%{missing}'])",
            &create_variables(&[]),
        )
        .is_empty()
    );
}

/// Java: `booleanValue(val)`.
fn boolean_value(val: bool) -> VariableValue {
    let mut b = CcToolchainVariables::builder();
    b.add_variable("name", VariableObject::Boolean(val));
    b.build().get_variable("name", &NOOP).unwrap()
}

#[test]
fn test_expand_if_true_expands_if_one() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(  expand_if_true = 'struct.bool',  flags = \
             ['-A%{struct.foo}','-B%{struct.bar}']),flag_group(  expand_if_false = \
             'struct.bool',  flags = ['-X%{struct.foo}','-Y%{struct.bar}'])",
            &create_structure_variables(
                "struct",
                StructureBuilder::new()
                    .add_field_value("bool", boolean_value(true))
                    .add_field("foo", "fooValue")
                    .add_field("bar", "barValue"),
            ),
        ),
        &["-AfooValue", "-BbarValue"],
    );
}

#[test]
fn test_expand_if_true_expands_if_zero() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(  expand_if_true = 'struct.bool',  flags = \
             ['-A%{struct.foo}','-B%{struct.bar}']),flag_group(  expand_if_false = \
             'struct.bool',  flags = ['-X%{struct.foo}', '-Y%{struct.bar}'])",
            &create_structure_variables(
                "struct",
                StructureBuilder::new()
                    .add_field_value("bool", boolean_value(false))
                    .add_field("foo", "fooValue")
                    .add_field("bar", "barValue"),
            ),
        ),
        &["-XfooValue", "-YbarValue"],
    );
}

#[test]
fn test_expand_if_equal() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(  expand_if_equal = variable_with_value(name = 'var', value = \
             'equal_value'),  flags = ['-foo_%{var}']),flag_group(  expand_if_equal = \
             variable_with_value(name = 'var', value = 'non_equal_value'),  flags = \
             ['-bar_%{var}']),flag_group(  expand_if_equal = variable_with_value(name = \
             'non_existing_var', value = 'non_existing'),  flags = \
             ['-baz_%{non_existing_var}'])",
            &create_variables(&["var", "equal_value"]),
        ),
        &["-foo_equal_value"],
    );
}

#[test]
fn test_list_variable_expansion() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(iterate_over = 'v', flags = ['%{v}'])",
            &create_variables(&["v", "1", "v", "2"]),
        ),
        &["1", "2"],
    );
}

#[test]
fn test_list_variable_expansion_mixed_with_non_list_variable() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(iterate_over = 'v1', flags = ['%{v1} %{v2}'])",
            &create_variables(&["v1", "a1", "v1", "a2", "v2", "b"]),
        ),
        &["a1 b", "a2 b"],
    );
}

#[test]
fn test_nested_list_variable_expansion() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(  iterate_over = 'v1',  flag_groups = [flag_group(    iterate_over = \
             'v2',    flags = ['%{v1} %{v2}'],  )])",
            &create_variables(&["v1", "a1", "v1", "a2", "v2", "b1", "v2", "b2"]),
        ),
        &["a1 b1", "a1 b2", "a2 b1", "a2 b2"],
    );
}

#[test]
fn test_list_variable_expansion_mixed_with_implicitly_accessed_list_variable_fails() {
    assert!(
        get_flag_groups_expansion_error(
            "flag_group(iterate_over = 'v1', flags = ['%{v1} %{v2}'])",
            &create_variables(&["v1", "a1", "v1", "a2", "v2", "b1", "v2", "b2"]),
        )
        .contains("Cannot expand variable 'v2': expected string, found sequence")
    );
}

#[test]
fn test_flag_group_variable_expansion() {
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(iterate_over = 'v', flags = ['-f', '%{v}']),flag_group(flags = ['-end'])",
            &create_variables(&["v", "1", "v", "2"]),
        ),
        &["-f", "1", "-f", "2", "-end"],
    );
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(iterate_over = 'v', flags = ['-f', '%{v}']),flag_group(iterate_over = 'v', flags = ['%{v}'])",
            &create_variables(&["v", "1", "v", "2"]),
        ),
        &["-f", "1", "-f", "2", "1", "2"],
    );
    assert_contains_exactly(
        &get_command_line_for_flag_groups(
            "flag_group(iterate_over = 'v', flags = ['-f', '%{v}']),flag_group(iterate_over = 'v', flags = ['%{v}'])",
            &create_variables(&["v", "1", "v", "2"]),
        ),
        &["-f", "1", "-f", "2", "1", "2"],
    );
}

/// Java: `createNestedSequence(depth, count, prefix)`.
fn create_nested_sequence(depth: i32, count: i32, prefix: &str) -> Vec<VariableObject> {
    let mut builder = Vec::new();
    if depth == 0 {
        for i in 0..count {
            let value = format!("{prefix}{i}");
            builder.push(VariableObject::Value(VariableValue::String(
                StringValue::new(value.as_str()),
            )));
        }
    } else {
        for i in 0..count {
            let value = format!("{prefix}{i}");
            builder.push(VariableObject::Value(VariableValue::Sequence(
                Sequence::new(create_nested_sequence(depth - 1, count, &value)),
            )));
        }
    }
    builder
}

fn create_nested_variables(name: &str, depth: i32, count: i32) -> CcToolchainVariablesRef {
    let mut b = CcToolchainVariables::builder();
    b.add_variable(
        name,
        VariableObject::list(create_nested_sequence(depth, count, "")),
    );
    b.build()
}

#[test]
fn test_flag_tree_variable_expansion() {
    let nested_group = "flag_group(  iterate_over = 'v',  flag_groups = [    flag_group(flags = \
                        ['-a']),    flag_group(iterate_over = 'v', flags = ['%{v}']),    \
                        flag_group(flags = ['-b']),  ],)";
    assert_contains_exactly(
        &get_command_line_for_flag_groups(nested_group, &create_nested_variables("v", 1, 3)),
        &[
            "-a", "00", "01", "02", "-b", "-a", "10", "11", "12", "-b", "-a", "20", "21", "22",
            "-b",
        ],
    );

    let e =
        command_line_for_flag_groups_with(nested_group, &create_nested_variables("v", 2, 3), &NOOP)
            .unwrap()
            .unwrap_err();
    assert!(e.message.contains("'v'"), "{}", e.message);

    let ae = build_features(&[
        "features = [feature(",
        "  name =  'a',",
        "  flag_sets = [flag_set(",
        "    action = 'c++-compile',",
        "    flag_groups = [flag_group(",
        "      flag_groups = [flag_group(flags = ['-f'])],",
        "      flags = ['-f'],",
        "    )],",
        "  )],",
        ")]",
    ])
    .err()
    .unwrap();
    assert!(
        ae.contains("flag_group must not contain both a flag and another flag_group."),
        "{ae}"
    );
}

#[test]
fn test_implies() {
    let features = bf(&[
        "features = [",
        "    feature(name = 'a', implies = ['b', 'c']),",
        "    feature(name = 'b'),",
        "    feature(name = 'c', implies = ['d']),",
        "    feature(name = 'd'),",
        "    feature(name = 'e'),",
        "]",
    ]);
    assert_eq!(
        get_enabled_features(&features, &["a"]),
        names(&["a", "b", "c", "d"])
    );
}

#[test]
fn test_requires() {
    let features = bf(&[
        "features = [",
        "    feature(name = 'a', requires = [feature_set(features = ['b'])]),",
        "    feature(name = 'b', requires = [feature_set(features = ['c'])]),",
        "    feature(name = 'c'),",
        "]",
    ]);
    assert!(get_enabled_features(&features, &["a"]).is_empty());
    assert!(get_enabled_features(&features, &["a", "b"]).is_empty());
    assert_eq!(get_enabled_features(&features, &["a", "c"]), names(&["c"]));
    assert_eq!(
        get_enabled_features(&features, &["a", "b", "c"]),
        names(&["a", "b", "c"])
    );
}

#[test]
fn test_disabled_requirement_chain() {
    let features = bf(&[
        "features = [",
        "    feature(name = 'a'),",
        "    feature(name = 'b', requires = [feature_set(features = ['c'])], implies = ['a']),",
        "    feature(name = 'c'),",
        "]",
    ]);
    assert!(get_enabled_features(&features, &["b"]).is_empty());
    let features = bf(&[
        "features = [",
        "    feature(name = 'a'),",
        "    feature(name = 'b', requires = [feature_set(features = ['a'])], implies = ['c']),",
        "    feature(name = 'c'),",
        "    feature(name = 'd', requires = [feature_set(features = ['c'])], implies = ['e']),",
        "    feature(name = 'e'),",
        "]",
    ]);
    assert!(get_enabled_features(&features, &["b", "d"]).is_empty());
}

#[test]
fn test_enabled_requirement_chain() {
    let features = bf(&[
        "features = [",
        "    feature(name = '0', implies = ['a']),",
        "    feature(name = 'a'),",
        "    feature(name = 'b', requires = [feature_set(features = ['a'])], implies = ['c']),",
        "    feature(name = 'c'),",
        "    feature(name = 'd', requires = [feature_set(features = ['c'])], implies = ['e']),",
        "    feature(name = 'e'),",
        "]",
    ]);
    assert_eq!(
        get_enabled_features(&features, &["0", "b", "d"]),
        names(&["0", "a", "b", "c", "d", "e"])
    );
}

#[test]
fn test_logic_in_requirements() {
    let features = bf(&[
        "features = [",
        "    feature(",
        "        name = 'a',",
        "        requires = [",
        "            feature_set(features = ['b', 'c']),",
        "            feature_set(features = ['d']),",
        "        ],",
        "    ),",
        "    feature(name = 'b'),",
        "    feature(name = 'c'),",
        "    feature(name = 'd'),",
        "]",
    ]);
    assert_eq!(
        get_enabled_features(&features, &["a", "b", "c"]),
        names(&["a", "b", "c"])
    );
    assert_eq!(get_enabled_features(&features, &["a", "b"]), names(&["b"]));
    assert_eq!(get_enabled_features(&features, &["a", "c"]), names(&["c"]));
    assert_eq!(
        get_enabled_features(&features, &["a", "d"]),
        names(&["a", "d"])
    );
}

#[test]
fn test_implies_implies_requires() {
    let features = bf(&[
        "features = [",
        "    feature(name = 'a', implies = ['b']),",
        "    feature(name = 'b', requires = [feature_set(features = ['c'])]),",
        "    feature(name = 'c'),",
        "]",
    ]);
    assert!(get_enabled_features(&features, &["a"]).is_empty());
}

#[test]
fn test_multiple_implies() {
    let features = bf(&[
        "features = [",
        "    feature(name = 'a', implies = ['b', 'c', 'd']),",
        "    feature(name = 'b'),",
        "    feature(name = 'c', requires = [feature_set(features = ['e'])]),",
        "    feature(name = 'd'),",
        "    feature(name = 'e'),",
        "]",
    ]);
    assert!(get_enabled_features(&features, &["a"]).is_empty());
    assert_eq!(
        get_enabled_features(&features, &["a", "e"]),
        names(&["a", "b", "c", "d", "e"])
    );
}

#[test]
fn test_disabled_features_do_not_enable_implications() {
    let features = bf(&[
        "features = [",
        "    feature(name = 'a', implies = ['b'], requires = [feature_set(features = ['c'])]),",
        "    feature(name = 'b'),",
        "    feature(name = 'c'),",
        "]",
    ]);
    assert!(get_enabled_features(&features, &["a"]).is_empty());
}

#[test]
fn test_feature_name_collision() {
    let e = build_features(&[
        "features = [",
        "    feature(name = '+++collision+++'),",
        "    feature(name = '+++collision+++'),",
        "]",
    ])
    .err()
    .unwrap();
    assert!(
        e.contains("feature or action config '+++collision+++' was specified multiple times."),
        "{e}"
    );
}

#[test]
fn test_reference_to_undefined_feature() {
    let e = build_features(&["features = [feature(name = 'a', implies = ['<<<undefined>>>'])]"])
        .err()
        .unwrap();
    assert!(
        e.contains(
            "feature '<<<undefined>>>', which is referenced from feature 'a', is not defined"
        ),
        "{e}"
    );
}

#[test]
fn test_implies_with_cycle() {
    let features = bf(&[
        "features = [",
        "    feature(name = 'a', implies = ['b']),",
        "    feature(name = 'b', implies = ['a']),",
        "]",
    ]);
    assert_eq!(get_enabled_features(&features, &["a"]), names(&["a", "b"]));
    assert_eq!(get_enabled_features(&features, &["b"]), names(&["a", "b"]));
}

#[test]
fn test_multiple_implies_cycle() {
    let features = bf(&[
        "features = [",
        "    feature(name = 'a', implies = ['b', 'c', 'd']),",
        "    feature(name = 'b'),",
        "    feature(name = 'c', requires = [feature_set(features = ['e'])]),",
        "    feature(name = 'd', requires = [feature_set(features = ['f'])]),",
        "    feature(name = 'e', requires = [feature_set(features = ['c'])]),",
        "    feature(name = 'f'),",
        "]",
    ]);
    assert!(get_enabled_features(&features, &["a", "e"]).is_empty());
    assert_eq!(
        get_enabled_features(&features, &["a", "e", "f"]),
        names(&["a", "b", "c", "d", "e", "f"])
    );
}

#[test]
fn test_requires_with_cycle() {
    let features = bf(&[
        "features = [",
        "    feature(name = 'a', requires = [feature_set(features = ['b'])]),",
        "    feature(name = 'b', requires = [feature_set(features = ['a'])]),",
        "    feature(name = 'c', implies = ['a']),",
        "    feature(name = 'd', implies = ['b']),",
        "]",
    ]);
    assert!(get_enabled_features(&features, &["c"]).is_empty());
    assert!(get_enabled_features(&features, &["d"]).is_empty());
    assert_eq!(
        get_enabled_features(&features, &["c", "d"]),
        names(&["a", "b", "c", "d"])
    );
}

#[test]
fn test_implied_by_one_enabled_and_one_disabled_feature() {
    let features = bf(&[
        "features = [",
        "    feature(name = 'a'),",
        "    feature(name = 'b', requires = [feature_set(features = ['a'])], implies = ['d']),",
        "    feature(name = 'c', implies = ['d']),",
        "    feature(name = 'd'),",
        "]",
    ]);
    assert_eq!(
        get_enabled_features(&features, &["b", "c"]),
        names(&["c", "d"])
    );
}

#[test]
fn test_requires_one_enabled_and_one_unsupported_feature() {
    let features = bf(&[
        "features = [",
        "    feature(",
        "        name = 'a',",
        "        requires = [",
        "            feature_set(features = ['b']),",
        "            feature_set(features = ['c'])",
        "        ],",
        "    ),",
        "    feature(name = 'b'),",
        "    feature(name = 'c', requires = [feature_set(features = ['d'])]),",
        "    feature(name = 'd'),",
        "]",
    ]);
    assert_eq!(
        get_enabled_features(&features, &["a", "b", "c"]),
        names(&["a", "b"])
    );
}

#[test]
fn test_flag_groups_with_missing_variable_is_not_expanded() {
    let configuration = get_feature_configuration(
        &bf(&[
            "features = [feature(",
            "    name = 'a',",
            "    flag_sets = [",
            "        flag_set(",
            "            actions = ['c++-compile'],",
            "            flag_groups = [",
            "                flag_group(expand_if_available = 'v', flags = ['%{v}'])",
            "            ],",
            "        ),",
            "        flag_set(",
            "            actions = ['c++-compile'],",
            "            flag_groups = [flag_group(flags = ['unconditional'])],",
            "        ),",
            "    ],",
            ")]",
        ]),
        &["a"],
    );
    assert_contains_exactly(
        &configuration
            .get_command_line(CppActionNames::CPP_COMPILE, &create_variables(&[]))
            .unwrap(),
        &["unconditional"],
    );
}

#[test]
fn test_only_flag_groups_with_all_variables_present_are_expanded() {
    let configuration = get_feature_configuration(
        &bf(&[
            "features = [feature(",
            "    name = 'a',",
            "    flag_sets = [",
            "        flag_set(",
            "            actions = ['c++-compile'],",
            "            flag_groups = [",
            "                flag_group(expand_if_available = 'v', flags = ['%{v}'])",
            "            ],",
            "        ),",
            "        flag_set(",
            "            actions = ['c++-compile'],",
            "            flag_groups = [",
            "                flag_group(",
            "                    expand_if_available = 'w',",
            "                    flag_groups = [flag_group(",
            "                    expand_if_available = 'v',",
            "                    flags = ['%{v}%{w}'])],",
            "                )",
            "            ],",
            "        ),",
            "        flag_set(",
            "            actions = ['c++-compile'],",
            "            flag_groups = [flag_group(flags = ['unconditional'])],",
            "        ),",
            "    ],",
            ")]",
        ]),
        &["a"],
    );
    assert_contains_exactly(
        &configuration
            .get_command_line(CppActionNames::CPP_COMPILE, &create_variables(&["v", "1"]))
            .unwrap(),
        &["1", "unconditional"],
    );
}

#[test]
fn test_only_inner_flag_group_is_iterated_with_sequence_variable() {
    let configuration = get_feature_configuration(
        &bf(&[
            "features = [feature(",
            "    name = 'a',",
            "    flag_sets = [",
            "        flag_set(",
            "            actions = ['c++-compile'],",
            "            flag_groups = [",
            "                flag_group(",
            "                    expand_if_available = 'v',",
            "                    iterate_over = 'v',",
            "                    flags = ['%{v}']",
            "                ),",
            "            ],",
            "        ),",
            "        flag_set(",
            "            actions = ['c++-compile'],",
            "            flag_groups = [",
            "                flag_group(",
            "                    expand_if_available = 'w',",
            "                    flag_groups = [flag_group(",
            "                    iterate_over = 'v',",
            "                    expand_if_available = 'v',",
            "                    flags = ['%{v}%{w}'])],",
            "                ),",
            "            ],",
            "        ),",
            "        flag_set(",
            "            actions = ['c++-compile'],",
            "            flag_groups = [flag_group(flags = ['unconditional'])],",
            "        ),",
            "    ],",
            ")]",
        ]),
        &["a"],
    );
    assert_in_order(
        &configuration
            .get_command_line(
                CppActionNames::CPP_COMPILE,
                &create_variables(&["v", "1", "v", "2"]),
            )
            .unwrap(),
        &["1", "2", "unconditional"],
    );
}

#[test]
fn test_flag_sets_are_iterated_individually_for_sequence_variables() {
    let configuration = get_feature_configuration(
        &bf(&[
            "features = [feature(",
            "    name = 'a',",
            "    flag_sets = [",
            "        flag_set(",
            "            actions = ['c++-compile'],",
            "            flag_groups = [",
            "                flag_group(",
            "                    expand_if_available = 'v',",
            "                    iterate_over = 'v',",
            "                    flags = ['%{v}']",
            "                ),",
            "            ],",
            "        ),",
            "        flag_set(",
            "            actions = ['c++-compile'],",
            "            flag_groups = [",
            "                flag_group(",
            "                    iterate_over = 'v',",
            "                    expand_if_available = 'v',",
            "                    flags = ['%{v}%{w}']",
            "                ),",
            "            ],",
            "        ),",
            "        flag_set(",
            "            actions = ['c++-compile'],",
            "            flag_groups = [flag_group(flags = ['unconditional'])],",
            "        ),",
            "    ],",
            ")]",
        ]),
        &["a"],
    );
    assert_in_order(
        &configuration
            .get_command_line(
                CppActionNames::CPP_COMPILE,
                &create_variables(&["v", "1", "v", "2", "w", "3"]),
            )
            .unwrap(),
        &["1", "2", "13", "23", "unconditional"],
    );
}

#[test]
fn test_configuration() {
    let features = bf(&[
        "features = [",
        "    feature(",
        "        name = 'a',",
        "        flag_sets = [",
        "            flag_set(",
        "                actions = ['c++-compile'],",
        "                flag_groups = [flag_group(flags = ['-f', '%{v}'])],",
        "            ),",
        "        ],",
        "    ),",
        "    feature(name = 'b', implies = ['a']),",
        "]",
    ]);
    assert_eq!(get_enabled_features(&features, &["b"]), names(&["a", "b"]));
    assert_contains_exactly(
        &get_feature_configuration(&features, &["b"])
            .get_command_line(CppActionNames::CPP_COMPILE, &create_variables(&["v", "1"]))
            .unwrap(),
        &["-f", "1"],
    );
    // PORT: the RoundTripping.roundTrip(features) half needs skyframe serialization; the Java
    // assertions after it re-check `features`, which the lines above already do.
}

#[test]
fn test_default_features() {
    let features = bf(&[
        "features = [feature(name = 'no_legacy_features'),feature(name = 'a'), feature(name = 'b', enabled = True)]",
    ]);
    assert_eq!(
        features.get_default_features_and_action_configs(),
        &["b".to_string()]
    );
}

#[test]
fn test_default_action_configs() {
    let features = bf(&[
        "features = [feature(name = 'no_legacy_features')],",
        "action_configs = [",
        "    action_config(action_name = 'a'),",
        "    action_config(action_name = 'b', enabled = True),",
        "]",
    ]);
    assert_eq!(
        features.get_default_features_and_action_configs(),
        &["b".to_string()]
    );
}

fn compile_cl(features: &CcToolchainFeatures, requested: &[&str]) -> Vec<String> {
    get_feature_configuration(features, requested)
        .get_command_line(CppActionNames::CPP_COMPILE, &create_variables(&[]))
        .unwrap()
}

#[test]
fn test_with_feature_one_set_one_feature() {
    let features = bf(&[
        "features = [",
        "feature(name = 'no_legacy_features'),",
        "    feature(",
        "        name = 'a',",
        "        flag_sets = [",
        "            flag_set(",
        "                with_features = [with_feature_set(features = ['b'])],",
        "                actions = ['c++-compile'],",
        "                flag_groups = [flag_group(flags = ['dummy_flag'])],",
        "            ),",
        "        ],",
        "    ),",
        "    feature(name = 'b'),",
        "]",
    ]);
    assert_contains_exactly(&compile_cl(&features, &["a", "b"]), &["dummy_flag"]);
    assert!(!compile_cl(&features, &["a"]).contains(&"dummy_flag".to_string()));
}

#[test]
fn test_with_feature_one_set_multiple_features() {
    let features = bf(&[
        "features = [",
        "    feature(",
        "        name = 'a',",
        "        flag_sets = [",
        "            flag_set(",
        "                with_features = [with_feature_set(features = ['b', 'c'])],",
        "                actions = ['c++-compile'],",
        "                flag_groups = [flag_group(flags = ['dummy_flag'])],",
        "            ),",
        "        ],",
        "    ),",
        "    feature(name = 'b'),",
        "    feature(name = 'c'),",
        "]",
    ]);
    assert_contains_exactly(&compile_cl(&features, &["a", "b", "c"]), &["dummy_flag"]);
    assert!(!compile_cl(&features, &["a", "b"]).contains(&"dummy_flag".to_string()));
    assert!(!compile_cl(&features, &["a"]).contains(&"dummy_flag".to_string()));
}

#[test]
fn test_with_feature_muliple_sets_multiple_features() {
    let features = bf(&[
        "features = [",
        "    feature(",
        "        name = 'a',",
        "        flag_sets = [",
        "            flag_set(",
        "                with_features = [",
        "                    with_feature_set(features = ['b1', 'c1']),",
        "                    with_feature_set(features = ['b2', 'c2']),",
        "                ],",
        "                actions = ['c++-compile'],",
        "                flag_groups = [flag_group(flags = ['dummy_flag'])],",
        "            ),",
        "        ],",
        "    ),",
        "    feature(name = 'b1'),",
        "    feature(name = 'c1'),",
        "    feature(name = 'b2'),",
        "    feature(name = 'c2'),",
        "]",
    ]);
    assert_contains_exactly(
        &compile_cl(&features, &["a", "b1", "c1", "b2", "c2"]),
        &["dummy_flag"],
    );
    assert_contains_exactly(&compile_cl(&features, &["a", "b1", "c1"]), &["dummy_flag"]);
    assert!(!compile_cl(&features, &["a", "b1", "b2"]).contains(&"dummy_flag".to_string()));
}

#[test]
fn test_with_feature_not_feature() {
    let features = bf(&[
        "features = [",
        "    feature(",
        "        name = 'a',",
        "        flag_sets = [",
        "            flag_set(",
        "                with_features = [",
        "                    with_feature_set(not_features = ['x', 'y'], features = ['z']),",
        "                    with_feature_set(not_features = ['q']),",
        "                ],",
        "                actions = ['c++-compile'],",
        "                flag_groups = [flag_group(flags = ['dummy_flag'])],",
        "            ),",
        "        ],",
        "    ),",
        "    feature(name = 'x'),",
        "    feature(name = 'y'),",
        "    feature(name = 'z'),",
        "    feature(name = 'q'),",
        "]",
    ]);
    let d = "dummy_flag".to_string();
    assert_contains_exactly(&compile_cl(&features, &["a"]), &["dummy_flag"]);
    assert!(!compile_cl(&features, &["a", "q"]).contains(&d));
    assert_contains_exactly(&compile_cl(&features, &["a", "q", "z"]), &["dummy_flag"]);
    assert!(!compile_cl(&features, &["a", "q", "x", "z"]).contains(&d));
    assert!(!compile_cl(&features, &["a", "q", "x", "y", "z"]).contains(&d));
}

const ACTION_A_FEATURE_A: &[&str] = &[
    "action_configs = [",
    "    action_config(",
    "        action_name = 'action-a',",
    "        tools = [",
    "            tool(",
    "                path = 'toolchain/feature-a',",
    "                with_features = [with_feature_set(features = ['feature-a'])],",
    "            ),",
    "        ],",
    "    ),",
    "],",
];

#[test]
fn test_activate_action_config_from_feature() {
    let mut content = ACTION_A_FEATURE_A.to_vec();
    content.extend([
        "features = [",
        "    feature(name = 'activates-action-a', implies = ['action-a']),",
        "]",
    ]);
    let toolchain_features = bf(&content);
    let feature_configuration =
        get_feature_configuration(&toolchain_features, &["activates-action-a"]);
    assert!(feature_configuration.action_is_configured("action-a"));
}

#[test]
fn test_feature_can_require_action_config() {
    let mut content = ACTION_A_FEATURE_A.to_vec();
    content.extend([
        "features = [",
        "    feature(",
        "        name = 'requires-action-a',",
        "        requires = [feature_set(features = ['action-a'])],",
        "    ),",
        "]",
    ]);
    let toolchain_features = bf(&content);
    let without = get_feature_configuration(&toolchain_features, &["requires-action-a"]);
    assert!(!without.is_enabled("requires-action-a"));
    let with = get_feature_configuration(&toolchain_features, &["action-a", "requires-action-a"]);
    assert!(with.is_enabled("requires-action-a"));
}

#[test]
fn test_simple_action_tool() {
    let configuration = get_feature_configuration(
        &bf(&[
            "action_configs = [",
            "    action_config(",
            "        action_name = 'action-a',",
            "        tools = [tool(path = 'toolchain/a')],",
            "    ),",
            "],",
            "features = [",
            "    feature(name = 'activates-action-a', implies = ['action-a']),",
            "]",
        ]),
        &["activates-action-a"],
    );
    assert_eq!(
        configuration.get_tool_path_for_action("action-a"),
        "crosstool/toolchain/a"
    );
}

#[test]
fn test_action_tool_from_feature_set() {
    let toolchain_features = bf(&[
        "action_configs = [",
        "    action_config(",
        "        action_name = 'action-a',",
        "        tools = [",
        "            tool(",
        "                path = 'toolchain/features-a-and-b',",
        "                with_features = [with_feature_set(features = ['feature-a', 'feature-b'])],",
        "            ),",
        "            tool(",
        "                path = 'toolchain/feature-a-and-not-c',",
        "                with_features = [with_feature_set(features = ['feature-a'], not_features = ['feature-c'])],",
        "            ),",
        "            tool(",
        "                path = 'toolchain/feature-b-or-c',",
        "                with_features = [",
        "                    with_feature_set(features = ['feature-b']),",
        "                    with_feature_set(features = ['feature-c'])",
        "                ],",
        "            ),",
        "            tool(path = 'toolchain/default'),",
        "        ],",
        "    ),",
        "],",
        "features = [",
        "    feature(name = 'feature-a'),",
        "    feature(name = 'feature-b'),",
        "    feature(name = 'feature-c'),",
        "    feature(name = 'activates-action-a', implies = ['action-a']),",
        "]",
    ]);
    let tool = |requested: &[&str]| {
        get_feature_configuration(&toolchain_features, requested)
            .get_tool_path_for_action("action-a")
    };
    assert_eq!(
        tool(&["feature-a", "activates-action-a"]),
        "crosstool/toolchain/feature-a-and-not-c"
    );
    assert_eq!(
        tool(&["feature-a", "feature-c", "activates-action-a"]),
        "crosstool/toolchain/feature-b-or-c"
    );
    assert_eq!(
        tool(&["feature-b", "activates-action-a"]),
        "crosstool/toolchain/feature-b-or-c"
    );
    assert_eq!(
        tool(&["feature-c", "activates-action-a"]),
        "crosstool/toolchain/feature-b-or-c"
    );
    assert_eq!(
        tool(&["feature-a", "feature-b", "activates-action-a"]),
        "crosstool/toolchain/features-a-and-b"
    );
    assert_eq!(tool(&["activates-action-a"]), "crosstool/toolchain/default");
}

#[test]
fn test_error_for_no_matching_tool() {
    let mut content = ACTION_A_FEATURE_A.to_vec();
    content.extend([
        "features = [",
        "    feature(name = 'feature-a'),",
        "    feature(name = 'activates-action-a', implies = ['action-a']),",
        "]",
    ]);
    let toolchain_features = bf(&content);
    let no_features_configuration =
        get_feature_configuration(&toolchain_features, &["activates-action-a"]);
    let e = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        no_features_configuration.get_tool_path_for_action("action-a")
    }))
    .unwrap_err();
    let message = e.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.starts_with("IllegalArgumentException"), "{message}");
    assert!(
        message.contains(
            "Matching tool for action action-a not found for given feature configuration"
        ),
        "{message}"
    );
}

#[test]
fn test_activate_action_config_directly() {
    let mut content = ACTION_A_FEATURE_A.to_vec();
    *content.last_mut().unwrap() = "]";
    let toolchain_features = bf(&content);
    let feature_configuration = get_feature_configuration(&toolchain_features, &["action-a"]);
    assert!(feature_configuration.action_is_configured("action-a"));
}

#[test]
fn test_action_config_can_activate_feature() {
    let toolchain_features = bf(&[
        "action_configs = [",
        "    action_config(",
        "        action_name = 'action-a',",
        "        tools = [",
        "            tool(",
        "                path = 'toolchain/feature-a',",
        "                with_features = [with_feature_set(features = ['feature-a'])],",
        "            ),",
        "        ],",
        "        implies = ['activated-feature'],",
        "    ),",
        "],",
        "features = [",
        "    feature(name = 'activated-feature'),",
        "]",
    ]);
    let feature_configuration = get_feature_configuration(&toolchain_features, &["action-a"]);
    assert!(feature_configuration.is_enabled("activated-feature"));
}

#[test]
fn test_invalid_action_configuration_multiple_action_configs_for_action() {
    let e = build_features(&[
        "action_configs = [",
        "    action_config(action_name = 'action-a'),",
        "    action_config(action_name = 'action-a'),",
        "]",
    ])
    .err()
    .unwrap();
    assert!(
        e.contains("multiple action configs for action 'action-a'"),
        "{e}"
    );
}

#[test]
fn test_flags_from_action_config() {
    let feature_configuration = get_feature_configuration(
        &bf(&[
            "action_configs = [",
            "    action_config(",
            "        action_name = 'c++-compile',",
            "        flag_sets = [flag_set(flag_groups = [flag_group(flags = ['foo'])])],",
            "    ),",
            "]",
        ]),
        &["c++-compile"],
    );
    let command_line = feature_configuration
        .get_command_line("c++-compile", &create_variables(&[]))
        .unwrap();
    assert!(command_line.contains(&"foo".to_string()));
}

#[test]
fn test_error_for_flag_from_action_config_with_specified_action() {
    let e = build_features(&[
        "action_configs = [",
        "    action_config(",
        "        action_name = 'c++-compile',",
        "        flag_sets = [",
        "            flag_set(",
        "                actions = ['c++-compile'],",
        "                flag_groups = [flag_group(flags = ['foo'])],",
        "            ),",
        "        ],",
        "    ),",
        "]",
    ])
    .err()
    .unwrap();
    assert!(
        e.contains(&ActionConfig::flag_set_with_action_error("c++-compile")),
        "{e}"
    );
}

#[test]
fn test_provides_collision() {
    let features = bf(&[
        "features = [",
        "    feature(name = 'a', provides = ['provides_string']),",
        "    feature(name = 'b', provides = ['provides_string']),",
        "]",
    ]);
    let e = features
        .get_feature_configuration(&set(&["a", "b"]))
        .unwrap_err();
    assert!(e.message.contains("a b"), "{}", e.message);
    assert_eq!(
        e.message,
        "Symbol provides_string is provided by all of the following features: a b"
    );
}

#[test]
fn test_get_artifact_name_extension_for_category() {
    let toolchain_features = bf(&[
        "artifact_name_patterns = [",
        "    artifact_name_pattern(",
        "        category_name = 'object_file',",
        "        prefix = '',",
        "        extension = '.obj',",
        "    ),",
        "    artifact_name_pattern(",
        "        category_name = 'executable',",
        "        prefix = '',",
        "        extension = '',",
        "    ),",
        "    artifact_name_pattern(",
        "        category_name = 'static_library',",
        "        prefix = '',",
        "        extension = '.a',",
        "    ),",
        "]",
    ]);
    assert_eq!(
        toolchain_features.get_artifact_name_extension_for_category(ArtifactCategory::OBJECT_FILE),
        ".obj"
    );
    assert!(
        toolchain_features
            .get_artifact_name_extension_for_category(ArtifactCategory::EXECUTABLE)
            .is_empty()
    );
    assert_eq!(
        toolchain_features
            .get_artifact_name_extension_for_category(ArtifactCategory::STATIC_LIBRARY),
        ".a"
    );
}

/// Not in the Java test: the position in the parse error counts the chars of the internal string
/// (one per byte of the flag), as Java's index does. `é` (c3 a9), `🐷` (f0 9f 90 b7) and the
/// byte ff are 2, 4 and 1 chars.
#[test]
fn flag_parsing_error_position_counts_internal_chars() {
    for (flag, position) in [
        ("\u{c3}\u{a9}%", 3),
        ("\u{f0}\u{9f}\u{90}\u{b7}%x", 5),
        ("\u{ff}%", 2),
    ] {
        let error = get_flag_parsing_error(flag);
        assert!(
            error.contains(&format!(
                "expected '{{' at position {position} while parsing a flag containing '{flag}'"
            )),
            "{error}"
        );
    }
    // The expansion keeps the chars as they are.
    let empty = create_variables(&[]);
    assert_eq!(
        get_expansion_of_flag("\u{c3}\u{a9}%%\u{ff}", &empty),
        "\u{c3}\u{a9}%\u{ff}"
    );
}
