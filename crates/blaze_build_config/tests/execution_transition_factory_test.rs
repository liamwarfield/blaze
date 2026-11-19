//! Port of
//! `src/test/java/com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactoryTest.java`.
//!
//! PORT: the Java class extends `BuildViewTestCase`; its `targetConfig` comes from the analysis
//! mock. Blaze has no `BuildViewTestCase` yet, so the tests that only need `BuildOptions` and the
//! exec transition stand in for it with:
//! - `targetConfig.getOptions().getFragmentClasses()`: every registered fragment options class
//!   ([`all_known_fragment_options_classes`]);
//! - `targetConfig.getOptions()`: the default options of those classes (so
//!   `--experimental_exec_config` is its default, `@_builtins`' `bazel_exec_transition`);
//! - `getSkyframeExecutor().getStarlarkExecTransition(...)`: [`load_starlark_exec_transition`];
//! - `ExecutionTransitionFactory.createFactory().create(...).patch(...)`:
//!   [`exec_transition_patch`] on a view restricted to [`exec_transition_required_fragments`].

use std::sync::Arc;

use blaze_build_config::build_options::{BuildOptions, BuildOptionsView};
use blaze_build_config::fragment_options_class::{
    FragmentOptionsClass, all_known_fragment_options_classes,
};
use blaze_build_config::function_transition_util::{
    exec_transition_patch, exec_transition_required_fragments,
};
use blaze_build_config::options_diff::OptionInfo;
use blaze_build_config::starlark_transition::{
    StarlarkDefinedConfigTransition, load_starlark_exec_transition,
};
use blaze_cmdline::Label;
use blaze_events::StoredEventHandler;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::platform_options::PlatformOptions;
use blaze_options::{OptionMetadataTag, Value};
use blaze_starlark::StarlarkSemantics;

fn execution_platform() -> Label {
    Label::parse_canonical_unchecked("//platform:exec")
}

fn semantics() -> Arc<StarlarkSemantics> {
    Arc::new(StarlarkSemantics::default())
}

/// `targetConfig.getOptions().getFragmentClasses()`.
fn target_fragment_classes() -> Vec<FragmentOptionsClass> {
    all_known_fragment_options_classes()
}

/// `targetConfig.getOptions()`.
fn target_options() -> BuildOptions {
    BuildOptions::get_default_build_options_for_fragments(&target_fragment_classes())
}

/// `BuildOptions.of(fragmentClasses, args...)`.
fn of(classes: &[FragmentOptionsClass], args: &[&str]) -> BuildOptions {
    BuildOptions::of_args(classes, args).unwrap_or_else(|e| panic!("{}", e.get_message()))
}

/// `getSkyframeExecutor().getStarlarkExecTransition(targetConfig.getOptions(), reporter)`.
fn get_starlark_exec_transition() -> Arc<StarlarkDefinedConfigTransition> {
    load_starlark_exec_transition(Some(&target_options()), &semantics())
        .unwrap_or_else(|e| panic!("{}", e))
        .expect("Starlark exec transition")
}

/// `getExecTransition(execPlatform).patch(new BuildOptionsView(options,
/// transition.requiresOptionFragments()), new StoredEventHandler())`.
fn patch_exec_transition(exec_platform: Option<&Label>, options: &BuildOptions) -> BuildOptions {
    let transition = get_starlark_exec_transition();
    let view = BuildOptionsView::new(options, exec_transition_required_fragments());
    exec_transition_patch(
        exec_platform,
        &transition,
        &view,
        &semantics(),
        &StoredEventHandler::new(),
    )
}

/// Java `String.hashCode()`.
fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for c in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(c as i32);
    }
    h
}

#[test]
fn execution_transition() {
    // Apply the transition.
    let options = of(
        &target_fragment_classes(),
        &["--platforms=//platform:target"],
    );

    let result = patch_exec_transition(Some(&execution_platform()), &options);
    assert!(!std::ptr::eq(&result, &options));

    assert!(result.get::<CoreOptions>().unwrap().is_exec);
    let platforms: Vec<Label> = result
        .get::<PlatformOptions>()
        .unwrap()
        .platforms
        .iter()
        .map(|p| p.0.clone())
        .collect();
    assert_eq!(platforms, vec![execution_platform()]);
}

#[test]
fn execution_transition_no_exec_platform() {
    // No execution platform available.
    let options = of(
        &target_fragment_classes(),
        &["--platforms=//platform:target"],
    );

    let result = patch_exec_transition(None, &options);
    assert!(result == options);
}

