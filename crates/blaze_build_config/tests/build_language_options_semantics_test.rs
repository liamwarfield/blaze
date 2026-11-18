//! `BuildLanguageOptions.toStarlarkSemantics` (analysis review S1-2): every option reaches its
//! `StarlarkSemantics` key. Expected values come from the Java at the pin: the key each option is
//! set on (`packages/semantics/BuildLanguageOptions.java:890 BuildLanguageOptions.setFlags`) and
//! the key defaults (`BuildLanguageOptions.java:1078` onwards), which equal the option defaults.
//! Java's own test (`BuildLanguageOptionsTest.optionsAreNotMissingFromSemantics` /
//! `canGetBuilderFromInstance`) compares randomly generated option sets through Java reflection.

#![allow(clippy::field_reassign_with_default)]

use blaze_build_config::top_level::{starlark_semantics_from, to_starlark_semantics};
use blaze_optdefs::command::build_language_options::BuildLanguageOptions;
use blaze_options::{Options, OptionsClass, OptionsParser};
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::semantics::Utf8EnforcementMode;

fn semantics_of(args: &[&str]) -> StarlarkSemantics {
    let mut parser = OptionsParser::builder()
        .options_classes(&[BuildLanguageOptions::class()])
        .build()
        .unwrap();
    parser
        .parse(args)
        .unwrap_or_else(|e| panic!("{}", e.get_message()));
    starlark_semantics_from(&parser)
}

fn mapped_semantics_of(args: &[&str]) -> StarlarkSemantics {
    let mut parser = OptionsParser::builder()
        .options_classes(&[BuildLanguageOptions::class()])
        .build()
        .unwrap();
    parser
        .parse(args)
        .unwrap_or_else(|e| panic!("{}", e.get_message()));
    to_starlark_semantics(&parser.get_options::<BuildLanguageOptions>().unwrap())
}

// Every key's default is its option's default, so the default options give the default
// semantics.
#[test]
fn default_options_give_default_semantics() {
    assert_eq!(
        to_starlark_semantics(&Options::<BuildLanguageOptions>::get_defaults()),
        StarlarkSemantics::default()
    );
    assert_eq!(semantics_of(&[]), StarlarkSemantics::default());
    // A parser without BuildLanguageOptions.
    let parser = OptionsParser::builder().build().unwrap();
    assert_eq!(
        starlark_semantics_from(&parser),
        StarlarkSemantics::default()
    );
}

// The scenario of the review finding: flags Blaze used to ignore.
#[test]
fn non_default_flags_reach_the_semantics() {
    let semantics = semantics_of(&[
        "--incompatible_autoload_externally=+foo,-bar",
        "--nocheck_bzl_visibility",
        "--experimental_platforms_api",
        "--incompatible_no_implicit_file_export",
        "--noincompatible_disallow_empty_glob",
        "--max_computation_steps=5",
        "--nested_set_depth_limit=7",
        "--incompatible_enforce_starlark_utf8=error",
        "--experimental_builtins_bzl_path=%workspace%",
        "--experimental_sibling_repository_layout",
        "--incompatible_disable_transitions_on=cpu,copt",
    ]);
    let mut expected = StarlarkSemantics::default();
    expected.incompatible_autoload_externally = vec!["+foo".to_string(), "-bar".to_string()];
    expected.check_bzl_visibility = false;
    expected.experimental_platforms_api = true;
    expected.incompatible_no_implicit_file_export = true;
    expected.incompatible_disallow_empty_glob = false;
    expected.max_computation_steps = 5;
    expected.nested_set_depth_limit = 7;
    expected.incompatible_enforce_starlark_utf8 = Utf8EnforcementMode::Error;
    expected.experimental_builtins_bzl_path = "%workspace%".to_string();
    expected.experimental_sibling_repository_layout = true;
    // Converters.CommaSeparatedOptionSetConverter: distinct and sorted.
    expected.incompatible_disable_transitions_on = vec!["copt".to_string(), "cpu".to_string()];
    assert_eq!(semantics, expected);
}

