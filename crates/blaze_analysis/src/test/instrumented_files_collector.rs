//! Port of `analysis/test/InstrumentedFilesCollector.java` (the collector and its spec live
//! with `InstrumentedFilesInfo` in `crate::test::instrumented_files_info`; this module exposes
//! them under the Java class's name plus the static predicates).

use blaze_actions::Artifact;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;

use crate::configured_target::ConfiguredTarget;
use crate::test::test_provider::TestProvider;

pub use crate::test::instrumented_files_info::{InstrumentationSpec, InstrumentedFilesCollector};

/// Returns whether the local sources of a target with the given label should be instrumented.
// Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:162 InstrumentedFilesCollector.shouldIncludeLocalSources
pub fn should_include_local_sources(
    config: &BuildConfigurationValue,
    label: &Label,
    is_test: bool,
) -> bool {
    InstrumentedFilesCollector::should_include_local_sources(config, label, is_test)
}

/// Returns whether the local sources of `target` should be instrumented.
// Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:156 InstrumentedFilesCollector.shouldIncludeLocalSources
pub fn should_include_local_sources_for_target(
    config: &BuildConfigurationValue,
    target: &ConfiguredTarget,
) -> bool {
    should_include_local_sources(
        config,
        &crate::configuredtargets::get_label(target),
        crate::configuredtargets::get_provider::<TestProvider>(target).is_some(),
    )
}

/// Returns whether `artifact` should be instrumented.
// Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesCollector.java:168 InstrumentedFilesCollector.shouldIncludeArtifact
pub fn should_include_artifact(config: &BuildConfigurationValue, artifact: &Artifact) -> bool {
    InstrumentedFilesCollector::should_include_artifact(config, artifact)
}
