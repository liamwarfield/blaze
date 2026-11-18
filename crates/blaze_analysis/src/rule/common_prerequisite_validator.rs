//! Ports of `analysis/CommonPrerequisiteValidator.java` and
//! `bazel/rules/BazelPrerequisiteValidator.java` (visibility, test-only, deprecation and
//! transitive-visibility checks of each direct prerequisite).

use std::sync::Arc;

use blaze_cmdline::{Label, PackageIdentifier};
use blaze_packages::attribute::{Attribute, AttributeMap, NonconfigurableAttributeMapper};
use blaze_packages::macros::MacroInstance;
use blaze_packages::rule_class::RuleClass;
use blaze_packages::target::{InputFile, Rule, RuleOrMacroInstance, Target};
use blaze_packages::types::Type;

use crate::configured_target::ConfiguredTarget;
use crate::configured_target_and_data::ConfiguredTargetAndData;
use crate::providers::alias_provider::{AliasProvider, TargetMode};
use crate::providers::package_specification_provider::{
    PackageSpecificationProvider, TransitiveVisibilityProvider,
};
use crate::rule::config_util;
use crate::rule::configured_target_and_data_ext::ConfiguredTargetAndDataExt;
use crate::rule::rule_context::{CONFIG_SETTING_DEPS_ATTRIBUTE, RuleContext, RuleContextBuilder};
use crate::rule::rule_error_consumer::RuleErrorConsumer;

/// Java `FunctionSplitTransitionAllowlist.ATTRIBUTE_NAME`.
// Java: com/google/devtools/build/lib/packages/FunctionSplitTransitionAllowlist.java:35 FunctionSplitTransitionAllowlist.ATTRIBUTE_NAME
const FUNCTION_SPLIT_TRANSITION_ALLOWLIST_ATTRIBUTE_NAME: &str = "$allowlist_function_transition";

/// Bazel's prerequisite validator (Java `BazelPrerequisiteValidator`, which extends
/// `CommonPrerequisiteValidator`).
// Java: com/google/devtools/build/lib/bazel/rules/BazelPrerequisiteValidator.java:24 BazelPrerequisiteValidator
pub struct BazelPrerequisiteValidator;

/// What a visibility check is about (Java `CommonPrerequisiteValidator.VisibilityCheckState`).
// Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:70 CommonPrerequisiteValidator.VisibilityCheckState
struct VisibilityCheckState<'a> {
    prerequisite: &'a ConfiguredTargetAndData,
    consumer: Arc<Rule>,
    /// The rule class owning an implicit dependency (Java `owningRule`).
    owning_rule: Option<Arc<RuleClass>>,
    /// Macros the dependency was delegated through, innermost first.
    delegated_through: Vec<Arc<MacroInstance>>,
    verbose_visibility_errors: bool,
}

impl VisibilityCheckState<'_> {
    fn is_implicit_dep(&self) -> bool {
        // PORT: aspect owners are not ported (no aspect contexts).
        self.owning_rule.is_some()
    }

    fn get_owner_kind(&self) -> Option<&'static str> {
        self.owning_rule.as_ref().map(|_| "rule")
    }

    fn get_owner_name(&self) -> Option<String> {
        self.owning_rule.as_ref().map(|r| r.get_name().to_owned())
    }

    fn get_owner_definition_bzl(&self) -> Option<Label> {
        self.owning_rule
            .as_ref()
            .and_then(|r| r.get_rule_definition_environment_label().cloned())
    }

    fn get_owner_definition_location(&self) -> Option<PackageIdentifier> {
        self.get_owner_definition_bzl()
            .map(|l| l.get_package_identifier().clone())
    }
}

