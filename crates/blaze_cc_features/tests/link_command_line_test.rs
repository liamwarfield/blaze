//! Port of `src/test/java/com/google/devtools/build/lib/rules/cpp/LinkCommandLineTest.java`.
//!
//! The mock toolchain is rules_cc's legacy feature set (no features of its own), evaluated with
//! starlark-rust. `useConfiguration(...)` calls only change Java's analysis configuration, which
//! these command lines never read (the variables are set by hand).

mod common;

use std::sync::Arc;

use blaze_cc_features::actions::{InputMetadataProvider, NOOP, ParameterFileType};
use blaze_cc_features::cc_toolchain_variables::{CcToolchainVariablesBuilder, VariableObject};
use blaze_cc_features::cpp_rule_classes;
use blaze_cc_features::link::LinkTargetType;
use blaze_cc_features::link_command_line::LinkCommandLineBuilder;
use blaze_cc_features::{CcToolchainVariables, CppActionNames, FeatureConfiguration};
use common::*;
use indexmap::IndexSet;

const SYSROOT_VARIABLE_NAME: &str = "sysroot";

fn get_mock_build_variables() -> CcToolchainVariablesBuilder {
    get_mock_build_variables_with(&[])
}

fn get_mock_build_variables_with(linkstamp_outputs: &[&str]) -> CcToolchainVariablesBuilder {
    let mut result = CcToolchainVariables::builder();
    result.add_variable("generate_interface_library", VariableObject::string("no"));
    result.add_variable(
        "interface_library_input_path",
        VariableObject::string("ignored"),
    );
    result.add_variable(
        "interface_library_output_path",
        VariableObject::string("ignored"),
    );
    result.add_variable(
        "interface_library_builder_path",
        VariableObject::string("ignored"),
    );
    result.add_string_sequence_variable("linkstamp_paths", linkstamp_outputs.iter());
    result
}

