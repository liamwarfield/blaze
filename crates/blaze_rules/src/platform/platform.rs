//! Port of `rules/platform/Platform.java`: defines a platform for execution contexts.

use blaze_analysis::config::config_matching_provider::ConfigMatchingProvider;
use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::info::freeze_native_info;
use blaze_analysis::platform::{PlatformInfo, PlatformInfoBuildError, platform_provider_utils};
use blaze_analysis::rule::{RuleContext, RuleErrorConsumer, RuleErrorException};
use blaze_cmdline::Label;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::types::Type;
use indexmap::IndexMap;

use crate::platform::common::build_with_empty_providers;

// Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:40 PlatformRule.CONSTRAINT_VALUES_ATTR
pub const CONSTRAINT_VALUES_ATTR: &str = "constraint_values";
// Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:41 PlatformRule.PARENTS_PLATFORM_ATTR
pub const PARENTS_PLATFORM_ATTR: &str = "parents";
// Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:42 PlatformRule.REMOTE_EXECUTION_PROPS_ATTR
pub const REMOTE_EXECUTION_PROPS_ATTR: &str = "remote_execution_properties";
// Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:43 PlatformRule.EXEC_PROPS_ATTR
pub const EXEC_PROPS_ATTR: &str = "exec_properties";
// Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:44 PlatformRule.FLAGS_ATTR
pub const FLAGS_ATTR: &str = "flags";
// Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:45 PlatformRule.REQUIRED_SETTINGS_ATTR
pub const REQUIRED_SETTINGS_ATTR: &str = "required_settings";
// Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:46 PlatformRule.MISSING_TOOLCHAIN_ERROR_ATTR
pub const MISSING_TOOLCHAIN_ERROR_ATTR: &str = "missing_toolchain_error";

/// Java `Platform` (the `platform` rule's factory).
// Java: com/google/devtools/build/lib/rules/platform/Platform.java:43 Platform
pub struct Platform;

impl RuleConfiguredTargetFactory for Platform {
    // Java: com/google/devtools/build/lib/rules/platform/Platform.java:45 Platform.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let mut platform_builder = PlatformInfo::builder();
        platform_builder.set_label(rule_context.get_label().clone());

        let parent_platforms: Vec<PlatformInfo> = platform_provider_utils::platforms(
            rule_context.get_prerequisites(PARENTS_PLATFORM_ATTR).iter(),
        );

        if parent_platforms.len() > 1 {
            return Err(rule_context
                .throw_with_attribute_error(
                    PARENTS_PLATFORM_ATTR,
                    &format!("{PARENTS_PLATFORM_ATTR} attribute must have a single value"),
                )
                .into());
        }
        let parent_platform = parent_platforms.into_iter().next();
        platform_builder.set_parent(parent_platform);

        // Add the declared constraints. Because setting the parent above also sets constraints,
        // these will override them.
        platform_builder.add_constraints(platform_provider_utils::constraint_values(
            rule_context
                .get_prerequisites(CONSTRAINT_VALUES_ATTR)
                .iter(),
        ));

        let remote_execution_properties: Option<String> = rule_context
            .attributes()
            .get(REMOTE_EXECUTION_PROPS_ATTR, Type::String)
            .and_then(|v| v.as_str().map(str::to_owned));
        platform_builder.set_remote_execution_properties(remote_execution_properties);

        let exec_properties: Option<IndexMap<String, String>> = rule_context
            .attributes()
            .get(EXEC_PROPS_ATTR, Type::StringDict)
            .and_then(|v| v.as_string_dict().cloned());
        if let Some(exec_properties) = exec_properties.filter(|p| !p.is_empty()) {
            platform_builder.set_exec_properties(exec_properties);
        }

        let flags: Option<Vec<String>> = rule_context
            .attributes()
            .get(FLAGS_ATTR, Type::StringList)
            .and_then(|v| v.as_string_list().map(<[String]>::to_vec));
        if let Some(flags) = flags.filter(|f| !f.is_empty()) {
            platform_builder.add_flags(flags);
        }
        let required_settings: Vec<ConfigMatchingProvider> = rule_context
            .get_prerequisites(REQUIRED_SETTINGS_ATTR)
            .iter()
            .map(|target| {
                // Java: ImmutableList rejects the null of a target without the provider.
                target
                    .get_provider::<ConfigMatchingProvider>()
                    .expect(
                        "NullPointerException: required_settings without ConfigMatchingProvider",
                    )
                    .clone()
            })
            .collect();
        platform_builder.add_required_settings(required_settings);