#[test]
fn execution_transition_conf_dist_legacy() {
    let options = of(
        &target_fragment_classes(),
        &[
            "--platforms=//platform:target",
            "--experimental_exec_configuration_distinguisher=legacy",
        ],
    );

    let result = patch_exec_transition(Some(&execution_platform()), &options);

    let core = result.get::<CoreOptions>().unwrap();
    assert!(core.affected_by_starlark_transition.is_empty());
    // Java `String.format("%X", int)` prints a negative int as its unsigned two's complement.
    let expected = format!(
        "{:X}",
        java_string_hash_code(&execution_platform().get_canonical_form()) as u32
    );
    let platform_suffix = core.platform_suffix.as_deref().expect("platform_suffix");
    assert!(
        platform_suffix.contains(&expected),
        "{} does not contain {}",
        platform_suffix,
        expected
    );
}

#[test]
#[ignore = "needs ExecTransitionFinalizer full_hash scheme (unported in exec_transition_patch) and BuildOptions.hashCode"]
fn execution_transition_conf_dist_full_hash() {}

#[test]
fn execution_transition_conf_dist_diff_to_affected() {
    let options = of(
        &target_fragment_classes(),
        &[
            "--platforms=//platform:target",
            "--experimental_exec_configuration_distinguisher=diff_to_affected",
        ],
    );

    let result = patch_exec_transition(Some(&execution_platform()), &options);

    let core = result.get::<CoreOptions>().unwrap();
    assert!(!core.affected_by_starlark_transition.is_empty());
    assert_eq!(core.platform_suffix.as_deref(), Some("exec"));
}

#[test]
fn execution_transition_conf_dist_off() {
    let options = of(
        &target_fragment_classes(),
        &[
            "--platforms=//platform:target",
            "--experimental_exec_configuration_distinguisher=off",
        ],
    );

    let result = patch_exec_transition(Some(&execution_platform()), &options);

    let core = result.get::<CoreOptions>().unwrap();
    assert!(core.affected_by_starlark_transition.is_empty());
    assert_eq!(core.platform_suffix.as_deref(), Some("exec"));
}

/// The `@TestParameters` cases of `starlarkExecFlagBadReferences` that fail in
/// `TransitionReference.create`, before any .bzl file is loaded.
///
/// PORT: Java goes through `useConfiguration("--experimental_exec_config=...")`, which wraps the
/// `StarlarkExecTransitionLoadingException` in an `InvalidConfigurationException` carrying the
/// same message; this port calls the loader directly. The other three cases
/// (`//test:defs.bzl%symbol_doesnt_exist`, `//test:file_doesnt_exist.bzl%symbol`,
/// `//test:defs.bzl%not_a_transition`) need scratch files and `BzlLoadFunction` outside
/// `@_builtins`: see `starlark_exec_flag_bad_references_loaded_bzl` in blaze_sky_top.
#[test]
fn starlark_exec_flag_bad_references() {
    let cases = [
        (
            "gibberish",
            "Doesn't match expected form //pkg:file.bzl%%symbol",
        ),
        (
            "//test:defs.bzl",
            "Doesn't match expected form //pkg:file.bzl%%symbol",
        ),
        (
            "//test:defs.bzl%",
            "Doesn't match expected form //pkg:file.bzl%%symbol",
        ),
    ];
    for (cmd_line_ref, expected_error) in cases {
        let flag = format!("--experimental_exec_config={}", cmd_line_ref);
        let options = of(&target_fragment_classes(), &[flag.as_str()]);
        let e = match load_starlark_exec_transition(Some(&options), &semantics()) {
            Ok(_) => panic!("expected an error for {}", cmd_line_ref),
            Err(e) => e,
        };
        assert!(
            e.to_string().contains(expected_error),
            "{}: {} does not contain {}",
            cmd_line_ref,
            e,
            expected_error
        );
    }
}

// The remaining `@TestParameters` cases of `starlarkExecFlagBadReferences` and
// `platformInOutputPathWorksInExecMode` live in crates/blaze_sky_top/src/tests/execution_transition_factory_test.rs.