impl BazelPrerequisiteValidator {
    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:51 CommonPrerequisiteValidator.validate
    pub fn validate(
        &self,
        context_builder: &RuleContextBuilder,
        prerequisite: &ConfiguredTargetAndData,
        attribute: &Attribute,
    ) {
        self.validate_direct_prerequisite_location(context_builder, prerequisite);
        self.check_for_misplaced_package_groups(context_builder, prerequisite, attribute);
        self.validate_direct_prerequisite_visibility(context_builder, prerequisite, attribute);
        self.validate_direct_prerequisite_for_test_only(context_builder, prerequisite);
        self.validate_direct_prerequisite_for_deprecation(
            context_builder,
            context_builder.get_rule(),
            prerequisite,
            context_builder.for_aspect(),
        );
        self.validate_transitive_visibility(context_builder, prerequisite, attribute);
    }

    // Java: com/google/devtools/build/lib/bazel/rules/BazelPrerequisiteValidator.java:27 BazelPrerequisiteValidator.isSameLogicalPackage
    fn is_same_logical_package(
        &self,
        this_package: &PackageIdentifier,
        prerequisite_package: &PackageIdentifier,
    ) -> bool {
        this_package == prerequisite_package
    }

    // Java: com/google/devtools/build/lib/bazel/rules/BazelPrerequisiteValidator.java:33 BazelPrerequisiteValidator.packageUnderExperimental
    pub fn package_under_experimental(&self, _package_identifier: &PackageIdentifier) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/bazel/rules/BazelPrerequisiteValidator.java:38 BazelPrerequisiteValidator.checkVisibilityForExperimental
    fn check_visibility_for_experimental(&self, _context: &RuleContextBuilder) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/bazel/rules/BazelPrerequisiteValidator.java:43 BazelPrerequisiteValidator.allowExperimentalDeps
    fn allow_experimental_deps(&self, _context: &RuleContextBuilder) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:119 CommonPrerequisiteValidator.validateDirectPrerequisiteVisibility
    fn validate_direct_prerequisite_visibility(
        &self,
        context: &RuleContextBuilder,
        prerequisite: &ConfiguredTargetAndData,
        attribute: &Attribute,
    ) {
        let attr_name = attribute.get_name();
        let rule = context.get_rule();

        if !config_util::core_options(context.get_configuration()).check_visibility {
            return;
        }

        // We don't check the visibility of late-bound attributes, because it would break some
        // features.
        if Attribute::is_analysis_dependent(attr_name) {
            return;
        }

        // Only verify visibility of implicit dependencies of the current aspect. Dependencies of
        // other aspects as well as the rule itself are checked when they are evaluated.
        if attr_name == RuleContext::TOOLCHAIN_ATTR_NAME {
            return;
        }
        // PORT: aspect contexts are not ported (`context.getMainAspect()` is always null).

        let check_experimental = self.check_visibility_for_experimental(context);

        // Determine if we should use the new visibility rules for tools.
        let mut is_implicit_dep = attribute.is_implicit() && context.is_starlark_rule_or_aspect();
        is_implicit_dep &= attribute.get_name() != CONFIG_SETTING_DEPS_ATTRIBUTE;

        let mut state = VisibilityCheckState {
            prerequisite,
            consumer: rule.clone(),
            owning_rule: None,
            delegated_through: Vec::new(),
            verbose_visibility_errors: config_util::core_options(context.get_configuration())
                .verbose_visibility_errors,
        };

        if is_implicit_dep {
            // Implicit dependency of a rule: the owning rule is the topmost Starlark ancestor
            // rule class that declares the attribute.
            let mut owning_rule = rule.get_rule_class_object().clone();
            while let Some(parent) = owning_rule.get_starlark_parent() {
                if parent
                    .get_attribute_provider()
                    .get_attribute_by_name_maybe(attr_name)
                    .is_none()
                {
                    break;
                }
                owning_rule = parent.clone();
            }
            state.owning_rule = Some(owning_rule);

            let owner_location = state.get_owner_definition_location();
            if !self.is_visible_to_location(
                prerequisite,
                owner_location.as_ref(),
                check_experimental,
            ) {
                // The implicit dependency is not visible from the rule (or aspect) definition;
                // fall back on checking the consumer itself.
                let consumer: &dyn RuleOrMacroInstance = rule.as_ref();
                if !self.is_visible_to_consumer(
                    prerequisite,
                    consumer,
                    check_experimental,
                    &mut state.delegated_through,
                ) {
                    context.rule_error(&self.generate_visibility_conflict_message(&state));
                }
            }
        } else {
            let consumer: &dyn RuleOrMacroInstance = rule.as_ref();
            if !self.is_visible_to_consumer(
                prerequisite,
                consumer,
                check_experimental,
                &mut state.delegated_through,
            ) {
                context.rule_error(&self.generate_visibility_conflict_message(&state));
            }
        }
    }

