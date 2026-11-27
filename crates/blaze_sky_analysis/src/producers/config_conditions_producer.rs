//! Port of `analysis/producers/ConfigConditionsProducer.java`.

use blaze_analysis::config::config_conditions::ConfigConditions;
use blaze_analysis::config::config_matching_provider::ConfigMatchingProvider;
use blaze_analysis::configured_target_and_data::ConfiguredTargetAndData;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::platform_info::PlatformInfo;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_cmdline::Label;
use blaze_packages::attribute::{AttributeMap, RawAttributeMapper};
use blaze_packages::exceptions::DetailedExitCode;
use blaze_packages::rule_class::rule_class::CONFIG_SETTING_DEPS_ATTRIBUTE;
use blaze_packages::target::{RuleOrMacroInstance, Target};
use blaze_packages::types::{AttrValue, Type};
use blaze_skyframe::Environment;
use indexmap::IndexMap;

use crate::producers::configured_target_and_data_producer::{
    ConfiguredTargetAndDataRequest, produce_configured_targets_and_data,
};
use crate::producers::errors::{
    ConfiguredTargetLookupError, ConfiguredValueCreationException, TransitiveDependencyState,
    choose_more_important_with_first_if_tie,
};
use crate::producers::transition_applier::Produced;

/// Computes the config conditions (`select()` keys) of a target in its configuration (Java
/// `ConfigConditionsProducer`, driven to completion).
// Java: com/google/devtools/build/lib/analysis/producers/ConfigConditionsProducer.java:92 ConfigConditionsProducer.step
// Java: com/google/devtools/build/lib/analysis/producers/ConfigConditionsProducer.java:70 ConfigConditionsProducer.<init>
// Java: com/google/devtools/build/lib/analysis/producers/ConfigConditionsProducer.java:44 ConfigConditionsProducer.ResultSink.acceptConfigConditions
// Java: com/google/devtools/build/lib/analysis/producers/ConfigConditionsProducer.java:46 ConfigConditionsProducer.ResultSink.acceptConfigConditionsError
// PORT: the result sink is the return value.
pub fn produce_config_conditions(
    env: &mut Environment<'_>,
    target: &Target,
    target_label: &Label,
    build_configuration_key: Option<&BuildConfigurationKey>,
    target_platform_info: Option<&PlatformInfo>,
    transitive_state: &mut TransitiveDependencyState,
) -> Produced<Result<ConfigConditions, ConfiguredValueCreationException>> {
    let Some(config_labels) = compute_config_labels(target) else {
        return Ok(Some(Ok(ConfigConditions::empty())));
    };

    // Collect the actual deps without a configuration transition (since by definition config
    // conditions evaluate over the current target's configuration). If the dependency is
    // (erroneously) something that needs the null configuration, its analysis will be
    // short-circuited. That error will be reported later.
    let requests: Vec<ConfiguredTargetAndDataRequest> = config_labels
        .iter()
        .map(|label| ConfiguredTargetAndDataRequest {
            key: ConfiguredTargetKey::builder()
                .set_label(label.clone())
                .set_configuration_key(build_configuration_key.cloned())
                .build(),
            transition_keys: Vec::new(),
        })
        .collect();
    let outcomes = produce_configured_targets_and_data(
        env,
        &requests,
        transitive_state,
        /* base_target_prerequisites_supplier= */ None,
    )?;

    let mut most_important_exit_code: Option<DetailedExitCode> = None;
    let mut error: Option<ConfiguredValueCreationException> = None;
    let mut prerequisites: Vec<Option<ConfiguredTargetAndData>> = Vec::new();
    let mut missing = false;
    for outcome in outcomes {
        match outcome {
            None => {
                missing = true;
                prerequisites.push(None);
            }
            // Java: com/google/devtools/build/lib/analysis/producers/ConfigConditionsProducer.java:119 ConfigConditionsProducer.acceptConfiguredTargetAndData
            Some(Ok(v)) => prerequisites.push(Some(v)),
            Some(Err(e)) => {
                prerequisites.push(None);
                let new_exit_code = match &e {
                    // Java: com/google/devtools/build/lib/analysis/producers/ConfigConditionsProducer.java:124 ConfigConditionsProducer.acceptConfiguredTargetAndDataError
                    ConfiguredTargetLookupError::Creation(c) => c.get_detailed_exit_code().clone(),
                    // Java: com/google/devtools/build/lib/analysis/producers/ConfigConditionsProducer.java:129 ConfigConditionsProducer.acceptConfiguredTargetAndDataError
                    ConfiguredTargetLookupError::NoSuchThing(n) => n.get_detailed_exit_code(),
                    // Java: com/google/devtools/build/lib/analysis/producers/ConfigConditionsProducer.java:134 ConfigConditionsProducer.acceptConfiguredTargetAndDataError
                    ConfiguredTargetLookupError::InconsistentNullConfig(_) => {
                        // A config label was evaluated with a null configuration. This should
                        // never happen as ConfigConditions are only present if the parent is a
                        // Rule, then always evaluated with the parent configuration.
                        panic!(
                            "IllegalArgumentException: ConfigCondition dependency should never be evaluated with a null configuration."
                        );
                    }
                };
                // Java: com/google/devtools/build/lib/analysis/producers/ConfigConditionsProducer.java:200 ConfigConditionsProducer.emitErrorIfMostImportant
                most_important_exit_code = choose_more_important_with_first_if_tie(
                    Some(new_exit_code.clone()),
                    most_important_exit_code,
                );
                if most_important_exit_code.as_ref() == Some(&new_exit_code) {
                    // The precise error is reported by the dependency that failed to load.
                    // TODO(gregce): beautify this error:
                    // https://github.com/bazelbuild/bazel/issues/11984.
                    error = Some(ConfiguredValueCreationException::for_target(
                        target,
                        format!(
                            "errors encountered resolving select() keys for {}",
                            target_label
                        ),
                    ));
                }
            }
        }
    }
    if let Some(error) = error {
        return Ok(Some(Err(error)));
    }
    if missing {
        return Ok(None);
    }

    // Java: com/google/devtools/build/lib/analysis/producers/ConfigConditionsProducer.java:142 ConfigConditionsProducer.constructConfigConditions
    let mut as_configured_targets: IndexMap<Label, ConfiguredTargetAndData> = IndexMap::new();
    let mut as_config_conditions: IndexMap<Label, ConfigMatchingProvider> = IndexMap::new();
    for (label, prerequisite) in config_labels.iter().zip(prerequisites) {
        let prerequisite = prerequisite.expect("present after a complete lookup");
        match ConfigConditions::from_configured_target(&prerequisite, target_platform_info) {
            Ok(p) => {
                put_or_throw(&mut as_config_conditions, label.clone(), p);
            }
            Err(_) => {
                let message = format!(
                    "{} is not a valid select() condition for {}.\n",
                    prerequisite.get_target_label(),
                    target_label
                ) + &format!(
                    "To inspect the select(), run: bazel query --output=build {}.\n",
                    target_label
                ) + "For more help, see https://bazel.build/reference/be/functions#select.\n\n";
                return Ok(Some(Err(ConfiguredValueCreationException::for_target(
                    target, message,
                ))));
            }
        }
        put_or_throw(&mut as_configured_targets, label.clone(), prerequisite);
    }
    Ok(Some(Ok(ConfigConditions::create(
        as_configured_targets,
        as_config_conditions,
    ))))
}

