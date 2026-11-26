//! Port of `analysis/RequiredFragmentsUtil.java`: what pieces of configuration a target or
//! aspect requires (`--include_config_fragments_provider`, read by cquery's
//! `--show_config_fragments`), and the `addRequiredFragments` methods of the configuration
//! transitions.

use std::sync::Arc;

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::fragment::FragmentClass;
use blaze_cmdline::label_constants::COMMAND_LINE_OPTION_PACKAGE_IDENTIFIER;
use blaze_optdefs::fragment::core_options::{CoreOptions, CoreOptionsIncludeConfigFragmentsEnum};
use blaze_packages::aspect::Aspect;
use blaze_packages::rule_class::configuration_fragment_policy::ConfigurationFragmentPolicy;
use blaze_packages::target::Rule;

use crate::config::build_option_details::BuildOptionDetails;
use crate::config::build_setting_provider::BuildSettingProvider;
use crate::config::config_conditions::ConfigConditions;
use crate::config::configured_attribute_mapper::ConfiguredAttributeMapper;
use crate::config::transitions::{
    AttributeTransitionData, ConfigurationTransition, RuleTransitionData,
    StarlarkAttributeTransitionProvider, create_attribute_transition, create_rule_transition,
};
use crate::configured_aspect_factory::ConfiguredAspectFactory;
use crate::configured_rule_class_provider::ConfiguredRuleClassProvider;
use crate::configured_target::ConfiguredTarget;
use crate::providers::required_config_fragments_provider::{
    RequiredConfigFragmentsProvider, RequiredConfigFragmentsProviderBuilder,
};

/// Java `ClassName.getSimpleNameWithOuter(clazz)`: the class name after its package.
// Java: com/google/devtools/build/lib/util/ClassName.java:26 ClassName.getSimpleNameWithOuter
pub fn get_simple_name_with_outer(java_name: &str) -> &str {
    match java_name.rfind('.') {
        Some(i) => &java_name[i + 1..],
        None => java_name,
    }
}

/// Returns a [`RequiredConfigFragmentsProvider`] identifying all pieces of configuration a
/// target requires, or `None` if required config fragments are not enabled (see
/// `CoreOptions.includeRequiredConfigFragmentsProvider`).
///
/// The returned config state includes things that are known to be required at the time when the
/// target's dependencies have already been analyzed but before it's been analyzed itself. See
/// `RuleConfiguredTargetBuilder.maybeAddRequiredConfigFragmentsProvider` for the remaining
/// pieces of config state.
///
/// `select()`s and toolchain dependencies are considered when looking at what config state is
/// required.
// Java: com/google/devtools/build/lib/analysis/RequiredFragmentsUtil.java:86 RequiredFragmentsUtil.getRuleRequiredFragmentsIfEnabled
pub fn get_rule_required_fragments_if_enabled<'a>(
    target: &Arc<Rule>,
    configuration: &BuildConfigurationValue,
    universally_required_fragments: &[FragmentClass],
    config_conditions: &ConfigConditions,
    prerequisites: impl IntoIterator<Item = &'a ConfiguredTarget>,
    starlark_exec_transition: Option<Arc<StarlarkAttributeTransitionProvider>>,
    rule_class_provider: &ConfiguredRuleClassProvider,
) -> Option<RequiredConfigFragmentsProvider> {
    let mode = get_required_fragments_mode(configuration);
    if mode == CoreOptionsIncludeConfigFragmentsEnum::OFF {
        return None;
    }
    let rule_class = target.get_rule_class_object();
    let attributes = ConfiguredAttributeMapper::of(
        target.clone(),
        Arc::new(config_conditions.as_providers().clone()),
        configuration,
    );
    let mut required_fragments = get_required_fragments(
        mode,
        configuration,
        universally_required_fragments,
        rule_class.get_configuration_fragment_policy(),
        config_conditions,
        prerequisites,
    );
    if !rule_class.is_starlark() {
        // ruleClass.getConfiguredTargetFactory(RuleConfiguredTargetFactory.class)
        if let Some(factory) = rule_class
            .get_configured_target_factory()
            .and_then(|name| rule_class_provider.get_configured_target_factory(name))
        {
            factory.add_rule_impl_specific_required_config_fragments(
                &mut required_fragments,
                &attributes,
                configuration,
            );
        }
    }
    let option_details = BuildOptionDetails::for_options(configuration.get_options());
    add_required_fragments_from_rule_transitions(
        config_conditions,
        configuration.checksum(),
        &mut required_fragments,
        target,
        &attributes,
        &option_details,
        starlark_exec_transition,
    );

    // We consider build settings (which are both targets and configuration) to require
    // themselves.
    if target.is_build_setting() {
        required_fragments.add_starlark_option(target.get_label().clone());
    }

    Some(required_fragments.build())
}

