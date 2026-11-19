//! Port of `analysis/ShellConfiguration.java` (the fragment; its options are
//! `blaze_optdefs::fragment::shell_configuration`).

use std::any::Any;
use std::sync::Arc;

use blaze_optdefs::fragment::shell_configuration::ShellConfigurationOptions;
use blaze_vfs::PathFragment;

use super::bazel_rule_class_provider;
use crate::build_options::BuildOptions;
use crate::fragment::{
    Fragment, FragmentClass, FragmentClassInfo, InvalidConfigurationException, required,
};
use crate::fragment_options_class::SHELL_CONFIGURATION_OPTIONS;

// Java: com/google/devtools/build/lib/analysis/ShellConfiguration.java:33 ShellConfiguration
/// A configuration fragment that tells where the shell is.
#[derive(Debug)]
pub struct ShellConfiguration {
    default_shell_executable_from_options: Option<PathFragment>,
}

/// `ShellConfiguration.class`.
pub static SHELL_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.analysis.ShellConfiguration",
    starlark_name: None,
    required_options: || vec![&SHELL_CONFIGURATION_OPTIONS],
    requires_starlark_options: false,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(ShellConfiguration::new(options));
    Ok(fragment)
}

// Java: com/google/devtools/build/lib/analysis/ShellConfiguration.java:37 ShellConfiguration.optionsBasedDefault
// PORT: Java holds the finder in a static field set by `injectShellExecutableFinder`, which Bazel's
// `BazelRuleClassProvider.BAZEL_SETUP` always calls with
// `BazelRuleClassProvider::getDefaultPathFromOptions` and `SHELL_EXECUTABLES`. Blaze has only the
// Bazel rule class provider, so the injected values are used directly.
fn options_based_default(options: &ShellConfigurationOptions) -> Option<PathFragment> {
    bazel_rule_class_provider::get_default_path_from_options(options)
}

impl ShellConfiguration {
    // Java: com/google/devtools/build/lib/analysis/ShellConfiguration.java:55 ShellConfiguration.<init>
    pub fn new(build_options: &BuildOptions) -> ShellConfiguration {
        let options = required::<ShellConfigurationOptions>(build_options);
        ShellConfiguration {
            default_shell_executable_from_options: options_based_default(options),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/ShellConfiguration.java:60 ShellConfiguration.getShellExecutable
    /// The shell for `os` (the `OS` enum constant's name), from the injected
    /// `BazelRuleClassProvider.SHELL_EXECUTABLES`.
    pub fn get_shell_executable(os: &str) -> Option<PathFragment> {
        bazel_rule_class_provider::SHELL_EXECUTABLES
            .iter()
            .find(|(k, _)| *k == os)
            .map(|(_, v)| PathFragment::create(v))
    }

    // Java: com/google/devtools/build/lib/analysis/ShellConfiguration.java:66 ShellConfiguration.getOptionsBasedDefault
    /// Returns the default shell from build options if set explicitly.
    pub fn get_options_based_default(&self) -> Option<&PathFragment> {
        self.default_shell_executable_from_options.as_ref()
    }
}

impl Fragment for ShellConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &SHELL_CONFIGURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ShellConfigurationTest lives in crates/blaze_sky_top/src/tests/shell_configuration_test.rs.
