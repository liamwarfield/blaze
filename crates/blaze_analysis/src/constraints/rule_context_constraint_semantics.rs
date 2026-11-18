//! Port of `analysis/constraints/RuleContextConstraintSemantics.java`: constraint semantics
//! that apply to {@link RuleContext}s (Bazel's only `ConstraintSemantics`).
//!
//! Implements the constraint semantics described in `ConstraintSemantics`: a rule's supported
//! environments come from `restricted_to` / `compatible_with` (and the rule class defaults), and
//! every dependency must support them.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::attribute::dependency_filter::DependencyFilter;
use blaze_packages::rule_class::rule_class::{
    COMPATIBLE_ENVIRONMENT_ATTR, DEFAULT_COMPATIBLE_ENVIRONMENT_ATTR,
    DEFAULT_RESTRICTED_ENVIRONMENT_ATTR, RESTRICTED_ENVIRONMENT_ATTR,
};
use blaze_packages::target::environment_group::EnvironmentLabels;
use blaze_packages::target::rule_or_macro_instance::RuleOrMacroInstance;
use blaze_packages::types::{AttrValue, LabelClass, Type};
use indexmap::{IndexMap, IndexSet};

use crate::configured_target::ConfiguredTarget;
use crate::constraints::environment_collection::{
    EnvironmentCollection, EnvironmentCollectionBuilder, EnvironmentWithGroup,
};
use crate::constraints::supported_environments::{
    LabelAndLocation, RemovedEnvironmentCulprit, SupportedEnvironments,
};
use crate::rule::{RuleContext, RuleErrorConsumer};

/// Java `RuleContextConstraintSemantics` (stateless).
// Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:76 RuleContextConstraintSemantics
pub struct RuleContextConstraintSemantics;

/// Java `DefaultsProvider`: provides a set of default environments for a given environment
/// group (`GroupDefaultsProvider`, or `RuleClassDefaultsProvider` over the rule class defaults).
// Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:90 RuleContextConstraintSemantics.DefaultsProvider
enum DefaultsProvider {
    Group,
    RuleClass(EnvironmentCollection),
}

impl DefaultsProvider {
    fn get_defaults(&self, group: &EnvironmentLabels) -> Vec<Label> {
        match self {
            // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:97 RuleContextConstraintSemantics.GroupDefaultsProvider.getDefaults
            DefaultsProvider::Group => group.get_defaults().iter().cloned().collect(),
            // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:111 RuleContextConstraintSemantics.RuleClassDefaultsProvider.getDefaults
            DefaultsProvider::RuleClass(rule_class_defaults) => {
                if rule_class_defaults.contains_group(group) {
                    rule_class_defaults.get_environments_of(group)
                } else {
                    // If there are no rule class defaults for this group, just inherit global
                    // defaults.
                    group.get_defaults().iter().cloned().collect()
                }
            }
        }
    }
}

/// Collects the set of supported environments for a given rule by merging its restriction
/// environments with its compatibility environments.
// Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:121 RuleContextConstraintSemantics.EnvironmentCollector
struct EnvironmentCollector {
    restriction_attr: &'static str,
    compatibility_attr: &'static str,
    restriction_environments: EnvironmentCollection,
    compatibility_environments: EnvironmentCollection,
    supported_environments: EnvironmentCollection,
}