/// Variation of [`get_rule_required_fragments_if_enabled`] for aspects.
// Java: com/google/devtools/build/lib/analysis/RequiredFragmentsUtil.java:145 RequiredFragmentsUtil.getAspectRequiredFragmentsIfEnabled
pub fn get_aspect_required_fragments_if_enabled<'a>(
    aspect: &Aspect,
    aspect_factory: &dyn ConfiguredAspectFactory,
    associated_target: &Arc<Rule>,
    configuration: &BuildConfigurationValue,
    universally_required_fragments: &[FragmentClass],
    config_conditions: &ConfigConditions,
    prerequisites: impl IntoIterator<Item = &'a ConfiguredTarget>,
    starlark_exec_transition: Option<Arc<StarlarkAttributeTransitionProvider>>,
) -> Option<RequiredConfigFragmentsProvider> {
    let mode = get_required_fragments_mode(configuration);
    if mode == CoreOptionsIncludeConfigFragmentsEnum::OFF {
        return None;
    }
    let mut required_fragments = get_required_fragments(
        mode,
        configuration,
        universally_required_fragments,
        aspect.get_definition().get_configuration_fragment_policy(),
        config_conditions,
        prerequisites,
    );
    aspect_factory.add_aspect_impl_specific_required_config_fragments(&mut required_fragments);
    let option_details = BuildOptionDetails::for_options(configuration.get_options());
    add_required_fragments_from_aspect_transitions(
        &mut required_fragments,
        aspect,
        &ConfiguredAttributeMapper::of(
            associated_target.clone(),
            Arc::new(config_conditions.as_providers().clone()),
            configuration,
        ),
        &option_details,
        starlark_exec_transition,
    );
    Some(required_fragments.build())
}

/// Internal implementation that handles requirements common to both rules and aspects.
// Java: com/google/devtools/build/lib/analysis/RequiredFragmentsUtil.java:174 RequiredFragmentsUtil.getRequiredFragments
fn get_required_fragments<'a>(
    mode: CoreOptionsIncludeConfigFragmentsEnum,
    configuration: &BuildConfigurationValue,
    universally_required_fragments: &[FragmentClass],
    configuration_fragment_policy: &ConfigurationFragmentPolicy,
    config_conditions: &ConfigConditions,
    prerequisites: impl IntoIterator<Item = &'a ConfiguredTarget>,
) -> RequiredConfigFragmentsProviderBuilder {
    let mut required_fragments = RequiredConfigFragmentsProvider::builder();

    if mode == CoreOptionsIncludeConfigFragmentsEnum::TRANSITIVE {
        // Add transitive requirements first, which results in better performance. See
        // explanation on RequiredConfigFragmentsProvider.Builder.
        add_transitively_required_fragments(&mut required_fragments, prerequisites);
    } else {
        add_starlark_build_settings(&mut required_fragments, prerequisites);
    }

    // Add directly required fragments:
    // Fragments explicitly required by the native target/aspect definition API:
    required_fragments.add_fragment_classes(
        configuration_fragment_policy
            .get_required_configuration_fragments()
            .iter()
            .map(|s| s.as_str()),
    );
    // Fragments explicitly required by the Starlark target/aspect definition API (nulls are
    // filtered because the rule definition may reference non-existent fragments):
    let starlark_fragments: Vec<FragmentClass> = configuration_fragment_policy
        .get_required_starlark_fragments()
        .iter()
        .filter_map(|name| configuration.get_starlark_fragment_by_name(name))
        .collect();
    required_fragments.add_fragment_classes(
        starlark_fragments
            .iter()
            .map(|f| get_simple_name_with_outer(f.get_name())),
    );
    // Fragments universally required by everything:
    required_fragments.add_fragment_classes(
        universally_required_fragments
            .iter()
            .map(|f| get_simple_name_with_outer(f.get_name())),
    );
    // Fragments required by attached select()s. Propagating fragments from the config
    // conditions as configured targets (rather than as providers) is necessary in case of a
    // dependency on an alias that resolves to a config setting. Providers only reflect the
    // resolved settings, which won't include fragments required to resolve a select within the
    // alias rule (b/237534193).
    for target_and_data in config_conditions.as_configured_targets().values() {
        required_fragments.merge(
            target_and_data
                .get_configured_target()
                .get_provider::<RequiredConfigFragmentsProvider>(),
        );
    }
    required_fragments
}

