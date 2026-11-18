//! Port of `analysis/test/CoverageConfiguration.java` (the fragment; its options are
//! `blaze_optdefs::fragment::coverage_configuration`).

use std::any::Any;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::coverage_configuration::CoverageConfigurationCoverageOptions;

use crate::build_options::BuildOptions;
use crate::fragment::{
    Fragment, FragmentClass, FragmentClassInfo, InvalidConfigurationException, required,
};
use crate::fragment_options_class::{CORE_OPTIONS, COVERAGE_OPTIONS};

// Java: com/google/devtools/build/lib/analysis/test/CoverageConfiguration.java:35 CoverageConfiguration
/// The coverage configuration fragment.
#[derive(Debug)]
pub struct CoverageConfiguration {
    coverage_options: Option<CoverageConfigurationCoverageOptions>,
}

/// `CoverageConfiguration.class`.
pub static COVERAGE_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.analysis.test.CoverageConfiguration",
    starlark_name: Some("coverage"),
    required_options: || vec![&CORE_OPTIONS, &COVERAGE_OPTIONS],
    requires_starlark_options: false,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(CoverageConfiguration::new(options));
    Ok(fragment)
}

impl CoverageConfiguration {
    // Java: com/google/devtools/build/lib/analysis/test/CoverageConfiguration.java:77 CoverageConfiguration.<init>
    pub fn new(build_options: &BuildOptions) -> CoverageConfiguration {
        if !required::<CoreOptions>(build_options).collect_code_coverage {
            return CoverageConfiguration {
                coverage_options: None,
            };
        }
        CoverageConfiguration {
            coverage_options: build_options
                .get::<CoverageConfigurationCoverageOptions>()
                .cloned(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/CoverageConfiguration.java:90 CoverageConfiguration.outputGenerator
    /// Label for the coverage output generator (`ctx.fragments.coverage.output_generator`).
    pub fn output_generator(&self) -> Option<&Label> {
        self.coverage_options
            .as_ref()
            .map(|o| o.coverage_output_generator.get())
    }

    // Java: com/google/devtools/build/lib/analysis/test/CoverageConfiguration.java:98 CoverageConfiguration.reportGenerator
    pub fn report_generator(&self) -> Option<&Label> {
        self.coverage_options
            .as_ref()
            .map(|o| o.coverage_report_generator.get())
    }
}

impl Fragment for CoverageConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &COVERAGE_CONFIGURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
