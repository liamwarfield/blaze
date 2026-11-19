//! Port of `src/test/java/com/google/devtools/build/lib/skyframe/ProjectFunctionTest.java`.
//!
//! `writeProjectSclDefinition` (BuildViewTestCase) copies `project_proto.scl` from the runfiles;
//! here it is read from the pinned source tree (`src/main/protobuf/project/project_proto.scl`).

use blaze_cmdline::Label;
use blaze_sky_loading::path_fragment_prefix_trie::PathFragmentPrefixTrie;
use blaze_sky_loading::project_function::{
    BuildableUnit, EnforcementPolicy, ProjectKey, ProjectValue,
};
use blaze_skyframe::{EvaluationResult, SkyKey};
use blaze_vfs::PathFragment;

use crate::testing::BuildViewTestCase;

fn set_up() -> BuildViewTestCase {
    let mut t = BuildViewTestCase::new();
    t.set_build_language_options(&["--experimental_enable_scl_dialect=true"]);
    write_project_scl_definition(&mut t, "test/project_proto.scl");
    t
}

/// `BuildViewTestCase.writeProjectSclDefinition(dest)`.
// PORT: Java reads `TestConstants.PROJECT_SCL_DEFINITION_PATH` from the runfiles; the file is
// the pinned source's `src/main/protobuf/project/project_proto.scl`.
fn write_project_scl_definition(t: &mut BuildViewTestCase, dest: &str) {
    let path = std::path::Path::new(&std::env::var_os("HOME").expect("HOME"))
        .join(".cache/blaze/src/bazel-9.3.2/src/main/protobuf/project/project_proto.scl");
    let contents = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e));
    t.scratch.file_bytes(dest, &contents);
}

