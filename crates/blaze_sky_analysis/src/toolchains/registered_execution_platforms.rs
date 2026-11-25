//! Ports of `skyframe/toolchains/RegisteredExecutionPlatformsFunction.java` and
//! `RegisteredExecutionPlatformsValue.java`.

use std::fmt;
use std::sync::Arc;

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::PlatformInfo;
use blaze_analysis::platform::platform_provider_utils;
use blaze_analysis::toolchains::{ToolchainException, ToolchainExceptionKind};
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::fragment::InvalidConfigurationException;
use blaze_build_config::fragments::platform_configuration::PlatformConfiguration;
use blaze_build_config::transitions::EMPTY_OPTIONS;
use blaze_bzlmod::skyframe::keys::BazelDepGraphKey;
use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{
    Label, SignedTargetPattern, TargetParsingException, TargetPattern, TargetPatternParser,
};
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_sky_loading::target_pattern_function::{FilterFn, FilteringPolicy};
use blaze_skyframe::{
    ComputeError, Environment, FunctionHermeticity, Key, SkyFunctionException, SkyFunctionName,
    SkyKey, Transience,
};
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use crate::toolchains::config_matching_util;
use crate::toolchains::lookup_error::catch_configured_value_creation_exception;
use crate::toolchains::platform_lookup_util::{
    has_platform_info_filter, invalid_platform_exception, invalid_platform_exception_with_cause,
};
use crate::toolchains::target_pattern_util::{self, InvalidTargetPatternException};

/// `SkyFunctions.REGISTERED_EXECUTION_PLATFORMS`.
pub const REGISTERED_EXECUTION_PLATFORMS: SkyFunctionName =
    SkyFunctionName("REGISTERED_EXECUTION_PLATFORMS");

/// The `PlatformRule.REQUIRED_SETTINGS_ATTR` attribute name.
// Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:45 PlatformRule.REQUIRED_SETTINGS_ATTR
const REQUIRED_SETTINGS_ATTR: &str = "required_settings";

/// A value which represents every execution platform known to Bazel and available to run
/// actions.
///
/// `rejected_platforms`: any execution platforms that were rejected, along with a reason. The
/// keys are the platform label, and the value is the rejection reason. Only present if the key's
/// `debug` is true.
// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsValue.java:42 RegisteredExecutionPlatformsValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredExecutionPlatformsValue {
    pub registered_execution_platform_keys: Vec<ConfiguredTargetKey>,
    pub rejected_platforms: Option<IndexMap<Label, String>>,
}

/// SkyKey implementation used for [`registered_execution_platforms_function`].
// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsValue.java:58 RegisteredExecutionPlatformsValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RegisteredExecutionPlatformsKey {
    configuration_key: BuildConfigurationKey,
    debug: bool,
}

impl RegisteredExecutionPlatformsKey {
    /// Returns the SkyKey for [`RegisteredExecutionPlatformsValue`]s.
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsValue.java:51 RegisteredExecutionPlatformsValue.key
    pub fn key(configuration_key: BuildConfigurationKey, debug: bool) -> Self {
        RegisteredExecutionPlatformsKey {
            configuration_key,
            debug,
        }
    }

    pub fn configuration_key(&self) -> &BuildConfigurationKey {
        &self.configuration_key
    }

    pub fn debug(&self) -> bool {
        self.debug
    }
}

impl fmt::Debug for RegisteredExecutionPlatformsKey {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsValue.java:107 RegisteredExecutionPlatformsValue.Key.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RegisteredExecutionPlatformsValue.Key{{configurationKey={:?}, debug={}}}",
            self.configuration_key, self.debug
        )
    }
}