/// Adds required fragments from transitions "attached" to a target.
///
/// "Attached" means the transition is attached to the target itself or one of its attributes.
///
/// These are the transitions required for a target to successfully analyze. Technically,
/// transitions attached to the target are evaluated during its parent's analysis, which is where
/// the configuration for the child is determined. We still consider these the child's
/// requirements because the child's properties determine that dependency.
// Java: com/google/devtools/build/lib/analysis/RequiredFragmentsUtil.java:230 RequiredFragmentsUtil.addRequiredFragmentsFromRuleTransitions
fn add_required_fragments_from_rule_transitions(
    config_conditions: &ConfigConditions,
    config_hash: &str,
    required_fragments: &mut RequiredConfigFragmentsProviderBuilder,
    target: &Arc<Rule>,
    attribute_map: &ConfiguredAttributeMapper,
    option_details: &BuildOptionDetails,
    starlark_exec_transition: Option<Arc<StarlarkAttributeTransitionProvider>>,
) {
    add_required_fragments(
        &create_rule_transition(
            target.get_rule_class_object().get_transition_factory(),
            &RuleTransitionData::create(
                target.clone(),
                Arc::new(config_conditions.as_providers().clone()),
                config_hash.to_owned(),
            ),
        ),
        required_fragments,
        option_details,
    );
    // We don't set the execution platform in this data because a) that doesn't affect which
    // fragments are required and b) it's one less parameter we have to pass to
    // RequiredFragmenstUtil's public interface.
    let attribute_transition_data = AttributeTransitionData {
        attributes: Some(attribute_map),
        execution_platform: None,
        analysis_data: starlark_exec_transition,
    };
    for attribute in target
        .get_rule_class_object()
        .get_attribute_provider()
        .get_attributes()
    {
        // attribute.getTransitionFactory() != null: every attribute has one (NoTransition by
        // default). TransitionCreationException is a RuntimeException.
        let transition = create_attribute_transition(
            attribute.get_transition_factory(),
            &attribute_transition_data,
        )
        .unwrap_or_else(|e| panic!("TransitionCreationException: {}", e.0));
        add_required_fragments(&transition, required_fragments, option_details);
    }
}

/// Adds required fragments from transitions "attached" to an aspect.
///
/// "Attached" means the transition is attached to one of the aspect's attributes. Transitions
/// can't be attached directly to aspects themselves.
// Java: com/google/devtools/build/lib/analysis/RequiredFragmentsUtil.java:267 RequiredFragmentsUtil.addRequiredFragmentsFromAspectTransitions
fn add_required_fragments_from_aspect_transitions(
    required_fragments: &mut RequiredConfigFragmentsProviderBuilder,
    aspect: &Aspect,
    attribute_map: &ConfiguredAttributeMapper,
    option_details: &BuildOptionDetails,
    starlark_exec_transition: Option<Arc<StarlarkAttributeTransitionProvider>>,
) {
    let attribute_transition_data = AttributeTransitionData {
        attributes: Some(attribute_map),
        execution_platform: None,
        analysis_data: starlark_exec_transition,
    };
    for attribute in aspect.get_definition().get_attributes().values() {
        let transition = create_attribute_transition(
            attribute.get_transition_factory(),
            &attribute_transition_data,
        )
        .unwrap_or_else(|e| panic!("TransitionCreationException: {}", e.0));
        add_required_fragments(&transition, required_fragments, option_details);
    }
}

