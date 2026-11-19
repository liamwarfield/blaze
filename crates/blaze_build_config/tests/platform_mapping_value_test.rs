//! Port of `src/test/java/com/google/devtools/build/lib/skyframe/config/PlatformMappingValueTest.java`.
//!
//! PORT: the Java test registers a test-only `DummyTestOptions` fragment with two string options
//! (`--str_option`, `--other_str_option`). Blaze's fragment options classes are a fixed static
//! registry, so the tests use `CoreOptions` instead: `--platform_suffix` (a string option) for
//! `--str_option` and `--compilation_mode` for `--other_str_option` (the Java test sets it to
//! "dbg"). The mapping logic under test does not depend on which fragment the options are in.

use std::sync::Arc;

use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::fragment_options_class::{
    CORE_OPTIONS, FragmentOptionsClass, PLATFORM_OPTIONS,
};
use blaze_build_config::parsed_flags::{NativeAndStarlarkFlags, ParsedFlagsValue};
use blaze_build_config::skyframe::PlatformMappingValue;
use blaze_cmdline::{Label, RepositoryMapping, RepositoryName};
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::platform_options::PlatformOptions;
use indexmap::IndexMap;

fn platform_one() -> Label {
    Label::parse_canonical_unchecked("//platforms:one")
}

fn platform_two() -> Label {
    Label::parse_canonical_unchecked("@dep+1.0//platforms:two")
}

fn repo_mapping() -> RepositoryMapping {
    RepositoryMapping::create(
        [
            ("", blaze_cmdline::repository_name::MAIN.clone()),
            ("dep", RepositoryName::create_unvalidated("dep+1.0")),
        ],
        blaze_cmdline::repository_name::MAIN.clone(),
    )
}

fn default_target_platform() -> Label {
    Label::parse_canonical_unchecked("@bazel_tools//tools:host_platform")
}

fn build_config_options() -> Vec<FragmentOptionsClass> {
    // PlatformOptions is required for mapping.
    vec![&PLATFORM_OPTIONS, &CORE_OPTIONS]
}

/// Caching of option default values does not consider conversion context (b/365420093). Parse
/// the default `PlatformOptions` up front with no conversion context so that the default value of
/// `PlatformOptions.hostPlatform` is deterministic.
// TODO: b/365420093 - Remove this workaround when the bug is fixed.
// PORT: Java's @BeforeClass computeDefaultPlatformOptions; every helper runs it once first.
fn compute_default_platform_options() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        BuildOptions::of_args(&[&PLATFORM_OPTIONS], &[]).expect("parses");
    });
}

fn create_build_options(args: &[&str]) -> BuildOptions {
    compute_default_platform_options();
    BuildOptions::of_args(&build_config_options(), args).expect("parses")
}

#[derive(Default)]
struct PlatformMappingBuilder {
    platforms_to_flags: IndexMap<Label, Arc<ParsedFlagsValue>>,
    flags_to_platforms: Vec<(Arc<ParsedFlagsValue>, Label)>,
}

impl PlatformMappingBuilder {
    fn add_platform(mut self, platform: Label, native_flags: &[&str]) -> Self {
        self.platforms_to_flags
            .insert(platform, Arc::new(create_flags(native_flags)));
        self
    }

    fn add_flags(mut self, platform: Label, native_flags: &[&str]) -> Self {
        self.flags_to_platforms
            .push((Arc::new(create_flags(native_flags)), platform));
        self
    }

    fn build(self) -> PlatformMappingValue {
        PlatformMappingValue::new(self.platforms_to_flags, self.flags_to_platforms)
    }
}

fn builder() -> PlatformMappingBuilder {
    PlatformMappingBuilder::default()
}

fn create_flags(native_flags: &[&str]) -> ParsedFlagsValue {
    compute_default_platform_options();
    let flags = NativeAndStarlarkFlags {
        native_flags: native_flags.iter().map(|s| s.to_string()).collect(),
        options_classes: build_config_options(),
        repo_mapping: Some(repo_mapping()),
        ..NativeAndStarlarkFlags::default()
    };
    ParsedFlagsValue::parse_and_create(flags).expect("parses")
}

