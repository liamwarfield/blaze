//! Port of `analysis/constraints/IncompatibleTargetChecker.java`.

use std::sync::Arc;

use blaze_analysis::config::config_conditions::ConfigConditions;
use blaze_analysis::config::config_matching_provider::{AccumulateResults, ConfigMatchingProvider};
use blaze_analysis::config::configured_attribute_mapper::{
    ConfiguredAttributeMapper, ValidationException,
};
use blaze_analysis::config::incompatible_platform_provider::{
    INCOMPATIBLE_PLATFORM_PROVIDER, IncompatiblePlatformProvider,
};
use blaze_analysis::config::ordered_set_multimap::{PrerequisiteMap, prerequisite_values};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_analysis::info::{downcast_info, freeze_native_info};
use blaze_analysis::platform::constraint_collection::ConstraintCollection;
use blaze_analysis::platform::constraint_value_info::ConstraintValueInfo;
use blaze_analysis::platform::platform_info::PlatformInfo;
use blaze_analysis::provider_map::TransitiveInfoProviderMap;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::rule_class::RuleClassId;
use blaze_packages::starlark_defs::platform_common::CONSTRAINT_VALUE_INFO;
use blaze_packages::types::{AttrValue, Type};
use blaze_skyframe::{Environment, SkyKey};

use crate::producers::errors::{TargetAndConfiguration, TransitiveDependencyState};
use crate::producers::transition_applier::Produced;

/// Thrown if this target is platform-incompatible with the current build.
// Java: com/google/devtools/build/lib/analysis/constraints/IncompatibleTargetChecker.java:293 IncompatibleTargetChecker.IncompatibleTargetException
#[derive(Clone, Debug)]
pub struct IncompatibleTargetException {
    pub target: Arc<ConfiguredTargetValue>,
}

/// Java `PlatformProviderUtils.constraintValue(target)`.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformProviderUtils.java:78 PlatformProviderUtils.constraintValue
pub fn constraint_value(target: &ConfiguredTarget) -> Option<ConstraintValueInfo> {
    let info = target.get(&CONSTRAINT_VALUE_INFO.key())?;
    downcast_info::<ConstraintValueInfo>(&info).cloned()
}