        let check_toolchain_types = rule_context
            .attributes()
            .get("check_toolchain_types", Type::Boolean)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if check_toolchain_types {
            let allowed_toolchain_types: Vec<Label> = rule_context
                .attributes()
                .get("allowed_toolchain_types", Type::NodepLabelList)
                .and_then(|v| v.as_label_list().map(<[Label]>::to_vec))
                .unwrap_or_default();
            platform_builder.check_toolchain_types(true);
            platform_builder.add_allowed_toolchain_types(allowed_toolchain_types);
        } else {
            platform_builder.check_toolchain_types(false);
        }

        let missing_toolchain_error_message: Option<String> = rule_context
            .attributes()
            .get(MISSING_TOOLCHAIN_ERROR_ATTR, Type::String)
            .and_then(|v| v.as_str().map(str::to_owned));
        platform_builder.set_missing_toolchain_error_message(missing_toolchain_error_message);

        let platform_info = match platform_builder.build() {
            Ok(p) => p,
            Err(PlatformInfoBuildError::DuplicateConstraint(e)) => {
                return Err(rule_context
                    .throw_with_attribute_error(CONSTRAINT_VALUES_ATTR, e.get_message())
                    .into());
            }
            Err(PlatformInfoBuildError::ExecProperties(e)) => {
                return Err(rule_context
                    .throw_with_attribute_error(EXEC_PROPS_ATTR, e.get_message())
                    .into());
            }
        };

        validate_refined_constraint_values(rule_context, &platform_info)?;

        build_with_empty_providers(rule_context, freeze_native_info(platform_info))
    }
}

/// Verifies that whenever a platform declares a non-default `constraint_value` for a
/// `constraint_setting` that refines another `constraint_value`, the refined `constraint_value`
/// is also declared on the platform (directly or via a parent).
///
/// The check is skipped for refining values that equal the `default_constraint_value` of their
/// setting, allowing a "no information" default that does not pull in the refined value.
// Java: com/google/devtools/build/lib/rules/platform/Platform.java:130 Platform.validateRefinedConstraintValues
fn validate_refined_constraint_values(
    rule_context: &RuleContext,
    platform_info: &PlatformInfo,
) -> Result<(), CreateConfiguredTargetError> {
    let constraints = platform_info.constraints();
    let direct_constraints = platform_provider_utils::constraint_values(
        rule_context
            .get_prerequisites(CONSTRAINT_VALUES_ATTR)
            .iter(),
    );
    let mut has_error = false;
    for constraint_value in &direct_constraints {
        let setting = constraint_value.constraint();
        if setting.default_constraint_value().as_ref() == Some(constraint_value) {
            // The default value is a "no information" value and does not refine anything.
            continue;
        }
        let Some(refined_value) = setting.refined_constraint_value() else {
            continue;
        };
        let actual_value = constraints.get(refined_value.constraint());
        if actual_value.as_ref() == Some(refined_value) {
            continue;
        }

        has_error = true;
        let main_repo_mapping = match rule_context
            .get_analysis_environment()
            .get_main_repo_mapping()
        {
            Ok(m) => Some(m.clone()),
            // Java: MissingDepException (a Skyframe restart); analysis computes the main
            // repository mapping before configured targets.
            Err(e) => blaze_util::unported!(
                "RuleContext.getAnalysisEnvironment().getMainRepoMapping",
                "{e:?}"
            ),
        };
        let main_repo_mapping = main_repo_mapping.as_ref();
        let self_label = rule_context.get_label().clone();
        match actual_value {
            None => {
                let message = format!(
                    "constraint_value {} refines {}, but platform {} does not set the latter. Fix with:\n  buildozer 'add {} {}' {}",
                    constraint_value
                        .label()
                        .get_shorthand_display_form(main_repo_mapping),
                    refined_value
                        .label()
                        .get_shorthand_display_form(main_repo_mapping),
                    self_label.get_shorthand_display_form(main_repo_mapping),
                    CONSTRAINT_VALUES_ATTR,
                    refined_value
                        .label()
                        .get_shorthand_display_form(main_repo_mapping),
                    self_label.get_shorthand_display_form(main_repo_mapping)
                );
                rule_context.attribute_error(CONSTRAINT_VALUES_ATTR, &message);
            }
            Some(actual_value) => {
                let message = format!(
                    "constraint_value {} refines {}, but platform {} sets the conflicting constraint_value {} for {}",
                    constraint_value
                        .label()
                        .get_shorthand_display_form(main_repo_mapping),
                    refined_value
                        .label()
                        .get_shorthand_display_form(main_repo_mapping),
                    self_label.get_shorthand_display_form(main_repo_mapping),
                    actual_value
                        .label()
                        .get_shorthand_display_form(main_repo_mapping),
                    refined_value
                        .constraint()
                        .label()
                        .get_shorthand_display_form(main_repo_mapping)
                );
                rule_context.attribute_error(CONSTRAINT_VALUES_ATTR, &message);
            }
        }
    }
    if has_error {
        return Err(RuleErrorException::empty().into());
    }
    Ok(())
}
