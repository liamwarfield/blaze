//! Top-level configuration creation from command options: `BlazeRuntime.createBuildOptions`
//! (BuildRequest -> BuildOptions) and the `InfoCommand`/`BuildView` sequence
//! (`setBaselineConfiguration`, then `createConfiguration`/`getConfiguration`).

use std::sync::Arc;

use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::build_language_options::BuildLanguageOptions;
use blaze_options::{OptionsClass, OptionsParser};
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::Path;

use crate::bazel_rule_class_provider::BazelRuleClassProvider;
use crate::build_configuration_value::BuildConfigurationValue;
use crate::build_options::BuildOptions;
use crate::fragment::InvalidConfigurationException;
use crate::output_directories::ConfigDirectories;
use crate::platform::SharedPlatformValueProvider;
use crate::skyframe::{ConfigurationRequest, ConfigurationSkyframe};

// Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:902 BlazeRuntime.createBuildOptions
pub fn create_build_options(
    rule_class_provider: &BazelRuleClassProvider,
    options_provider: &OptionsParser,
) -> BuildOptions {
    BuildOptions::of(rule_class_provider.get_options_classes(), options_provider)
}

/// The Starlark semantics the configuration functions read (`PrecomputedValue.STARLARK_SEMANTICS`
/// fields `experimental_sibling_repository_layout` and `incompatible_disable_transitions_on`).
///
/// PORT: Java derives the full `StarlarkSemantics` from `BuildLanguageOptions` in
/// `PackageOptions`/`syncPackageLoading`; only the two flags configuration code reads are taken
/// from the parsed options here.
pub fn starlark_semantics_from(options: &OptionsParser) -> StarlarkSemantics {
    let mut semantics = StarlarkSemantics::default();
    if let Some(build_language) = options.get_options_dyn(BuildLanguageOptions::class()) {
        for def in BuildLanguageOptions::class().definitions() {
            let name = def.get_option_name();
            if name == "experimental_sibling_repository_layout" {
                semantics.experimental_sibling_repository_layout =
                    def.get_boolean_value(build_language.as_ref());
            } else if name == "incompatible_disable_transitions_on" {
                semantics.incompatible_disable_transitions_on = def
                    .get_value(build_language.as_ref())
                    .and_then(|v| {
                        v.as_list()
                            .map(|l| l.iter().map(|e| e.java_to_string()).collect())
                    })
                    .unwrap_or_default();
            }
        }
    }
    semantics
}

/// Inputs to create the top-level configuration of a command.
pub struct TopLevelConfigurationInputs {
    pub directories: ConfigDirectories,
    pub rule_class_provider: Arc<BazelRuleClassProvider>,
    pub starlark_semantics: Arc<StarlarkSemantics>,
    pub package_path_roots: Vec<Path>,
    pub platforms: SharedPlatformValueProvider,
    pub event_handler: Arc<dyn ExtendedEventHandler>,
}

/// The top-level configuration and the configuration evaluator it was created in.
pub struct TopLevelConfiguration {
    pub skyframe: ConfigurationSkyframe,
    pub build_options: Arc<BuildOptions>,
    pub configuration: Arc<BuildConfigurationValue>,
}

// Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:169 InfoCommand.exec
/// The body of InfoCommand's configuration supplier (and of `BuildView`'s configuration
/// creation): `setBaselineConfiguration(buildOptions)` then `getConfiguration(buildOptions,
/// keepGoing)`.
pub fn create_top_level_configuration(
    inputs: TopLevelConfigurationInputs,
    build_options: BuildOptions,
    keep_going: bool,
) -> Result<TopLevelConfiguration, InvalidConfigurationException> {
    let build_options = Arc::new(build_options);
    let skyframe = ConfigurationSkyframe::set_baseline_configuration(
        ConfigurationRequest {
            directories: inputs.directories,
            rule_class_provider: inputs.rule_class_provider,
            starlark_semantics: inputs.starlark_semantics,
            package_path_roots: inputs.package_path_roots,
            platforms: inputs.platforms,
        },
        build_options.clone(),
        inputs.event_handler,
    )?;
    let configuration = skyframe.get_configuration(build_options.clone(), keep_going)?;
    Ok(TopLevelConfiguration {
        skyframe,
        build_options,
        configuration,
    })
}
