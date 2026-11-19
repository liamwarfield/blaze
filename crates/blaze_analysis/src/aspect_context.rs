//! Port of `analysis/AspectContext.java`: the data available during the analysis of an aspect.
//!
//! Java's `AspectContext extends RuleContext`. Here the subclass's own state is
//! [`AspectContext`], held by the `RuleContext` it extends
//! (`RuleContext::as_aspect_context`); the `RuleContext` methods `AspectContext` overrides
//! (`getOwningPrerequisitesCollection`, `getAspects`, `getMainAspect`, `getAspectDescriptors`,
//! `useAutoExecGroups`, `getAllPrerequisites`, `getDefaultTemplateVariableProviders`) dispatch
//! to it, and its own public methods are `RuleContext` methods defined in this file.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use blaze_cmdline::Label;
use blaze_packages::aspect::{Aspect, AspectDescriptor};
use blaze_packages::attribute::Attribute;
use blaze_packages::rule_class::DeclaredExecGroup;
use indexmap::{IndexMap, IndexSet};

use crate::aspect_aware_attribute_mapper::AspectAwareAttributeMapper;
use crate::config::configured_attribute_mapper::ConfiguredAttributeMapper;
use crate::configured_target::ConfiguredTarget;
use crate::configured_target_and_data::ConfiguredTargetAndData;
use crate::dependency_kind::DependencyKind;
use crate::platform::template_variable_info::TemplateVariableInfo;
use crate::rule::configuration_make_variable_context::ConfigurationMakeVariableContext;
use crate::rule::prerequisites_collection::{AttributesMapper, PrerequisitesCollection};
use crate::rule::rule_context::{
    DEFAULT_MAKE_VARIABLE_ATTRIBUTES, RuleContext, RuleContextBuilder,
};
use crate::toolchains::aspect_base_target_resolved_toolchain_context::{
    AspectBaseTargetResolvedToolchainContext, ToolchainAspectsProviders,
};
use crate::toolchains::exec_group_collection::ExecGroupCollection;
use crate::toolchains::toolchain_collection::ToolchainCollection;

/// Extends `RuleContext` to provide all data available during the analysis of an aspect: the
/// fields Java's `AspectContext` adds to `RuleContext`.
// Java: com/google/devtools/build/lib/analysis/AspectContext.java:40 AspectContext
pub struct AspectContext {
    /// A list of all aspects applied to the target.
    ///
    /// The last aspect in the list is the main aspect that this context is for.
    aspects: Vec<Aspect>,
    aspect_descriptors: Vec<AspectDescriptor>,
    main_aspect_prerequisites: PrerequisitesCollection,
    /// The toolchain contexts for the base target.
    ///
    /// It only contains the providers created by the aspects that propagate to the toolchains.
    base_target_toolchain_contexts:
        Option<Arc<ToolchainCollection<AspectBaseTargetResolvedToolchainContext>>>,
    /// Whether the target uses auto exec groups.
    target_uses_auto_exec_groups: bool,
    /// The make variables for the base target.
    base_target_configuration_make_variable_context:
        OnceLock<Arc<ConfigurationMakeVariableContext>>,
}

impl AspectContext {
    /// Java's constructor (it returns the `RuleContext` it extends).
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:69 AspectContext.<init>
    #[allow(clippy::new_ret_no_self)]
    fn new(
        builder: &RuleContextBuilder,
        aspect_aware_attribute_mapper: Arc<OwnedAspectAwareAttributeMapper>,
        rule_and_base_aspects_prerequisites: PrerequisitesCollection,
        main_aspect_prerequisites: PrerequisitesCollection,
        exec_group_collection: ExecGroupCollection,
        base_target_toolchain_contexts: Option<
            Arc<ToolchainCollection<AspectBaseTargetResolvedToolchainContext>>,
        >,
        target_uses_auto_exec_groups: bool,
    ) -> RuleContext {
        let mut rule_context = RuleContext::new(
            builder,
            aspect_aware_attribute_mapper,
            rule_and_base_aspects_prerequisites,
            exec_group_collection,
        );
        let aspects = builder.get_aspects().to_vec();
        let aspect_descriptors = aspects.iter().map(|a| a.get_descriptor().clone()).collect();
        rule_context.set_aspect_context(AspectContext {
            aspects,
            aspect_descriptors,
            main_aspect_prerequisites,
            base_target_toolchain_contexts,
            target_uses_auto_exec_groups,
            base_target_configuration_make_variable_context: OnceLock::new(),
        });
        rule_context
    }

