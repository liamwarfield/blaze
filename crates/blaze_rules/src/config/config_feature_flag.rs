//! Port of `rules/config/ConfigFeatureFlag.java`: the `config_feature_flag` configured target
//! factory.

use std::sync::Arc;

use crate::config::config_feature_flag_provider::{ConfigFeatureFlagProvider, ValidityPredicate};
use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::info::freeze_native_info;
use blaze_analysis::providers::file_provider::RunfilesProvider;
use blaze_analysis::rule::{RuleConfiguredTargetBuilder, RuleContext, RuleErrorConsumer};
use blaze_build_config::build_options::StarlarkOptionValue;
use blaze_build_config::feature_flag_value::FeatureFlagValue;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::types::Type;
use blaze_starlark::printer::quote;
use indexmap::{IndexMap, IndexSet};

/// The implementation of the config_feature_flag rule for defining custom flags for Android
/// rules.
// Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlag.java:49 ConfigFeatureFlag
pub struct ConfigFeatureFlag;

/// Java `Starlark.repr(List<String>)`.
fn repr_string_list<'a>(values: impl IntoIterator<Item = &'a String>) -> String {
    let elems: Vec<String> = values.into_iter().map(|v| quote(v)).collect();
    format!("[{}]", elems.join(", "))
}

impl RuleConfiguredTargetFactory for ConfigFeatureFlag {
    // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlag.java:116 ConfigFeatureFlag.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let specified_values: Vec<String> = rule_context
            .attributes()
            .get("allowed_values", Type::StringList)
            .and_then(|v| v.as_string_list().map(|s| s.to_vec()))
            .unwrap_or_default();
        let values: IndexSet<String> = specified_values.iter().cloned().collect();
        if values.len() != specified_values.len() {
            // ImmutableMultiset.copyOf(specifiedValues): counts in first-occurrence order.
            let mut grouped_values: IndexMap<&String, usize> = IndexMap::new();
            for value in &specified_values {
                *grouped_values.entry(value).or_insert(0) += 1;
            }
            let duplicates: Vec<&String> = grouped_values
                .iter()
                .filter(|(_, count)| **count > 1)
                .map(|(value, _)| *value)
                .collect();
            // This is a problem with attributes of config_feature_flag itself so throw error
            // here.
            rule_context.attribute_error(
                "allowed_values",
                &format!(
                    "cannot contain duplicates, but contained multiple of {}",
                    repr_string_list(duplicates)
                ),
            );
        }

        let default_value: Option<String> = if rule_context
            .attributes()
            .is_attribute_value_explicitly_specified("default_value")
        {
            Some(
                rule_context
                    .attributes()
                    .get("default_value", Type::String)
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .expect("NullPointerException: default_value"),
            )
        } else {
            None
        };
        if let Some(default) = &default_value {
            if !values.contains(default) {
                // This is a problem with attributes of config_feature_flag itself so throw error
                // here.
                rule_context.attribute_error(
                    "default_value",
                    &format!(
                        "must be one of {}, but was {}",
                        repr_string_list(&values),
                        quote(default)
                    ),
                );
            }
        }

        if rule_context.has_errors() {
            // Don't bother validating the value if the flag was already incorrectly specified
            // without looking at the value.
            return Ok(None);
        }

        let raw_starlark_value = rule_context
            .get_configuration()
            .get_options()
            .get_starlark_options()
            .get(rule_context.get_label())
            .cloned();
        let feature_flag_value = match raw_starlark_value {
            Some(StarlarkOptionValue::FeatureFlag(v)) => v,
            // Retain legacy behavior of treating feature flags that somehow are not
            // FeatureFlagValue as set to default
            _ => FeatureFlagValue::DefaultValue,
        };

        let Some(provider) =
            construct_provider(&feature_flag_value, &default_value, values, rule_context)
        else {
            return Ok(None);
        };

        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder
            .set_files_to_build(NestedSetBuilder::empty_set(Order::STABLE_ORDER))
            .add_provider(RunfilesProvider::empty())
            .add_native_declared_provider(freeze_native_info(provider));
        Ok(builder.build()?)
    }
}

/// Calculate and return a ConfigFeatureFlagProvider.
///
/// At this point any errors here are due to something being 'wrong' with the configuration. In
/// particular, this provider may be constructed BEFORE a rule transition sets the expected
/// configuration and thus want to defer errors until later in analysis when this value is
/// actually consumed as the errors would otherwise be unfixable.
///
/// An exception is made for if the value is explicitly set to value not in the allowed values
/// list (either on the commandline or as part of a previous transition). In that case, that is
/// an immediate error as can be fixed by ensuring those places set to allowed values. This is
/// consistent with the behavior of build_setting.
// Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlag.java:189 ConfigFeatureFlag.constructProvider
fn construct_provider(
    feature_flag_value: &FeatureFlagValue,
    default_value: &Option<String>,
    values: IndexSet<String>,
    rule_context: &RuleContext,
) -> Option<ConfigFeatureFlagProvider> {
    let values = Arc::new(values);
    let is_valid_value: ValidityPredicate = {
        let values = values.clone();
        Arc::new(move |v: &str| values.contains(v))
    };
    match feature_flag_value {
        FeatureFlagValue::SetValue(set_value) => {
            if !is_valid_value(set_value) {
                // This is consistent with build_setting, which also immediate checks that
                // explicitly set values are valid values.
                rule_context.rule_error(&format!(
                    "value must be one of {}, but was {}",
                    repr_string_list(values.iter()),
                    quote(set_value)
                ));
                return None;
            }
            Some(ConfigFeatureFlagProvider::create(
                Some(set_value.clone()),
                None,
                is_valid_value,
            ))
        }
        FeatureFlagValue::DefaultValue => {
            let Some(default) = default_value else {
                // Should defer error in case value is set by upcoming rule transition.
                // (Although, rule authors could just add a default.)
                // build_setting always has a default so this can't happen for them.
                return Some(ConfigFeatureFlagProvider::create(
                    None,
                    Some(format!(
                        "Feature flag {} has no default but no value was explicitly specified.",
                        rule_context.get_label()
                    )),
                    is_valid_value,
                ));
            };
            Some(ConfigFeatureFlagProvider::create(
                Some(default.clone()),
                None,
                is_valid_value,
            ))
        }
        FeatureFlagValue::UnknownValue => {
            // Must defer error in case value is set by upcoming rule transition.
            // build_setting doesn't have trimming logic so this can't happen for them.
            let label = rule_context.get_label();
            Some(ConfigFeatureFlagProvider::create(
                None,
                Some(format!(
                    "Feature flag {label} was accessed in a configuration it is not present in. All targets which depend on {label} directly or indirectly must name it in their transitive_configs attribute."
                )),
                is_valid_value,
            ))
        }
    }
}

/// Registers the factory.
pub fn factory() -> Arc<dyn RuleConfiguredTargetFactory> {
    Arc::new(ConfigFeatureFlag)
}