/// Creates an incompatible configured target if it is "directly incompatible": because of
/// constraints or `config_settings` in its `target_compatible_with` attribute (Java
/// `IncompatibleTargetChecker.IncompatibleTargetProducer`).
///
/// `Ok(Some(Ok(None)))`: the target is not directly incompatible; analysis can continue.
// Java: com/google/devtools/build/lib/analysis/constraints/IncompatibleTargetChecker.java:120 IncompatibleTargetProducer.step
pub fn produce_incompatible_target(
    env: &mut Environment<'_>,
    target_and_configuration: &TargetAndConfiguration,
    configured_target_key: &ConfiguredTargetKey,
    config_conditions: &ConfigConditions,
    platform_info: Option<&PlatformInfo>,
    transitive_state: &mut TransitiveDependencyState,
) -> Produced<Result<Option<Arc<ConfiguredTargetValue>>, ValidationException>> {
    let rule = target_and_configuration.get_target().get_associated_rule();
    let (Some(rule), Some(platform_info)) = (rule, platform_info) else {
        return Ok(Some(Ok(None)));
    };
    if !rule.use_toolchain_resolution() {
        return Ok(Some(Ok(None)));
    }

    let configuration = target_and_configuration
        .get_configuration()
        .expect("configured rule");
    // Retrieves the label list for the target_compatible_with attribute.
    let attrs = ConfiguredAttributeMapper::of(
        rule.clone(),
        Arc::new(config_conditions.as_providers().clone()),
        configuration,
    );
    if !attrs.has_typed("target_compatible_with", Type::LabelList) {
        return Ok(Some(Ok(None)));
    }

    // Resolves the constraint labels, checking for invalid configured attributes.
    let target_compatible_with: Vec<Label> =
        match attrs.get_and_validate("target_compatible_with", Type::LabelList) {
            Err(e) => return Ok(Some(Err(e))),
            Ok(Some(AttrValue::LabelList(l))) => l,
            Ok(_) => Vec::new(),
        };
    let keys: Vec<ConfiguredTargetKey> = target_compatible_with
        .iter()
        .map(|label| {
            ConfiguredTargetKey::builder()
                .set_label(label.clone())
                .set_configuration(Some(&**configuration))
                .build()
        })
        .collect();
    let sky_keys: Vec<SkyKey> = keys.iter().cloned().map(SkyKey::new).collect();
    let lookup = env.get_values_and_exceptions(&sky_keys)?;

    let mut all_constraint_values: Vec<ConstraintValueInfo> = Vec::new();
    let mut invalid_constraint_values: Vec<ConstraintValueInfo> = Vec::new();
    let mut config_settings: Vec<ConfigMatchingProvider> = Vec::new();
    let mut missing = false;
    for key in &keys {
        let Some(value) = lookup.get_typed(key) else {
            missing = true;
            continue;
        };
        // Java: com/google/devtools/build/lib/analysis/constraints/IncompatibleTargetChecker.java:152 IncompatibleTargetProducer.accept
        let configured_target = value.get_configured_target();
        if let Some(info) = constraint_value(configured_target) {
            if !platform_info.constraints().has_constraint_value(&info) {
                invalid_constraint_values.push(info.clone());
            }
            all_constraint_values.push(info);
        }
        if let Some(config_setting) = configured_target.get_provider::<ConfigMatchingProvider>() {
            config_settings.push(ConfigMatchingProvider::create(
                configured_target.get_original_label(),
                config_setting.settings_map().to_vec(),
                config_setting.flag_settings_map().clone(),
                config_setting.constraint_values_setting().clone(),
                config_setting.result().clone(),
            ));
        }
    }
    if missing {
        return Ok(None);
    }

    // Java: com/google/devtools/build/lib/analysis/constraints/IncompatibleTargetChecker.java:175 IncompatibleTargetProducer.processResult
    // Validate that there are no duplicate constraint values from the same constraint setting
    if let Err(e) = ConstraintCollection::validate_constraints(&all_constraint_values) {
        return Ok(Some(Err(ValidationException(e.to_string()))));
    }

    let config_setting_results = ConfigMatchingProvider::accumulate_match_results(&config_settings);
    if !config_setting_results.errors.is_empty() {
        return Ok(Some(Err(ValidationException(
            format_config_setting_errors(&config_setting_results),
        ))));
    }

    if !invalid_constraint_values.is_empty() || !config_setting_results.non_matching.is_empty() {
        return Ok(Some(Ok(Some(create_incompatible_rule_configured_target(
            configured_target_key,
            configuration,
            config_conditions,
            IncompatiblePlatformProvider::incompatible_due_to_constraints_and_config_settings(
                Some(platform_info.label().clone()),
                invalid_constraint_values,
                config_setting_results.non_matching.clone(),
            ),
            rule.get_rule_class_object().get_rule_class_id().clone(),
            target_and_configuration,
            transitive_state,
        )))));
    }
    Ok(Some(Ok(None)))
}