/// `ImmutableMap.Builder.put` + `buildOrThrow`.
fn put_or_throw<V>(map: &mut IndexMap<Label, V>, key: Label, value: V) {
    if map.contains_key(&key) {
        panic!("IllegalArgumentException: Multiple entries with same key: {key}");
    }
    map.insert(key, value);
}

/// Computes the config labels belonging to the given target; `None` if there were none
/// (implying a `ConfigConditions.EMPTY` result).
// Java: com/google/devtools/build/lib/analysis/producers/ConfigConditionsProducer.java:180 ConfigConditionsProducer.computeConfigLabels
pub fn compute_config_labels(target: &Target) -> Option<Vec<Label>> {
    let rule = target.get_associated_rule()?;

    let attrs = RawAttributeMapper::of(&**rule as &dyn RuleOrMacroInstance);
    if !attrs.has(CONFIG_SETTING_DEPS_ATTRIBUTE) {
        return None;
    }

    // Collects the labels of the configured targets we need to resolve.
    let config_labels = match attrs.get(CONFIG_SETTING_DEPS_ATTRIBUTE, Type::LabelList) {
        Some(AttrValue::LabelList(l)) => l,
        _ => Vec::new(),
    };
    if config_labels.is_empty() {
        return None;
    }
    Some(config_labels)
}