impl Key for RegisteredExecutionPlatformsKey {
    type Value = RegisteredExecutionPlatformsValue;
    const FUNCTION: SkyFunctionName = REGISTERED_EXECUTION_PLATFORMS;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:140 SkyFunctions.REGISTERED_EXECUTION_PLATFORMS (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: a record of the platform keys (a list) and the nullable rejection reasons (a map).
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsValue.java:42 RegisteredExecutionPlatformsValue
    fn values_equal(
        old: &RegisteredExecutionPlatformsValue,
        new: &RegisteredExecutionPlatformsValue,
    ) -> bool {
        old == new
    }
}

fn persistent(e: ToolchainException) -> ComputeError {
    SkyFunctionException::new(e, Transience::Persistent).into()
}

/// SkyFunction that returns all registered execution platforms available.
// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunction.java:75 RegisteredExecutionPlatformsFunction.compute
pub fn registered_execution_platforms_function(
    key: &RegisteredExecutionPlatformsKey,
    env: &mut Environment<'_>,
) -> Result<Option<RegisteredExecutionPlatformsValue>, ComputeError> {
    let configuration = env.get_value(key.configuration_key().clone())?;
    let main_repo_mapping = env.get_value(RepositoryMappingKey::key(MAIN.clone()))?;
    if env.values_missing() {
        return Ok(None);
    }
    let configuration = configuration.unwrap();
    let main_repo_mapping = main_repo_mapping.unwrap();

    let main_repo_parser = TargetPatternParser::new(
        PathFragment::empty_fragment(),
        MAIN.clone(),
        main_repo_mapping
            .repository_mapping()
            .expect("NullPointerException: main repository mapping")
            .clone(),
    );
    let mut target_patterns: Vec<SignedTargetPattern> = Vec::new();

    // Get the execution platforms from the configuration.
    if let Some(platform_configuration) = configuration.get_fragment::<PlatformConfiguration>() {
        match target_pattern_util::parse_all_signed(
            platform_configuration.get_extra_execution_platforms(),
            &main_repo_parser,
        ) {
            Ok(p) => target_patterns.extend(p),
            Err(e) => {
                return Err(persistent(
                    invalid_execution_platform_label_exception_from_pattern(&e),
                ));
            }
        }
    }

    // Get registered execution platforms from the external dep graph.
    let Some(bzlmod_execution_platforms) = get_bzlmod_execution_platforms(env)? else {
        return Ok(None);
    };
    target_patterns.extend(target_pattern_util::to_signed(bzlmod_execution_platforms));

    // Expand target patterns.
    let platform_labels: IndexSet<Label> = match target_pattern_util::expand_target_patterns(
        env,
        &target_patterns,
        // Java: RegisteredExecutionPlatformsFunction.HAS_PLATFORM_INFO.
        FilteringPolicy::Custom("HAS_PLATFORM_INFO", FilterFn(has_platform_info_filter)),
    )? {
        Ok(Some(labels)) => {
            if env.values_missing() {
                return Ok(None);
            }
            labels
        }
        Ok(None) => return Ok(None),
        Err(e) => {
            return Err(persistent(
                invalid_execution_platform_label_exception_from_pattern(&e),
            ));
        }
    };

    // Load the configured target for each, and get the declared execution platforms providers.
    let Some(registered_execution_platforms) =
        configure_registered_execution_platforms(env, &configuration, &platform_labels)?
    else {
        return Ok(None);
    };
    if env.values_missing() {
        return Ok(None);
    }

    // Check which platforms are valid according to their configuration.
    let mut platform_keys: Vec<ConfiguredTargetKey> = Vec::new();
    let mut rejected_platforms: Option<IndexMap<Label, String>> = if key.debug() {
        Some(IndexMap::new())
    } else {
        None
    };
    for (configured_target_key, platform_info) in &registered_execution_platforms {
        let mut record_rejection = |message: String| {
            if let Some(rejected) = rejected_platforms.as_mut() {
                // Java: ImmutableMap.Builder.buildKeepingLast.
                rejected.insert(platform_info.label().clone(), message);
            }
        };
        let error_handler: Option<&mut dyn FnMut(String)> = if key.debug() {
            Some(&mut record_rejection)
        } else {
            None
        };
        match config_matching_util::validate(
            platform_info.label(),
            platform_info.required_settings(),
            error_handler,
            REQUIRED_SETTINGS_ATTR,
        ) {
            Ok(true) => platform_keys.push(configured_target_key.clone()),
            Ok(false) => {}
            Err(e) => {
                return Err(persistent(
                    invalid_execution_platform_label_exception_from_configuration(
                        platform_info.label(),
                        e,
                    ),
                ));
            }
        }
    }

    Ok(Some(RegisteredExecutionPlatformsValue {
        registered_execution_platform_keys: platform_keys,
        rejected_platforms,
    }))
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunction.java:165 RegisteredExecutionPlatformsFunction.getBzlmodExecutionPlatforms
fn get_bzlmod_execution_platforms(
    env: &mut Environment<'_>,
) -> Result<Option<Vec<TargetPattern>>, ComputeError> {
    let Some(bazel_dep_graph_value) = env.get_value(BazelDepGraphKey)? else {
        return Ok(None);
    };
    let module_key_to_repository_names = bazel_dep_graph_value.module_key_to_repository_names();
    let mut execution_platforms = Vec::new();
    for module in bazel_dep_graph_value.dep_graph.values() {
        let parser = TargetPatternParser::new(
            PathFragment::empty_fragment(),
            module_key_to_repository_names[module.get_key()].clone(),
            bazel_dep_graph_value.get_full_repo_mapping(module.get_key()),
        );
        for pattern in &module.execution_platforms_to_register {
            match parser.parse(pattern) {
                Ok(p) => execution_platforms.push(p),
                Err(e) => {
                    return Err(persistent(invalid_execution_platform_label_exception(
                        pattern, &e,
                    )));
                }
            }
        }
    }
    Ok(Some(execution_platforms))
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunction.java:193 RegisteredExecutionPlatformsFunction.configureRegisteredExecutionPlatforms
fn configure_registered_execution_platforms(
    env: &mut Environment<'_>,
    configuration: &BuildConfigurationValue,
    labels: &IndexSet<Label>,
) -> Result<Option<IndexMap<ConfiguredTargetKey, Arc<PlatformInfo>>>, ComputeError> {
    let keys: IndexSet<ConfiguredTargetKey> = labels
        .iter()
        .map(|label| {
            ConfiguredTargetKey::builder()
                .set_label(label.clone())
                .set_configuration(Some(configuration))
                .build()
        })
        .collect();

    let sky_keys: Vec<SkyKey> = keys.iter().cloned().map(SkyKey::new).collect();
    let values = env.get_values_and_exceptions(&sky_keys)?;
    // Java: ImmutableMap.Builder (duplicate keys are found by buildOrThrow, below).
    let mut platforms: Vec<(ConfiguredTargetKey, Arc<PlatformInfo>)> = Vec::new();
    let mut values_missing = false;
    for platform_key in &keys {
        let platform_label = platform_key.get_label();
        match values.get_or_throw_with(platform_key, catch_configured_value_creation_exception) {
            Ok(None) => {
                values_missing = true;
                continue;
            }
            Ok(Some(value)) => {
                let target = value.get_configured_target();
                let Some(platform_info) = platform_provider_utils::platform(Some(target)) else {
                    return Err(persistent(invalid_platform_exception(platform_label)));
                };

                // Update the key so that any aliases are resolved.
                let platform_label = target.get_label();
                let platform_key = ConfiguredTargetKey::builder()
                    .set_label(platform_label)
                    .set_configuration_key(Some(BuildConfigurationKey::create(Arc::new(
                        EMPTY_OPTIONS.clone(),
                    ))))
                    .build();

                platforms.push((platform_key, Arc::new(platform_info)));
            }
            Err(e) => {
                return Err(persistent(invalid_platform_exception_with_cause(
                    platform_label,
                    e,
                )));
            }
        }
    }

    if values_missing {
        return Ok(None);
    }
    // Java: platforms.buildOrThrow().
    let mut built: IndexMap<ConfiguredTargetKey, Arc<PlatformInfo>> = IndexMap::new();
    for (platform_key, platform_info) in platforms {
        if built.contains_key(&platform_key) {
            panic!("IllegalArgumentException: Multiple entries with same key: {platform_key}");
        }
        built.insert(platform_key, platform_info);
    }
    Ok(Some(built))
}

/// Java `InvalidExecutionPlatformLabelException(TargetPatternUtil.InvalidTargetPatternException)`.
// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunction.java:253 RegisteredExecutionPlatformsFunction.InvalidExecutionPlatformLabelException.<init>
fn invalid_execution_platform_label_exception_from_pattern(
    e: &InvalidTargetPatternException,
) -> ToolchainException {
    invalid_execution_platform_label_exception(e.get_invalid_pattern(), e.get_tpe())
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunction.java:257 RegisteredExecutionPlatformsFunction.InvalidExecutionPlatformLabelException.<init>
fn invalid_execution_platform_label_exception(
    invalid_pattern: &str,
    e: &TargetParsingException,
) -> ToolchainException {
    ToolchainException::with_cause(
        ToolchainExceptionKind::InvalidExecutionPlatformLabel,
        format!(
            "invalid registered execution platform '{}': {}",
            invalid_pattern,
            e.get_message()
        ),
        Arc::new(e.clone()),
    )
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunction.java:264 RegisteredExecutionPlatformsFunction.InvalidExecutionPlatformLabelException.<init>
fn invalid_execution_platform_label_exception_from_configuration(
    platform: &Label,
    e: InvalidConfigurationException,
) -> ToolchainException {
    ToolchainException::with_cause(
        ToolchainExceptionKind::InvalidExecutionPlatformLabel,
        format!(
            "invalid registered execution platform '{}': {}",
            platform,
            e.get_message()
        ),
        Arc::new(e),
    )
}