/// The PROJECT.scl of a protolark-schema test: the load of `project_proto.scl`, then `body`.
fn proto_project(body: &[&str]) -> Vec<String> {
    let mut lines: Vec<String> = [
        "load(",
        "    \"//test:project_proto.scl\",",
        "    \"buildable_unit_pb2\",",
        "    \"project_pb2\",",
        ")",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    lines.extend(body.iter().map(|s| s.to_string()));
    lines
}

fn scratch_project(t: &mut BuildViewTestCase, lines: &[String]) {
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    t.scratch.file("test/PROJECT.scl", &refs);
    t.scratch.file("test/BUILD", &[]);
}

/// `assertParseError`: Truth `matches` with a `.*` pattern.
fn assert_parse_error(t: &mut BuildViewTestCase, body: &[&str], expected_error: &str) {
    scratch_project(t, &proto_project(body));
    let (_, result) = evaluate(t);
    let message = error_message(&result);
    match expected_error.split_once(".*") {
        Some((prefix, suffix)) => assert!(
            message.starts_with(prefix) && message.ends_with(suffix),
            "{message:?} does not match {expected_error:?}"
        ),
        None => assert_eq!(message, expected_error),
    }
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
fn project_function_returns_active_directories() {
    let mut t = set_up();
    t.scratch.file(
        "test/PROJECT.scl",
        &[
            "project = {",
            "  \"active_directories\": {'default': ['foo'], 'a': ['bar', '-bar/baz']},",
            "}",
        ],
    );
    t.scratch.file("test/BUILD", &[]);
    let (key, result) = evaluate(&mut t);
    let value = value(&result, &key);
    let trie = PathFragmentPrefixTrie::transform_values(value.get_project_directories()).unwrap();
    let p = PathFragment::create;
    assert!(trie["default"].includes(&p("foo")));
    assert!(!trie["default"].includes(&p("bar")));
    assert!(trie["a"].includes(&p("bar")));
    assert!(!trie["a"].includes(&p("bar/baz")));
    assert!(trie["a"].includes(&p("bar/qux")));
    assert!(!trie.contains_key("b"));
}

// Java: ProjectFunctionTest.projectFunction_returnsDefaultActiveDirectories
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
    let trie = PathFragmentPrefixTrie::of(&value(&result, &key).get_default_project_directories())
        .unwrap();
    assert!(trie.includes(&PathFragment::create("a")));
    assert!(trie.includes(&PathFragment::create("b/c")));
    assert!(!trie.includes(&PathFragment::create("d")));
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
    let trie = PathFragmentPrefixTrie::of(&value(&result, &key).get_default_project_directories())
        .unwrap();
    assert!(trie.includes(&PathFragment::create("a")));
    assert!(trie.includes(&PathFragment::create("b/c")));
    assert!(!trie.includes(&PathFragment::create("d")));
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
fn project_function_buildable_units_format() {
    let mut t = set_up();
    scratch_project(
        &mut t,
        &proto_project(&[
            "project = project_pb2.Project.create(",
            "  name = \"test\",",
            "  enforcement_policy = \"warn\",",
            "  project_directories = [ \"//test/...\"],",
            "  always_allowed_configs = [\"--config=foo\"],",
            "  buildable_units = [",
            "      buildable_unit_pb2.BuildableUnit.create(",
            "          name = \"default\",",
            "          target_patterns = [\"//test/...\"],",
            "          description = \"default\",",
            "          flags = [\"--define=foo=bar\"],",
            "          is_default = True,",
            "      ),",
            "      buildable_unit_pb2.BuildableUnit.create(",
            "          name = \"non_default\",",
            "          target_patterns = [\"//test/...\"],",
            "          description = \"non default\",",
            "          flags = [\"--define=bar=baz\"],",
            "          is_default = False,",
            "      ),",
            "  ],",
            ")",
        ]),
    );
    let (key, result) = evaluate(&mut t);
    let value = value(&result, &key);
    assert_eq!(value.get_enforcement_policy(), EnforcementPolicy::WARN);
    assert_eq!(
        value.get_always_allowed_configs(),
        Some(&["--config=foo".to_string()][..])
    );
    assert_eq!(
        value.get_actual_project_file(),
        Some(&Label::parse_canonical("//test:PROJECT.scl").unwrap())
    );
    let units = value.get_buildable_units().unwrap();
    assert!(units["default"].is_default());
    assert!(!units["non_default"].is_default());
    assert_eq!(value.get_project_directories().len(), 1);
    assert_eq!(
        value.get_project_directories()["default"],
        vec!["//test/...".to_string()]
    );
    assert_eq!(
        units["default"],
        BuildableUnit::create(
            "default".to_string(),
            vec!["//test/...".to_string()],
            "default".to_string(),
            vec!["--define=foo=bar".to_string()],
            true,
        )
        .unwrap()
    );
    assert_eq!(
        units["non_default"],
        BuildableUnit::create(
            "non_default".to_string(),
            vec!["//test/...".to_string()],
            "non default".to_string(),
            vec!["--define=bar=baz".to_string()],
            false,
        )
        .unwrap()
    );
}

// Java: ProjectFunctionTest.duplicateBuildableUnitNames
#[test]
fn duplicate_buildable_unit_names() {
    let mut t = set_up();
    scratch_project(
        &mut t,
        &proto_project(&[
            "project = project_pb2.Project.create(",
            "  name = \"test\",",
            "  buildable_units = [",
            "      buildable_unit_pb2.BuildableUnit.create(name = \"foo\"),",
            "      buildable_unit_pb2.BuildableUnit.create(name = \"foo\"),",
            "  ],",
            ")",
        ]),
    );
    let (_, result) = evaluate(&mut t);
    assert_eq!(
        error_message(&result),
        "buildable_unit name='foo' is repeated. Buildable units must have unique names."
    );
}

// Java: ProjectFunctionTest.buildableUnitSchemaDefaults
#[test]
fn buildable_unit_schema_defaults() {
    let mut t = set_up();
    scratch_project(
        &mut t,
        &proto_project(&[
            "project = project_pb2.Project.create(",
            "  name = \"test\",",
            "  buildable_units = [",
            "      buildable_unit_pb2.BuildableUnit.create(name = \"foo\"),",
            "  ],",
            ")",
        ]),
    );
    let (key, result) = evaluate(&mut t);
    let value = value(&result, &key);
    // Project-wide defaults:
    assert_eq!(value.get_enforcement_policy(), EnforcementPolicy::WARN);
    assert_eq!(value.get_always_allowed_configs(), None);
    assert_eq!(value.get_project_directories().len(), 1);
    assert!(value.get_project_directories()["default"].is_empty());
    // Buildable unit defaults:
    let foo = &value.get_buildable_units().unwrap()["foo"];
    assert!(foo.target_pattern_matcher().is_empty());
    assert!(foo.flags().is_empty());
    assert_eq!(foo.description(), "foo");
    assert!(!foo.is_default());
}

// Java: ProjectFunctionTest.projectNameTypeError_notCurrentlyParsedSoNotYetAnError
#[test]
fn project_name_type_error_not_currently_parsed_so_not_yet_an_error() {
    // TODO: b/415068036 - update when/if we start reading project(name = "foo") to catch type
    // errors.
    let mut t = set_up();
    scratch_project(
        &mut t,
        &proto_project(&[
            "project = project_pb2.Project.create(",
            "  name = 123,",
            "  buildable_units = [],",
            ")",
        ]),
    );
    let (key, result) = evaluate(&mut t);
    let _ = value(&result, &key);
}

fn unit(fields: &[&str]) -> Vec<String> {
    let mut lines = vec![
        "project = project_pb2.Project.create(".to_string(),
        "  name = \"test\",".to_string(),
        "  buildable_units = [".to_string(),
        "      buildable_unit_pb2.BuildableUnit.create(".to_string(),
    ];
    lines.extend(fields.iter().map(|f| format!("          {},", f)));
    lines.extend(["      ),".to_string(), "  ],".to_string(), ")".to_string()]);
    lines
}

fn assert_unit_error(fields: &[&str], expected_error: &str) {
    let mut t = set_up();
    let body = unit(fields);
    let refs: Vec<&str> = body.iter().map(String::as_str).collect();
    assert_parse_error(&mut t, &refs, expected_error);
}

// Java: ProjectFunctionTest.buildableUnitFieldTypeError
#[test]
fn buildable_unit_field_type_error() {
    let mut t = set_up();
    assert_parse_error(
        &mut t,
        &[
            "project = project_pb2.Project.create(",
            "  name = \"test\",",
            "  buildable_units = \"bad value\",",
            ")",
        ],
        "buildable_units must be a list of buildable unit definitions, got .*String",
    );
}

// Java: ProjectFunctionTest.builableUnitEntryTypeError
#[test]
fn builable_unit_entry_type_error() {
    let mut t = set_up();
    assert_parse_error(
        &mut t,
        &[
            "project = project_pb2.Project.create(",
            "  name = \"test\",",
            "  buildable_units = [",
            "      \"bad value\",",
            "  ],",
            ")",
        ],
        "buildable_units entries must be structured objects, got .*String",
    );
}

// Java: ProjectFunctionTest.buildableUnitNameTypeError
#[test]
fn buildable_unit_name_type_error() {
    assert_unit_error(
        &[
            "name = 2",
            "target_patterns = [\"//test/...\"]",
            "description = \"default\"",
            "flags = [\"--define=foo=bar\"]",
            "is_default = True",
        ],
        "buildable_unit names must be strings, got .*Int32",
    );
}

// Java: ProjectFunctionTest.buildableUnitTargetPatternsFieldTypeError
#[test]
fn buildable_unit_target_patterns_field_type_error() {
    assert_unit_error(
        &[
            "name = \"default\"",
            "target_patterns = 1",
            "description = \"default\"",
            "flags = [\"--define=foo=bar\"]",
            "is_default = True",
        ],
        "target_patterns must be a list of strings, got .*Int32",
    );
}

// Java: ProjectFunctionTest.buildableUnitTargetPatternsEntryTypeError
#[test]
fn buildable_unit_target_patterns_entry_type_error() {
    assert_unit_error(
        &[
            "name = \"default\"",
            "target_patterns = [2, 3]",
            "description = \"default\"",
            "flags = [\"--define=foo=bar\"]",
            "is_default = True",
        ],
        "target_patterns entries must be strings, got .*Int32",
    );
}

// Java: ProjectFunctionTest.buildableUnitDescriptionTypeError
#[test]
fn buildable_unit_description_type_error() {
    assert_unit_error(
        &[
            "name = \"default\"",
            "target_patterns = [\"//test/...\"]",
            "description = 123",
            "flags = [\"--define=foo=bar\"]",
            "is_default = True",
        ],
        "buildable_unit descriptions must be strings, got .*Int32",
    );
}

// Java: ProjectFunctionTest.buildableUnitFlagsFieldTypeError
#[test]
fn buildable_unit_flags_field_type_error() {
    assert_unit_error(
        &[
            "name = \"default\"",
            "target_patterns = [\"//test/...\"]",
            "description = \"default\"",
            "flags = \"bad value\"",
            "is_default = True",
        ],
        "flags must be a list of strings, got .*String",
    );
}

// Java: ProjectFunctionTest.buildableUnitFlagsEntryTypeError
#[test]
fn buildable_unit_flags_entry_type_error() {
    assert_unit_error(
        &[
            "name = \"default\"",
            "target_patterns = [\"//test/...\"]",
            "description = \"default\"",
            "flags = [123]",
            "is_default = True",
        ],
        "flags entries must be strings, got .*Int32",
    );
}

// Java: ProjectFunctionTest.buildableUnitIsDefaultTypeError
#[test]
fn buildable_unit_is_default_type_error() {
    assert_unit_error(
        &[
            "name = \"default\"",
            "target_patterns = [\"//test/...\"]",
            "description = \"default\"",
            "flags = [\"--define=foo=bar\"]",
            "is_default = \"not valid\"",
        ],
        "is_default must be a boolean, got .*String",
    );
}

// Java: ProjectFunctionTest.unknownProjectFieldError
#[test]
fn unknown_project_field_error() {
    let mut t = set_up();
    t.remove_fail_fast_handler(); // expect errors
    assert_parse_error(
        &mut t,
        &[
            "project = project_pb2.Project.create(",
            "  name = \"test\",",
            "  invalid_field = \"not valid\",",
            "  buildable_units = [",
            "      buildable_unit_pb2.BuildableUnit.create(",
            "          name = \"default\",",
            "      ),",
            "  ],",
            ")",
        ],
        // The direct exception doesn't explain the cause but actual builds fail with more
        // context: "Error: project_project_Project() got unexpected keyword argument:
        // invalid_field"
        "initialization of module 'test/PROJECT.scl' failed",
    );
}

// Java: ProjectFunctionTest.unknownBuildableUntFieldError
#[test]
fn unknown_buildable_unt_field_error() {
    let mut t = set_up();
    t.remove_fail_fast_handler(); // expect errors
    assert_parse_error(
        &mut t,
        &unit(&["name = \"default\"", "invalid_field = \"not valid\""])
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        // The direct exception doesn't explain the cause but actual builds fail with more
        // context: "Error: project_project_Project() got unexpected keyword argument:
        // invalid_field"
        "initialization of module 'test/PROJECT.scl' failed",
    );
}

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
