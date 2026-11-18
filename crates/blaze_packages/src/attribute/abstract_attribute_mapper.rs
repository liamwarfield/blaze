//! Port of `packages/AbstractAttributeMapper.java`: the shared base of the attribute mappers.
//!
//! Java subclasses override `get` (and `AggregatingAttributeMapper` the package-private
//! `visitLabels(Attribute, Type, LabelVisitor)`); in Rust each mapper wraps an
//! [`AbstractAttributeMapper`] and implements [`AttributeMap`] through
//! [`delegate_attribute_map`], supplying `get_impl`, `visit_labels_of_attribute_impl` and
//! `visit_labels_with_filter_impl`.

use std::sync::Arc;

use blaze_cmdline::Label;

use crate::attribute::Attribute;
use crate::attribute::AttributeInfoProvider;
use crate::attribute::AttributeMap;
use crate::attribute::AttributeProvider;
use crate::attribute::ComputedDefault;
use crate::attribute::DependencyFilter;
use crate::attribute::LateBoundDefault;
use crate::attribute::MaterializingDefault;
use crate::package::PackageArgs;
use crate::target::RuleOrMacroInstance;
use crate::types::AttrValue;
use crate::types::BuildTypeSelectorList;
use crate::types::LabelVisitor;
use crate::types::Type;

/// Java passes the rule itself as the `DependencyFilter.AttributeInfoProvider`; a
/// `&dyn RuleOrMacroInstance` cannot be re-coerced to another trait object, so it is wrapped.
pub(crate) struct RuleInfoProvider<'a>(pub(crate) &'a dyn RuleOrMacroInstance);

impl AttributeInfoProvider for RuleInfoProvider<'_> {
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:316 RuleOrMacroInstance.isAttributeValueExplicitlySpecified
    fn is_attribute_value_explicitly_specified_for(&self, attribute: &Attribute) -> bool {
        self.0
            .is_attribute_value_explicitly_specified(attribute.get_name())
    }
}

/// Java `AbstractAttributeMapper`.
// Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:33 AbstractAttributeMapper
#[derive(Clone, Copy)]
pub struct AbstractAttributeMapper<'a> {
    pub(crate) rule: &'a dyn RuleOrMacroInstance,
}

impl<'a> AbstractAttributeMapper<'a> {
    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:38 AbstractAttributeMapper.<init>
    pub fn new(rule: &'a dyn RuleOrMacroInstance) -> Self {
        AbstractAttributeMapper { rule }
    }