fn platforms(options: &BuildOptions) -> Vec<Label> {
    options
        .get::<PlatformOptions>()
        .unwrap()
        .platforms
        .iter()
        .map(|p| p.0.clone())
        .collect()
}

#[test]
fn map_no_mappings() {
    let mapping_value = builder().build();
    let mapped = mapping_value.map(&create_build_options(&[])).unwrap();
    assert_eq!(
        platforms(mapped.get_options()),
        vec![default_target_platform()]
    );
}

#[test]
fn map_platform_to_flags() {
    let mapping_value = builder()
        .add_platform(
            platform_one(),
            &["--platform_suffix=one", "--compilation_mode=dbg"],
        )
        .build();
    let modified_options = create_build_options(&["--platforms=//platforms:one"]);
    let mapped = mapping_value.map(&modified_options).unwrap();
    assert_eq!(
        mapped
            .get_options()
            .get::<CoreOptions>()
            .unwrap()
            .platform_suffix
            .as_deref(),
        Some("one")
    );
}

#[test]
fn map_flags_to_platform() {
    let mapping_value = builder()
        .add_flags(
            platform_one(),
            &["--platform_suffix=one", "--compilation_mode=dbg"],
        )
        .build();
    let modified_options =
        create_build_options(&["--platform_suffix=one", "--compilation_mode=dbg"]);
    let mapped = mapping_value.map(&modified_options).unwrap();
    assert_eq!(platforms(mapped.get_options()), vec![platform_one()]);
}

#[test]
fn map_flags_to_platform_check_priority() {
    let mapping_value = builder()
        .add_flags(
            platform_one(),
            &["--platform_suffix=one", "--compilation_mode=dbg"],
        )
        .add_flags(platform_two(), &["--platform_suffix=two"])
        .build();
    let modified_options = create_build_options(&["--platform_suffix=two"]);
    let mapped = mapping_value.map(&modified_options).unwrap();
    assert_eq!(platforms(mapped.get_options()), vec![platform_two()]);
}

#[test]
fn map_flags_to_platform_none_matching() {
    let mapping_value = builder()
        .add_flags(
            platform_one(),
            &["--platform_suffix=foo", "--compilation_mode=dbg"],
        )
        .build();
    let modified_options = create_build_options(&["--platform_suffix=bar"]);
    let mapped = mapping_value.map(&modified_options).unwrap();
    assert_eq!(
        platforms(mapped.get_options()),
        vec![default_target_platform()]
    );
}

#[test]
#[should_panic(
    expected = "When using platform mappings, all configurations must contain platform options"
)]
fn map_no_platform_options() {
    let mapping_value = builder().build();
    // Does not contain PlatformOptions.
    let options = BuildOptions::of_args(&[], &[]).unwrap();
    let _ = mapping_value.map(&options);
}

#[test]
fn map_no_mapping_if_platform_is_set_but_not_matching() {
    let mapping_value = builder()
        // Add a mapping for a different platform.
        .add_platform(
            platform_one(),
            &["--platform_suffix=one", "--compilation_mode=dbg"],
        )
        .build();
    let modified_options =
        create_build_options(&["--platform_suffix=one", "--platforms=//platforms:two"]);
    let mapped = mapping_value.map(&modified_options).unwrap();
    // No change because the platform is not in the mapping.
    assert_eq!(&modified_options, mapped.get_options().as_ref());
}

#[test]
fn map_no_mapping_if_platform_is_set_and_no_platform_mapping() {
    let mapping_value = builder()
        // Add a flag mapping that would match.
        .add_flags(platform_one(), &["--platform_suffix=one"])
        .build();
    let modified_options =
        create_build_options(&["--platform_suffix=one", "--platforms=//platforms:two"]);
    let mapped = mapping_value.map(&modified_options).unwrap();
    // No change because the platform is not in the mapping.
    assert_eq!(&modified_options, mapped.get_options().as_ref());
}
