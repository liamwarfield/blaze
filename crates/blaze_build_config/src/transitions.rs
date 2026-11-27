//! Native transitions the configuration code needs: `TestTrimmingLogic`, `NoConfigTransition`
//! (with `CommonOptions.EMPTY_OPTIONS`) and `TransitionUtil.restrict`.

use std::sync::LazyLock;

use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::run_under::RunUnder;
use blaze_options::Options;

use crate::build_options::{BuildOptions, BuildOptionsView};
use crate::fragment_options_class::{CORE_OPTIONS, FragmentOptionsClass, TEST_OPTIONS};

// Java: com/google/devtools/build/lib/analysis/config/CommonOptions.java:32 CommonOptions.EMPTY_OPTIONS
/// Ideally the empty build options should be actually empty: no fragment options and no flags.
/// But core Bazel code assumes CoreOptions exists. For example CoreOptions.check_visibility is
/// required for basic configured target graph evaluation. So we provide CoreOptions with default
/// values (not inherited from parent configuration).
pub static EMPTY_OPTIONS: LazyLock<BuildOptions> = LazyLock::new(create_empty_options);

// Java: com/google/devtools/build/lib/analysis/config/CommonOptions.java:34 CommonOptions.createEmptyOptions
fn create_empty_options() -> BuildOptions {
    // Disable the exec transition. Since this config is empty it shouldn't trigger any exec
    // transitions.
    // PORT: Java then sets `starlarkExecConfig = null`. The Rust field is a non-null `String`.
    // Java's `OptionDefinition.getValue` (asMap, cacheKey, the checksum, `config` output) returns
    // the option's default for a null field, so every observable value is unchanged; the only
    // raw reader (`StarlarkExecTransitionLoader`) first checks `options.equals(EMPTY_OPTIONS)`.
    BuildOptions::builder()
        .add_fragment_options(&Options::<CoreOptions>::get_defaults())
        .build()
}

// Java: com/google/devtools/build/lib/analysis/config/transitions/NoConfigTransition.java:72 NoConfigTransition.patch
/// Transitions to the empty configuration (`CommonOptions.EMPTY_OPTIONS`).
pub fn no_config_transition_patch() -> BuildOptions {
    EMPTY_OPTIONS.clone()
}

// Java: com/google/devtools/build/lib/analysis/test/TestTrimmingLogic.java:32 TestTrimmingLogic.REQUIRED_FRAGMENTS
pub fn test_trimming_required_fragments() -> Vec<FragmentOptionsClass> {
    vec![&CORE_OPTIONS, &TEST_OPTIONS]
}

// PORT: skipped, a cache of TestTrimmingLogic.trim per input options (the result is computed each time; not observable): BuildOptionsCache.CacheRetrievalFunction.apply, BuildOptionsCache.<init>, BuildOptionsCache.applyTransition, BuildOptionsCache.CacheKey.<init>, BuildOptionsCache.CacheKey.create
// Java: com/google/devtools/build/lib/analysis/test/TestTrimmingLogic.java:61 TestTrimmingLogic.trim
/// Returns a new `BuildOptions` instance with test configuration removed.
///
/// PORT: Java caches the result per input options (`BuildOptionsCache`); skipped (incremental).
pub fn test_trimming_trim(build_options: &BuildOptions) -> BuildOptions {
    test_trimming_trim_view(&BuildOptionsView::new(
        build_options,
        test_trimming_required_fragments(),
    ))
}

// Java: com/google/devtools/build/lib/analysis/test/TestTrimmingLogic.java:61 TestTrimmingLogic.trim
pub fn test_trimming_trim_view(options: &BuildOptionsView) -> BuildOptions {
    let mut builder = options.underlying().to_builder();
    builder.remove_fragment_options(&TEST_OPTIONS);
    // Only the label of the --run_under target (if any) needs to be part of the configuration for
    // non-test targets, all other information is directly obtained from the options in
    // RunCommand.
    let core_options = builder
        .get_fragment_options::<CoreOptions>()
        .expect("CoreOptions");
    core_options.run_under =
        RunUnder::trim_for_non_test_configuration(core_options.run_under.as_ref());
    builder.build()
}

// Java: com/google/devtools/build/lib/analysis/config/transitions/TransitionUtil.java:27 TransitionUtil.restrict
/// Returns a `BuildOptionsView` restricted to the transition's required fragments.
pub fn restrict<'a>(
    required_fragments: Vec<FragmentOptionsClass>,
    options: &'a BuildOptions,
) -> BuildOptionsView<'a> {
    BuildOptionsView::new(options, required_fragments)
}