/// Shared body of `incompatibleOptionsPreservedInExec` and `experimentalOptionsPreservedInExec`:
/// flips every selected boolean option, applies the exec transition and returns the options whose
/// value the exec configuration does not preserve.
fn unpreserved_options_after_flip(selected: &[(String, OptionInfo)]) -> Vec<String> {
    let default_options = target_options();

    // Flip all selected (boolean) options to their non-default value.
    let mut flipped = default_options.clone(); // To be flipped by below logic.
    for (_, option) in selected {
        let fragment = flipped
            .get_dyn_mut(option.get_option_class())
            .expect("fragment");
        let value = option.get_definition().get_boolean_value(&*fragment);
        assert!(
            option
                .get_definition()
                .set_value(&mut *fragment, Some(Value::new(!value)))
                .is_ok()
        );
    }

    // Fix the details of the exec transition so that the check passes.
    let target_exec_config = target_options()
        .get::<CoreOptions>()
        .unwrap()
        .starlark_exec_config
        .clone();
    flipped
        .get_mut::<CoreOptions>()
        .unwrap()
        .starlark_exec_config = target_exec_config;
    // No checksum of `flipped` has been computed; clone anyway so it reflects the mutations.
    let flipped = flipped.clone();

    let exec_options = patch_exec_transition(Some(&execution_platform()), &flipped);

    // Find which options are different in the exec config (shouldn't be any).
    let mut unpreserved_options = Vec::new();
    for (_, option) in selected {
        let option_class = option.get_option_class();
        let exec_value = option
            .get_definition()
            .get_boolean_value(exec_options.get_dyn(option_class).expect("fragment"));
        let flipped_value = option
            .get_definition()
            .get_boolean_value(flipped.get_dyn(option_class).expect("fragment"));
        if exec_value != flipped_value {
            unpreserved_options.push(format!(
                "ChangedFlag[fragment={}, flag={}, expectedValue={}, foundValue={}]",
                option_class.get_name(),
                option.get_definition().get_option_name(),
                flipped_value,
                exec_value
            ));
        }
    }
    unpreserved_options
}

/// Checks all incompatible options propagate to the exec configuration.
#[test]
fn incompatible_options_preserved_in_exec() {
    let default_options = target_options();
    let option_info_map = OptionInfo::build_map_from(&default_options);

    // Find all options with the INCOMPATIBLE_CHANGE metadata tag or start with "--incompatible_".
    let incompatible_options: Vec<(String, OptionInfo)> = option_info_map
        .iter()
        .filter(|(k, v)| {
            k.starts_with("incompatible_")
                || v.has_option_metadata_tag(OptionMetadataTag::INCOMPATIBLE_CHANGE)
        })
        // `getType().isAssignableFrom(boolean.class)`.
        .filter(|(_, v)| v.get_definition().is_boolean_field())
        .filter(|(_, v)| !v.get_definition().is_deprecated())
        // TODO: b/328442047 - Remove this when the flag is removed.
        // Skipping this explicitly because it is a no-op but can't be removed yet.
        .filter(|(k, _)| k.as_str() != "incompatible_enable_android_toolchain_resolution")
        .map(|(k, v)| (k.clone(), *v))
        .collect();

    // Verify all "--incompatible_*" options also have the INCOMPATIBLE_CHANGE metadata tag.
    let missing_metadata_tag_options: Vec<String> = incompatible_options
        .iter()
        .filter(|(_, o)| !o.has_option_metadata_tag(OptionMetadataTag::INCOMPATIBLE_CHANGE))
        .map(|(_, o)| format!("--{}", o.get_definition().get_option_name()))
        .collect();
    assert_eq!(missing_metadata_tag_options, Vec::<String>::new());

    let unpreserved_options = unpreserved_options_after_flip(&incompatible_options);
    assert_eq!(unpreserved_options, Vec::<String>::new());
}

/// Checks all experimental options propagate to the exec configuration.
#[test]
fn experimental_options_preserved_in_exec() {
    let default_options = target_options();
    let option_info_map = OptionInfo::build_map_from(&default_options);

    // Find all options with the EXPERIMENTAL metadata tag or that start with "--experimental_".
    let experimental_options: Vec<(String, OptionInfo)> = option_info_map
        .iter()
        .filter(|(k, v)| {
            k.starts_with("experimental_")
                || v.has_option_metadata_tag(OptionMetadataTag::EXPERIMENTAL)
        })
        // `getType().isAssignableFrom(boolean.class)`.
        .filter(|(_, v)| v.get_definition().is_boolean_field())
        .filter(|(_, v)| !v.get_definition().is_deprecated())
        // Skipping this explicitly as propagating it causes a cycle when compiling
        // the optimizer itself.
        .filter(|(k, _)| k.as_str() != "experimental_local_java_optimizations")
        // A rare (only?) case of a flag named "--experimental_..." that isn't
        // actually experimental.
        .filter(|(k, _)| k.as_str() != "experimental_deps_ok")
        .map(|(k, v)| (k.clone(), *v))
        .collect();

    // Verify all "--experimental_*" options also have the EXPERIMENTAL metadata tag.
    let missing_metadata_tag_options: Vec<String> = experimental_options
        .iter()
        .filter(|(_, o)| !o.has_option_metadata_tag(OptionMetadataTag::EXPERIMENTAL))
        .map(|(_, o)| format!("--{}", o.get_definition().get_option_name()))
        .collect();
    assert_eq!(missing_metadata_tag_options, Vec::<String>::new());

    let unpreserved_options = unpreserved_options_after_flip(&experimental_options);
    assert_eq!(unpreserved_options, Vec::<String>::new());
}