// Java: com/google/devtools/build/lib/analysis/constraints/IncompatibleTargetChecker.java:217 IncompatibleTargetProducer.formatConfigSettingErrors
fn format_config_setting_errors(config_setting_results: &AccumulateResults) -> String {
    config_setting_results
        .errors
        .iter()
        .map(|(label, errors)| format!("For config_setting {}: {}", label, errors.join(", ")))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Rules where it doesn't make sense to check for platform compatibility.
// Java: com/google/devtools/build/lib/analysis/constraints/IncompatibleTargetChecker.java:230 IncompatibleTargetChecker.NO_COMPATIBILITY_CHECK_RULES
const NO_COMPATIBILITY_CHECK_RULES: &[&str] = &["toolchain", "config_setting", "label_flag"];

/// Java `RuleContextConstraintSemantics.checkForIncompatibility(target).isIncompatible()`.
// Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:853 RuleContextConstraintSemantics.checkForIncompatibility
pub fn is_incompatible(target: &ConfiguredTarget) -> bool {
    let target = match target {
        // For generated files, we want to query the generating rule for providers. genrule() for
        // example doesn't attach providers like IncompatiblePlatformProvider to its outputs.
        ConfiguredTarget::OutputFile(o) => ConfiguredTarget::Rule(o.get_generating_rule().clone()),
        other => other.clone(),
    };
    target.get(&INCOMPATIBLE_PLATFORM_PROVIDER.key()).is_some()
}

/// Creates an incompatible target if it is "indirectly incompatible": one of its dependencies
/// is incompatible. `None`: the target is not indirectly incompatible; analysis can continue.
// Java: com/google/devtools/build/lib/analysis/constraints/IncompatibleTargetChecker.java:251 IncompatibleTargetChecker.createIndirectlyIncompatibleTarget
pub fn create_indirectly_incompatible_target(
    target_and_configuration: &TargetAndConfiguration,
    configured_target_key: &ConfiguredTargetKey,
    dep_value_map: &PrerequisiteMap,
    config_conditions: &ConfigConditions,
    platform_info: Option<&PlatformInfo>,
    transitive_state: &TransitiveDependencyState,
) -> Option<Arc<ConfiguredTargetValue>> {
    let target = target_and_configuration.get_target();
    let rule = target.get_associated_rule()?;

    if NO_COMPATIBILITY_CHECK_RULES.contains(&rule.get_rule_class()) {
        return None;
    }

    // Find all the incompatible dependencies.
    let incompatible_deps: Vec<ConfiguredTarget> = prerequisite_values(dep_value_map)
        .map(|d| d.get_configured_target())
        .filter(|dep| is_incompatible(dep))
        .cloned()
        .collect();
    if incompatible_deps.is_empty() {
        return None;
    }

    let configuration = target_and_configuration
        .get_configuration()
        .expect("configured rule");
    let platform_label = platform_info.map(|p| p.label().clone());
    Some(create_incompatible_rule_configured_target(
        configured_target_key,
        configuration,
        config_conditions,
        IncompatiblePlatformProvider::incompatible_due_to_targets(
            platform_label,
            incompatible_deps,
        ),
        rule.get_rule_class_object().get_rule_class_id().clone(),
        target_and_configuration,
        transitive_state,
    ))
}

/// Creates an incompatible target.
// Java: com/google/devtools/build/lib/analysis/constraints/IncompatibleTargetChecker.java:307 IncompatibleTargetChecker.createIncompatibleRuleConfiguredTarget
fn create_incompatible_rule_configured_target(
    configured_target_key: &ConfiguredTargetKey,
    configuration: &BuildConfigurationValue,
    config_conditions: &ConfigConditions,
    incompatible_platform_provider: IncompatiblePlatformProvider,
    rule_class_id: RuleClassId,
    target_and_configuration: &TargetAndConfiguration,
    transitive_state: &TransitiveDependencyState,
) -> Arc<ConfiguredTargetValue> {
    use blaze_actions::analysis::runfiles::Runfiles;
    use blaze_analysis::providers::file_provider::{
        FileProvider, FilesToRunProvider, RunfilesProvider,
    };

    // Create dummy instances of the necessary data for a configured target. None of this data
    // will actually be used because actions associated with incompatible targets must not be
    // evaluated.
    let mut provider_builder = TransitiveInfoProviderMap::builder();
    provider_builder
        .put_info(freeze_native_info(incompatible_platform_provider))
        .add_native(RunfilesProvider::simple(Runfiles::empty()))
        .add_native_arc(FileProvider::empty())
        .add_native_arc(FilesToRunProvider::empty())
        .add_native(
            blaze_analysis::constraints::supported_environments::SupportedEnvironments::empty(),
        );
    if configuration
        .get_fragment::<blaze_build_config::fragments::test_configuration::TestConfiguration>()
        .is_some()
    {
        // Create a dummy TestProvider instance so that other parts of the code base stay happy.
        // Even though this test will never execute, some code still expects the provider.
        blaze_util::unported!(
            "TestActionBuilder.createEmptyTestParams",
            "incompatible test target {}",
            configured_target_key.get_label()
        );
    }

    let configured_target =
        blaze_analysis::configuredtargets::RuleConfiguredTarget::new_incompatible(
            blaze_actions::ActionLookupKeyRef::new(configured_target_key.clone()),
            // Generates visibility for an incompatible target: we pretend that all incompatible
            // targets are public (Java `convertVisibility()` = `VisibilityProvider.PUBLIC_VISIBILITY`).
            blaze_analysis::providers::visibility::public_visibility(),
            /* is_created_in_symbolic_macro= */ false,
            provider_builder.build(),
            Arc::new(config_conditions.as_providers().clone()),
            // PORT: Blaze's RuleConfiguredTarget keeps the rule class name of the RuleClassId.
            rule_class_id.name,
        );
    Arc::new(ConfiguredTargetValue::new(
        ConfiguredTarget::Rule(Arc::new(configured_target)),
        Vec::new(),
        transitive_state.transitive_packages(),
        Some(target_and_configuration.get_target().clone()),
    ))
}
