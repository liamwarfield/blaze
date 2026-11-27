//! Port of `packages/AspectDefinition.java`.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use indexmap::IndexMap;
use indexmap::IndexSet;

use super::Aspect;
use super::AspectClass;
use super::AspectPropagationEdgesSupplier;
use super::AspectPropagationPredicate;
use super::aspect_propagation_edges_supplier::default_attr_aspects_supplier;
use super::aspect_propagation_edges_supplier::default_toolchains_aspects_supplier;
use crate::attribute::Attribute;
use crate::attribute::AttributeBuilder;
use crate::attribute::DependencyFilter;
use crate::rule_class::AdvertisedProviderSet;
use crate::rule_class::AdvertisedProviderSetBuilder;
use crate::rule_class::ConfigurationFragmentPolicy;
use crate::rule_class::ConfigurationFragmentPolicyBuilder;
use crate::rule_class::DeclaredExecGroup;
use crate::rule_class::MissingFragmentPolicy;
use crate::rule_class::RequiredProviders;
use crate::rule_class::RequiredProvidersBuilder;
use crate::rule_class::StarlarkCallable;
use crate::rule_class::StarlarkProviderIdentifier;
use crate::rule_class::ToolchainTypeRequirement;
use crate::types::AttrValue;
use crate::types::LabelClass;
use crate::types::Type;

/// The definition of an aspect (Java `AspectDefinition`; see [`Aspect`]).
///
/// Contains enough information to build up the configured target graph except for the actual
/// way to build the Skyframe node: the condition a configured target must fulfil for the aspect
/// to operate on it, the (implicit or late-bound) attributes of the aspect that denote
/// dependencies the aspect itself needs, and the aspects this aspect requires from its direct
/// dependencies.
///
/// Java does not override `equals`: definitions are compared by identity. They are handed out
/// as `Arc<AspectDefinition>` and compared with `Arc::ptr_eq`; this type deliberately has no
/// `PartialEq`.
// Java: com/google/devtools/build/lib/packages/AspectDefinition.java:59 AspectDefinition
pub struct AspectDefinition {
    aspect_class: AspectClass,
    advertised_providers: AdvertisedProviderSet,
    required_providers: RequiredProviders,
    required_providers_for_aspects: RequiredProviders,
    attributes: IndexMap<String, Attribute>,
    toolchain_types: IndexSet<ToolchainTypeRequirement>,
    /// A supplier of the attributes to which the aspect will propagate.
    restrict_to_attributes: AspectPropagationEdgesSupplier<String>,
    /// A supplier of the toolchains types for which the aspect will propagate to matching
    /// resolved toolchains.
    propagate_to_toolchains_types: AspectPropagationEdgesSupplier<Label>,
    configuration_fragment_policy: ConfigurationFragmentPolicy,
    apply_to_files: bool,
    apply_to_generating_rules: bool,
    required_aspect_classes: IndexSet<AspectClass>,
    propagation_predicate: Option<AspectPropagationPredicate>,
    exec_compatible_with: IndexSet<Label>,
    exec_groups: IndexMap<String, DeclaredExecGroup>,
    /// Java `ImmutableSet<? extends StarlarkSubruleApi>`: keep-alive handles to the frozen
    /// subrule values.
    subrules: Vec<StarlarkCallable>,
}

