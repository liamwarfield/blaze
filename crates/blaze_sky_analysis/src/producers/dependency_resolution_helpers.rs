//! Port of `analysis/DependencyResolutionHelpers.java`: the labels of a configured target's
//! dependencies by dependency kind (for a configured target, or for the path of aspects applied
//! to it).

use std::collections::HashSet;
use std::sync::Arc;

use blaze_analysis::config::config_matching_provider::ConfigMatchingProvider;
use blaze_analysis::config::configured_attribute_mapper::ConfiguredAttributeMapper;
use blaze_analysis::config::ordered_set_multimap::{DependencyLabels, put_all_labels, put_label};
use blaze_analysis::config::transitions::find_execution_transition_factory;
use blaze_analysis::dependency_kind::{AspectClassName, DependencyKind};
use blaze_analysis::toolchains::toolchain_context::ToolchainContext;
use blaze_analysis::toolchains::{ToolchainCollection, UnloadedToolchainContext};
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;
use blaze_config::transition_factories::DEFAULT_EXEC_GROUP_NAME;
use blaze_packages::attribute::{Attribute, AttributeMap};
use blaze_packages::rule_class::rule_class::{
    COMPATIBLE_ENVIRONMENT_ATTR, CONFIG_SETTING_DEPS_ATTRIBUTE, RESTRICTED_ENVIRONMENT_ATTR,
};
use blaze_packages::target::{Rule, RuleOrMacroInstance, Target};
use blaze_packages::types::{AttrValue, Type};
use blaze_starlark::Location;
use indexmap::IndexMap;

use crate::late_bound_defaults::{self, LateBoundInput};
use crate::producers::errors::TargetAndConfiguration;

pub use blaze_packages::aspect::Aspect;

/// The tuple [`compute_dependency_labels`] outputs.
// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:62 DependencyResolutionHelpers.DependencyLabels
pub struct DependencyLabelsResult {
    pub labels: DependencyLabels,
    /// Non-null for rules and output files when there are aspects that apply to files.
    pub attribute_map: Option<ConfiguredAttributeMapper>,
}

/// Indicates a failure during dependency resolution.
// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:263 DependencyResolutionHelpers.Failure
#[derive(Clone, Debug)]
pub struct Failure {
    pub location: Option<Location>,
    pub message: String,
}

// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:84 DependencyResolutionHelpers.computeDependencyLabels
pub fn compute_dependency_labels(
    node: &TargetAndConfiguration,
    aspects: &[Aspect],
    config_conditions: &Arc<IndexMap<Label, ConfigMatchingProvider>>,
    toolchain_contexts: Option<&ToolchainCollection<UnloadedToolchainContext>>,
    base_target_unloaded_toolchain_contexts: Option<&ToolchainCollection<UnloadedToolchainContext>>,
) -> Result<DependencyLabelsResult, Failure> {
    let target = node.get_target();
    let config = node.get_configuration();
    let mut outgoing_labels: DependencyLabels = IndexMap::new();

    // TODO(bazel-team): Figure out a way to implement the below using LabelVisitationUtils.
    let mut attribute_map = None;
    match target {
        Target::OutputFile(output_file) => {
            let config = config.expect("NullPointerException: output file configuration");
            add_visibility_dep_labels(
                &target.get_visibility_dependency_labels(),
                &mut outgoing_labels,
            );
            add_transitive_visibility_dep_label(
                target
                    .get_package_declarations()
                    .get_package_args()
                    .transitive_visibility(),
                &mut outgoing_labels,
            );
            let rule = output_file.get_generating_rule();
            put_label(
                &mut outgoing_labels,
                DependencyKind::OUTPUT_FILE_RULE_DEPENDENCY,
                Some(rule.get_label().clone()),
            );
            if aspects.iter().any(|a| a.get_definition().apply_to_files()) {
                let map =
                    ConfiguredAttributeMapper::of(rule.clone(), config_conditions.clone(), config);
                resolve_attributes(
                    &get_aspect_attributes(aspects),
                    &mut outgoing_labels,
                    rule,
                    &map,
                    config,
                );
                attribute_map = Some(map);
            }
            add_toolchain_deps(toolchain_contexts, &mut outgoing_labels);
        }
        Target::InputFile(_) => {
            add_visibility_dep_labels(
                &target.get_visibility_dependency_labels(),
                &mut outgoing_labels,
            );
            add_transitive_visibility_dep_label(
                target
                    .get_package_declarations()
                    .get_package_args()
                    .transitive_visibility(),
                &mut outgoing_labels,
            );
        }
        Target::EnvironmentGroup(_) => {
            add_visibility_dep_labels(
                &target.get_visibility_dependency_labels(),
                &mut outgoing_labels,
            );
        }
        Target::Rule(rule) => {
            let config = config.expect("NullPointerException: rule configuration");
            let map =
                ConfiguredAttributeMapper::of(rule.clone(), config_conditions.clone(), config);
            add_transitive_visibility_dep_label(
                rule.get_package_declarations()
                    .get_package_args()
                    .transitive_visibility(),
                &mut outgoing_labels,
            );
            visit_rule(
                node,
                aspects,
                &map,
                toolchain_contexts,
                base_target_unloaded_toolchain_contexts,
                &mut outgoing_labels,
            )?;
            attribute_map = Some(map);
        }
        Target::PackageGroup(package_group) => {
            put_all_labels(
                &mut outgoing_labels,
                DependencyKind::VISIBILITY_DEPENDENCY,
                package_group.get_includes(),
            );
        }
        other => panic!("IllegalStateException: {}", other.get_label()),
    }
    Ok(DependencyLabelsResult {
        labels: outgoing_labels,
        attribute_map,
    })
}

