//! Ports of `rules/Alias.java` (the `alias` factory) and `rules/LateBoundAlias.java` (the
//! factory of `label_flag`, `label_setting` and the native late-bound aliases).

use std::sync::Arc;

use blaze_actions::analysis::runfiles::Runfiles;
use blaze_analysis::config::build_setting_provider::BuildSettingProvider;
use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configuredtargets::AliasConfiguredTarget;
use blaze_analysis::provider_map::TransitiveInfoProviderMap;
use blaze_analysis::providers::alias_provider::LateBoundAliasProvider;
use blaze_analysis::providers::file_provider::RunfilesProvider;
use blaze_analysis::rule::{RuleConfiguredTargetBuilder, RuleContext, RuleErrorConsumer};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::rule_class::rule_class::STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME;

/// The `alias` factory (Java `Alias`).
// Java: com/google/devtools/build/lib/rules/Alias.java:39 Alias
pub struct Alias;

impl Alias {
    // Java: com/google/devtools/build/lib/rules/Alias.java:41 Alias.RULE_NAME
    pub const RULE_NAME: &'static str = "alias";
    // Java: com/google/devtools/build/lib/rules/Alias.java:42 Alias.ACTUAL_ATTRIBUTE_NAME
    const ACTUAL_ATTRIBUTE_NAME: &'static str = "actual";
}

impl RuleConfiguredTargetFactory for Alias {
    // Java: com/google/devtools/build/lib/rules/Alias.java:45 Alias.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let actual = rule_context
            .get_prerequisite(Self::ACTUAL_ATTRIBUTE_NAME)
            .expect("NullPointerException: mandatory 'actual' prerequisite");

        // TODO(b/129045294): Remove the logic below completely when repo is deleted.
        if rule_context
            .get_label()
            .get_canonical_form()
            .starts_with("@bazel_tools//platforms")
        {
            return Err(rule_context
                .throw_with_rule_error(
                    "Constraints from @bazel_tools//platforms have been removed. Please use constraints from @platforms repository embedded in Bazel, or preferably declare dependency on https://github.com/bazelbuild/platforms. See https://github.com/bazelbuild/bazel/issues/8622 for details.",
                )
                .into());
        }

        let visibility = rule_context.get_visibility().clone();
        Ok(Some(ConfiguredTarget::Alias(Arc::new(
            AliasConfiguredTarget::create(rule_context, actual, visibility),
        ))))
    }
}

/// The factory of late-bound alias rules (Java `LateBoundAlias`): `label_flag`,
/// `label_setting`, and the native aliases reading a configuration fragment.
// Java: com/google/devtools/build/lib/rules/LateBoundAlias.java:43 LateBoundAlias
pub struct LateBoundAlias;

impl LateBoundAlias {
    // Java: com/google/devtools/build/lib/rules/LateBoundAlias.java:45 LateBoundAlias.ATTRIBUTE_NAME
    const ATTRIBUTE_NAME: &'static str = ":alias";

    // Java: com/google/devtools/build/lib/rules/LateBoundAlias.java:79 LateBoundAlias.createEmptyConfiguredTarget
    fn create_empty_configured_target(
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder
            .add_provider(RunfilesProvider::simple(Runfiles::empty()))
            .add_provider(LateBoundAliasProvider);
        Ok(builder.build()?)
    }
}

impl RuleConfiguredTargetFactory for LateBoundAlias {
    // Java: com/google/devtools/build/lib/rules/LateBoundAlias.java:48 LateBoundAlias.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let Some(actual) = rule_context.get_prerequisite(Self::ATTRIBUTE_NAME) else {
            return Self::create_empty_configured_target(rule_context);
        };

        let mut overrides = TransitiveInfoProviderMap::builder();
        overrides.add_native(LateBoundAliasProvider);

        let visibility = rule_context.get_visibility().clone();
        if !rule_context.get_rule().is_build_setting() {
            return Ok(Some(ConfiguredTarget::Alias(Arc::new(
                AliasConfiguredTarget::create_with_overrides(
                    rule_context,
                    actual,
                    visibility,
                    overrides.build(),
                ),
            ))));
        }

        // This makes label_setting and label_flag work with select().
        let build_setting = rule_context
            .get_rule()
            .get_rule_class_object()
            .get_build_setting()
            .expect("build setting")
            .clone();
        let default_value = rule_context.attributes().get(
            STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME,
            build_setting.get_type(),
        );
        let label = rule_context.get_label().clone();
        overrides.add_native(BuildSettingProvider::new(
            build_setting,
            default_value,
            label,
        ));
        Ok(Some(ConfiguredTarget::Alias(Arc::new(
            AliasConfiguredTarget::create_with_overrides(
                rule_context,
                actual,
                visibility,
                overrides.build(),
            ),
        ))))
    }
}

pub fn alias_factory() -> Arc<dyn RuleConfiguredTargetFactory> {
    Arc::new(Alias)
}

pub fn late_bound_alias_factory() -> Arc<dyn RuleConfiguredTargetFactory> {
    Arc::new(LateBoundAlias)
}