    /// Returns whether `prerequisite` is visible to `consumer` (a rule or a macro), following
    /// delegation through symbolic macros.
    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:197 CommonPrerequisiteValidator.isVisibleToConsumer
    fn is_visible_to_consumer(
        &self,
        prerequisite: &ConfiguredTargetAndData,
        consumer: &dyn RuleOrMacroInstance,
        check_experimental: bool,
        delegated_through: &mut Vec<Arc<MacroInstance>>,
    ) -> bool {
        let declaring_macro = consumer.get_declaring_macro().cloned();

        if let Some(declaring_macro) = &declaring_macro {
            // The consumer was declared in a macro. Check whether the macro's caller passed in
            // the prerequisite as an explicit attribute value.
            let prereq_label =
                AliasProvider::get_dependency_label(prerequisite.get_configured_target());
            let mut declaring_macro_was_given_prereq_by_caller = false;
            declaring_macro.visit_explicit_attribute_labels(&mut |label: &Label| {
                declaring_macro_was_given_prereq_by_caller |= *label == prereq_label;
            });
            if declaring_macro_was_given_prereq_by_caller {
                delegated_through.push(declaring_macro.clone());
                let macro_consumer: &dyn RuleOrMacroInstance = declaring_macro.as_ref();
                return self.is_visible_to_consumer(
                    prerequisite,
                    macro_consumer,
                    check_experimental,
                    delegated_through,
                );
            }
        }

        let package_of_consumer = consumer.get_package_metadata().package_identifier.clone();

        // Finalizer macros can see everything their package can see.
        if let Some(m) = &declaring_macro {
            if m.get_macro_class().is_finalizer()
                && self.is_visible_to_location(
                    prerequisite,
                    Some(&package_of_consumer),
                    check_experimental,
                )
            {
                return true;
            }
        }

        let declaring_location = match &declaring_macro {
            Some(m) => m.get_definition_package().clone(),
            None => package_of_consumer,
        };
        self.is_visible_to_location(prerequisite, Some(&declaring_location), check_experimental)
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:243 CommonPrerequisiteValidator.isVisibleToLocation
    fn is_visible_to_location(
        &self,
        prerequisite: &ConfiguredTargetAndData,
        location: Option<&PackageIdentifier>,
        check_experimental: bool,
    ) -> bool {
        let Some(location) = location else {
            // Java: a null location (no owner definition) would throw in containsPackage.
            panic!("NullPointerException: visibility location");
        };
        if self.package_under_experimental(location) && !check_experimental {
            return true;
        }

        let visibility =
            crate::configuredtargets::get_visibility_provider(prerequisite.get_configured_target());

        // If the prerequisite was not created in a symbolic macro, it is visible to its own
        // package.
        if !visibility.is_created_in_symbolic_macro() {
            if self.is_same_logical_package(
                location,
                AliasProvider::get_dependency_label(prerequisite.get_configured_target())
                    .get_package_identifier(),
            ) {
                return true;
            }
        }

        for specification in visibility.get_visibility().to_list().iter() {
            if specification.get().contains_package(location) {
                return true;
            }
        }
        false
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:272 CommonPrerequisiteValidator.checkForMisplacedPackageGroups
    fn check_for_misplaced_package_groups(
        &self,
        context: &RuleContextBuilder,
        prerequisite: &ConfiguredTargetAndData,
        attribute: &Attribute,
    ) {
        let attr_name = attribute.get_name();
        let rule = context.get_rule();

        if matches!(
            prerequisite.get_configured_target().unwrap_if_merged(),
            ConfiguredTarget::PackageGroup(_)
        ) {
            let raw = blaze_packages::attribute::RawAttributeMapper::of(rule.as_ref());
            let configured_attribute = raw.get_attribute_definition(attr_name).unwrap_or(attribute); // handles aspects
            let description = configured_attribute
                .get_required_providers()
                .get_description();
            let contains_package_specification_provider = description
                .contains("PackageSpecificationProvider")
                || description.contains("PackageSpecificationInfo");
            if attr_name != "visibility"
                && attr_name != FUNCTION_SPLIT_TRANSITION_ALLOWLIST_ATTRIBUTE_NAME
                && !contains_package_specification_provider
            {
                context.attribute_error(
                    attr_name,
                    &format!(
                        "in {} attribute of {} rule {}: {} is misplaced here (they are only allowed in the visibility attribute)",
                        attr_name,
                        rule.get_rule_class(),
                        rule.get_label(),
                        AliasProvider::describe_target_with_aliases(prerequisite, TargetMode::WithKind)
                    ),
                );
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:310 CommonPrerequisiteValidator.generateVisibilityConflictMessage
    fn generate_visibility_conflict_message(&self, state: &VisibilityCheckState) -> String {
        if !state.verbose_visibility_errors {
            let consumer_or_owner_location = if state.is_implicit_dep() {
                state.get_owner_definition_bzl().expect("owner bzl")
            } else {
                state.consumer.get_label().clone()
            };
            let mut error_message = format!(
                "Visibility error:\n{} is not visible from\ntarget '{}'\nRecommendation: modify the visibility declaration if you think the dependency is legitimate. For more info see https://bazel.build/concepts/visibility",
                AliasProvider::describe_target_with_aliases(
                    state.prerequisite,
                    TargetMode::WithoutKind
                ),
                consumer_or_owner_location.get_canonical_form()
            );
            if state.prerequisite.get_target_kind() == InputFile::target_kind() {
                error_message.push_str(
                    ". To set the visibility of that source file target, use the exports_files() function",
                );
            }
            error_message
        } else {
            let dependency_desc = state.prerequisite.get_target_label().get_canonical_form();
            let mut error_message = format!(
                "dependency on target {dependency_desc} violates its visibility. Additional diagnostics:"
            );
            let mut bullets: Vec<String> = Vec::new();
            self.maybe_add_implicit_dep_bullet(state, &mut bullets);
            self.add_consuming_location_bullet(state, &mut bullets);
            self.maybe_add_alias_disclaimer_bullet(state, &mut bullets);
            self.maybe_add_same_package_disclaimer_bullet(state, &mut bullets);
            self.maybe_add_more_delegation_needed_bullet(state, &mut bullets);
            self.add_edit_visibility_bullet(state, &mut bullets);
            for bullet in bullets {
                error_message.push_str("\n\n    * ");
                error_message.push_str(&bullet);
            }
            error_message
        }
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:358 CommonPrerequisiteValidator.maybeAddImplicitDepBullet
    fn maybe_add_implicit_dep_bullet(
        &self,
        state: &VisibilityCheckState,
        bullets: &mut Vec<String>,
    ) {
        if !state.is_implicit_dep() {
            return;
        }
        bullets.push(format!(
            "The dependency is an implicit dependency of the consuming target's {}, {}, which is defined in {}. Since that file's package, {}, does not match the dependency's visibility, we are falling back on checking the consuming target itself. The following bullet points explain why the consuming target also did not match.",
            state.get_owner_kind().unwrap(),
            state.get_owner_name().unwrap(),
            state.get_owner_definition_bzl().unwrap(),
            state.get_owner_definition_location().unwrap().get_canonical_form()
        ));
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:378 CommonPrerequisiteValidator.addConsumingLocationBullet
    fn add_consuming_location_bullet(
        &self,
        state: &VisibilityCheckState,
        bullets: &mut Vec<String>,
    ) {
        let consumer = &state.consumer;
        let declaring_package = Target::Rule(consumer.clone())
            .get_declaring_package()
            .get_canonical_form();
        if state.delegated_through.is_empty() {
            match consumer.get_declaring_macro() {
                None => bullets.push(format!(
                    "The location being checked is the package where the consuming target lives, {declaring_package}."
                )),
                Some(declaring_macro) => bullets.push(format!(
                    "Because the consuming target was declared in the body of the symbolic macro {} defined in {}, the location being checked is this file's package, {}.",
                    declaring_macro.get_macro_class().get_name(),
                    declaring_macro
                        .get_macro_class()
                        .get_defining_bzl_label()
                        .get_canonical_form(),
                    declaring_package
                )),
            }
        } else {
            let outermost_delegated = state.delegated_through.last().unwrap();
            let consuming_location = match outermost_delegated.get_parent() {
                None => format!(
                    "package {}",
                    outermost_delegated
                        .get_package_metadata()
                        .package_identifier
                        .get_canonical_form()
                ),
                Some(delegation_parent) => format!(
                    "the body of the calling macro {}, defined in {} of package {}",
                    delegation_parent.get_macro_class().get_name(),
                    delegation_parent.get_macro_class().get_defining_bzl_label(),
                    delegation_parent
                        .get_definition_package()
                        .get_canonical_form()
                ),
            };
            bullets.push(format!(
                "Because the dependency was{} passed to the consuming target from an attribute of the symbolic macro {}, the location being checked is the place where this macro is declared: {}.",
                if state.delegated_through.len() > 1 {
                    " transitively"
                } else {
                    ""
                },
                outermost_delegated.get_label(),
                consuming_location
            ));
        }
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:429 CommonPrerequisiteValidator.maybeAddAliasDisclaimerBullet
    fn maybe_add_alias_disclaimer_bullet(
        &self,
        state: &VisibilityCheckState,
        bullets: &mut Vec<String>,
    ) {
        if AliasProvider::is_alias(state.prerequisite.get_configured_target()) {
            bullets.push(
                "The dependency is an alias target. Note that it is the visibility of the alias we care about, not the visibility of the underlying target it refers to."
                    .to_owned(),
            );
        }
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:439 CommonPrerequisiteValidator.maybeAddSamePackageDisclaimerBullet
    fn maybe_add_same_package_disclaimer_bullet(
        &self,
        state: &VisibilityCheckState,
        bullets: &mut Vec<String>,
    ) {
        if state.is_implicit_dep() {
            return;
        }
        let dependency_location = state
            .prerequisite
            .get_target_label()
            .get_package_identifier()
            .clone();
        let consumer_location = state.consumer.get_label().get_package_identifier();
        if *consumer_location != dependency_location {
            return;
        }
        bullets.push(
            "Although both targets live in the same package, they cannot automatically see each other because they are declared by different symbolic macros."
                .to_owned(),
        );
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:460 CommonPrerequisiteValidator.maybeAddMoreDelegationNeededBullet
    fn maybe_add_more_delegation_needed_bullet(
        &self,
        state: &VisibilityCheckState,
        bullets: &mut Vec<String>,
    ) {
        let delegation_parent = match state.delegated_through.last() {
            None => state.consumer.get_declaring_macro().cloned(),
            Some(outermost) => outermost.get_declaring_macro().cloned(),
        };
        let Some(delegation_parent) = delegation_parent else {
            return;
        };

        let mut more_than_one_level_up = false;
        let mut m = delegation_parent.get_parent().cloned();
        while let Some(current) = m {
            if self.is_visible_to_location(
                state.prerequisite,
                Some(current.get_definition_package()),
                /* check_experimental= */ true,
            ) {
                bullets.push(format!(
                    "Although the dependency is not visible to the location being checked, it is visible to this location's{} caller, {}, a {} macro defined in {}. (Perhaps {} needs to pass in the dependency as an argument?)",
                    if more_than_one_level_up { " transitive" } else { "" },
                    current.get_label(),
                    current.get_macro_class().get_name(),
                    current.get_definition_package().get_canonical_form(),
                    if more_than_one_level_up {
                        "this caller, or an intermediate caller,"
                    } else {
                        "the caller"
                    }
                ));
                return;
            }
            more_than_one_level_up = true;
            m = current.get_parent().cloned();
        }

        let consumer_package = state
            .consumer
            .get_package_metadata()
            .package_identifier
            .clone();
        if self.is_visible_to_location(state.prerequisite, Some(&consumer_package), true) {
            bullets.push(format!(
                "Although the dependency is not visible to the location being checked, it is visible to this location's{} caller, the BUILD file of package {}. (Perhaps {} needs to pass in the dependency as an argument?)",
                if more_than_one_level_up { " transitive" } else { "" },
                consumer_package.get_canonical_form(),
                if more_than_one_level_up {
                    "this caller, or an intermediate caller,"
                } else {
                    "the caller"
                }
            ));
        }
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:519 CommonPrerequisiteValidator.addEditVisibilityBullet
    fn add_edit_visibility_bullet(&self, state: &VisibilityCheckState, bullets: &mut Vec<String>) {
        let is_source_file = state.prerequisite.get_target_kind() == InputFile::target_kind();
        bullets.push(format!(
            "If you think the dependency{} is legitimate, consider updating its visibility declaration{}. For more info see https://bazel.build/concepts/visibility.",
            if is_source_file { " on this source file" } else { "" },
            if is_source_file { " using exports_files()" } else { "" }
        ));
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:532 CommonPrerequisiteValidator.validateTransitiveVisibility
    fn validate_transitive_visibility(
        &self,
        context_builder: &RuleContextBuilder,
        prerequisite: &ConfiguredTargetAndData,
        attribute: &Attribute,
    ) {
        if !config_util::enforce_transitive_visibility(context_builder.get_configuration()) {
            return;
        }
        let Some(tv_provider) = crate::configuredtargets::get_provider::<
            TransitiveVisibilityProvider,
        >(prerequisite.get_configured_target()) else {
            return;
        };
        for transitive_visibility_declaration in tv_provider.get_transitive_visibility() {
            if !PackageSpecificationProvider::is_available_for(
                transitive_visibility_declaration.get_package_specifications(),
                context_builder.get_rule().get_label(),
            ) {
                context_builder.attribute_error(
                    attribute.get_name(),
                    &format!(
                        "Transitive visibility error: {1} is not transitively visible from {0}. {1} inherits a transitive_visibility declaration from its package or one of its dependencies that does not allow {0}",
                        context_builder.get_rule().get_label(),
                        prerequisite.get_target_label()
                    ),
                );
                return;
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:563 CommonPrerequisiteValidator.validateDirectPrerequisiteLocation
    fn validate_direct_prerequisite_location(
        &self,
        context: &RuleContextBuilder,
        prerequisite: &ConfiguredTargetAndData,
    ) {
        let rule = context.get_rule();
        let prerequisite_label = prerequisite.get_target_label();
        if self.package_under_experimental(prerequisite_label.get_package_identifier())
            && !self.package_under_experimental(rule.get_label().get_package_identifier())
        {
            let message = format!(
                "non-experimental target '{}' depends on experimental target '{}'",
                rule.get_label(),
                prerequisite_label
            );
            if self.allow_experimental_deps(context) {
                context.rule_warning(&format!(
                    "{message} (ignored due to --experimental_deps_ok; do not submit)"
                ));
            } else {
                context.rule_error(&format!(
                    "{message} (you may not check in such a dependency, though you can test against it by passing --experimental_deps_ok)"
                ));
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:592 CommonPrerequisiteValidator.validateDirectPrerequisiteForDeprecation
    fn validate_direct_prerequisite_for_deprecation(
        &self,
        errors: &dyn RuleErrorConsumer,
        rule: &Arc<Rule>,
        prerequisite: &ConfiguredTargetAndData,
        for_aspect: bool,
    ) {
        // Java: attributeOrNull(rule, "deprecation", Type.STRING) != null.
        let rule_deprecation = if rule.is_attr_defined("deprecation", Type::String) {
            rule.get_attr("deprecation")
                .filter(|v| v.as_str().is_some())
        } else {
            None
        };
        if for_aspect || rule_deprecation.is_some() {
            return;
        }

        let Some(warning) = prerequisite.get_target().get_deprecation_warning() else {
            return; // No warning if it's not deprecated.
        };

        let this_package = rule.get_label().get_package_identifier();
        let prerequisite_label = prerequisite.get_target_label();
        let that_package = prerequisite_label.get_package_identifier();
        if self.is_same_logical_package(this_package, that_package) {
            return; // Doesn't report deprecation edges within a package.
        }

        match prerequisite.get_target().get_generating_rule_label() {
            Some(generating_rule_label) => errors.rule_warning(&format!(
                "target '{}' depends on the output file {} of a deprecated rule {}': {}",
                rule.get_label(),
                prerequisite_label,
                generating_rule_label,
                warning
            )),
            None => errors.rule_warning(&format!(
                "target '{}' depends on deprecated target '{}': {}",
                rule.get_label(),
                prerequisite_label,
                warning
            )),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:637 CommonPrerequisiteValidator.validateDirectPrerequisiteForTestOnly
    fn validate_direct_prerequisite_for_test_only(
        &self,
        context: &RuleContextBuilder,
        prerequisite: &ConfiguredTargetAndData,
    ) {
        let rule = context.get_rule();

        if rule
            .get_rule_class_object()
            .get_advertised_providers()
            .can_have_any_provider()
        {
            // testonly-ness will be checked directly between the depender and the target of the
            // alias; getTarget() called by the depender will not return the alias rule, but its
            // actual target
            return;
        }

        if !prerequisite.get_target().is_test_only() || Self::is_test_only_rule(rule) {
            return;
        }

        let generating_rule_label = prerequisite.get_target().get_generating_rule_label();
        let message = match generating_rule_label {
            None => format!(
                "non-test target '{}' depends on testonly {} and doesn't have testonly attribute set",
                rule.get_label(),
                AliasProvider::describe_target_with_aliases(prerequisite, TargetMode::WithoutKind)
            ),
            Some(generating_rule_label)
                if config_util::core_options(context.get_configuration())
                    .check_testonly_for_output_files =>
            {
                format!(
                    "non-test target '{}' depends on the output file {} of a testonly rule {} and doesn't have testonly attribute set",
                    rule.get_label(),
                    AliasProvider::describe_target_with_aliases(
                        prerequisite,
                        TargetMode::WithoutKind
                    ),
                    generating_rule_label
                )
            }
            Some(_) => return,
        };

        let this_package = rule.get_label().get_package_identifier();
        if self.package_under_experimental(this_package) {
            context.rule_warning(&message);
        } else {
            context.rule_error(&message);
        }
    }

    // Java: com/google/devtools/build/lib/analysis/CommonPrerequisiteValidator.java:683 CommonPrerequisiteValidator.isTestOnlyRule
    fn is_test_only_rule(rule: &Rule) -> bool {
        let mapper = NonconfigurableAttributeMapper::of(rule);
        mapper.has_typed("testonly", Type::Boolean)
            && mapper
                .get("testonly", Type::Boolean)
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
    }
}
