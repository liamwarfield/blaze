//! The Starlark value of the `coverage` fragment (`analysis/test/CoverageConfiguration.java`,
//! `starlarkbuildapi/test/CoverageConfigurationApi.java`).

use blaze_build_config::fragments::coverage_configuration::{
    COVERAGE_CONFIGURATION, CoverageConfiguration,
};
use blaze_starlark::label::LabelValue;
use starlark::environment::MethodsBuilder;
use starlark::starlark_module;
use starlark::values::none::NoneOr;

use super::fragment_value;

fragment_value!(
    /// `ctx.fragments.coverage`: a configuration fragment representing the coverage
    /// configuration.
    // Java: com/google/devtools/build/lib/analysis/test/CoverageConfiguration.java:35 CoverageConfiguration
    CoverageConfigurationValue,
    CoverageConfiguration,
    "coverage",
    "com.google.devtools.build.lib.analysis.test.CoverageConfiguration",
    coverage_configuration_methods
);

pub(super) fn register() {
    super::register(&COVERAGE_CONFIGURATION, CoverageConfigurationValue::alloc);
}

#[starlark_module]
fn coverage_configuration_methods(builder: &mut MethodsBuilder) {
    /// Returns the label pointed to by the --coverage_output_generator option if coverage
    /// collection is enabled, otherwise returns None.
    // Java: com/google/devtools/build/lib/analysis/test/CoverageConfiguration.java:90 CoverageConfiguration.outputGenerator
    #[starlark(attribute)]
    fn output_generator(this: &CoverageConfigurationValue) -> starlark::Result<NoneOr<LabelValue>> {
        Ok(match this.fragment().output_generator() {
            Some(l) => NoneOr::Other(LabelValue(l.clone())),
            None => NoneOr::None,
        })
    }
}