/// The results of [`get_execution_platform_label`] as a tagged union.
// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:148 DependencyResolutionHelpers.ExecutionPlatformResult
#[derive(Clone, Debug)]
pub enum ExecutionPlatformResult {
    /// A label was successfully determined.
    Label(Label),
    /// A label was successfully determined to be null.
    NullLabel,
    /// The dependency should be skipped.
    Skip,
    /// An error message.
    Error(String),
}

// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:194 DependencyResolutionHelpers.getExecutionPlatformLabel
pub fn get_execution_platform_label(
    kind: &DependencyKind,
    toolchain_contexts: Option<&ToolchainCollection<UnloadedToolchainContext>>,
    base_target_unloaded_toolchain_contexts: Option<&ToolchainCollection<UnloadedToolchainContext>>,
    aspects_list: &[Aspect],
) -> ExecutionPlatformResult {
    if aspects_list.is_empty() || is_main_aspect(aspects_list, kind.get_owning_aspect()) {
        get_execution_platform_label_of(kind, toolchain_contexts)
    } else if kind.get_owning_aspect().is_none() {
        // During aspect evaluation, use `baseTargetUnloadedToolchainContexts` for the base
        // target's dependencies.
        get_execution_platform_label_of(kind, base_target_unloaded_toolchain_contexts)
    } else {
        let execution_platform_result = get_execution_platform_label_of(kind, toolchain_contexts);
        if let ExecutionPlatformResult::Error(_) = execution_platform_result {
            // TODO(b/373963347): Make the toolchain contexts of base aspects available to be
            // used with their corresponding dependencies.
            // Currently dependencies of the base aspects are resolved with the toolchain context
            // of the main aspect, skip errors as actual errors would be reported during the base
            // aspect evaluation.
            ExecutionPlatformResult::Skip
        } else {
            execution_platform_result
        }
    }
}

/// True if `owning_aspect` is the main aspect, the last one in `aspects_list`.
// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:276 DependencyResolutionHelpers.isMainAspect
fn is_main_aspect(aspects_list: &[Aspect], owning_aspect: Option<&AspectClassName>) -> bool {
    // `AspectClass.equals(null)` is false.
    let main_aspect = aspects_list.last().expect("NoSuchElementException");
    owning_aspect.is_some_and(|o| main_aspect.get_aspect_class() == o)
}

// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:219 DependencyResolutionHelpers.getExecutionPlatformLabel
fn get_execution_platform_label_of(
    kind: &DependencyKind,
    toolchain_contexts: Option<&ToolchainCollection<UnloadedToolchainContext>>,
) -> ExecutionPlatformResult {
    let Some(toolchain_contexts) = toolchain_contexts else {
        return ExecutionPlatformResult::NullLabel;
    };
    let attribute = kind.get_attribute().expect("attribute dependency kind");

    // The exec transition may be composed with other transitions (via transition.and_then), so
    // look for it anywhere in the (possibly composed) transition factory.
    let Some(exec_group) = find_execution_transition_factory(attribute.get_transition_factory())
    else {
        return ExecutionPlatformResult::Label(
            toolchain_contexts
                .get_toolchain_context(DEFAULT_EXEC_GROUP_NAME)
                .expect("NullPointerException: default exec group toolchain context")
                .execution_platform()
                .label()
                .clone(),
        );
    };

    if toolchain_contexts.has_toolchain_context(&exec_group) {
        // PORT: the Rust ToolchainContext's execution platform is never null.
        let platform = Some(
            toolchain_contexts
                .get_toolchain_context(&exec_group)
                .expect("has context")
                .execution_platform(),
        );
        return match platform {
            None => ExecutionPlatformResult::NullLabel,
            Some(p) => ExecutionPlatformResult::Label(p.label().clone()),
        };
    }

    ExecutionPlatformResult::Error(format!(
        "Attr '{}' declares a transition for non-existent exec group '{}'",
        attribute.get_name(),
        exec_group
    ))
}

// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:278 DependencyResolutionHelpers.visitRule
fn visit_rule(
    node: &TargetAndConfiguration,
    aspects: &[Aspect],
    attribute_map: &ConfiguredAttributeMapper,
    toolchain_contexts: Option<&ToolchainCollection<UnloadedToolchainContext>>,
    base_target_unloaded_toolchain_contexts: Option<&ToolchainCollection<UnloadedToolchainContext>>,
    outgoing_labels: &mut DependencyLabels,
) -> Result<(), Failure> {
    let rule = node
        .get_target()
        .as_rule()
        .unwrap_or_else(|| panic!("IllegalArgumentException: {}", node.get_label()));
    let rule_config = node
        .get_configuration()
        .unwrap_or_else(|| panic!("NullPointerException: {}", node.get_label()));

    if let Err(ex) = attribute_map.validate_attributes() {
        return Err(Failure {
            location: Some(rule.get_location().clone()),
            message: ex.0,
        });
    }

    let visibility_dep_labels = rule.get_visibility_dependency_labels();
    add_visibility_dep_labels(&visibility_dep_labels, outgoing_labels);
    resolve_attributes(
        &get_attributes(rule, aspects),
        outgoing_labels,
        rule,
        attribute_map,
        rule_config,
    );

    // Add the rule's visibility labels (which may come from the rule or from package defaults).
    add_explicit_deps(outgoing_labels, rule, "visibility", &visibility_dep_labels);

    // Add package default constraints when the rule doesn't explicitly declare them.
    //
    // Rather than try to answer the questions of how rule declarations and package defaults
    // intermix, we take the simple approach of assigning the rule's "restriction" attribute to
    // the rule-declared value if it exists, else the package defaults value (and likewise for
    // "compatibility").
    let package_args = rule.get_package_declarations().get_package_args();
    if !rule.is_attribute_value_explicitly_specified(COMPATIBLE_ENVIRONMENT_ATTR) {
        let defaults: Vec<Label> = package_args
            .default_compatible_with()
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        add_explicit_deps(
            outgoing_labels,
            rule,
            COMPATIBLE_ENVIRONMENT_ATTR,
            &defaults,
        );
    }
    if !rule.is_attribute_value_explicitly_specified(RESTRICTED_ENVIRONMENT_ATTR) {
        let defaults: Vec<Label> = package_args
            .default_restricted_to()
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        add_explicit_deps(
            outgoing_labels,
            rule,
            RESTRICTED_ENVIRONMENT_ATTR,
            &defaults,
        );
    }

    add_toolchain_deps(toolchain_contexts, outgoing_labels);
    add_base_target_toolchain_deps(base_target_unloaded_toolchain_contexts, outgoing_labels);
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:355 DependencyResolutionHelpers.addToolchainDeps
fn add_toolchain_deps(
    toolchain_contexts: Option<&ToolchainCollection<UnloadedToolchainContext>>,
    outgoing_labels: &mut DependencyLabels,
) {
    if let Some(toolchain_contexts) = toolchain_contexts {
        for (exec_group, context) in toolchain_contexts.context_map() {
            put_all_labels(
                outgoing_labels,
                DependencyKind::for_exec_group(exec_group),
                &context.resolved_toolchain_labels(),
            );
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:367 DependencyResolutionHelpers.addBaseTargetToolchainDeps
fn add_base_target_toolchain_deps(
    toolchain_contexts: Option<&ToolchainCollection<UnloadedToolchainContext>>,
    outgoing_labels: &mut DependencyLabels,
) {
    let Some(toolchain_contexts) = toolchain_contexts else {
        return;
    };
    for (exec_group, context) in toolchain_contexts.context_map() {
        // `toolchainTypeToResolved().asMap()`: the multimap's keys in first-insertion order,
        // each with its values in insertion order.
        let mut by_type: IndexMap<Label, Vec<Label>> = IndexMap::new();
        for (toolchain_type, resolved) in context.toolchain_type_to_resolved() {
            by_type
                .entry(toolchain_type.type_label().clone())
                .or_default()
                .push(resolved.clone());
        }
        for (type_label, resolved) in by_type {
            // map entries from (exec group, toolchain type) to resolved toolchain labels. We need
            // to distinguish the resolved toolchains per type because aspects propagate on
            // toolchains based on the types specified in `toolchains_aspects`. So even if 2 types
            // resolved to the same toolchain target, their CT will be different if an aspect
            // propagates to one type but not the other.
            put_all_labels(
                outgoing_labels,
                DependencyKind::for_base_target_exec_group(exec_group, type_label),
                &resolved,
            );
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:390 DependencyResolutionHelpers.resolveAttributes
fn resolve_attributes(
    attribute_dependency_kinds: &[DependencyKind],
    outgoing_labels: &mut DependencyLabels,
    rule: &Arc<Rule>,
    attribute_map: &ConfiguredAttributeMapper,
    rule_config: &BuildConfigurationValue,
) {
    for dependency_kind in attribute_dependency_kinds {
        let attribute = dependency_kind
            .get_attribute()
            .expect("attribute dependency kind");
        // Not only is resolving CONFIG_SETTING_DEPS_ATTRIBUTE deps here wasteful, since the only
        // place they're used is in ConfiguredTargetFunction.getConfigConditions, but it actually
        // breaks trimming because it resolves a dep that trimming (correctly) doesn't account
        // for because it's part of an unchosen select() branch.
        if attribute.get_name() == CONFIG_SETTING_DEPS_ATTRIBUTE {
            continue;
        }
        let type_ = attribute.get_type();
        if type_ == Type::Output
            || type_ == Type::OutputList
            || type_ == Type::NodepLabel
            || type_ == Type::NodepLabelList
            || type_ == Type::DormantLabel
            || type_ == Type::DormantLabelList
            || type_ == Type::GenqueryScopeType
            || type_ == Type::GenqueryScopeTypeList
        {
            // These types invoke visitLabels() so that they are reported in "bazel query" but do
            // not create a dependency. Maybe it's better to remove that, but then the labels()
            // query function would need to be rethought.
            continue;
        }

        resolve_attribute(
            attribute,
            type_,
            dependency_kind,
            outgoing_labels,
            rule,
            attribute_map,
            rule_config,
        );
    }
}

// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:425 DependencyResolutionHelpers.resolveAttribute
fn resolve_attribute(
    attribute: &Attribute,
    type_: Type,
    dependency_kind: &DependencyKind,
    outgoing_labels: &mut DependencyLabels,
    rule: &Arc<Rule>,
    attribute_map: &ConfiguredAttributeMapper,
    rule_config: &BuildConfigurationValue,
) {
    let mut attribute_value: Option<AttrValue> = None;
    if attribute.is_implicit() {
        // Since the attributes that come from aspects do not appear in attributeMap, we have to
        // get their values from somewhere else. This incidentally means that aspects attributes
        // are not configurable.
        if dependency_kind.get_owning_aspect().is_none() {
            attribute_value = attribute_map.get(attribute.get_name(), type_);
        } else {
            let default_value =
                attribute.get_default_value(Some(&**rule as &dyn RuleOrMacroInstance));
            attribute_value = match default_value {
                Some(AttrValue::ComputedDefault(computed_default)) => {
                    computed_default.get_default(attribute_map)
                }
                other => other,
            };
        }
    } else if attribute.is_materializing() {
        // These attributes are resolved by calling the materializer function in
        // DependencyMapProducer. The reason is that they need the analyzed versions some direct
        // dependencies and we can't do that here.
        put_label(outgoing_labels, dependency_kind.clone(), None);
    } else if attribute.is_late_bound() {
        attribute_value = resolve_late_bound_default(rule, attribute_map, attribute, rule_config);
    } else if dependency_kind.get_owning_aspect().is_none()
        && attribute_map.has(attribute.get_name())
    {
        // This condition is false for aspect attributes that do not give rise to dependencies
        // because attributes that come from aspects do not appear in attributeMap.
        attribute_value = attribute_map.get(attribute.get_name(), type_);
    }

    let Some(attribute_value) = attribute_value else {
        return;
    };

    let mut visitor = |dep_label: &Label, _ctx: Option<&Attribute>| {
        put_label(
            outgoing_labels,
            dependency_kind.clone(),
            Some(dep_label.clone()),
        );
    };
    type_.visit_labels(&mut visitor, &attribute_value, /* context= */ None);
}

/// Resolves a late-bound default against the rule's configuration.
// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:478 DependencyResolutionHelpers.resolveLateBoundDefault
pub fn resolve_late_bound_default(
    rule: &Arc<Rule>,
    attribute_map: &dyn AttributeMap,
    attribute: &Attribute,
    rule_config: &BuildConfigurationValue,
) -> Option<AttrValue> {
    blaze_util::check_state!(!attribute.get_transition_factory().is_split());
    let late_bound_default = attribute
        .get_late_bound_default()
        .expect("late-bound attribute has a late-bound default");

    let fragment_class = late_bound_default.get_fragment_class();
    // TODO(b/65746853): remove this when nothing uses it anymore
    if fragment_class == Some("BuildConfigurationValue")
        // noconfig targets can't meaningfully parse late-bound defaults. See NoConfigTransition.
        && !rule_config.get_options().has_no_config()
    {
        return late_bound_defaults::resolve(
            late_bound_default,
            Some(rule),
            attribute_map,
            LateBoundInput::Configuration(rule_config),
        );
    }
    let Some(fragment_class) = fragment_class else {
        // Void.class
        return late_bound_defaults::resolve(
            late_bound_default,
            Some(rule),
            attribute_map,
            /* input= */ LateBoundInput::Null,
        );
    };
    // PORT: `ruleConfig.getFragment(fragmentClass)`; for `BuildConfigurationValue.class` under
    // the noconfig configuration this finds no fragment and returns null, as in Java.
    let fragment = rule_config
        .get_fragments()
        .iter()
        .find(|(class, _)| class.get_simple_name() == fragment_class)
        .map(|(_, f)| f.clone())?;
    late_bound_defaults::resolve(
        late_bound_default,
        Some(rule),
        attribute_map,
        LateBoundInput::Fragment(&*fragment),
    )
}

/// Adds new dependencies to the given rule under the given attribute name.
// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:518 DependencyResolutionHelpers.addExplicitDeps
fn add_explicit_deps(
    outgoing_labels: &mut DependencyLabels,
    rule: &Rule,
    attr_name: &str,
    labels: &[Label],
) {
    if !rule.is_attr_defined(attr_name, Type::LabelList)
        && !rule.is_attr_defined(attr_name, Type::NodepLabelList)
    {
        return;
    }
    let attribute = rule
        .get_rule_class_object()
        .get_attribute_provider()
        .get_attribute_by_name(attr_name);
    put_all_labels(
        outgoing_labels,
        DependencyKind::for_rule(Arc::new(attribute.clone())),
        labels,
    );
}

/// Returns the attributes that should be visited for this rule/aspect combination.
// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:534 DependencyResolutionHelpers.getAttributes
fn get_attributes(rule: &Rule, aspects: &[Aspect]) -> Vec<DependencyKind> {
    let mut result = Vec::new();
    // PORT: `DependencyKind.forRule` holds the attribute; Blaze's attributes are values, so
    // each is shared through an `Arc` copy (equality is by value).
    let mut rule_and_base_aspects_processed_attributes: HashSet<String> = HashSet::new();

    // For aspects evaluation, all attributes of the main aspect (last aspect in `aspects`) should
    // be added, even if they have the same name as an attribute in the rule or a base aspect
    // because main aspect attributes are separated and retrieved from `ctx.attr`.

    // Attributes of the underlying rule and base aspects are merged and retrieved from
    // `ctx.rule.attr` with rule attributes taking precedence then aspects' attributes based on
    // the aspect order in the aspects path (lowest order to highest).
    for attribute in rule
        .get_rule_class_object()
        .get_attribute_provider()
        .get_attributes()
    {
        result.push(DependencyKind::for_rule(rule_attribute(rule, attribute)));
        rule_and_base_aspects_processed_attributes.insert(attribute.get_name().to_owned());
    }

    add_aspect_attributes(
        aspects,
        &mut rule_and_base_aspects_processed_attributes,
        &mut result,
    );
    result
}

// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:574 DependencyResolutionHelpers.getAspectAttributes
fn get_aspect_attributes(aspects: &[Aspect]) -> Vec<DependencyKind> {
    let mut result = Vec::new();
    add_aspect_attributes(aspects, &mut HashSet::new(), &mut result);
    result
}

// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:581 DependencyResolutionHelpers.addAspectAttributes
fn add_aspect_attributes(
    aspects: &[Aspect],
    processed_attributes: &mut HashSet<String>,
    attributes: &mut Vec<DependencyKind>,
) {
    let Some(main_aspect) = aspects.last() else {
        return;
    };

    // Add all the main aspect's attributes
    for attribute in main_aspect.get_definition().get_attributes().values() {
        attributes.push(DependencyKind::for_aspect(
            aspect_attribute(attribute),
            main_aspect.get_aspect_class().clone(),
        ));
    }

    // For base aspects, if multiple attributes have the same name, take the first encountered
    // in the aspects path.
    for aspect in &aspects[..aspects.len() - 1] {
        for attribute in aspect.get_definition().get_attributes().values() {
            if processed_attributes.insert(attribute.get_name().to_owned()) {
                attributes.push(DependencyKind::for_aspect(
                    aspect_attribute(attribute),
                    aspect.get_aspect_class().clone(),
                ));
            }
        }
    }
}

/// The shared `Arc<Attribute>` of an aspect definition attribute.
fn aspect_attribute(attribute: &Attribute) -> Arc<Attribute> {
    Arc::new(attribute.clone())
}

/// The shared `Arc<Attribute>` of a rule class attribute.
fn rule_attribute(_rule: &Rule, attribute: &Attribute) -> Arc<Attribute> {
    Arc::new(attribute.clone())
}

// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:600 DependencyResolutionHelpers.addVisibilityDepLabels
fn add_visibility_dep_labels(labels: &[Label], outgoing_labels: &mut DependencyLabels) {
    put_all_labels(
        outgoing_labels,
        DependencyKind::VISIBILITY_DEPENDENCY,
        labels,
    );
}

// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:605 DependencyResolutionHelpers.addTransitiveVisibilityDepLabel
fn add_transitive_visibility_dep_label(
    label: Option<&Label>,
    outgoing_labels: &mut DependencyLabels,
) {
    if let Some(label) = label {
        put_label(
            outgoing_labels,
            DependencyKind::TRANSITIVE_VISIBILITY_DEPENDENCY,
            Some(label.clone()),
        );
    }
}