impl EnvironmentCollector {
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:144 RuleContextConstraintSemantics.EnvironmentCollector.<init>
    fn new(
        rule_context: &RuleContext,
        restriction_attr: &'static str,
        compatibility_attr: &'static str,
        defaults_provider: &DefaultsProvider,
    ) -> EnvironmentCollector {
        let mut environments_builder = EnvironmentCollectionBuilder::new();
        // Java: collectRestrictionEnvironments
        let restriction_environments =
            collect_environments(rule_context, restriction_attr, &mut environments_builder);
        // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:213 RuleContextConstraintSemantics.EnvironmentCollector.collectCompatibilityEnvironments
        let compatibility_environments =
            collect_environments(rule_context, compatibility_attr, &mut environments_builder);
        for group in compatibility_environments.get_groups() {
            environments_builder.put_all(&group, defaults_provider.get_defaults(&group));
        }
        EnvironmentCollector {
            restriction_attr,
            compatibility_attr,
            restriction_environments,
            compatibility_environments,
            supported_environments: environments_builder.build(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:162 RuleContextConstraintSemantics.EnvironmentCollector.getEnvironments
    fn get_environments(&self) -> &EnvironmentCollection {
        &self.supported_environments
    }

    /// Checks that if this is an environment-restricted rule, classes that it's restricted to
    /// and compatible with are disjoint; reports errors otherwise.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:171 RuleContextConstraintSemantics.EnvironmentCollector.validateEnvironmentSpecifications
    fn validate_environment_specifications(&self, rule_context: &RuleContext) -> bool {
        let mut has_errors = false;

        for group in self.compatibility_environments.get_groups() {
            if self.restriction_environments.contains_group(&group) {
                // To avoid error-spamming the user, when we find a conflict we only report one
                // example environment from each attribute for that group.
                let compatibility_env =
                    self.compatibility_environments.get_environments_of(&group)[0].clone();
                let restriction_env =
                    self.restriction_environments.get_environments_of(&group)[0].clone();

                if compatibility_env == restriction_env {
                    rule_context.attribute_error(
                        self.compatibility_attr,
                        &format!(
                            "{} cannot appear both here and in {}",
                            compatibility_env, self.restriction_attr
                        ),
                    );
                } else {
                    rule_context.attribute_error(
                        self.compatibility_attr,
                        &format!(
                            "{} and {} belong to the same environment group. They should be declared together either here or in {}",
                            compatibility_env, restriction_env, self.restriction_attr
                        ),
                    );
                }
                has_errors = true;
            }
        }

        !has_errors
    }
}

/// Adds environments specified in the given attribute to the set of supported environments and
/// returns the environments added.
// Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:229 RuleContextConstraintSemantics.EnvironmentCollector.collectEnvironments
fn collect_environments(
    rule_context: &RuleContext,
    attr_name: &str,
    supported_environments: &mut EnvironmentCollectionBuilder,
) -> EnvironmentCollection {
    if !rule_context
        .get_rule()
        .is_attr_defined(attr_name, Type::LabelList)
    {
        return EnvironmentCollection::empty();
    }
    let mut environments = EnvironmentCollectionBuilder::new();
    for env_target in rule_context.get_prerequisites(attr_name) {
        let env_info = resolve_environment(&env_target);
        environments.put(&env_info.group, env_info.environment.clone());
        supported_environments.put(&env_info.group, env_info.environment);
    }
    environments.build()
}

/// Returns the environment and its group. An `environment` rule provides this through
/// SupportedEnvironmentsProvider.
// Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:248 RuleContextConstraintSemantics.EnvironmentCollector.resolveEnvironment
fn resolve_environment(env_rule: &ConfiguredTarget) -> EnvironmentWithGroup {
    let prereq = env_rule
        .get_provider::<SupportedEnvironments>()
        .expect("NullPointerException: SupportedEnvironmentsProvider");
    let grouped = prereq.get_static_environments().get_grouped_environments();
    // Iterables.getOnlyElement
    if grouped.len() != 1 {
        panic!(
            "IllegalArgumentException: expected one element but was: {:?}",
            grouped
        );
    }
    grouped.into_iter().next().unwrap()
}

/// Helper container for checking constraints on a rule's dependencies: all the dependencies to
/// check, and the ones that only appear in select()s.
// Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:376 RuleContextConstraintSemantics.DepsToCheck
struct DepsToCheck {
    all_deps: Vec<ConfiguredTarget>,
    select_only_deps: Vec<ConfiguredTarget>,
}

impl DepsToCheck {
    fn is_select_only(&self, dep: &ConfiguredTarget) -> bool {
        self.select_only_deps.iter().any(|d| same_target(d, dep))
    }
}

/// Java object identity of two `TransitiveInfoCollection`s (`LinkedHashSet` membership).
fn same_target(a: &ConfiguredTarget, b: &ConfiguredTarget) -> bool {
    use std::sync::Arc as A;
    match (a, b) {
        (ConfiguredTarget::Rule(x), ConfiguredTarget::Rule(y)) => A::ptr_eq(x, y),
        (ConfiguredTarget::InputFile(x), ConfiguredTarget::InputFile(y)) => A::ptr_eq(x, y),
        (ConfiguredTarget::OutputFile(x), ConfiguredTarget::OutputFile(y)) => A::ptr_eq(x, y),
        (ConfiguredTarget::PackageGroup(x), ConfiguredTarget::PackageGroup(y)) => A::ptr_eq(x, y),
        (ConfiguredTarget::EnvironmentGroup(x), ConfiguredTarget::EnvironmentGroup(y)) => {
            A::ptr_eq(x, y)
        }
        (ConfiguredTarget::Alias(x), ConfiguredTarget::Alias(y)) => A::ptr_eq(x, y),
        (ConfiguredTarget::Merged(x), ConfiguredTarget::Merged(y)) => A::ptr_eq(x, y),
        _ => false,
    }
}

fn add_target(set: &mut Vec<ConfiguredTarget>, target: &ConfiguredTarget) {
    if !set.iter().any(|t| same_target(t, target)) {
        set.push(target.clone());
    }
}

impl RuleContextConstraintSemantics {
    /// Returns the set of environments this rule supports, applying the logic described in
    /// `ConstraintSemantics`. Note this set is *not* the same as the set of environments the
    /// rule is ultimately compatible with: the refined environments also depend on select()
    /// choices. `None` if the specification is invalid (errors are reported).
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:273 RuleContextConstraintSemantics.getSupportedEnvironments
    pub fn get_supported_environments(rule_context: &RuleContext) -> Option<EnvironmentCollection> {
        if !Self::validate_attributes(rule_context) {
            return None;
        }

        // This rule's rule class defaults (or null if the rule class has no defaults).
        let rule_class_collector = Self::maybe_get_rule_class_defaults(rule_context);
        // Default environments for this rule. If the rule has rule class defaults, this is
        // those defaults. Otherwise it's the global defaults specified by environment_group
        // declarations.
        let rule_defaults = match &rule_class_collector {
            Some(collector) => {
                if !collector.validate_environment_specifications(rule_context) {
                    return None;
                }
                DefaultsProvider::RuleClass(collector.get_environments().clone())
            }
            None => DefaultsProvider::Group,
        };

        let rule_collector = EnvironmentCollector::new(
            rule_context,
            RESTRICTED_ENVIRONMENT_ATTR,
            COMPATIBLE_ENVIRONMENT_ATTR,
            &rule_defaults,
        );
        if !rule_collector.validate_environment_specifications(rule_context) {
            return None;
        }

        let mut supported_environments = rule_collector.get_environments().clone();
        if let Some(collector) = &rule_class_collector {
            // If we have rule class defaults from groups that aren't referenced from the rule
            // itself, we need to add them in too to override the global defaults.
            supported_environments = Self::add_unknown_groups_to_collection(
                &supported_environments,
                collector.get_environments(),
            );
        }
        Some(supported_environments)
    }

    /// Returns the rule class defaults specified for this rule, or `None` if there are no such
    /// defaults.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:315 RuleContextConstraintSemantics.maybeGetRuleClassDefaults
    fn maybe_get_rule_class_defaults(rule_context: &RuleContext) -> Option<EnvironmentCollector> {
        let rule = rule_context.get_rule();
        let restriction_attr = DEFAULT_RESTRICTED_ENVIRONMENT_ATTR;
        let compatibility_attr = DEFAULT_COMPATIBLE_ENVIRONMENT_ATTR;

        if rule.is_attr_defined(restriction_attr, Type::LabelList)
            || rule.is_attr_defined(compatibility_attr, Type::LabelList)
        {
            Some(EnvironmentCollector::new(
                rule_context,
                restriction_attr,
                compatibility_attr,
                &DefaultsProvider::Group,
            ))
        } else {
            None
        }
    }

    /// Adds environments from `to_add` for groups not represented in `environments`.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:338 RuleContextConstraintSemantics.addUnknownGroupsToCollection
    fn add_unknown_groups_to_collection(
        environments: &EnvironmentCollection,
        to_add: &EnvironmentCollection,
    ) -> EnvironmentCollection {
        let mut builder = EnvironmentCollectionBuilder::new();
        builder.put_all_collection(environments);
        for candidate_group in to_add.get_groups() {
            if !environments.contains_group(&candidate_group) {
                builder.put_all(
                    &candidate_group,
                    to_add.get_environments_of(&candidate_group),
                );
            }
        }
        builder.build()
    }

    /// Validity-checks this rule's constraint-related attributes. Returns true if all is good,
    /// returns false and reports appropriate errors if there are any problems.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:354 RuleContextConstraintSemantics.validateAttributes
    fn validate_attributes(rule_context: &RuleContext) -> bool {
        let attributes = rule_context.attributes();

        // Report an error if "restricted to" is explicitly set to nothing. Even if this made
        // conceptual sense, we don't know which groups we should apply that to.
        let restriction_attr = RESTRICTED_ENVIRONMENT_ATTR;
        let restriction_environments = rule_context.get_prerequisites(restriction_attr);
        if restriction_environments.is_empty()
            && attributes.is_attribute_value_explicitly_specified(restriction_attr)
        {
            rule_context.attribute_error(restriction_attr, "attribute cannot be empty");
            return false;
        }

        true
    }

    /// Performs constraint checking on the given rule's dependencies and reports any errors.
    /// This includes:
    ///
    /// - Static environment checking: if this rule supports environment E, all deps outside
    ///   selects must also support E.
    /// - Refined environment computation: this rule's refined environments are its static
    ///   environments intersected with the refined environments of all dependencies (including
    ///   chosen deps in selects).
    /// - Refined environment checking: no environment groups can be "emptied" due to refinement.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:414 RuleContextConstraintSemantics.checkConstraints
    pub fn check_constraints(
        rule_context: &RuleContext,
        static_environments: &EnvironmentCollection,
        refined_environments: &mut EnvironmentCollectionBuilder,
        removed_environment_culprits: &mut IndexMap<Label, RemovedEnvironmentCulprit>,
    ) {
        // Start with the full set of static environments:
        let mut refined_environments_so_far: IndexSet<EnvironmentWithGroupKey> =
            static_environments
                .get_grouped_environments()
                .into_iter()
                .map(EnvironmentWithGroupKey)
                .collect();
        let mut groups_with_environments_removed: Vec<Arc<EnvironmentLabels>> = Vec::new();
        // Maps the label results of getUnsupportedEnvironments() to EnvironmentWithGroups. We
        // can't have that method just return EnvironmentWithGroups because it also collects
        // group defaults, which we only have labels for.
        let mut labels_to_environments: IndexMap<Label, EnvironmentWithGroup> = IndexMap::new();
        for env_with_group in static_environments.get_grouped_environments() {
            labels_to_environments.insert(env_with_group.environment.clone(), env_with_group);
        }

        let deps_to_check = Self::get_constraint_checked_dependencies(rule_context);

        for dep in &deps_to_check.all_deps {
            if !deps_to_check.is_select_only(dep) {
                // TODO(bazel-team): support static constraint checking for selects. A selectable
                // constraint is valid if the union of all deps in the select includes all of
                // this rule's static environments. Determining that requires following the
                // select paths that don't get chosen, which means we won't have
                // ConfiguredTargets for those deps and need to find another way to get their
                // environments.
                Self::check_static_constraints(rule_context, static_environments, dep);
            }
            Self::refine_environments_for_dep(
                rule_context,
                static_environments,
                dep,
                &mut labels_to_environments,
                &mut refined_environments_so_far,
                &mut groups_with_environments_removed,
                removed_environment_culprits,
            );
        }

        Self::check_refined_constraints(
            rule_context,
            &groups_with_environments_removed,
            &refined_environments_so_far,
            refined_environments,
            removed_environment_culprits,
        );
    }

    /// Performs static constraint checking against the given dep.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:457 RuleContextConstraintSemantics.checkStaticConstraints
    fn check_static_constraints(
        rule_context: &RuleContext,
        static_environments: &EnvironmentCollection,
        dep: &ConfiguredTarget,
    ) {
        let dep_environments = dep
            .get_provider::<SupportedEnvironments>()
            .expect("NullPointerException: SupportedEnvironmentsProvider");
        let unsupported_environments = Self::get_unsupported_environments(
            dep_environments.get_static_environments(),
            static_environments,
        );
        if !unsupported_environments.is_empty() {
            let joined: Vec<String> = unsupported_environments
                .iter()
                .map(|l| l.to_string())
                .collect();
            rule_context.rule_error(&format!(
                "dependency {} doesn't support expected environment{}: {}",
                dep.get_label(),
                if unsupported_environments.len() == 1 {
                    ""
                } else {
                    "s"
                },
                joined.join(", ")
            ));
        }
    }

    /// Helper method for `check_constraints`: refines a rule's environments with the given dep.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:476 RuleContextConstraintSemantics.refineEnvironmentsForDep
    fn refine_environments_for_dep(
        rule_context: &RuleContext,
        static_environments: &EnvironmentCollection,
        dep: &ConfiguredTarget,
        labels_to_environments: &mut IndexMap<Label, EnvironmentWithGroup>,
        refined_environments_so_far: &mut IndexSet<EnvironmentWithGroupKey>,
        groups_with_environments_removed: &mut Vec<Arc<EnvironmentLabels>>,
        removed_environment_culprits: &mut IndexMap<Label, RemovedEnvironmentCulprit>,
    ) {
        let dep_environments = dep
            .get_provider::<SupportedEnvironments>()
            .expect("NullPointerException: SupportedEnvironmentsProvider");

        // Stores the environments that are pruned from the refined set because of this dep. Even
        // though they're removed, some subset of the environments they fulfill may belong in
        // the refined set. For example, if environment "both" fulfills "a" and "b" and "lib"
        // statically sets restricted_to = ["both"] and "dep" sets restricted_to = ["a"], then
        // lib's refined set excludes "both". But rather than be emptied out it can be reduced to
        // "a".
        let mut pruned_environments_from_this_dep: IndexSet<Label> = IndexSet::new();

        // Refine this rule's environments by intersecting with the dep's refined environments:
        for refined_environment_to_prune in Self::get_unsupported_environments(
            dep_environments.get_refined_environments(),
            static_environments,
        ) {
            if !labels_to_environments.contains_key(&refined_environment_to_prune) {
                // If we have no record of this environment, that means the current rule
                // implicitly uses the defaults for this group. So explicitly opt that group's
                // defaults into the refined set before trying to remove specific items.
                for default_env in Self::get_defaults(
                    &refined_environment_to_prune,
                    dep_environments.get_refined_environments(),
                ) {
                    refined_environments_so_far
                        .insert(EnvironmentWithGroupKey(default_env.clone()));
                    labels_to_environments.insert(default_env.environment.clone(), default_env);
                }
            }
            // Verify.verifyNotNull
            let env_to_prune = labels_to_environments
                .get(&refined_environment_to_prune)
                .cloned()
                .expect("VerifyException");
            refined_environments_so_far
                .shift_remove(&EnvironmentWithGroupKey(env_to_prune.clone()));
            if !groups_with_environments_removed
                .iter()
                .any(|g| **g == *env_to_prune.group)
            {
                groups_with_environments_removed.push(env_to_prune.group.clone());
            }
            removed_environment_culprits.insert(
                env_to_prune.environment.clone(),
                Self::find_original_refiner(
                    rule_context,
                    &dep.get_label(),
                    dep_environments,
                    &env_to_prune,
                ),
            );
            pruned_environments_from_this_dep.insert(env_to_prune.environment.clone());
        }

        // Add in any dep environment that one of the environments we removed fulfills. In other
        // words, the removed environment is no good, but some subset of it may be.
        for dep_env in dep_environments
            .get_refined_environments()
            .get_grouped_environments()
        {
            for fulfiller in dep_env
                .group
                .get_fulfillers(&dep_env.environment)
                .to_list()
                .iter()
            {
                if pruned_environments_from_this_dep.contains(fulfiller) {
                    refined_environments_so_far.insert(EnvironmentWithGroupKey(dep_env.clone()));
                }
            }
        }
    }

    /// Helper method for `check_constraints`: performs refined environment constraint checking.
    ///
    /// Refined environment expectations: no environment group should be emptied out due to
    /// refining. This reflects the idea that some of the static declared environments get
    /// pruned out by the build configuration, but at least one environment should remain.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:538 RuleContextConstraintSemantics.checkRefinedConstraints
    fn check_refined_constraints(
        rule_context: &RuleContext,
        groups_with_environments_removed: &[Arc<EnvironmentLabels>],
        refined_environments_so_far: &IndexSet<EnvironmentWithGroupKey>,
        refined_environments: &mut EnvironmentCollectionBuilder,
        removed_environment_culprits: &IndexMap<Label, RemovedEnvironmentCulprit>,
    ) {
        let mut refined_groups: Vec<Arc<EnvironmentLabels>> = Vec::new();
        for env_with_group in refined_environments_so_far {
            let env_with_group = &env_with_group.0;
            refined_environments.put(&env_with_group.group, env_with_group.environment.clone());
            if !refined_groups.iter().any(|g| **g == *env_with_group.group) {
                refined_groups.push(env_with_group.group.clone());
            }
        }
        let newly_empty_groups: Vec<Arc<EnvironmentLabels>> = groups_with_environments_removed
            .iter()
            .filter(|g| !refined_groups.iter().any(|r| **r == ***g))
            .cloned()
            .collect();
        if !newly_empty_groups.is_empty() {
            rule_context.rule_error(&Self::get_over_refinement_error(
                rule_context.get_label(),
                &newly_empty_groups,
                removed_environment_culprits,
            ));
        }
    }

    /// Constructs an error message for when all environments have been pruned out of one or
    /// more environment groups due to refining.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:565 RuleContextConstraintSemantics.getOverRefinementError
    fn get_over_refinement_error(
        current_target: &Label,
        newly_empty_groups: &[Arc<EnvironmentLabels>],
        removed_environment_culprits: &IndexMap<Label, RemovedEnvironmentCulprit>,
    ) -> String {
        let mut message: Vec<String> = vec![
            "the current command line flags disqualify all supported environments because of incompatible select() paths:".to_owned(),
        ];
        for group in newly_empty_groups {
            if newly_empty_groups.len() > 1 {
                message.push(" ".to_owned());
                message.push(format!("environment group: {}:", group.get_label()));
            }
            for pruned_environment in group.get_environments() {
                // Only environments this rule statically declared support for have culprits.
                if let Some(culprit) = removed_environment_culprits.get(pruned_environment) {
                    message.push(" ".to_owned());
                    message.push(Self::get_missing_environment_culprit_message(
                        current_target,
                        pruned_environment,
                        culprit,
                    ));
                }
            }
        }
        message.join("\n")
    }

    /// Given an environment that should be refined out of the current rule because of the given
    /// dep, returns the original dep that caused the removal.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:591 RuleContextConstraintSemantics.getMissingEnvironmentCulpritMessage
    pub fn get_missing_environment_culprit_message(
        current_target: &Label,
        environment: &Label,
        reason: &RemovedEnvironmentCulprit,
    ) -> String {
        let culprit = &reason.culprit;
        let target_to_explore = if *current_target == culprit.label {
            &reason.selected_dep_for_culprit
        } else {
            &culprit.label
        };

        [
            format!("  environment: {environment}"),
            format!("    removed by: {} ({})", culprit.label, culprit.location),
            format!(
                "    because of a select() that chooses dep: {}",
                reason.selected_dep_for_culprit
            ),
            format!("    which lacks: {environment}"),
            String::new(),
            format!(
                "To see why, run: blaze build --target_environment={} {}",
                environment, target_to_explore
            ),
        ]
        .join("\n")
    }

    /// Finds the original dep that caused the removal of `env_to_prune` (the current rule if the
    /// dep has no record of it).
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:622 RuleContextConstraintSemantics.findOriginalRefiner
    fn find_original_refiner(
        rule_context: &RuleContext,
        dep: &Label,
        dep_environments: &SupportedEnvironments,
        env_to_prune: &EnvironmentWithGroup,
    ) -> RemovedEnvironmentCulprit {
        if let Some(dep_culprit) =
            dep_environments.get_removed_environment_culprit(&env_to_prune.environment)
        {
            return dep_culprit.clone();
        }
        // If the dep has no record of this environment being refined, that means the current
        // rule is the culprit.
        RemovedEnvironmentCulprit::create(
            LabelAndLocation::of(rule_context.get_target()),
            // While it'd be nice to know the dep's location too, it isn't strictly necessary.
            dep.clone(),
        )
    }

    /// Finds the given environment in the given set and returns the default environments for
    /// its group.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:646 RuleContextConstraintSemantics.getDefaults
    fn get_defaults(
        env: &Label,
        all_environments: &EnvironmentCollection,
    ) -> Vec<EnvironmentWithGroup> {
        let group = all_environments
            .get_groups()
            .into_iter()
            .find(|candidate_group| candidate_group.get_defaults().contains(env))
            .expect("VerifyException");
        let mut builder: Vec<EnvironmentWithGroup> = Vec::new();
        for default_env in group.get_defaults() {
            let e = EnvironmentWithGroup::create(default_env.clone(), group.clone());
            if !builder.contains(&e) {
                builder.push(e);
            }
        }
        builder
    }

    /// Given a collection of environments and a collection of expected environments, returns
    /// the missing environments that would cause constraint expectations to be violated.
    /// Includes the effects of environment group defaults.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:668 RuleContextConstraintSemantics.getUnsupportedEnvironments
    pub fn get_unsupported_environments(
        actual_environments: &EnvironmentCollection,
        expected_environments: &EnvironmentCollection,
    ) -> IndexSet<Label> {
        let mut missing_environments: IndexSet<Label> = IndexSet::new();
        let actual_environment_labels = actual_environments.get_environments();

        // Check if each explicitly expected environment is satisfied.
        for expected_env in expected_environments.get_grouped_environments() {
            let group = &expected_env.group;
            let environment = &expected_env.environment;
            let mut is_satisfied = false;
            if actual_environments.contains_group(group) {
                // If the actual environments include members from the expected environment's
                // group, we need to either find the environment itself or another one that
                // transitively fulfills it.
                if actual_environment_labels.contains(environment)
                    || intersect(
                        &actual_environment_labels,
                        &group.get_fulfillers(environment).to_list(),
                    )
                {
                    is_satisfied = true;
                }
            } else {
                // If the actual environments don't reference the expected environment's group at
                // all, the group's defaults are implicitly included. So we need to check those
                // defaults for either the expected environment or another environment that
                // transitively fulfills it.
                let defaults: Vec<Label> = group.get_defaults().iter().cloned().collect();
                if group.is_default(environment)
                    || intersect(&group.get_fulfillers(environment).to_list(), &defaults)
                {
                    is_satisfied = true;
                }
            }
            if !is_satisfied {
                missing_environments.insert(environment.clone());
            }
        }

        // For any environment group not referenced by the expected environments, its defaults
        // are implicitly expected. We can ignore this if the actual environments also don't
        // reference the group (since in that case the same defaults apply), otherwise we have to
        // check.
        for group in actual_environments.get_groups() {
            if !expected_environments.contains_group(&group) {
                for expected_default in group.get_defaults() {
                    if !actual_environment_labels.contains(expected_default)
                        && !intersect(
                            &actual_environment_labels,
                            &group.get_fulfillers(expected_default).to_list(),
                        )
                    {
                        missing_environments.insert(expected_default.clone());
                    }
                }
            }
        }

        missing_environments
    }

    /// Returns all dependencies that should be constraint-checked against the current rule,
    /// including both "uncoditional" deps (outside selects) and deps that only appear in selects.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:725 RuleContextConstraintSemantics.getConstraintCheckedDependencies
    fn get_constraint_checked_dependencies(rule_context: &RuleContext) -> DepsToCheck {
        let mut deps_to_check: Vec<ConfiguredTarget> = Vec::new();
        let mut select_only_deps: Vec<ConfiguredTarget> = Vec::new();
        let mut deps_outside_selects: Vec<ConfiguredTarget> = Vec::new();

        let attributes = rule_context.attributes();
        for attr in attributes.get_attribute_names() {
            let attr_def = attributes
                .get_attribute_definition(&attr)
                .expect("attribute definition");
            if attr_def.get_type().get_label_class() != LabelClass::Dependency
                || attr_def.skip_constraints_override()
            {
                continue;
            }
            if !attr_def.check_constraints_override() {
                // Use the same implicit deps check that query uses. This facilitates running
                // queries to determine exactly which rules need to be constraint-annotated for
                // depot migrations.
                if !DependencyFilter::no_implicit_deps()
                    .test(rule_context.get_rule().as_ref(), attr_def)
                    || attr_def.is_tool_dependency()
                {
                    continue;
                }
            }

            let select_only_deps_for_this_attribute = Self::get_deps_only_in_selects(
                rule_context,
                &attr,
                attributes
                    .get_attribute_type(&attr)
                    .expect("attribute type"),
            );
            for dep in rule_context.get_prerequisites(&attr) {
                // For normal configured targets the target's label is the same label appearing
                // in the select(). But for AliasConfiguredTargets the label in the select()
                // refers to the alias, while dep.getLabel() refers to the target the alias
                // points to. So add this quick check to make sure we're comparing the same
                // labels.
                let dep_label_in_select = dep.get_original_label();
                // Output files inherit the environment spec of their generating rule.
                let dep = match &dep {
                    // Note this reassignment means constraint violation errors reference the
                    // generating rule, not the file. This makes the source of the environmental
                    // mismatch more clear.
                    ConfiguredTarget::OutputFile(output_file) => {
                        ConfiguredTarget::Rule(output_file.get_generating_rule().clone())
                    }
                    _ => dep,
                };
                // Input files don't support environments. We may subsequently opt them into
                // constraint checking, but for now just pass them by.
                if dep.get_provider::<SupportedEnvironments>().is_some() {
                    add_target(&mut deps_to_check, &dep);
                    if !select_only_deps_for_this_attribute.contains(&dep_label_in_select) {
                        add_target(&mut deps_outside_selects, &dep);
                    }
                }
            }
        }

        for dep in &deps_to_check {
            if !deps_outside_selects.iter().any(|d| same_target(d, dep)) {
                add_target(&mut select_only_deps, dep);
            }
        }

        DepsToCheck {
            all_deps: deps_to_check,
            select_only_deps,
        }
    }

    /// Returns the deps for this attribute that only appear in selects.
    ///
    /// For example: `deps = [":a"] + select({"//foo:cond": [":b"]}) + select({"//conditions:default": [":c"]})`
    /// returns `[":b"]`. Even though `":c"` also appears in a select, that's a degenerate case
    /// with only one always-chosen condition. So that's considered the same as an unconditional
    /// dep.
    ///
    /// Note that just because a dep only appears in selects for this attribute doesn't mean it
    /// won't appear unconditionally in another attribute.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:799 RuleContextConstraintSemantics.getDepsOnlyInSelects
    fn get_deps_only_in_selects(
        rule_context: &RuleContext,
        attr: &str,
        attr_type: Type,
    ) -> IndexSet<Label> {
        let rule = rule_context.get_rule();
        if !rule.is_configurable_attribute(attr) || !attr_type.is_label_type() {
            return IndexSet::new();
        }
        let mut unconditional_deps: IndexSet<Label> = IndexSet::new();
        let mut selectable_deps: IndexSet<Label> = IndexSet::new();
        // RawAttributeMapper.of(rule).getSelectorList(attr, attrType)
        let select_list = rule
            .get_selector_list(attr, attr_type)
            .expect("configurable attribute has a selector list");
        for select in select_list.get_selectors() {
            // Java: addSelectValuesToSet
            let set = if select.is_unconditional() {
                &mut unconditional_deps
            } else {
                &mut selectable_deps
            };
            let type_ = select.get_original_type();
            select.for_each(|_, value: Option<&AttrValue>| {
                // PORT: a `None` branch holds the type's default, which has no labels.
                if let Some(value) = value {
                    type_.visit_labels(
                        &mut |label, _| {
                            set.insert(label.clone());
                        },
                        value,
                        None,
                    );
                }
            });
        }
        // Sets.difference(selectableDeps, unconditionalDeps)
        selectable_deps
            .into_iter()
            .filter(|l| !unconditional_deps.contains(l))
            .collect()
    }
}

/// Provides information about a target's incompatibility.
///
/// After calling `check_for_incompatibility()`, the `is_incompatible` getter tells you whether
/// the target is incompatible. If the target is incompatible, then `underlying_target` tells you
/// which underlying target provided the incompatibility. For the vast majority of targets this is
/// the same one passed to `check_for_incompatibility()`. In some instances like
/// `OutputFileConfiguredTarget`, however, the `underlying_target` is the rule that generated the
/// file.
// Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:835 RuleContextConstraintSemantics.IncompatibleCheckResult
#[derive(Clone, Debug)]
pub struct IncompatibleCheckResult {
    is_incompatible: bool,
    underlying_target: ConfiguredTarget,
}

impl IncompatibleCheckResult {
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:840 RuleContextConstraintSemantics.IncompatibleCheckResult.create
    fn create(is_incompatible: bool, underlying_target: ConfiguredTarget) -> Self {
        IncompatibleCheckResult {
            is_incompatible,
            underlying_target,
        }
    }

    pub fn is_incompatible(&self) -> bool {
        self.is_incompatible
    }

    pub fn underlying_target(&self) -> &ConfiguredTarget {
        &self.underlying_target
    }
}

impl RuleContextConstraintSemantics {
    /// Checks whether the target is incompatible.
    ///
    /// See the documentation for [`IncompatibleCheckResult`] for more information.
    // Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:853 RuleContextConstraintSemantics.checkForIncompatibility
    pub fn check_for_incompatibility(target: &ConfiguredTarget) -> IncompatibleCheckResult {
        let target = match target {
            // For generated files, we want to query the generating rule for providers. genrule()
            // for example doesn't attach providers like IncompatiblePlatformProvider to its
            // outputs.
            ConfiguredTarget::OutputFile(o) => {
                ConfiguredTarget::Rule(o.get_generating_rule().clone())
            }
            other => other.clone(),
        };
        IncompatibleCheckResult::create(
            target
                .get(
                    &crate::config::incompatible_platform_provider::INCOMPATIBLE_PLATFORM_PROVIDER
                        .key(),
                )
                .is_some(),
            target,
        )
    }
}

/// `EnvironmentWithGroup` as a set element (Java record equality and hashing).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentWithGroupKey(EnvironmentWithGroup);

impl std::hash::Hash for EnvironmentWithGroupKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Equal records have equal environments and equally labelled groups.
        self.0.environment.hash(state);
        self.0.group.label.hash(state);
    }
}

// Java: com/google/devtools/build/lib/analysis/constraints/RuleContextConstraintSemantics.java:717 RuleContextConstraintSemantics.intersect
fn intersect(labels1: &[Label], labels2: &[Label]) -> bool {
    let set2: HashSet<&Label> = labels2.iter().collect();
    labels1.iter().any(|l| set2.contains(l))
}
