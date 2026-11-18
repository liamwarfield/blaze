//! Port of `src/test/java/com/google/devtools/build/lib/rules/cpp/CompileCommandLineTest.java`.

mod common;

use std::sync::Arc;

use blaze_cc_features::actions::NOOP;
use blaze_cc_features::cc_common::CoptsFilter;
use blaze_cc_features::compile_command_line::{CompileCommandLine, CompileCommandLineBuilder};
use blaze_cc_features::cpp_rule_classes::UNFILTERED_COMPILE_FLAGS_FEATURE_NAME;
use blaze_cc_features::{CppActionNames, FeatureConfiguration};
use common::*;
use indexmap::IndexSet;

fn get_mock_feature_configuration_from_starlark(starlark: &[&str]) -> Arc<FeatureConfiguration> {
    let features = build_features_with_tools_directory(starlark, "").unwrap();
    let requested: IndexSet<String> = [
        CppActionNames::ASSEMBLE,
        CppActionNames::PREPROCESS_ASSEMBLE,
        CppActionNames::C_COMPILE,
        CppActionNames::CPP_COMPILE,
        CppActionNames::CPP_HEADER_PARSING,
        CppActionNames::CPP_MODULE_CODEGEN,
        CppActionNames::CPP_MODULE_COMPILE,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    features.get_feature_configuration(&requested).unwrap()
}

fn make_compile_command_line_builder() -> CompileCommandLineBuilder {
    CompileCommandLine::builder(CoptsFilter::always_passes(), "c++-compile")
}

#[test]
fn test_feature_configuration_command_line_is_used() {
    let compile_command_line = make_compile_command_line_builder()
        .set_feature_configuration(get_mock_feature_configuration_from_starlark(&[
            "action_configs = [",
            "    action_config(",
            "        action_name = 'c++-compile',",
            "        implies = ['some_foo_feature'],",
            "        tools = [tool(path = 'foo/bar/DUMMY_COMPILER')],",
            "    ),",
            "],",
            "features = [",
            "    feature(",
            "        name = 'some_foo_feature',",
            "        flag_sets = [",
            "            flag_set(",
            "                actions = ['c++-compile'],",
            "                flag_groups = [flag_group(flags = ['-some_foo_flag'])],",
            "            ),",
            "        ],",
            "    ),",
            "]",
        ]))
        .build();
    assert!(
        compile_command_line
            .get_arguments(None, None, &NOOP)
            .unwrap()
            .contains(&"-some_foo_flag".to_string())
    );
}

#[test]
fn test_unfiltered_flags_are_not_filtered() {
    let actual_command_line =
        get_compile_command_line_with_copts_filter(UNFILTERED_COMPILE_FLAGS_FEATURE_NAME);
    assert!(actual_command_line.contains(&"-i_am_a_flag".to_string()));
}

#[test]
fn test_non_unfiltered_flags_are_filtered() {
    let actual_command_line = get_compile_command_line_with_copts_filter("filtered_flags");
    assert!(!actual_command_line.contains(&"-i_am_a_flag".to_string()));
}

fn get_compile_command_line_with_copts_filter(feature_name: &str) -> Vec<String> {
    let implies = format!("        implies = ['{feature_name}'],");
    let name = format!("        name = '{feature_name}',");
    let compile_command_line = make_compile_command_line_builder()
        .set_feature_configuration(get_mock_feature_configuration_from_starlark(&[
            "action_configs = [",
            "    action_config(",
            "        action_name = 'c++-compile',",
            &implies,
            "        tools = [tool(path = 'foo/bar/DUMMY_COMPILER')],",
            "    ),",
            "],",
            "features = [",
            "    feature(",
            &name,
            "        flag_sets = [",
            "            flag_set(",
            "                actions = ['c++-compile'],",
            "                flag_groups = [flag_group(flags = ['-i_am_a_flag'])],",
            "            ),",
            "        ],",
            "    ),",
            "]",
        ]))
        .set_copts_filter(CoptsFilter::from_regex(".*i_am_a_flag.*").unwrap())
        .build();
    compile_command_line
        .get_arguments(None, None, &NOOP)
        .unwrap()
}