    /// Java field `ruleClass`: the rule's attribute provider.
    pub fn rule_class(&self) -> &'a AttributeProvider {
        self.rule.get_attribute_provider()
    }

    /// Java field `ruleLabel`.
    pub fn rule_label(&self) -> &'a Label {
        self.rule.get_label()
    }

    pub fn rule(&self) -> &'a dyn RuleOrMacroInstance {
        self.rule
    }

    /// Resolves computed / late-bound / materializing defaults; panics on a select. `map` is the
    /// mapper computed defaults read (Java `this`, the concrete mapper).
    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:61 AbstractAttributeMapper.getFromRawAttributeValue
    pub fn get_from_raw_attribute_value(
        &self,
        map: &dyn AttributeMap,
        value: Option<AttrValue>,
        attribute_name: &str,
        type_: Type,
    ) -> Option<AttrValue> {
        match value {
            Some(AttrValue::ComputedDefault(c)) => c.get_default(map),
            Some(AttrValue::LateBound(l)) => l.get_default(Some(self.rule)),
            Some(AttrValue::Materializer(m)) => m.get_default(),
            Some(AttrValue::SelectorList(s)) => panic!(
                "IllegalArgumentException: Unexpected configurable attribute \"{}\" in {} rule {}: \
                 expected {}, is {:?}",
                attribute_name,
                self.rule_class(),
                self.rule.get_label(),
                type_,
                s
            ),
            // Hot code path - avoid the overhead of calling type.cast(value). The rule would
            // have already failed on construction if one of its attributes was of the wrong type
            // (including computed defaults).
            other => other,
        }
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:89 AbstractAttributeMapper.getComputedDefault
    pub fn get_computed_default(
        &self,
        attribute_name: &str,
        type_: Type,
    ) -> Option<Arc<ComputedDefault>> {
        match self.rule.get_attr_typed(attribute_name, type_) {
            Some(AttrValue::ComputedDefault(c)) => Some(c),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:106 AbstractAttributeMapper.getLateBoundDefault
    pub fn get_late_bound_default(
        &self,
        attribute_name: &str,
        type_: Type,
    ) -> Option<Arc<LateBoundDefault>> {
        match self.rule.get_attr_typed(attribute_name, type_) {
            Some(AttrValue::LateBound(l)) => Some(l),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:118 AbstractAttributeMapper.getMaterializer
    pub fn get_materializer(
        &self,
        attribute_name: &str,
        type_: Type,
    ) -> Option<Arc<MaterializingDefault>> {
        match self.rule.get_attr_typed(attribute_name, type_) {
            Some(AttrValue::Materializer(m)) => Some(m),
            _ => None,
        }
    }

    /// Visits the labels of `attributes` that `filter` accepts (skipping output and nodep
    /// types), using `visit_attr` for each attribute (Java's overridable
    /// `visitLabels(Attribute, Type, LabelVisitor)`).
    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:173 AbstractAttributeMapper.visitLabels
    pub(crate) fn visit_labels_list(
        &self,
        attributes: &[&Attribute],
        filter: &DependencyFilter,
        consumer: &mut dyn FnMut(&Attribute, &Label),
        visit_attr: &mut dyn FnMut(&Attribute, Type, &mut LabelVisitor<'_>),
    ) {
        let info = RuleInfoProvider(self.rule);
        for &attribute in attributes {
            let type_ = attribute.get_type();
            // TODO(bazel-team): clean up the typing / visitation interface so we don't have to
            // special-case these types.
            if type_ != Type::Output
                && type_ != Type::OutputList
                && type_ != Type::NodepLabel
                && type_ != Type::NodepLabelList
                && filter.test(&info, attribute)
            {
                // PORT: Java's visitor drops null labels; Rust visitors never see null labels.
                // The visitor's context attribute is always `attribute` here.
                let mut visitor =
                    |label: &Label, _context: Option<&Attribute>| consumer(attribute, label);
                visit_attr(attribute, type_, &mut visitor);
            }
        }
    }

    /// All attributes of the rule class, as references.
    pub(crate) fn all_attributes(&self) -> Vec<&'a Attribute> {
        self.rule_class().get_attributes().iter().collect()
    }

    /// Java `visitLabels(String attributeName, Consumer<Label>)`: the attribute must exist.
    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:161 AbstractAttributeMapper.visitLabels
    pub(crate) fn visit_labels_by_name(
        &self,
        attribute_name: &str,
        consumer: &mut dyn FnMut(&Label),
        visit_attr: &mut dyn FnMut(&Attribute, Type, &mut LabelVisitor<'_>),
    ) {
        let attribute = self.rule_class().get_attribute_by_name(attribute_name);
        self.visit_labels_list(
            &[attribute],
            &DependencyFilter::all_deps(),
            &mut |_attr, label| consumer(label),
            visit_attr,
        );
    }

    /// Visits all labels reachable from the given attribute (Java's default implementation of
    /// the overridable `visitLabels(Attribute, Type, LabelVisitor)`): its value via `map.get`.
    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:196 AbstractAttributeMapper.visitLabels
    pub(crate) fn visit_labels_of_attribute(
        &self,
        map: &dyn AttributeMap,
        attribute: &Attribute,
        type_: Type,
        visitor: &mut LabelVisitor<'_>,
    ) {
        // null values are particularly possible for computed defaults.
        if let Some(value) = map.get(attribute.get_name(), type_) {
            type_.visit_labels(visitor, &value, Some(attribute));
        }
    }

    /// Check if an attribute is configurable (uses select) or, if it's a computed default, if
    /// any of its inputs are configurable (Java static `isConfigurable(rule, attributeName)`).
    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:212 AbstractAttributeMapper.isConfigurable
    pub fn is_configurable_in(rule: &dyn RuleOrMacroInstance, attribute_name: &str) -> bool {
        Self::is_configurable_in_with(rule, attribute_name, true)
    }

    /// Checks if an attribute uses select. If `include_computed_defaults`, also returns true on
    /// computed defaults that have any configurable inputs.
    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:220 AbstractAttributeMapper.isConfigurable
    pub fn is_configurable_in_with(
        rule: &dyn RuleOrMacroInstance,
        attribute_name: &str,
        include_computed_defaults: bool,
    ) -> bool {
        let attr = rule.get_attr(attribute_name);
        if let Some(AttrValue::ComputedDefault(computed_default)) = attr {
            if !include_computed_defaults {
                return false;
            }
            for dep in computed_default.dependencies() {
                if Self::is_configurable_in(rule, dep) {
                    return true;
                }
            }
            return false;
        }
        match rule
            .get_attribute_provider()
            .get_attribute_by_name_maybe(attribute_name)
        {
            Some(attr_def) => rule
                .get_selector_list(attribute_name, attr_def.get_type())
                .is_some(),
            None => false,
        }
    }

    /// The select of a configurable attribute, or `None` if it is not configurable.
    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:248 AbstractAttributeMapper.getSelectorList
    pub fn get_selector_list(
        &self,
        attribute_name: &str,
        type_: Type,
    ) -> Option<Arc<BuildTypeSelectorList>> {
        self.rule.get_selector_list(attribute_name, type_)
    }

    /// Panics (Java IllegalArgumentException) unless the attribute exists with `type_`.
    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:256 AbstractAttributeMapper.checkType
    pub fn check_type(&self, attr_name: &str, type_: Type) {
        let Some(index) = self.rule_class().get_attribute_index(attr_name) else {
            panic!(
                "IllegalArgumentException: No such attribute {} in {} rule {}",
                attr_name,
                self.rule_class(),
                self.rule_label()
            );
        };
        let attr = self.rule_class().get_attribute(index);
        if attr.get_type() != type_ {
            panic!(
                "IllegalArgumentException: Attribute {} is of type {} and not of type {} in {} \
                 rule {}",
                attr_name,
                attr.get_type(),
                type_,
                self.rule_class(),
                self.rule_label()
            );
        }
    }
}

impl AttributeMap for AbstractAttributeMapper<'_> {
    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:45 AbstractAttributeMapper.describeRule
    fn describe_rule(&self) -> String {
        format!("{} {}", self.rule_class(), self.get_label())
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:50 AbstractAttributeMapper.getLabel
    fn get_label(&self) -> &Label {
        self.rule.get_label()
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:272 AbstractAttributeMapper.has
    fn has(&self, attr_name: &str) -> bool {
        self.rule_class()
            .get_attribute_by_name_maybe(attr_name)
            .is_some()
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:278 AbstractAttributeMapper.has
    fn has_typed(&self, attr_name: &str, type_: Type) -> bool {
        self.get_attribute_type(attr_name) == Some(type_)
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:56 AbstractAttributeMapper.get
    fn get(&self, attribute_name: &str, type_: Type) -> Option<AttrValue> {
        self.get_from_raw_attribute_value(
            self,
            self.rule.get_attr_typed(attribute_name, type_),
            attribute_name,
            type_,
        )
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:204 AbstractAttributeMapper.isConfigurable
    fn is_configurable(&self, attribute_name: &str) -> bool {
        AbstractAttributeMapper::is_configurable_in(self.rule, attribute_name)
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:128 AbstractAttributeMapper.getAttributeNames
    fn get_attribute_names(&self) -> Vec<String> {
        self.rule_class()
            .get_attributes()
            .iter()
            .map(|a| a.get_name().to_owned())
            .collect()
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:134 AbstractAttributeMapper.getAttributeType
    fn get_attribute_type(&self, attr_name: &str) -> Option<Type> {
        self.get_attribute_definition(attr_name)
            .map(Attribute::get_type)
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:141 AbstractAttributeMapper.getAttributeDefinition
    fn get_attribute_definition(&self, attr_name: &str) -> Option<&Attribute> {
        self.rule_class().get_attribute_by_name_maybe(attr_name)
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:146 AbstractAttributeMapper.isAttributeValueExplicitlySpecified
    fn is_attribute_value_explicitly_specified(&self, attribute_name: &str) -> bool {
        self.rule
            .is_attribute_value_explicitly_specified(attribute_name)
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:156 AbstractAttributeMapper.visitAllLabels
    fn visit_all_labels(&self, consumer: &mut dyn FnMut(&Attribute, &Label)) {
        self.visit_labels_with_filter(&DependencyFilter::all_deps(), consumer)
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:161 AbstractAttributeMapper.visitLabels
    fn visit_labels(&self, attribute_name: &str, consumer: &mut dyn FnMut(&Label)) {
        self.visit_labels_by_name(attribute_name, consumer, &mut |attr, type_, visitor| {
            self.visit_labels_of_attribute(self, attr, type_, visitor)
        })
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:169 AbstractAttributeMapper.visitLabels
    fn visit_labels_with_filter(
        &self,
        filter: &DependencyFilter,
        consumer: &mut dyn FnMut(&Attribute, &Label),
    ) {
        self.visit_labels_list(
            &self.all_attributes(),
            filter,
            consumer,
            &mut |attr, type_, visitor| self.visit_labels_of_attribute(self, attr, type_, visitor),
        )
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:151 AbstractAttributeMapper.getPackageArgs
    fn get_package_args(&self) -> PackageArgs {
        self.rule.get_package_args()
    }
}

/// Implements [`AttributeMap`] for a mapper struct with a `base: AbstractAttributeMapper` field.
/// The mapper defines inherent `get_impl(&self, &str, Type) -> Option<AttrValue>` (Java `get`),
/// `visit_labels_of_attribute_impl(&self, &Attribute, Type, &mut LabelVisitor)` (Java
/// `visitLabels(Attribute, Type, LabelVisitor)`) and `visit_labels_with_filter_impl` (Java
/// `visitLabels(DependencyFilter, BiConsumer)`); everything else delegates to the base.
macro_rules! delegate_attribute_map {
    ($t:ident) => {
        impl crate::attribute::AttributeMap for $t<'_> {
            fn describe_rule(&self) -> String {
                crate::attribute::AttributeMap::describe_rule(&self.base)
            }
            fn get_label(&self) -> &blaze_cmdline::Label {
                crate::attribute::AttributeMap::get_label(&self.base)
            }
            fn has(&self, attr_name: &str) -> bool {
                crate::attribute::AttributeMap::has(&self.base, attr_name)
            }
            fn has_typed(&self, attr_name: &str, type_: crate::types::Type) -> bool {
                crate::attribute::AttributeMap::has_typed(&self.base, attr_name, type_)
            }
            fn get(
                &self,
                attribute_name: &str,
                type_: crate::types::Type,
            ) -> Option<crate::types::AttrValue> {
                self.get_impl(attribute_name, type_)
            }
            fn is_configurable(&self, attribute_name: &str) -> bool {
                crate::attribute::AttributeMap::is_configurable(&self.base, attribute_name)
            }
            fn get_attribute_names(&self) -> Vec<String> {
                crate::attribute::AttributeMap::get_attribute_names(&self.base)
            }
            fn get_attribute_type(&self, attr_name: &str) -> Option<crate::types::Type> {
                crate::attribute::AttributeMap::get_attribute_type(&self.base, attr_name)
            }
            fn get_attribute_definition(
                &self,
                attr_name: &str,
            ) -> Option<&crate::attribute::Attribute> {
                crate::attribute::AttributeMap::get_attribute_definition(&self.base, attr_name)
            }
            fn is_attribute_value_explicitly_specified(&self, attribute_name: &str) -> bool {
                crate::attribute::AttributeMap::is_attribute_value_explicitly_specified(
                    &self.base,
                    attribute_name,
                )
            }
            fn visit_all_labels(
                &self,
                consumer: &mut dyn FnMut(&crate::attribute::Attribute, &blaze_cmdline::Label),
            ) {
                crate::attribute::AttributeMap::visit_labels_with_filter(
                    self,
                    &crate::attribute::DependencyFilter::all_deps(),
                    consumer,
                )
            }
            fn visit_labels(
                &self,
                attribute_name: &str,
                consumer: &mut dyn FnMut(&blaze_cmdline::Label),
            ) {
                self.base.visit_labels_by_name(
                    attribute_name,
                    consumer,
                    &mut |attr, type_, visitor| {
                        self.visit_labels_of_attribute_impl(attr, type_, visitor)
                    },
                )
            }
            fn visit_labels_with_filter(
                &self,
                filter: &crate::attribute::DependencyFilter,
                consumer: &mut dyn FnMut(&crate::attribute::Attribute, &blaze_cmdline::Label),
            ) {
                self.visit_labels_with_filter_impl(filter, consumer)
            }
            fn get_package_args(&self) -> crate::package::PackageArgs {
                crate::attribute::AttributeMap::get_package_args(&self.base)
            }
        }
    };
}

pub(crate) use delegate_attribute_map;
