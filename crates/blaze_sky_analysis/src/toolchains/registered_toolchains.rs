//! Ports of `skyframe/toolchains/RegisteredToolchainsFunction.java` and
//! `RegisteredToolchainsValue.java`.

use std::fmt;
use std::sync::Arc;

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::DeclaredToolchainInfo;
use blaze_analysis::platform::platform_provider_utils;
use blaze_analysis::toolchains::{ToolchainException, ToolchainExceptionKind};
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::fragment::InvalidConfigurationException;
use blaze_build_config::fragments::platform_configuration::PlatformConfiguration;
use blaze_bzlmod::skyframe::keys::BazelDepGraphKey;
use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{
    Label, SignedTargetPattern, TargetParsingException, TargetPattern, TargetPatternParser,
};
use blaze_packages::target::Target;
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_sky_loading::target_pattern_function::{FilterFn, FilteringPolicy};
use blaze_skyframe::{
    ComputeError, Environment, FunctionHermeticity, Key, SkyError, SkyFunctionException,
    SkyFunctionName, SkyKey, Transience,
};
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use crate::toolchains::config_matching_util;
use crate::toolchains::lookup_error::catch_configured_value_creation_exception;
use crate::toolchains::target_pattern_util::{self, InvalidTargetPatternException};

/// `SkyFunctions.REGISTERED_TOOLCHAINS`.
pub const REGISTERED_TOOLCHAINS: SkyFunctionName = SkyFunctionName("REGISTERED_TOOLCHAINS");

/// `ToolchainRule.TARGET_SETTING_ATTR`.
// Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:39 ToolchainRule.TARGET_SETTING_ATTR
const TARGET_SETTING_ATTR: &str = "target_settings";

/// A value which represents every toolchain known to Bazel and available for toolchain
/// resolution.
///
/// `rejected_toolchains`: any toolchains that were rejected, along with a reason. The row keys
/// are the toolchain type labels, column keys are toolchain target (not implementation) labels,
/// and cells are the reason. Only present if the key's `debug` is true.
// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsValue.java:42 RegisteredToolchainsValue
// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsValue.java:121 RegisteredToolchainsValue.create
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredToolchainsValue {
    pub registered_toolchains: Vec<DeclaredToolchainInfo>,
    pub rejected_toolchains: Option<IndexMap<Label, IndexMap<Label, String>>>,
}

/// A SkyKey for [`RegisteredToolchainsValue`].
// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsValue.java:57 RegisteredToolchainsValue.Key
// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsValue.java:102 RegisteredToolchainsValue.Key.equals
// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsValue.java:111 RegisteredToolchainsValue.Key.hashCode
// PORT: skipped, key interning (the keys are plain values here): RegisteredToolchainsValue.Key.intern, RegisteredToolchainsValue.Key.getSkyKeyInterner
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RegisteredToolchainsKey {
    configuration_key: BuildConfigurationKey,
    debug: bool,
}