// Java: com/google/devtools/build/lib/analysis/RequiredFragmentsUtil.java:288 RequiredFragmentsUtil.addTransitivelyRequiredFragments
fn add_transitively_required_fragments<'a>(
    required_fragments: &mut RequiredConfigFragmentsProviderBuilder,
    prerequisites: impl IntoIterator<Item = &'a ConfiguredTarget>,
) {
    for prereq in prerequisites {
        let dep_provider = prereq.get_provider::<RequiredConfigFragmentsProvider>();
        if dep_provider.is_some() {
            required_fragments.merge(dep_provider);
        }
    }
}

/// Adds dependencies on Starlark build settings.
///
/// Starlark build settings are considered direct requirements on the rule even though they are
/// technically dependencies. Note that this method only needs to be called in DIRECT mode, since
/// TRANSITIVE mode will already pick up these requirements from dependencies.
// Java: com/google/devtools/build/lib/analysis/RequiredFragmentsUtil.java:308 RequiredFragmentsUtil.addStarlarkBuildSettings
fn add_starlark_build_settings<'a>(
    required_fragments: &mut RequiredConfigFragmentsProviderBuilder,
    prerequisites: impl IntoIterator<Item = &'a ConfiguredTarget>,
) {
    for prereq in prerequisites {
        if let Some(build_setting_provider) = prereq.get_provider::<BuildSettingProvider>() {
            required_fragments.add_starlark_option(build_setting_provider.get_label().clone());
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/RequiredFragmentsUtil.java:318 RequiredFragmentsUtil.getRequiredFragmentsMode
fn get_required_fragments_mode(
    config: &BuildConfigurationValue,
) -> CoreOptionsIncludeConfigFragmentsEnum {
    config
        .get_options()
        .get::<CoreOptions>()
        .expect("CoreOptions")
        .include_required_config_fragments_provider
}

/// Java `ConfigurationTransition.addRequiredFragments` and its overrides.
// Java: com/google/devtools/build/lib/analysis/config/transitions/ConfigurationTransition.java:56 ConfigurationTransition.addRequiredFragments
pub fn add_required_fragments(
    transition: &ConfigurationTransition,
    required_fragments: &mut RequiredConfigFragmentsProviderBuilder,
    option_details: &BuildOptionDetails,
) {
    match transition {
        // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:154 ComposingTransition.addRequiredFragments
        ConfigurationTransition::Composing(transition1, transition2) => {
            // At first glance this code looks wrong. A composing transition applies transition2
            // over transition1's outputs, not the original options. We don't have to worry
            // about that here because the reason we pass the options is so Starlark transitions
            // can map individual flags like "//command_line_option:copts" to the fragments that
            // own them. This doesn't depend on the flags' values. This is fortunate, because it
            // producers simpler, faster code and cleaner interfaces.
            add_required_fragments(transition1, required_fragments, option_details);
            add_required_fragments(transition2, required_fragments, option_details);
        }
        // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkTransition.java:68 StarlarkTransition.addRequiredFragments
        ConfigurationTransition::Starlark(t) => {
            for option in t.get_inputs().into_iter().chain(t.get_outputs()) {
                if option.get_package_identifier() != &*COMMAND_LINE_OPTION_PACKAGE_IDENTIFIER {
                    required_fragments.add_starlark_option(option);
                } else {
                    let option_native_name = option.get_name();
                    // A null optionsClass means the flag is invalid. Starlark transitions
                    // independently catch and report that (search the code for "do not
                    // correspond to valid settings").
                    if let Some(options_class) = option_details.get_option_class(option_native_name)
                    {
                        required_fragments.add_options_class(get_simple_name_with_outer(
                            options_class.get_name(),
                        ));
                    }
                }
            }
        }
        // The default: requiredFragments.addOptionsClasses(requiresOptionFragments()).
        other => {
            let classes = other.requires_option_fragments();
            required_fragments.add_options_classes(
                classes
                    .iter()
                    .map(|c| get_simple_name_with_outer(c.get_name())),
            );
        }
    }
}
