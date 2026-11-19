//! Port of `src/test/java/com/google/devtools/build/lib/skyframe/config/ParsedFlagsValueTest.java`.
//!
//! PORT: the Java test registers a test-only `DummyTestOptions` fragment. Blaze's fragment
//! options classes are a fixed static registry, so the tests use options of `CoreOptions` with the
//! same shape: `--platform_suffix` (string) for `--str_option`, `--stamp` (boolean) for
//! `--bool_option`, `--features` (allowMultiple) for `--accumulating_option`, and `CppOptions`
//! (`--copt`) for `BuildOptionsTest.SecondDummyTestOptions`. The two tests that need options
//! Bazel's fragments do not have (a comma-separated list option that does not accumulate, an
//! option with `implicitRequirements`) are ignored.

use std::collections::BTreeMap;

use blaze_build_config::build_options::{BuildOptions, ScopeType, StarlarkOptionValue};
use blaze_build_config::fragment_options_class::{CORE_OPTIONS, CPP_OPTIONS, FragmentOptionsClass};
use blaze_build_config::parsed_flags::{NativeAndStarlarkFlags, ParsedFlagsValue};
use blaze_cmdline::Label;
use blaze_optdefs::fragment::core_options::CoreOptions;

fn build_config_options() -> Vec<FragmentOptionsClass> {
    vec![&CORE_OPTIONS]
}

fn str_value(s: &str) -> StarlarkOptionValue {
    StarlarkOptionValue::from_option_value(&blaze_options::Value::new(s.to_string()))
}

fn map(entries: &[(&str, &str)]) -> BTreeMap<String, StarlarkOptionValue> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), str_value(v)))
        .collect()
}

fn flags(
    native_flags: &[&str],
    starlark_flags: &[(&str, &str)],
    starlark_flag_defaults: &[(&str, &str)],
) -> NativeAndStarlarkFlags {
    NativeAndStarlarkFlags {
        native_flags: native_flags.iter().map(|s| s.to_string()).collect(),
        starlark_flags: map(starlark_flags),
        starlark_flag_defaults: map(starlark_flag_defaults),
        options_classes: build_config_options(),
        ..NativeAndStarlarkFlags::default()
    }
}