impl RegisteredToolchainsKey {
    /// Returns the SkyKey for [`RegisteredToolchainsValue`]s.
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsValue.java:51 RegisteredToolchainsValue.key
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsValue.java:68 RegisteredToolchainsValue.Key.of
    pub fn key(configuration_key: BuildConfigurationKey, debug: bool) -> Self {
        RegisteredToolchainsKey {
            configuration_key,
            debug,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsValue.java:83 RegisteredToolchainsValue.Key.getConfigurationKey
    pub fn get_configuration_key(&self) -> &BuildConfigurationKey {
        &self.configuration_key
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsValue.java:87 RegisteredToolchainsValue.Key.debug
    pub fn debug(&self) -> bool {
        self.debug
    }
}

impl fmt::Debug for RegisteredToolchainsKey {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsValue.java:92 RegisteredToolchainsValue.Key.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RegisteredToolchainsValue.Key{{configurationKey: {:?}, debug: {}}}",
            self.configuration_key, self.debug
        )
    }
}

impl Key for RegisteredToolchainsKey {
    type Value = RegisteredToolchainsValue;
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsValue.java:79 RegisteredToolchainsValue.Key.functionName
    const FUNCTION: SkyFunctionName = REGISTERED_TOOLCHAINS;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:141 SkyFunctions.REGISTERED_TOOLCHAINS (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: a record of the declared toolchains (a list; `DeclaredToolchainInfo` by value) and the nullable rejected toolchains (a table, equal as cell sets).
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsValue.java:42 RegisteredToolchainsValue
    fn values_equal(old: &RegisteredToolchainsValue, new: &RegisteredToolchainsValue) -> bool {
        old == new
    }
}

fn persistent(e: ToolchainException) -> ComputeError {
    SkyFunctionException::new(e, Transience::Persistent).into()
}

/// Java `FilteringPolicies.ruleTypeExplicit("toolchain")` (`RuleTypeFilter.shouldRetain`).
// Java: com/google/devtools/build/lib/pkgcache/FilteringPolicies.java:115 FilteringPolicies.RuleTypeFilter.shouldRetain
fn rule_type_explicit_toolchain(target: &Target, explicit: bool) -> bool {
    if explicit {
        return true;
    }
    target
        .get_associated_rule()
        .expect("NullPointerException: target is not a rule")
        .get_rule_class()
        == "toolchain"
}

/// SkyFunction that returns all registered toolchains available for toolchain resolution.
// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunction.java:66 RegisteredToolchainsFunction.compute
pub fn registered_toolchains_function(
    key: &RegisteredToolchainsKey,
    env: &mut Environment<'_>,
) -> Result<Option<RegisteredToolchainsValue>, ComputeError> {
    let configuration = env.get_value(key.get_configuration_key().clone())?;
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

    // Get the toolchains from the configuration.
    // Reverse the list so the last one defined takes precedences.
    let platform_configuration = configuration
        .get_fragment::<PlatformConfiguration>()
        .expect("NullPointerException: PlatformConfiguration");
    let mut extra_toolchains: Vec<String> = platform_configuration.get_extra_toolchains().to_vec();
    extra_toolchains.reverse();
    match target_pattern_util::parse_all_signed(&extra_toolchains, &main_repo_parser) {
        Ok(p) => target_patterns.extend(p),
        Err(e) => {
            return Err(persistent(invalid_toolchain_label_exception_from_pattern(
                &e,
            )));
        }
    }

    // Get registered toolchains from the external dep graph.
    let Some(bzlmod_toolchains) = get_bzlmod_toolchains(env)? else {
        return Ok(None);
    };
    target_patterns.extend(target_pattern_util::to_signed(bzlmod_toolchains));

    // Expand target patterns.
    let toolchain_labels: IndexSet<Label> = match target_pattern_util::expand_target_patterns(
        env,
        &target_patterns,
        FilteringPolicy::Custom(
            "RuleTypeFilter[ruleName=toolchain, keepExplicit=true]",
            FilterFn(rule_type_explicit_toolchain),
        ),
    )? {
        Ok(Some(labels)) => {
            if env.values_missing() {
                return Ok(None);
            }
            labels
        }
        Ok(None) => return Ok(None),
        Err(e) => {
            return Err(persistent(invalid_toolchain_label_exception_from_pattern(
                &e,
            )));
        }
    };

    // Load the configured target for each, and get the declared toolchain providers.
    let Some(registered_toolchains) =
        configure_registered_toolchains(env, &configuration, &toolchain_labels)?
    else {
        return Ok(None);
    };
    if env.values_missing() {
        return Ok(None);
    }

    // Check which toolchains are valid according to their configuration.
    // Some toolchains end up with repeated reasons, so use a HashBasedTable to handle duplicates.
    let mut valid_toolchains: Vec<DeclaredToolchainInfo> = Vec::new();
    let mut rejected_toolchains: Option<IndexMap<Label, IndexMap<Label, String>>> = if key.debug() {
        Some(IndexMap::new())
    } else {
        None
    };
    for toolchain in &registered_toolchains {
        let mut record_rejection = |message: String| {
            if let Some(rejected) = rejected_toolchains.as_mut() {
                rejected
                    .entry(toolchain.toolchain_type().type_label().clone())
                    .or_default()
                    .insert(toolchain.target_label().clone(), message);
            }
        };
        let error_handler: Option<&mut dyn FnMut(String)> = if key.debug() {
            Some(&mut record_rejection)
        } else {
            None
        };
        match config_matching_util::validate(
            toolchain.target_label(),
            toolchain.target_settings(),
            error_handler,
            TARGET_SETTING_ATTR,
        ) {
            Ok(true) => valid_toolchains.push(toolchain.clone()),
            Ok(false) => {}
            Err(e) => {
                return Err(persistent(
                    invalid_toolchain_label_exception_from_configuration(
                        toolchain.target_label(),
                        e,
                    ),
                ));
            }
        }
    }

    Ok(Some(RegisteredToolchainsValue {
        registered_toolchains: valid_toolchains,
        rejected_toolchains,
    }))
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunction.java:154 RegisteredToolchainsFunction.getBzlmodToolchains
fn get_bzlmod_toolchains(
    env: &mut Environment<'_>,
) -> Result<Option<Vec<TargetPattern>>, ComputeError> {
    let Some(bazel_dep_graph_value) = env.get_value(BazelDepGraphKey)? else {
        return Ok(None);
    };
    let module_key_to_repository_names = bazel_dep_graph_value.module_key_to_repository_names();
    let mut toolchains = Vec::new();
    for module in bazel_dep_graph_value.dep_graph.values() {
        let parser = TargetPatternParser::new(
            PathFragment::empty_fragment(),
            module_key_to_repository_names[module.get_key()].clone(),
            bazel_dep_graph_value.get_full_repo_mapping(module.get_key()),
        );
        for pattern in &module.toolchains_to_register {
            match parser.parse(pattern) {
                Ok(p) => toolchains.push(p),
                Err(e) => {
                    return Err(persistent(invalid_toolchain_label_exception(pattern, &e)));
                }
            }
        }
    }
    Ok(Some(toolchains))
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunction.java:181 RegisteredToolchainsFunction.configureRegisteredToolchains
fn configure_registered_toolchains(
    env: &mut Environment<'_>,
    configuration: &BuildConfigurationValue,
    labels: &IndexSet<Label>,
) -> Result<Option<Vec<DeclaredToolchainInfo>>, ComputeError> {
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
    let mut toolchains: Vec<DeclaredToolchainInfo> = Vec::new();
    let mut values_missing = false;
    for key in &keys {
        let toolchain_label = key.get_label();
        match values.get_or_throw_with(key, catch_configured_value_creation_exception) {
            Ok(None) => {
                values_missing = true;
                continue;
            }
            Ok(Some(value)) => {
                let target = value.get_configured_target();
                let Some(toolchain_info) =
                    platform_provider_utils::declared_toolchain_info(Some(target))
                else {
                    return Err(persistent(invalid_toolchain_label_exception_for_label(
                        toolchain_label,
                    )));
                };
                toolchains.push(toolchain_info);
            }
            Err(e) => {
                return Err(persistent(invalid_toolchain_label_exception_from_creation(
                    toolchain_label,
                    e,
                )));
            }
        }
    }

    if values_missing {
        return Ok(None);
    }
    Ok(Some(toolchains))
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunction.java:259 RegisteredToolchainsFunction.InvalidToolchainLabelException.formatMessage
fn format_message(invalid_pattern: &str, reason: &str) -> String {
    format!("invalid registered toolchain '{invalid_pattern}': {reason}")
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunction.java:231 RegisteredToolchainsFunction.InvalidToolchainLabelException.<init>
pub fn invalid_toolchain_label_exception_for_label(invalid_label: &Label) -> ToolchainException {
    ToolchainException::new(
        ToolchainExceptionKind::InvalidToolchainLabel,
        format_message(
            &invalid_label.get_canonical_form(),
            "target does not provide the DeclaredToolchainInfo provider",
        ),
    )
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunction.java:238 RegisteredToolchainsFunction.InvalidToolchainLabelException.<init>
fn invalid_toolchain_label_exception_from_pattern(
    e: &InvalidTargetPatternException,
) -> ToolchainException {
    invalid_toolchain_label_exception(e.get_invalid_pattern(), e.get_tpe())
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunction.java:242 RegisteredToolchainsFunction.InvalidToolchainLabelException.<init>
fn invalid_toolchain_label_exception(
    invalid_pattern: &str,
    e: &TargetParsingException,
) -> ToolchainException {
    ToolchainException::with_cause(
        ToolchainExceptionKind::InvalidToolchainLabel,
        format_message(invalid_pattern, e.get_message()),
        Arc::new(e.clone()),
    )
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunction.java:246 RegisteredToolchainsFunction.InvalidToolchainLabelException.<init>
fn invalid_toolchain_label_exception_from_creation(
    invalid_label: &Label,
    e: SkyError,
) -> ToolchainException {
    ToolchainException::with_cause(
        ToolchainExceptionKind::InvalidToolchainLabel,
        format_message(&invalid_label.get_canonical_form(), &e.to_string()),
        e,
    )
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunction.java:250 RegisteredToolchainsFunction.InvalidToolchainLabelException.<init>
fn invalid_toolchain_label_exception_from_configuration(
    invalid_label: &Label,
    e: InvalidConfigurationException,
) -> ToolchainException {
    ToolchainException::with_cause(
        ToolchainExceptionKind::InvalidToolchainLabel,
        format_message(&invalid_label.get_canonical_form(), e.get_message()),
        Arc::new(e),
    )
}