    /// Merge the attributes of the aspects in the aspects path.
    ///
    /// For attributes with the same name, the one that is first encountered takes precedence.
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:145 AspectContext.mergeAspectsAttributes
    fn merge_aspects_attributes(aspects: &[Aspect]) -> IndexMap<String, Attribute> {
        if aspects.is_empty() {
            IndexMap::new()
        } else if aspects.len() == 1 {
            aspects[0].get_definition().get_attributes().clone()
        } else {
            let mut aspect_attributes: IndexMap<String, Attribute> = IndexMap::new();
            for aspect in aspects {
                let current_attributes = aspect.get_definition().get_attributes();
                for (k, v) in current_attributes {
                    aspect_attributes
                        .entry(k.clone())
                        .or_insert_with(|| v.clone());
                }
            }
            aspect_attributes
        }
    }

    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:163 AspectContext.create
    pub(crate) fn create(
        builder: &RuleContextBuilder,
        rule_attributes: Arc<ConfiguredAttributeMapper>,
        targets_map: Vec<(DependencyKind, Vec<ConfiguredTargetAndData>)>,
        exec_group_collection: ExecGroupCollection,
        base_target_toolchain_contexts: Option<
            Arc<ToolchainCollection<AspectBaseTargetResolvedToolchainContext>>,
        >,
    ) -> RuleContext {
        Self::create_aspect_context_with_separated_prerequisites(
            builder,
            rule_attributes,
            targets_map,
            exec_group_collection,
            base_target_toolchain_contexts,
        )
    }

