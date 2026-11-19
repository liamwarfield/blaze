//! Port of `src/test/java/com/google/devtools/build/lib/skyframe/ProjectFunctionTest.java`.
//!
//! The tests of the protolark schema (`project_pb2.Project.create(...)`) need the test's
//! `writeProjectSclDefinition` helper (the `project_proto.scl` module generated from
//! `project.proto`), which is not ported; they are ignored.

use blaze_cmdline::Label;
use blaze_sky_loading::project_function::{ProjectKey, ProjectValue};
use blaze_skyframe::{EvaluationResult, SkyKey};

use crate::testing::BuildViewTestCase;

fn set_up() -> BuildViewTestCase {
    let mut t = BuildViewTestCase::new();
    t.set_build_language_options(&["--experimental_enable_scl_dialect=true"]);
    t
}

fn evaluate(t: &mut BuildViewTestCase) -> (ProjectKey, EvaluationResult) {
    let key = ProjectKey::new(Label::parse_canonical("//test:PROJECT.scl").unwrap());
    let result = t.evaluate(&[SkyKey::new(key.clone())], false);
    (key, result)
}

fn value(result: &EvaluationResult, key: &ProjectKey) -> std::sync::Arc<ProjectValue> {
    assert!(!result.has_error(), "{:?}", result.error_map());
    result.get(key).expect("value")
}

fn error_message(result: &EvaluationResult) -> String {
    assert!(result.has_error());
    result
        .get_first_error()
        .and_then(|e| e.get_exception())
        .expect("exception")
        .to_string()
}

// Java: ProjectFunctionTest.projectFunction_emptyFile_isValid
#[test]
fn project_function_empty_file_is_valid() {
    let mut t = set_up();
    t.scratch.file("test/PROJECT.scl", &["project = {}"]);
    t.scratch.file("test/BUILD", &[]);
    let (key, result) = evaluate(&mut t);
    assert!(
        value(&result, &key)
            .get_default_project_directories()
            .is_empty()
    );
}

// Java: ProjectFunctionTest.projectFunction_returnsActiveDirectories
#[test]
#[ignore = "needs PathFragmentPrefixTrie (lib/vfs), not ported"]
fn project_function_returns_active_directories() {}

// Java: ProjectFunctionTest.projectFunction_returnsDefaultActiveDirectories
// PORT: Java checks the directories through PathFragmentPrefixTrie (not ported): "a" and "b/c"
// are included, "d" is not. The directories themselves are compared here.
#[test]
fn project_function_returns_default_active_directories() {
    let mut t = set_up();
    t.scratch.file(
        "test/PROJECT.scl",
        &[
            "project = {",
            "  \"active_directories\": { 'default': ['a', 'b/c'] },",
            "}",
        ],
    );
    t.scratch.file("test/BUILD", &[]);
    let (key, result) = evaluate(&mut t);
    let dirs: Vec<String> = value(&result, &key)
        .get_default_project_directories()
        .into_iter()
        .collect();
    assert_eq!(dirs, vec!["a".to_string(), "b/c".to_string()]);
}

// Java: ProjectFunctionTest.projectFunction_returnsDefaultActiveDirectories_topLevelProjectSchema
#[test]
fn project_function_returns_default_active_directories_top_level_project_schema() {
    let mut t = set_up();
    t.scratch.file(
        "test/PROJECT.scl",
        &[
            "project = {",
            "  \"active_directories\": { \"default\": [\"a\", \"b/c\"] }",
            "}",
        ],
    );
    t.scratch.file("test/BUILD", &[]);
    let (key, result) = evaluate(&mut t);
    let dirs: Vec<String> = value(&result, &key)
        .get_default_project_directories()
        .into_iter()
        .collect();
    assert_eq!(dirs, vec!["a".to_string(), "b/c".to_string()]);
}

// Java: ProjectFunctionTest.projectFunction_nonEmptyActiveDirectoriesMustHaveADefault
#[test]
fn project_function_non_empty_active_directories_must_have_a_default() {
    let mut t = set_up();
    t.scratch.file(
        "test/PROJECT.scl",
        &[
            "project = {",
            "  \"active_directories\": { 'foo': ['a', 'b/c'] },",
            "}",
        ],
    );
    t.scratch.file("test/BUILD", &[]);
    let (_, result) = evaluate(&mut t);
    assert!(
        error_message(&result)
            .contains("non-empty active_directories must contain the 'default' key")
    );
}

/// Truth `matches` for the patterns of these tests: `<prefix>.+<suffix>` over the whole message.
fn assert_matches(message: &str, pattern: &str) {
    let (prefix, suffix) = pattern.split_once(".+").expect("pattern with .+");
    assert!(
        message.len() > prefix.len() + suffix.len()
            && message.starts_with(prefix)
            && message.ends_with(suffix),
        "{message:?} does not match {pattern:?}"
    );
}