fn get_mock_feature_configuration() -> Arc<FeatureConfiguration> {
    let features = build_features_with_tools_directory(&[], "").unwrap();
    let requested: IndexSet<String> = [
        LinkTargetType::EXECUTABLE.get_action_name(),
        LinkTargetType::NODEPS_DYNAMIC_LIBRARY.get_action_name(),
        LinkTargetType::STATIC_LIBRARY.get_action_name(),
        CppActionNames::CPP_COMPILE,
        CppActionNames::LINKSTAMP_COMPILE,
        cpp_rule_classes::INCLUDES,
        cpp_rule_classes::PREPROCESSOR_DEFINES,
        cpp_rule_classes::INCLUDE_PATHS,
        cpp_rule_classes::PIC,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    features.get_feature_configuration(&requested).unwrap()
}

fn minimal_configuration_with(variables: &CcToolchainVariablesBuilder) -> LinkCommandLineBuilder {
    let mut b = LinkCommandLineBuilder::new();
    b.set_build_variables(variables.build())
        .set_feature_configuration(get_mock_feature_configuration());
    b
}

fn minimal_configuration() -> LinkCommandLineBuilder {
    minimal_configuration_with(&get_mock_build_variables())
}

/// Tests that when linking without linkstamps, the exec command is the same as the link command.
#[test]
fn test_link_command_is_exec_command_when_no_linkstamps() {
    let link_config = minimal_configuration()
        .set_action_name(LinkTargetType::EXECUTABLE.get_action_name())
        .build();
    let raw_link_argv = link_config.arguments().unwrap();
    assert_eq!(link_config.arguments().unwrap(), raw_link_argv);
}

/// Tests that symbol count output does not appear in argv when it should not.
#[test]
fn test_symbol_counts_disabled() {
    let link_config = minimal_configuration()
        .force_tool_path("foo/bar/gcc")
        .build();
    let argv = link_config.arguments().unwrap();
    for arg in argv {
        assert!(!arg.contains("print-symbol-counts"));
    }
}

fn for_static_library(name: &str, is_whole_archive: bool) -> VariableObject {
    TestStruct::object(vec![
        ("type", VariableObject::string("static_library")),
        ("name", VariableObject::string(name)),
        (
            "is_whole_archive",
            VariableObject::Boolean(is_whole_archive),
        ),
    ])
}

fn for_object_file(path: &str, is_whole_archive: bool) -> VariableObject {
    TestStruct::object(vec![
        ("type", VariableObject::string("object_file")),
        ("name", VariableObject::string(path)),
        (
            "is_whole_archive",
            VariableObject::Boolean(is_whole_archive),
        ),
    ])
}

fn for_object_file_group(
    object_files: Vec<blaze_cc_features::actions::ArtifactRef>,
    is_whole_archive: bool,
) -> VariableObject {
    TestStruct::object(vec![
        ("type", VariableObject::string("object_file_group")),
        (
            "object_files",
            VariableObject::list(
                object_files
                    .into_iter()
                    .map(VariableObject::Artifact)
                    .collect(),
            ),
        ),
        (
            "is_whole_archive",
            VariableObject::Boolean(is_whole_archive),
        ),
    ])
}

#[test]
fn test_libraries_to_link() {
    let mut variables = get_mock_build_variables();
    variables.add_variable(
        "libraries_to_link",
        VariableObject::list(vec![
            for_static_library("foo", false),
            for_static_library("bar", true),
        ]),
    );

    let link_config = minimal_configuration_with(&variables)
        .force_tool_path("foo/bar/gcc")
        .set_action_name(LinkTargetType::NODEPS_DYNAMIC_LIBRARY.get_action_name())
        .build();
    let command_line = link_config.arguments().unwrap().join(" ");
    assert!(
        regex::Regex::new("^.*foo -Wl,-whole-archive bar -Wl,-no-whole-archive.*$")
            .unwrap()
            .is_match(&command_line),
        "{command_line}"
    );
}

#[test]
fn test_library_search_directories() {
    let mut variables = get_mock_build_variables();
    variables.add_string_sequence_variable("library_search_directories", ["foo", "bar"]);

    let link_config = minimal_configuration_with(&variables)
        .set_action_name(LinkTargetType::NODEPS_DYNAMIC_LIBRARY.get_action_name())
        .build();
    let args = link_config.arguments().unwrap();
    let foo = args.iter().position(|a| a == "-Lfoo").unwrap();
    let bar = args.iter().position(|a| a == "-Lbar").unwrap();
    assert!(foo < bar);
}

#[test]
fn test_linker_param_file_for_static_library() {
    let mut variables = get_mock_build_variables();
    variables.add_variable(
        "linker_param_file",
        VariableObject::string("LINKER_PARAM_FILE_PLACEHOLDER"),
    );

    let link_config = minimal_configuration_with(&variables)
        .set_action_name(LinkTargetType::STATIC_LIBRARY.get_action_name())
        .set_split_command_line(true)
        .build();
    assert!(
        link_config
            .get_command_lines()
            .unwrap()
            .1
            .unwrap()
            .use_always
    );
}

#[test]
fn test_linker_param_file_for_dynamic_library() {
    let mut variables = get_mock_build_variables();
    variables.add_variable(
        "linker_param_file",
        VariableObject::string("LINKER_PARAM_FILE_PLACEHOLDER"),
    );

    let link_config = minimal_configuration_with(&variables)
        .set_action_name(LinkTargetType::NODEPS_DYNAMIC_LIBRARY.get_action_name())
        .set_split_command_line(true)
        .build();
    assert!(
        link_config
            .get_command_lines()
            .unwrap()
            .1
            .unwrap()
            .use_always
    );
}

fn basic_argv(target_type: LinkTargetType) -> Vec<String> {
    basic_argv_with(target_type, &get_mock_build_variables())
}

fn basic_argv_with(
    target_type: LinkTargetType,
    variables: &CcToolchainVariablesBuilder,
) -> Vec<String> {
    let link_config = minimal_configuration_with(variables)
        .set_action_name(target_type.get_action_name())
        .build();
    link_config.arguments().unwrap()
}

/// Tests that a "--force_pic" configuration applies "-pie" to executable links.
#[test]
fn test_pic_mode() {
    let pie_arg = "-pie".to_string();

    // Disabled:
    assert!(!basic_argv(LinkTargetType::EXECUTABLE).contains(&pie_arg));
    assert!(!basic_argv(LinkTargetType::NODEPS_DYNAMIC_LIBRARY).contains(&pie_arg));
    assert!(!basic_argv(LinkTargetType::STATIC_LIBRARY).contains(&pie_arg));
    assert!(!basic_argv(LinkTargetType::PIC_STATIC_LIBRARY).contains(&pie_arg));
    assert!(!basic_argv(LinkTargetType::ALWAYS_LINK_STATIC_LIBRARY).contains(&pie_arg));
    assert!(!basic_argv(LinkTargetType::ALWAYS_LINK_PIC_STATIC_LIBRARY).contains(&pie_arg));

    let mut pic_variables = get_mock_build_variables();
    pic_variables.add_variable("force_pic", VariableObject::string(""));
    // Enabled:
    assert!(basic_argv_with(LinkTargetType::EXECUTABLE, &pic_variables).contains(&pie_arg));
    for t in [
        LinkTargetType::NODEPS_DYNAMIC_LIBRARY,
        LinkTargetType::STATIC_LIBRARY,
        LinkTargetType::PIC_STATIC_LIBRARY,
        LinkTargetType::ALWAYS_LINK_STATIC_LIBRARY,
        LinkTargetType::ALWAYS_LINK_PIC_STATIC_LIBRARY,
    ] {
        assert!(!basic_argv_with(t, &pic_variables).contains(&pie_arg));
    }
}

#[test]
fn test_split_static_link_command() {
    let mut variables = get_mock_build_variables();
    variables.add_variable("output_execpath", VariableObject::string("a/FakeOutput"));
    variables.add_variable(
        "linker_param_file",
        VariableObject::string("LINKER_PARAM_FILE_PLACEHOLDER"),
    );
    let link_config = minimal_configuration_with(&variables)
        .set_action_name(LinkTargetType::STATIC_LIBRARY.get_action_name())
        .force_tool_path("foo/bar/ar")
        .set_split_command_line(true)
        .set_parameter_file_type(ParameterFileType::UNQUOTED)
        .build();
    let (tool, param_file_info) = link_config.get_command_lines().unwrap();
    assert_eq!(tool, "foo/bar/ar");
    assert!(param_file_info.unwrap().use_always);
    assert_eq!(
        link_config.get_param_command_line(None, &NOOP).unwrap(),
        vec!["rcsD", "a/FakeOutput"]
    );
}

#[test]
fn test_split_dynamic_link_command() {
    let mut variables = get_mock_build_variables();
    variables.add_variable("output_execpath", VariableObject::string("a/FakeOutput"));
    variables.add_variable(
        "linker_param_file",
        VariableObject::string("some/file.params"),
    );
    variables.add_string_sequence_variable("user_link_flags", [""]);
    let link_config = minimal_configuration_with(&variables)
        .set_action_name(LinkTargetType::DYNAMIC_LIBRARY.get_action_name())
        .force_tool_path("foo/bar/linker")
        .set_split_command_line(true)
        .build();
    assert_eq!(link_config.get_command_lines().unwrap().0, "foo/bar/linker");
    assert_eq!(
        link_config.get_param_command_line(None, &NOOP).unwrap(),
        vec!["-shared", "-o", "a/FakeOutput", ""]
    );
}

#[test]
fn test_static_link_command() {
    let mut variables = get_mock_build_variables();
    variables.add_variable("output_execpath", VariableObject::string("a/FakeOutput"));
    let link_config = minimal_configuration_with(&variables)
        .force_tool_path("foo/bar/ar")
        .set_action_name(LinkTargetType::STATIC_LIBRARY.get_action_name())
        .build();
    let result = link_config.arguments().unwrap();
    assert_eq!(result, vec!["rcsD", "a/FakeOutput"]);
    assert_eq!(link_config.get_linker_path_string().unwrap(), "foo/bar/ar");
}

#[test]
fn test_split_always_link_link_command() {
    let mut variables = CcToolchainVariables::builder();
    variables.add_variable(
        SYSROOT_VARIABLE_NAME,
        VariableObject::string("/usr/grte/v1"),
    );
    variables.add_variable("output_execpath", VariableObject::string("a/FakeOutput"));
    variables.add_variable(
        "linker_param_file",
        VariableObject::string("some/file.params"),
    );
    variables.add_variable(
        "libraries_to_link",
        VariableObject::list(vec![
            for_object_file("foo.o", false),
            for_object_file("bar.o", false),
        ]),
    );

    let link_config = minimal_configuration_with(&variables)
        .set_action_name(LinkTargetType::ALWAYS_LINK_STATIC_LIBRARY.get_action_name())
        .force_tool_path("foo/bar/ar")
        .set_split_command_line(true)
        .build();

    assert_eq!(link_config.get_command_lines().unwrap().0, "foo/bar/ar");
    assert_eq!(
        link_config.get_param_command_line(None, &NOOP).unwrap(),
        vec!["rcsD", "a/FakeOutput", "foo.o", "bar.o"]
    );
}

fn verify_arguments(arguments: &[String], allowed: &[String], disallowed: &[String]) {
    for a in allowed {
        assert!(arguments.contains(a), "{a} not in {arguments:?}");
    }
    for d in disallowed {
        assert!(!arguments.contains(d), "{d} in {arguments:?}");
    }
}

#[test]
fn test_tree_artifact_link() {
    let test_tree_artifact = TestArtifact::tree("out/library_directory");
    let library0 = TestArtifact::derived("out/library_directory/library0.o");
    let library1 = TestArtifact::derived("out/library_directory/library1.o");

    let fake_action_input_file_cache = FakeInputMetadataProvider {
        trees: vec![(
            test_tree_artifact.clone(),
            vec![library0.clone(), library1.clone()],
        )],
    };

    let tree_artifacts_paths = vec![test_tree_artifact.get_exec_path_string()];
    let tree_file_artifacts_paths = vec![
        library0.get_exec_path_string(),
        library1.get_exec_path_string(),
    ];

    let mut variables = get_mock_build_variables();
    variables.add_variable(
        "linker_param_file",
        VariableObject::string("some/file.params"),
    );
    variables.add_variable(
        "libraries_to_link",
        VariableObject::list(vec![for_object_file_group(
            vec![test_tree_artifact.clone()],
            false,
        )]),
    );
    let link_config = minimal_configuration_with(&variables)
        .force_tool_path("foo/bar/gcc")
        .set_action_name(LinkTargetType::STATIC_LIBRARY.get_action_name())
        .set_split_command_line(true)
        .build();

    // Should only reference the tree artifact.
    verify_arguments(
        &link_config.arguments_with(None, &NOOP).unwrap(),
        &tree_artifacts_paths,
        &tree_file_artifacts_paths,
    );
    verify_arguments(
        &link_config.arguments().unwrap(),
        &tree_artifacts_paths,
        &tree_file_artifacts_paths,
    );
    verify_arguments(
        &link_config.get_param_command_line(None, &NOOP).unwrap(),
        &tree_artifacts_paths,
        &tree_file_artifacts_paths,
    );

    // Should only reference tree file artifacts.
    let imp: &dyn InputMetadataProvider = &fake_action_input_file_cache;
    verify_arguments(
        &link_config.arguments_with(Some(imp), &NOOP).unwrap(),
        &tree_file_artifacts_paths,
        &tree_artifacts_paths,
    );
    verify_arguments(
        &link_config.arguments_with(Some(imp), &NOOP).unwrap(),
        &tree_file_artifacts_paths,
        &tree_artifacts_paths,
    );
    verify_arguments(
        &link_config
            .get_param_command_line(Some(imp), &NOOP)
            .unwrap(),
        &tree_file_artifacts_paths,
        &tree_artifacts_paths,
    );
}