fn label(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

fn starlark(options: &BuildOptions, flag: &str) -> Option<String> {
    options
        .get_starlark_options()
        .get(&label(flag))
        .map(|v| v.java_to_string())
}

#[test]
fn parse() {
    let flags = flags(
        &["--platform_suffix=bar", "--nostamp"],
        &[("//custom:flag", "hello")],
        &[("//custom:flag", "default")],
    );

    let parsed_flags = ParsedFlagsValue::parse_and_create(flags).unwrap();

    let result = parsed_flags.parsing_result();
    let core = result.get_options::<CoreOptions>().unwrap();
    assert_eq!(core.platform_suffix.as_deref(), Some("bar"));
    assert!(!core.stamp_binaries);
    assert_eq!(
        result
            .get_starlark_options()
            .get("//custom:flag")
            .map(|v| StarlarkOptionValue::from_option_value(v).java_to_string()),
        Some("hello".to_string())
    );
}

#[test]
fn merge_with() {
    let original = BuildOptions::of_args(
        &build_config_options(),
        &["--platform_suffix=foo", "--stamp"],
    )
    .unwrap();

    let flags = flags(
        &["--platform_suffix=bar", "--nostamp"],
        &[("//custom:flag", "hello")],
        &[("//custom:flag", "default")],
    );
    let parsed_flags = ParsedFlagsValue::parse_and_create(flags).unwrap();

    let modified = parsed_flags.merge_with(&original).get_options().clone();

    // Ensure the original wasn't modified.
    assert_ne!(original.get::<CoreOptions>(), modified.get::<CoreOptions>());

    // Check the modified values.
    let core = modified.get::<CoreOptions>().unwrap();
    assert_eq!(core.platform_suffix.as_deref(), Some("bar"));
    assert!(!core.stamp_binaries);
    assert_eq!(
        starlark(&modified, "//custom:flag").as_deref(),
        Some("hello")
    );
}

#[test]
fn merge_with_unknown_native_fragment() {
    // Only use the basic flags.
    let original = BuildOptions::of_args(&build_config_options(), &[]).unwrap();

    // Add another fragment with different flags.
    let mut flags = flags(&["--copt=bar"], &[], &[]);
    flags.options_classes = vec![&CORE_OPTIONS, &CPP_OPTIONS];
    let parsed_flags = ParsedFlagsValue::parse_and_create(flags).unwrap();

    // The native flags that are unknown to the original options should not be present.
    let modified = parsed_flags.merge_with(&original).get_options().clone();
    assert!(!modified.contains(&CPP_OPTIONS));
}

#[test]
#[should_panic]
fn merge_with_illegal_starlark_label() {
    let original = BuildOptions::of_args(&build_config_options(), &[]).unwrap();

    let flags = flags(&[], &[("@@@", "hello")], &[]);
    let parsed_flags_value = ParsedFlagsValue::parse_and_create(flags).unwrap();

    // BuildOptions, unlike OptionsParser, uses a Label for the key, so this is the only code path
    // that validates that a starlark flag is actually a Label.
    let _ = parsed_flags_value.merge_with(&original);
}

#[test]
#[ignore = "needs a test-only fragment with a non-accumulating comma-separated list option (DummyTestOptions.list_option)"]
fn merge_with_multi_value_option_non_accumulating() {}

#[test]
#[ignore = "needs a test-only fragment with an implicitRequirements option (DummyTestOptions.dummy_option)"]
fn merge_with_implicit_option() {}

#[test]
fn merge_with_accumulating() {
    let original = BuildOptions::of_args(&build_config_options(), &[]).unwrap();

    let flags = flags(&["--features=foo", "--features=bar"], &[], &[]);
    let parsed_flags = ParsedFlagsValue::parse_and_create(flags).unwrap();

    let modified = parsed_flags.merge_with(&original).get_options().clone();

    assert_eq!(
        modified.get::<CoreOptions>().unwrap().default_features,
        vec!["foo".to_string(), "bar".to_string()]
    );
}

fn original_with(entries: &[(&str, &str, Option<ScopeType>)]) -> BuildOptions {
    let mut builder = BuildOptions::of_args(&build_config_options(), &[])
        .unwrap()
        .to_builder();
    for (flag, value, scope) in entries {
        builder.add_starlark_option(label(flag), str_value(value));
        if let Some(scope) = scope {
            builder.add_scope_type(label(flag), *scope);
        }
    }
    builder.build()
}

#[test]
fn merge_with_starlark() {
    let original = original_with(&[("//custom:flag", "direct", None)]);

    let flags = flags(
        &[],
        &[("//custom:flag", "override")],
        &[("//custom:flag", "default")],
    );
    let parsed_flags = ParsedFlagsValue::parse_and_create(flags).unwrap();

    let modified = parsed_flags.merge_with(&original).get_options().clone();

    // Check the modified values.
    assert_eq!(
        starlark(&modified, "//custom:flag").as_deref(),
        Some("override")
    );
}

#[test]
fn merge_with_starlark_reset_to_default() {
    let original = original_with(&[("//custom:flag", "direct", None)]);

    let flags = flags(
        &[],
        &[("//custom:flag", "default")],
        &[("//custom:flag", "default")],
    );
    let parsed_flags = ParsedFlagsValue::parse_and_create(flags).unwrap();

    let modified = parsed_flags.merge_with(&original).get_options().clone();

    // The Starlark flag should not be present since it was reset to the default value
    assert_eq!(starlark(&modified, "//custom:flag"), None);
}

#[test]
fn merge_with_starlark_reset_to_default_removes_scope() {
    // Regression test: when a starlark flag is reset to its default value, the scope entry
    // should also be removed.
    let flag_label = label("//custom:flag");
    let original = original_with(&[("//custom:flag", "non_default", Some(ScopeType::DEFAULT))]);

    let mut flags = flags(
        &[],
        &[("//custom:flag", "default")],
        &[("//custom:flag", "default")],
    );
    flags
        .scopes_attributes
        .insert("//custom:flag".to_string(), "project".to_string());
    let parsed_flags = ParsedFlagsValue::parse_and_create(flags).unwrap();

    let modified = parsed_flags.merge_with(&original).get_options().clone();

    // The flag should be removed (at default).
    assert!(!modified.get_starlark_options().contains_key(&flag_label));
    // The scope should also be removed - not leaked from the source.
    assert!(!modified.get_scope_type_map().contains_key(&flag_label));
}

#[test]
fn merge_with_scope_preserved_for_retained_flag() {
    // Verifies that scopes already present in the source are preserved through mergeWith when
    // the flag is not removed.
    let flag_label = label("//custom:flag");
    let original = original_with(&[("//custom:flag", "original_value", Some(ScopeType::TARGET))]);

    // mergeWith with no starlark flags - original flags should be untouched.
    let flags = flags(&[], &[], &[]);
    let parsed_flags = ParsedFlagsValue::parse_and_create(flags).unwrap();

    let modified = parsed_flags.merge_with(&original).get_options().clone();

    assert_eq!(
        starlark(&modified, "//custom:flag").as_deref(),
        Some("original_value")
    );
    assert_eq!(
        modified.get_scope_type_map().get(&flag_label),
        Some(&ScopeType::TARGET)
    );
}

#[test]
fn merge_with_scope_preserved_for_new_flag() {
    // When mergeWith introduces a new starlark flag that already has a scope in the source, the
    // scope should be preserved in the result.
    let original = original_with(&[
        ("//custom:existing", "val", Some(ScopeType::TARGET)),
        (
            "//custom:new_flag",
            "source_val",
            Some(ScopeType::UNIVERSAL),
        ),
    ]);

    let flags = flags(
        &[],
        &[("//custom:new_flag", "new_val")],
        &[("//custom:new_flag", "default")],
    );
    let parsed_flags = ParsedFlagsValue::parse_and_create(flags).unwrap();

    let modified = parsed_flags.merge_with(&original).get_options().clone();

    assert_eq!(
        starlark(&modified, "//custom:new_flag").as_deref(),
        Some("new_val")
    );
    // Both flags' scopes should be preserved.
    assert_eq!(
        modified
            .get_scope_type_map()
            .get(&label("//custom:new_flag")),
        Some(&ScopeType::UNIVERSAL)
    );
    assert_eq!(
        modified
            .get_scope_type_map()
            .get(&label("//custom:existing")),
        Some(&ScopeType::TARGET)
    );
}

#[test]
fn merge_with_new_flag_not_in_source() {
    // A flag not present in the source config is added by the merged flags, together with the
    // scope from its parsed scope attribute.
    let original = original_with(&[("//custom:existing", "val", Some(ScopeType::TARGET))]);

    let mut flags = flags(
        &[],
        &[("//custom:new_flag", "new_val")],
        &[("//custom:new_flag", "default")],
    );
    flags
        .scopes_attributes
        .insert("//custom:new_flag".to_string(), "project".to_string());
    let parsed_flags = ParsedFlagsValue::parse_and_create(flags).unwrap();

    let modified = parsed_flags.merge_with(&original).get_options().clone();

    // New flag should be present with the merged value and the scope from its parsed scope
    // attribute.
    assert_eq!(
        starlark(&modified, "//custom:new_flag").as_deref(),
        Some("new_val")
    );
    assert_eq!(
        modified
            .get_scope_type_map()
            .get(&label("//custom:new_flag")),
        Some(&ScopeType::PROJECT)
    );
    // Existing flag and its scope should be unaffected.
    assert_eq!(
        starlark(&modified, "//custom:existing").as_deref(),
        Some("val")
    );
    assert_eq!(
        modified
            .get_scope_type_map()
            .get(&label("//custom:existing")),
        Some(&ScopeType::TARGET)
    );
}