// Java: ProjectFunctionTest.projectFunction_incorrectType
#[test]
fn project_function_incorrect_type() {
    let mut t = set_up();
    t.scratch.file(
        "test/PROJECT.scl",
        &["project = {", "  \"active_directories\": 42,", "}"],
    );
    t.scratch.file("test/BUILD", &[]);
    let (_, result) = evaluate(&mut t);
    assert_matches(
        &error_message(&result),
        "expected a map of string to list of strings, got .+Int32",
    );
}

// Java: ProjectFunctionTest.projectFunction_incorrectType_inList
#[test]
fn project_function_incorrect_type_in_list() {
    let mut t = set_up();
    t.scratch.file(
        "test/PROJECT.scl",
        &[
            "project = {",
            "  \"active_directories\": { 'default': [42] },",
            "}",
        ],
    );
    t.scratch.file("test/BUILD", &[]);
    let (_, result) = evaluate(&mut t);
    assert_matches(
        &error_message(&result),
        "expected a list of strings, got element of .+Int32",
    );
}

// Java: ProjectFunctionTest.projectFunction_incorrectProjectType
#[test]
fn project_function_incorrect_project_type() {
    let mut t = set_up();
    t.scratch.file("test/PROJECT.scl", &["project = 1"]);
    t.scratch.file("test/BUILD", &[]);
    let (_, result) = evaluate(&mut t);
    assert_matches(
        &error_message(&result),
        "project variable: expected a map of string to objects, got .+Int32",
    );
}

// Java: ProjectFunctionTest.projectFunction_incorrectProjectKeyType
#[test]
fn project_function_incorrect_project_key_type() {
    let mut t = set_up();
    t.scratch.file("test/PROJECT.scl", &["project = {1: [] }"]);
    t.scratch.file("test/BUILD", &[]);
    let (_, result) = evaluate(&mut t);
    assert_matches(
        &error_message(&result),
        "project variable: expected string key, got element of .+Int32",
    );
}

// Java: ProjectFunctionTest.projectFunction_buildableUnitsFormat
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn project_function_buildable_units_format() {}

// Java: ProjectFunctionTest.duplicateBuildableUnitNames
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn duplicate_buildable_unit_names() {}

// Java: ProjectFunctionTest.buildableUnitSchemaDefaults
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn buildable_unit_schema_defaults() {}

// Java: ProjectFunctionTest.projectNameTypeError_notCurrentlyParsedSoNotYetAnError
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn project_name_type_error_not_currently_parsed_so_not_yet_an_error() {}

// Java: ProjectFunctionTest.buildableUnitFieldTypeError
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn buildable_unit_field_type_error() {}

// Java: ProjectFunctionTest.builableUnitEntryTypeError
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn builable_unit_entry_type_error() {}

// Java: ProjectFunctionTest.buildableUnitNameTypeError
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn buildable_unit_name_type_error() {}

// Java: ProjectFunctionTest.buildableUnitTargetPatternsFieldTypeError
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn buildable_unit_target_patterns_field_type_error() {}

// Java: ProjectFunctionTest.buildableUnitTargetPatternsEntryTypeError
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn buildable_unit_target_patterns_entry_type_error() {}

// Java: ProjectFunctionTest.buildableUnitDescriptionTypeError
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn buildable_unit_description_type_error() {}

// Java: ProjectFunctionTest.buildableUnitFlagsFieldTypeError
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn buildable_unit_flags_field_type_error() {}

// Java: ProjectFunctionTest.buildableUnitFlagsEntryTypeError
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn buildable_unit_flags_entry_type_error() {}

// Java: ProjectFunctionTest.buildableUnitIsDefaultTypeError
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn buildable_unit_is_default_type_error() {}

// Java: ProjectFunctionTest.unknownProjectFieldError
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn unknown_project_field_error() {}

// Java: ProjectFunctionTest.unknownBuildableUntFieldError
#[test]
#[ignore = "needs writeProjectSclDefinition (the project_proto.scl protolark module)"]
fn unknown_buildable_unt_field_error() {}

// Java: ProjectFunctionTest.projectFunction_catchSyntaxError
// PORT: Java's test harness turns the reported syntax error into an AssertionError (fail-fast
// event handler); the same happens through `check_fail_fast`.
#[test]
#[should_panic(expected = "syntax error at 'newline': expected expression")]
fn project_function_catch_syntax_error() {
    let mut t = set_up();
    t.scratch
        .file("test/PROJECT.scl", &["something_is_wrong ="]);
    t.scratch.file("test/BUILD", &[]);
    let _ = evaluate(&mut t);
}