// Options whose Java field or semantics key has another name than the option
// (BuildLanguageOptions.setFlags): --incompatible_allow_tags_propagation (field
// experimentalAllowTagsPropagation), --internal_starlark_flag_test_canary sets
// StarlarkSemantics.PRINT_TEST_MARKER, --incompatible_enable_deprecated_label_apis (field
// enableDeprecatedLabelApis), --experimental_repository_ctx_execute_wasm (field
// repositoryCtxExecuteWasm), --internal_starlark_utf_8_byte_strings sets
// StarlarkSemantics.INTERNAL_BAZEL_ONLY_UTF_8_BYTE_STRINGS,
// --incompatible_use_cc_configure_from_rules_cc sets "incompatible_use_cc_configure_from_rules",
// --experimental_starlark_types_allowed_paths sets EXPERIMENTAL_STARLARK_TYPES_ALLOWED_PATHS (key
// "experimental_starlark_types").
#[test]
fn options_with_differently_named_keys() {
    // The mapping itself (`to_starlark_semantics`): two of these keys are guarded by
    // `starlark_semantics_from` (R2-2, see below).
    let semantics = mapped_semantics_of(&[
        "--noincompatible_allow_tags_propagation",
        "--internal_starlark_flag_test_canary",
        "--noincompatible_enable_deprecated_label_apis",
        "--experimental_repository_ctx_execute_wasm",
        "--nointernal_starlark_utf_8_byte_strings",
        "--incompatible_use_cc_configure_from_rules_cc",
        "--experimental_starlark_types_allowed_paths=//a,//b",
    ]);
    let mut expected = StarlarkSemantics::default();
    expected.incompatible_allow_tags_propagation = false;
    expected.internal_starlark_flag_test_canary = true;
    expected.incompatible_enable_deprecated_label_apis = false;
    expected.experimental_repository_ctx_execute_wasm = true;
    expected.internal_starlark_utf_8_byte_strings = false;
    expected.incompatible_use_cc_configure_from_rules_cc = true;
    expected.experimental_starlark_types_allowed_paths = vec!["//a".to_string(), "//b".to_string()];
    assert_eq!(semantics, expected);
}

// --experimental_starlark_types and --experimental_cc_skylark_api_enabled_packages have no
// StarlarkSemantics key in Java (setFlags does not read them).
#[test]
fn options_without_a_semantics_key() {
    assert_eq!(
        semantics_of(&[
            "--experimental_starlark_types",
            "--experimental_cc_skylark_api_enabled_packages=a,b"
        ]),
        StarlarkSemantics::default()
    );
}

// Analysis review R2-2: keys whose Java readers are not ported fail loudly when set to a
// non-default value instead of being accepted and ignored.
#[test]
#[should_panic(expected = "--incompatible_check_external_repo_source_dir_package_boundary=true")]
fn unread_key_check_external_repo_source_dir_package_boundary_is_loud() {
    semantics_of(&["--incompatible_check_external_repo_source_dir_package_boundary"]);
}

#[test]
#[should_panic(expected = "--incompatible_use_cc_configure_from_rules_cc=true")]
fn unread_key_use_cc_configure_from_rules_cc_is_loud() {
    semantics_of(&["--incompatible_use_cc_configure_from_rules_cc"]);
}

#[test]
#[should_panic(expected = "--internal_starlark_utf_8_byte_strings=false")]
fn unread_key_utf_8_byte_strings_is_loud() {
    semantics_of(&["--nointernal_starlark_utf_8_byte_strings"]);
}

// Keys that Bazel 9.3.2 itself never reads stay accepted (no Java reader at the pin).
#[test]
fn keys_without_java_reader_are_accepted() {
    let semantics = semantics_of(&[
        "--experimental_enable_android_migration_apis",
        "--experimental_disable_external_package",
        "--noincompatible_do_not_split_linking_cmdline",
    ]);
    assert!(semantics.experimental_enable_android_migration_apis);
    assert!(semantics.experimental_disable_external_package);
    assert!(!semantics.incompatible_do_not_split_linking_cmdline);
}