    /// Create prerequisites collection for aspect evaluation separating the main aspect
    /// prerequisites from the underlying rule and base aspects prerequisites.
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:177 AspectContext.createAspectContextWithSeparatedPrerequisites
    fn create_aspect_context_with_separated_prerequisites(
        rule_context_builder: &RuleContextBuilder,
        rule_attributes: Arc<ConfiguredAttributeMapper>,
        prerequisites_map: Vec<(DependencyKind, Vec<ConfiguredTargetAndData>)>,
        exec_group_collection: ExecGroupCollection,
        base_target_toolchain_contexts: Option<
            Arc<ToolchainCollection<AspectBaseTargetResolvedToolchainContext>>,
        >,
    ) -> RuleContext {
        // Java: ImmutableSortedKeyListMultimap (sorted by attribute name).
        let mut main_aspect_prerequisites: BTreeMap<String, Vec<ConfiguredTargetAndData>> =
            BTreeMap::new();
        let mut rule_and_base_aspects_prerequisites: BTreeMap<
            String,
            Vec<ConfiguredTargetAndData>,
        > = BTreeMap::new();

        let main_aspect = rule_context_builder
            .get_aspects()
            .last()
            .expect("NoSuchElementException: aspects")
            .clone();

        for (kind, values) in prerequisites_map {
            let attribute_name = kind
                .get_attribute()
                .expect("attribute dependency")
                .get_name()
                .to_owned();

            if Some(main_aspect.get_aspect_class()) == kind.get_owning_aspect() {
                main_aspect_prerequisites
                    .entry(attribute_name)
                    .or_default()
                    .extend(values);
            } else {
                rule_and_base_aspects_prerequisites
                    .entry(attribute_name)
                    .or_default()
                    .extend(values);
            }
        }

        let target_uses_auto_exec_groups = rule_context_builder
            .get_rule()
            .get_rule_class_object()
            .get_auto_exec_groups_mode()
            .is_enabled(
                rule_attributes.as_ref(),
                crate::rule::config_util::use_auto_exec_groups(
                    rule_context_builder.get_configuration(),
                ),
            );

        let rule_and_base_aspects_attributes = Self::merge_rule_and_base_aspects_attributes(
            rule_attributes.clone(),
            rule_context_builder.get_aspects(),
        );
        AspectContext::new(
            rule_context_builder,
            Arc::new(OwnedAspectAwareAttributeMapper::new(
                rule_attributes,
                Self::merge_aspects_attributes(rule_context_builder.get_aspects()),
            )),
            PrerequisitesCollection::new(
                rule_and_base_aspects_prerequisites,
                AttributesMapper::of_attribute_map(Arc::new(rule_and_base_aspects_attributes)),
                rule_context_builder.get_error_consumer().clone(),
                rule_context_builder.get_rule().clone(),
                rule_context_builder.get_rule_class_name_for_logging(),
            ),
            PrerequisitesCollection::new(
                main_aspect_prerequisites,
                AttributesMapper::of_attributes(
                    main_aspect.get_definition().get_attributes().clone(),
                ),
                rule_context_builder.get_error_consumer().clone(),
                rule_context_builder.get_rule().clone(),
                rule_context_builder.get_rule_class_name_for_logging(),
            ),
            exec_group_collection,
            base_target_toolchain_contexts,
            target_uses_auto_exec_groups,
        )
    }

    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:229 AspectContext.mergeRuleAndBaseAspectsAttributes
    fn merge_rule_and_base_aspects_attributes(
        rule_attributes: Arc<ConfiguredAttributeMapper>,
        aspects: &[Aspect],
    ) -> OwnedAspectAwareAttributeMapper {
        let mut merged_base_aspects_attributes: IndexMap<String, Attribute> = IndexMap::new();
        for aspect in aspects.iter().take(aspects.len().saturating_sub(1)) {
            for attribute in aspect.get_definition().get_attributes().values() {
                merged_base_aspects_attributes
                    .entry(attribute.get_name().to_owned())
                    .or_insert_with(|| attribute.clone());
            }
        }
        OwnedAspectAwareAttributeMapper::new(rule_attributes, merged_base_aspects_attributes)
    }

    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:242 AspectContext.getOwningPrerequisitesCollection
    pub(crate) fn get_owning_prerequisites_collection<'a>(
        &'a self,
        rule_context: &'a RuleContext,
        attribute_name: &str,
    ) -> &'a PrerequisitesCollection {
        if self.main_aspect_prerequisites.has(attribute_name) {
            return &self.main_aspect_prerequisites;
        }
        rule_context.get_rule_prerequisites_collection()
    }

    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:249 AspectContext.getMainAspectPrerequisitesCollection
    pub fn get_main_aspect_prerequisites_collection(&self) -> &PrerequisitesCollection {
        &self.main_aspect_prerequisites
    }

    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:254 AspectContext.getAspects
    pub fn get_aspects(&self) -> &[Aspect] {
        &self.aspects
    }

    /// Return the main aspect of this context.
    ///
    /// It is the last aspect in the list of aspects applied to a target; all other aspects are
    /// the ones main aspect sees as specified by its "required_aspect_providers").
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:265 AspectContext.getMainAspect
    pub fn get_main_aspect(&self) -> &Aspect {
        self.aspects
            .last()
            .expect("NoSuchElementException: aspects")
    }

    /// All aspects applied to the rule.
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:271 AspectContext.getAspectDescriptors
    pub fn get_aspect_descriptors(&self) -> &[AspectDescriptor] {
        &self.aspect_descriptors
    }

    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:276 AspectContext.useAutoExecGroups
    pub(crate) fn use_auto_exec_groups(&self, rule_context: &RuleContext) -> bool {
        // TODO: b/370558813 - Use AutoExecGroupsMode for aspects, as well.
        let aspect_attributes = self.get_main_aspect().get_definition().get_attributes();
        match aspect_attributes.get("$use_auto_exec_groups") {
            // Java: (boolean) getDefaultValueUnchecked()
            Some(attribute) => attribute
                .get_default_value_unchecked()
                .and_then(|v| v.as_bool())
                .expect("ClassCastException: $use_auto_exec_groups default is not a boolean"),
            None => {
                crate::rule::config_util::use_auto_exec_groups(rule_context.get_configuration())
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:288 AspectContext.getAllPrerequisites
    pub(crate) fn get_all_prerequisites(
        &self,
        rule_context: &RuleContext,
    ) -> Vec<ConfiguredTarget> {
        let mut all = self.main_aspect_prerequisites.get_all_prerequisites();
        all.extend(
            rule_context
                .get_rule_prerequisites_collection()
                .get_all_prerequisites(),
        );
        all
    }

    /// The `TemplateVariableInfo` variables of the given attributes of the main aspect.
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:295 AspectContext.getTemplateVariablesFromAspectAttributes
    pub(crate) fn get_template_variables_from_aspect_attributes(
        &self,
        attribute_names: &[&str],
    ) -> Vec<IndexMap<String, String>> {
        let key = TemplateVariableInfo::provider_key();
        let mut out = Vec::new();
        for attr_name in attribute_names {
            // Only process this attribute it if is present in the aspect directly.
            if !self
                .get_main_aspect_prerequisites_collection()
                .has(attr_name)
            {
                continue;
            }
            // Get the TemplateVariableInfo providers from this attribute.
            for info in self
                .get_main_aspect_prerequisites_collection()
                .get_prerequisites_infos(attr_name, &key)
            {
                out.push(TemplateVariableInfo::variables_of(&info));
            }
        }
        out
    }
}

/// The aspect-only methods of Java's `AspectContext` (called on `(AspectContext) ruleContext`;
/// they panic like Java's `ClassCastException` on a rule's context).
impl RuleContext {
    fn aspect_context_or_fail(&self) -> &AspectContext {
        self.as_aspect_context()
            .expect("ClassCastException: RuleContext cannot be cast to AspectContext")
    }

    /// Returns the toolchain contexts for the base target. Can be `None` if no aspect in the
    /// `aspects` path propagate to the toolchains.
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:92 AspectContext.getBaseTargetToolchainContexts
    pub fn get_base_target_toolchain_contexts(
        &self,
    ) -> Option<&Arc<ToolchainCollection<AspectBaseTargetResolvedToolchainContext>>> {
        self.aspect_context_or_fail()
            .base_target_toolchain_contexts
            .as_ref()
    }

    /// Returns the labels of default the toolchain types that aspects have propagated.
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:99 AspectContext.getRequestedToolchainTypesLabels
    pub fn get_requested_toolchain_types_labels(&self) -> IndexSet<Label> {
        let aspect_context = self.aspect_context_or_fail();
        let base_target_toolchain_contexts = aspect_context
            .base_target_toolchain_contexts
            .as_ref()
            .expect("NullPointerException: baseTargetToolchainContexts");
        if aspect_context.target_uses_auto_exec_groups {
            let mut result = IndexSet::new();
            for (name, context) in base_target_toolchain_contexts.context_map() {
                if !DeclaredExecGroup::is_automatic(name) {
                    continue;
                }
                for label in context.requested_toolchain_type_labels().keys() {
                    result.insert(label.clone());
                }
            }
            result
        } else {
            base_target_toolchain_contexts
                .get_default_toolchain_context()
                .requested_toolchain_type_labels()
                .keys()
                .cloned()
                .collect()
        }
    }

    /// Returns the toolchain data for the given type, or `None` if the toolchain type was not
    /// required in this context.
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:118 AspectContext.getToolchainTarget
    pub fn get_toolchain_target(
        &self,
        toolchain_type: &Label,
    ) -> Option<&ToolchainAspectsProviders> {
        let aspect_context = self.aspect_context_or_fail();
        let base_target_toolchain_contexts = aspect_context
            .base_target_toolchain_contexts
            .as_ref()
            .expect("NullPointerException: baseTargetToolchainContexts");
        let mut exec_group_context = base_target_toolchain_contexts.get_default_toolchain_context();
        if aspect_context.target_uses_auto_exec_groups {
            let found = base_target_toolchain_contexts
                .context_map()
                .iter()
                .find(|(name, context)| {
                    DeclaredExecGroup::is_automatic(name)
                        && context
                            .requested_toolchain_type_labels()
                            .contains_key(toolchain_type)
                })
                .map(|(_, context)| context);
            exec_group_context = found?;
        }
        // Java: toolchains().get(requestedToolchainTypeLabels().get(toolchainType)) (a missing
        // label maps to a null key, which ImmutableMap.get answers with null).
        let toolchain_type_info = exec_group_context
            .requested_toolchain_type_labels()
            .get(toolchain_type)?;
        exec_group_context.toolchains().get(toolchain_type_info)
    }

    /// The main aspect's prerequisites (Java
    /// `((AspectContext) ruleContext).getMainAspectPrerequisitesCollection()`).
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:249 AspectContext.getMainAspectPrerequisitesCollection
    pub fn get_main_aspect_prerequisites_collection(&self) -> &PrerequisitesCollection {
        self.aspect_context_or_fail()
            .get_main_aspect_prerequisites_collection()
    }

    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:310 AspectContext.getTemplateVariablesFromBaseRuleAttributes
    fn get_template_variables_from_base_rule_attributes(
        &self,
        attribute_names: &[&str],
    ) -> Vec<IndexMap<String, String>> {
        let key = TemplateVariableInfo::provider_key();
        let mut out = Vec::new();
        for attr_name in attribute_names {
            // Only process this attribute it if is present in the target directly.
            if !self.get_rule_prerequisites_collection().has(attr_name) {
                continue;
            }
            // Get the TemplateVariableInfo providers from this attribute.
            for info in self
                .get_rule_prerequisites_collection()
                .get_prerequisites_infos(attr_name, &key)
            {
                out.push(TemplateVariableInfo::variables_of(&info));
            }
        }
        out
    }

    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:325 AspectContext.getTemplateVariablesFromBaseRuleToolchains
    fn get_template_variables_from_base_rule_toolchains(&self) -> Vec<IndexMap<String, String>> {
        let Some(base_target_toolchain_contexts) = self.get_base_target_toolchain_contexts() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for context in base_target_toolchain_contexts.context_map().values() {
            for info in context.template_variable_providers() {
                out.push(TemplateVariableInfo::variables_of(&info));
            }
        }
        out
    }

    /// Returns the `ConfigurationMakeVariableContext` for the base rule, but not including the
    /// aspect and its toolchains.
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:356 AspectContext.getBaseTargetConfigurationMakeVariableContext
    pub fn get_base_target_configuration_make_variable_context(
        &self,
    ) -> Arc<ConfigurationMakeVariableContext> {
        self.aspect_context_or_fail()
            .base_target_configuration_make_variable_context
            .get_or_init(|| {
                let mut template_variable_providers = self
                    .get_template_variables_from_base_rule_attributes(
                        &DEFAULT_MAKE_VARIABLE_ATTRIBUTES,
                    );
                template_variable_providers
                    .extend(self.get_template_variables_from_base_rule_toolchains());
                Arc::new(ConfigurationMakeVariableContext::new(
                    self.get_rule().get_package_declarations(),
                    self.get_configuration(),
                    template_variable_providers,
                ))
            })
            .clone()
    }
}

/// The actions layer's copy of packages `AspectDescriptor`s (`blaze_actions::AspectDescriptor`
/// keeps what `ActionOwner` users read: the class name, the parameters and the description).
// Java: com/google/devtools/build/lib/packages/AspectDescriptor.java:37 AspectDescriptor
pub fn to_action_aspect_descriptors(
    aspect_descriptors: &[AspectDescriptor],
) -> Vec<blaze_actions::AspectDescriptor> {
    aspect_descriptors
        .iter()
        .map(to_action_aspect_descriptor)
        .collect()
}

/// One packages `AspectDescriptor` as the actions layer's `AspectDescriptor`.
pub fn to_action_aspect_descriptor(
    aspect_descriptor: &AspectDescriptor,
) -> blaze_actions::AspectDescriptor {
    blaze_actions::AspectDescriptor {
        aspect_class_name: aspect_descriptor.get_aspect_class().get_name().to_owned(),
        // Java: getParameters().getAttributes().entries()
        parameters: aspect_descriptor
            .get_parameters()
            .entries()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
        description: aspect_descriptor.get_description(),
    }
}

/// An `AspectAwareAttributeMapper` that shares the rule's attribute map.
///
/// PORT: Java's mapper holds a reference to the rule's `AttributeMap`, kept alive by the
/// `AspectContext`; here the mapper holds the `Arc` the context got it in.
pub(crate) type OwnedAspectAwareAttributeMapper =
    AspectAwareAttributeMapper<Arc<ConfiguredAttributeMapper>>;