impl fmt::Debug for AspectDefinition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AspectDefinition")
            .field("aspect_class", &self.aspect_class)
            .field("attributes", &self.attributes.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl AspectDefinition {
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:88 AspectDefinition.getAdvertisedProviders
    pub fn get_advertised_providers(&self) -> &AdvertisedProviderSet {
        &self.advertised_providers
    }

    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:127 AspectDefinition.getName
    pub fn get_name(&self) -> &str {
        self.aspect_class.get_name()
    }

    /// The aspect class the definition was built for (Java field `aspectClass`).
    pub fn get_aspect_class(&self) -> &AspectClass {
        &self.aspect_class
    }

    /// Returns the attributes of the aspect. All attributes are either implicit or late-bound.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:136 AspectDefinition.getAttributes
    pub fn get_attributes(&self) -> &IndexMap<String, Attribute> {
        &self.attributes
    }

    /// Returns the required toolchains declared by this aspect.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:141 AspectDefinition.getToolchainTypes
    pub fn get_toolchain_types(&self) -> &IndexSet<ToolchainTypeRequirement> {
        &self.toolchain_types
    }

    /// Returns the constraint values that must be present on an execution platform for this
    /// aspect.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:148 AspectDefinition.execCompatibleWith
    pub fn exec_compatible_with(&self) -> &IndexSet<Label> {
        &self.exec_compatible_with
    }

    /// Returns the execution groups that this aspect can use when creating actions.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:153 AspectDefinition.execGroups
    pub fn exec_groups(&self) -> &IndexMap<String, DeclaredExecGroup> {
        &self.exec_groups
    }

    /// Returns the subrules declared by this aspect.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:158 AspectDefinition.getSubrules
    pub fn get_subrules(&self) -> &[StarlarkCallable] {
        &self.subrules
    }

    /// Returns the [`RequiredProviders`] a configured target must have so that this aspect can
    /// be applied to it. If a configured target does not satisfy them, the aspect is silently
    /// not created for it.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:169 AspectDefinition.getRequiredProviders
    pub fn get_required_providers(&self) -> &RequiredProviders {
        &self.required_providers
    }

    /// Aspects do not depend on other aspects applied to the same target *unless* the other
    /// aspect satisfies the [`RequiredProviders`] this method returns.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:177 AspectDefinition.getRequiredProvidersForAspects
    pub fn get_required_providers_for_aspects(&self) -> &RequiredProviders {
        &self.required_providers_for_aspects
    }

    /// Returns the supplier of the attributes to which the aspect will propagate.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:182 AspectDefinition.getAttributeAspects
    pub fn get_attribute_aspects(&self) -> &AspectPropagationEdgesSupplier<String> {
        &self.restrict_to_attributes
    }

    /// Returns the supplier of the toolchains types for which the aspect will propagate to
    /// matching resolved toolchains.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:190 AspectDefinition.getToolchainsAspects
    pub fn get_toolchains_aspects(&self) -> &AspectPropagationEdgesSupplier<Label> {
        &self.propagate_to_toolchains_types
    }

    /// Returns the set of configuration fragments required by this aspect.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:195 AspectDefinition.getConfigurationFragmentPolicy
    pub fn get_configuration_fragment_policy(&self) -> &ConfigurationFragmentPolicy {
        &self.configuration_fragment_policy
    }

    /// Returns whether this aspect applies to (output) files. Currently only supported for
    /// top-level aspects and targets, and only for output files.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:204 AspectDefinition.applyToFiles
    pub fn apply_to_files(&self) -> bool {
        self.apply_to_files
    }

    /// Returns whether this aspect should, when it would be applied to an output file, instead
    /// apply to the generating rule of that output file.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:212 AspectDefinition.applyToGeneratingRules
    pub fn apply_to_generating_rules(&self) -> bool {
        self.apply_to_generating_rules
    }

    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:216 AspectDefinition.satisfies
    pub fn satisfies(aspect: &Aspect, advertised_provider_set: &AdvertisedProviderSet) -> bool {
        aspect
            .get_definition()
            .required_providers
            .is_satisfied_by(advertised_provider_set)
    }

    /// Checks if the given `maybe_required_aspect` is required by this aspect definition.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:221 AspectDefinition.requires
    pub fn requires(&self, maybe_required_aspect: &Aspect) -> bool {
        self.required_aspect_classes
            .contains(maybe_required_aspect.get_aspect_class())
    }

    /// The classes of the aspects this aspect requires (Java field `requiredAspectClasses`).
    pub fn get_required_aspect_classes(&self) -> &IndexSet<AspectClass> {
        &self.required_aspect_classes
    }

    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:226 AspectDefinition.getPropagationPredicate
    pub fn get_propagation_predicate(&self) -> Option<&AspectPropagationPredicate> {
        self.propagation_predicate.as_ref()
    }

    /// Collects all attribute labels from the specified aspect definition. `label_builder` is
    /// Java's `Multimap<Attribute, Label>`: every (attribute, label) pair is appended in visit
    /// order; deduplication is the caller's multimap semantics.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:231 AspectDefinition.addAllAttributesOfAspect
    pub fn add_all_attributes_of_aspect(
        label_builder: &mut Vec<(Attribute, Label)>,
        aspect: &Aspect,
        dependency_filter: &DependencyFilter,
    ) {
        AspectDefinition::for_each_label_dep_from_all_attributes_of_aspect(
            aspect,
            dependency_filter,
            &mut |attribute, label| label_builder.push((attribute.clone(), label.clone())),
        );
    }

    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:236 AspectDefinition.forEachLabelDepFromAllAttributesOfAspect
    pub fn for_each_label_dep_from_all_attributes_of_aspect(
        aspect: &Aspect,
        dependency_filter: &DependencyFilter,
        consumer: &mut dyn FnMut(&Attribute, &Label),
    ) {
        for aspect_attribute in aspect.get_definition().attributes.values() {
            if !dependency_filter.test(aspect, aspect_attribute) {
                continue;
            }
            let type_: Type = aspect_attribute.get_type();
            if type_.get_label_class() != LabelClass::Dependency {
                continue;
            }
            // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:259 AspectDefinition.visitSingleAttribute
            // Java: visitSingleAttribute (AspectDefinition.java:258); a null default visits
            // nothing (the LabelVisitor skips null labels).
            if let Some(default_value) = aspect_attribute.get_default_value(None) {
                type_.visit_labels(
                    &mut |label: &Label, _attr: Option<&Attribute>| {
                        consumer(aspect_attribute, label)
                    },
                    &default_value,
                    Some(aspect_attribute),
                );
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:264 AspectDefinition.builder
    pub fn builder(aspect_class: AspectClass) -> AspectDefinitionBuilder {
        AspectDefinitionBuilder::new(aspect_class)
    }
}

/// Builder class for [`AspectDefinition`] (Java `AspectDefinition.Builder`).
// Java: com/google/devtools/build/lib/packages/AspectDefinition.java:269 AspectDefinition.Builder
#[derive(Clone)]
pub struct AspectDefinitionBuilder {
    aspect_class: AspectClass,
    attributes: IndexMap<String, Attribute>,
    advertised_providers: AdvertisedProviderSetBuilder,
    required_providers: RequiredProvidersBuilder,
    required_aspect_providers: RequiredProvidersBuilder,
    propagate_along_attributes: AspectPropagationEdgesSupplier<String>,
    propagate_to_toolchains_types: AspectPropagationEdgesSupplier<Label>,
    configuration_fragment_policy: ConfigurationFragmentPolicyBuilder,
    apply_to_files: bool,
    apply_to_generating_rules: bool,
    // PORT: java-hash-order. Java collects the toolchain types in a `new HashSet<>()` and
    // `build()` copies it into an `ImmutableSet`, so the definition's order is the iteration
    // order of that `HashSet`: kept here in insertion order and reordered in `build`.
    toolchain_types: IndexSet<ToolchainTypeRequirement>,
    required_aspect_classes: IndexSet<AspectClass>,
    propagation_predicate: Option<AspectPropagationPredicate>,
    exec_compatible_with: IndexSet<Label>,
    exec_groups: IndexMap<String, DeclaredExecGroup>,
    subrules: Vec<StarlarkCallable>,
}

impl AspectDefinitionBuilder {
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:293 AspectDefinition.Builder.<init>
    pub fn new(aspect_class: AspectClass) -> AspectDefinitionBuilder {
        AspectDefinitionBuilder {
            aspect_class,
            attributes: IndexMap::new(),
            advertised_providers: AdvertisedProviderSet::builder(),
            required_providers: RequiredProviders::accept_any_builder(),
            required_aspect_providers: RequiredProviders::accept_none_builder(),
            propagate_along_attributes: default_attr_aspects_supplier(),
            propagate_to_toolchains_types: default_toolchains_aspects_supplier(),
            configuration_fragment_policy: ConfigurationFragmentPolicyBuilder::new(),
            apply_to_files: false,
            apply_to_generating_rules: false,
            toolchain_types: IndexSet::new(),
            required_aspect_classes: IndexSet::new(),
            propagation_predicate: None,
            exec_compatible_with: IndexSet::new(),
            exec_groups: IndexMap::new(),
            subrules: Vec::new(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:298 AspectDefinition.Builder.requireProviders
    pub fn require_providers(&mut self, required_providers: &RequiredProviders) -> &mut Self {
        self.require_starlark_provider_sets(
            required_providers
                .get_starlark_providers()
                .iter()
                .map(|s| s.iter().cloned().collect::<Vec<_>>()),
        );
        self
    }

    /// Asserts that this aspect can only be evaluated for rules that supply all of the
    /// providers from at least one set of required providers.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:308 AspectDefinition.Builder.requireStarlarkProviderSets
    pub fn require_starlark_provider_sets<S>(
        &mut self,
        provider_sets: impl IntoIterator<Item = S>,
    ) -> &mut Self
    where
        S: IntoIterator<Item = StarlarkProviderIdentifier>,
    {
        for provider_set in provider_sets {
            let provider_set = dedup(provider_set);
            if !provider_set.is_empty() {
                self.required_providers.add_starlark_set(provider_set);
            }
        }
        self
    }

    /// Asserts that this aspect can only be evaluated for rules that supply all of the specified
    /// Starlark providers.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:323 AspectDefinition.Builder.requireStarlarkProviders
    pub fn require_starlark_providers(
        &mut self,
        starlark_providers: &[StarlarkProviderIdentifier],
    ) -> &mut Self {
        self.required_providers
            .add_starlark_set(dedup(starlark_providers.iter().cloned()));
        self
    }

    /// Asserts that this aspect requires a list of aspects to be applied before it on the
    /// configured target.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:333 AspectDefinition.Builder.requiredAspectClasses
    pub fn required_aspect_classes(
        &mut self,
        required_aspect_classes: IndexSet<AspectClass>,
    ) -> &mut Self {
        self.required_aspect_classes = required_aspect_classes;
        self
    }

    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:339 AspectDefinition.Builder.propagationPredicate
    pub fn propagation_predicate(
        &mut self,
        propagation_predicate: Option<AspectPropagationPredicate>,
    ) -> &mut Self {
        self.propagation_predicate = propagation_predicate;
        self
    }

    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:345 AspectDefinition.Builder.requireAspectsWithProviders
    pub fn require_aspects_with_providers<S>(
        &mut self,
        provider_sets: impl IntoIterator<Item = S>,
    ) -> &mut Self
    where
        S: IntoIterator<Item = StarlarkProviderIdentifier>,
    {
        for provider_set in provider_sets {
            let provider_set = dedup(provider_set);
            if !provider_set.is_empty() {
                self.required_aspect_providers
                    .add_starlark_set(provider_set);
            }
        }
        self
    }

    /// State that the aspect being built provides given providers.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:357 AspectDefinition.Builder.advertiseProvider
    pub fn advertise_provider(&mut self, providers: &[StarlarkProviderIdentifier]) -> &mut Self {
        for provider in providers {
            self.advertised_providers.add_starlark(provider.clone());
        }
        self
    }

    /// Sets the supplier of the attributes to which the aspect will propagate.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:366 AspectDefinition.Builder.propagateToAttributes
    pub fn propagate_to_attributes(
        &mut self,
        attributes: AspectPropagationEdgesSupplier<String>,
    ) -> &mut Self {
        self.propagate_along_attributes = attributes;
        self
    }

    /// Sets the supplier of the toolchains types for which the aspect will propagate to
    /// matching resolved toolchains.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:376 AspectDefinition.Builder.propagateToToolchainsTypes
    pub fn propagate_to_toolchains_types(
        &mut self,
        toolchains_types: AspectPropagationEdgesSupplier<Label>,
    ) -> &mut Self {
        self.propagate_to_toolchains_types = toolchains_types;
        self
    }

    /// Adds an attribute to the aspect (Java `add(Attribute.Builder)`).
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:385 AspectDefinition.Builder.add
    pub fn add_builder(&mut self, attr: &AttributeBuilder) -> &mut Self {
        let attribute = attr.build();
        self.add(attribute)
    }

    /// Adds an attribute to the aspect.
    ///
    /// Aspects attributes can be of any data type if they are not public, i.e. implicit
    /// (starting with '$') or late-bound (starting with ':'). Public attributes can only be of
    /// types string, integer or boolean.
    ///
    /// Aspect definition currently cannot handle `ComputedDefault` dependencies (type LABEL or
    /// LABEL_LIST), because all the dependencies are resolved from the aspect definition and
    /// the defining rule.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:402 AspectDefinition.Builder.add
    pub fn add(&mut self, attribute: Attribute) -> &mut Self {
        blaze_util::check_argument!(
            attribute.is_implicit()
                || attribute.is_late_bound()
                || (attribute.get_type() == Type::String && attribute.check_allowed_values())
                || (attribute.get_type() == Type::Integer && attribute.check_allowed_values())
                || attribute.get_type() == Type::Boolean,
            "{}: Invalid attribute '{}' ({})",
            self.aspect_class.get_name(),
            attribute.get_name(),
            attribute.get_type()
        );

        // Attributes specifying dependencies using ComputedDefault value are currently not
        // supported. The limitation is in place because:
        //  - blaze query requires that all possible values are knowable without
        //    BuildConguration
        //  - aspects can attach to any rule
        // Current logic in #forEachLabelDepFromAllAttributesOfAspect is not enough,
        // however {Conservative,Precise}AspectResolver can probably be improved to make that
        // work.
        blaze_util::check_argument!(
            !(attribute.get_type().get_label_class() == LabelClass::Dependency
                && matches!(
                    attribute.get_default_value_unchecked(),
                    Some(AttrValue::ComputedDefault(_))
                )),
            "{}: Invalid attribute '{}' ({}) with computed default dependencies",
            self.aspect_class.get_name(),
            attribute.get_name(),
            attribute.get_type()
        );
        blaze_util::check_argument!(
            !self.attributes.contains_key(attribute.get_name()),
            "{}: An attribute with the name '{}' already exists.",
            self.aspect_class.get_name(),
            attribute.get_name()
        );
        self.attributes
            .insert(attribute.get_name().to_owned(), attribute);
        self
    }

    /// Declares that the implementation of the associated aspect definition requires the given
    /// fragments (Java classes, by simple name) to be present in this rule's exec and target
    /// configurations.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:443 AspectDefinition.Builder.requiresConfigurationFragments
    pub fn requires_configuration_fragments(
        &mut self,
        configuration_fragments: &[&str],
    ) -> &mut Self {
        self.configuration_fragment_policy
            .requires_configuration_fragments(configuration_fragments);
        self
    }

    /// Declares the configuration fragments that are required by this rule for the target
    /// configuration, by their Starlark module names.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:458 AspectDefinition.Builder.requiresConfigurationFragmentsByStarlarkBuiltinName
    pub fn requires_configuration_fragments_by_starlark_builtin_name(
        &mut self,
        configuration_fragment_names: &[String],
    ) -> &mut Self {
        self.configuration_fragment_policy
            .requires_configuration_fragments_by_starlark_builtin_name(
                configuration_fragment_names,
            );
        self
    }

    /// Sets the policy for the case where the configuration is missing the required fragment
    /// class.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:470 AspectDefinition.Builder.setMissingFragmentPolicy
    pub fn set_missing_fragment_policy(
        &mut self,
        fragment_class: &str,
        missing_fragment_policy: MissingFragmentPolicy,
    ) -> &mut Self {
        self.configuration_fragment_policy
            .set_missing_fragment_policy(fragment_class, missing_fragment_policy);
        self
    }

    /// Sets whether this aspect should apply to files. Default is `false`. Currently only
    /// supported for top-level aspects and targets, and only for output files.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:483 AspectDefinition.Builder.applyToFiles
    pub fn apply_to_files(&mut self, propagate_over_generated_files: bool) -> &mut Self {
        self.apply_to_files = propagate_over_generated_files;
        self
    }

    /// Sets whether this aspect should, when it would be applied to an output file, instead
    /// apply to the generating rule of that output file. Default is `false`.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:496 AspectDefinition.Builder.applyToGeneratingRules
    pub fn apply_to_generating_rules(&mut self, apply_to_generating_rules: bool) -> &mut Self {
        self.apply_to_generating_rules = apply_to_generating_rules;
        self
    }

    /// Adds the given toolchains as requirements for this aspect.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:502 AspectDefinition.Builder.addToolchainTypes
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:508 AspectDefinition.Builder.addToolchainTypes
    pub fn add_toolchain_types(
        &mut self,
        toolchain_types: impl IntoIterator<Item = ToolchainTypeRequirement>,
    ) -> &mut Self {
        self.toolchain_types.extend(toolchain_types);
        self
    }

    /// Adds the given constraint values to the set required for execution platforms for this
    /// aspect.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:517 AspectDefinition.Builder.execCompatibleWith
    pub fn exec_compatible_with(&mut self, exec_compatible_with: IndexSet<Label>) -> &mut Self {
        self.exec_compatible_with = exec_compatible_with;
        self
    }

    /// Sets the execution groups that are available for actions created by this aspect.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:524 AspectDefinition.Builder.execGroups
    pub fn exec_groups(&mut self, exec_groups: IndexMap<String, DeclaredExecGroup>) -> &mut Self {
        // TODO(b/230337573): validate names
        // TODO(b/230337573): handle copy_from_default
        self.exec_groups = exec_groups;
        self
    }

    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:532 AspectDefinition.Builder.subrules
    pub fn subrules(&mut self, subrules: Vec<StarlarkCallable>) -> &mut Self {
        self.subrules = subrules;
        self
    }

    /// Builds the aspect definition. The builder object is reusable afterwards.
    // Java: com/google/devtools/build/lib/packages/AspectDefinition.java:542 AspectDefinition.Builder.build
    pub fn build(&self) -> Arc<AspectDefinition> {
        let required_providers = self.required_providers.build();
        if self.apply_to_generating_rules {
            if !required_providers.accepts_any() {
                panic!(
                    "IllegalStateException: An aspect cannot simultaneously have required \
                     providers and apply to generating rules."
                );
            }

            if self.propagation_predicate.is_some() {
                panic!(
                    "IllegalStateException: An aspect cannot simultaneously have a propagation \
                     predicate and apply to generating rules."
                );
            }
        }

        if self.apply_to_files {
            if !required_providers.accepts_any() {
                panic!(
                    "IllegalStateException: An aspect cannot simultaneously have required \
                     providers and apply to files."
                );
            }
            if self.propagation_predicate.is_some() {
                panic!(
                    "IllegalStateException: An aspect cannot simultaneously have a propagation \
                     predicate and apply to files."
                );
            }
        }

        Arc::new(AspectDefinition {
            aspect_class: self.aspect_class.clone(),
            advertised_providers: self.advertised_providers.build(),
            required_providers,
            required_providers_for_aspects: self.required_aspect_providers.build(),
            attributes: self.attributes.clone(),
            toolchain_types: java_hash_set_order_of_toolchain_types(&self.toolchain_types),
            restrict_to_attributes: self.propagate_along_attributes.clone(),
            propagate_to_toolchains_types: self.propagate_to_toolchains_types.clone(),
            configuration_fragment_policy: self.configuration_fragment_policy.build(),
            apply_to_files: self.apply_to_files,
            apply_to_generating_rules: self.apply_to_generating_rules,
            required_aspect_classes: self.required_aspect_classes.clone(),
            propagation_predicate: self.propagation_predicate.clone(),
            exec_compatible_with: self.exec_compatible_with.clone(),
            exec_groups: self.exec_groups.clone(),
            subrules: self.subrules.clone(),
        })
    }
}

/// Java `ToolchainTypeRequirement.hashCode()`.
///
/// PORT: java-hash-order. `ToolchainTypeRequirement` is a record without a `hashCode()` of its
/// own, so the value comes from `java.lang.runtime.ObjectMethods`: OpenJDK (25, which Bazel 9.3.2
/// embeds) folds the components in declaration order (`toolchainType`, `mandatory`,
/// `ignoreIfInvalid`) with `result = result * 31 + hash(component)` starting from 0; a `boolean`
/// component hashes with `Boolean.hashCode` (1231 for true, 1237 for false). Same derivation as
/// `blaze_bzlmod_model::ModuleKey::java_hash_code`.
// Java: java.base/java/lang/runtime/ObjectMethods.java ObjectMethods.makeHashCode
fn toolchain_type_requirement_java_hash_code(requirement: &ToolchainTypeRequirement) -> i32 {
    use blaze_query_engine::java_concurrent_hash_map::JavaHashCode;
    let java_boolean_hash_code = |b: bool| if b { 1231 } else { 1237 };
    let mut result: i32 = 0;
    result = result
        .wrapping_mul(31)
        .wrapping_add(requirement.toolchain_type.java_hash_code());
    result = result
        .wrapping_mul(31)
        .wrapping_add(java_boolean_hash_code(requirement.mandatory));
    result = result
        .wrapping_mul(31)
        .wrapping_add(java_boolean_hash_code(requirement.ignore_if_invalid));
    result
}

/// Java `ImmutableSet.copyOf(toolchainTypes)` over the builder's `HashSet`: the elements in the
/// iteration order of a `new HashSet<>()` they were added to in `toolchain_types`' order.
// Java: com/google/devtools/build/lib/packages/AspectDefinition.java:580 AspectDefinition.Builder.build
fn java_hash_set_order_of_toolchain_types(
    toolchain_types: &IndexSet<ToolchainTypeRequirement>,
) -> IndexSet<ToolchainTypeRequirement> {
    blaze_query_engine::java_concurrent_hash_map::java_hash_map_order_by(
        16,
        toolchain_types.iter().cloned().map(|t| (t, ())).collect(),
        toolchain_type_requirement_java_hash_code,
    )
    .into_iter()
    .map(|(t, ())| t)
    .collect()
}

/// Java `ImmutableSet.copyOf` of a provider set.
fn dedup(
    providers: impl IntoIterator<Item = StarlarkProviderIdentifier>,
) -> Vec<StarlarkProviderIdentifier> {
    let set: IndexSet<StarlarkProviderIdentifier> = providers.into_iter().collect();
    set.into_iter().collect()
}

#[cfg(test)]
mod java_hash_order_tests {
    use super::*;

    fn requirement(name: &str) -> ToolchainTypeRequirement {
        ToolchainTypeRequirement::create(Label::parse_canonical_unchecked(&format!("//tc:{name}")))
    }

    /// The expected values are derived by hand from the Java formulas, not from Blaze:
    /// `String.hashCode`: "" = 0, "tc" = 116 * 31 + 99 = 3695, "a" = 97.
    /// `RepositoryName("")`: `hashObjects(name, null, null)` = 0 + 31 * (0 + 31 * (0 + 31)) = 29791.
    /// `PackageIdentifier(@@, "tc")`: `hashObjects(repo, pkg)` = 3695 + 31 * (29791 + 31) = 928177.
    /// `Label //tc:a`: 31 * (31 + 97) + 928177 = 932145.
    /// record `ToolchainTypeRequirement(//tc:a, true, false)`:
    /// ((932145) * 31 + 1231) * 31 + 1237 = 895830743.
    #[test]
    fn toolchain_type_requirement_hash_code_is_the_java_record_hash() {
        assert_eq!(
            toolchain_type_requirement_java_hash_code(&requirement("a")),
            895830743
        );
        // //tc:b: label 932176, record (932176 * 31 + 1231) * 31 + 1237 = 895860534.
        assert_eq!(
            toolchain_type_requirement_java_hash_code(&requirement("b")),
            895860534
        );
    }

    /// `HashMap.hash(h) = h ^ (h >>> 16)`, bin `hash & 15` in the 16-slot table of
    /// `new HashSet<>()`: //tc:a (895830743) is in bin 2, //tc:b (895860534) and //tc:c
    /// (895890325) in bin 3, //tc:e (895949907) in bin 4. Iteration walks the bins from 0 and
    /// each bin in insertion order, so adding e, c, b, a gives a, c, b, e.
    #[test]
    fn toolchain_types_are_in_java_hash_set_order() {
        let inserted: IndexSet<ToolchainTypeRequirement> =
            ["e", "c", "b", "a"].into_iter().map(requirement).collect();
        let ordered: Vec<String> = java_hash_set_order_of_toolchain_types(&inserted)
            .iter()
            .map(|t| t.toolchain_type.get_name().to_owned())
            .collect();
        assert_eq!(ordered, ["a", "c", "b", "e"]);
    }
}
