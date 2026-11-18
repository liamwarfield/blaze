//! Port of `packages/RawAttributeMapper.java`: returns raw attribute values, which may be
//! selects.

use blaze_cmdline::Label;
use indexmap::IndexSet;

use crate::attribute::Attribute;
use crate::attribute::AttributeMap;
use crate::attribute::DependencyFilter;
use crate::attribute::abstract_attribute_mapper::AbstractAttributeMapper;
use crate::attribute::abstract_attribute_mapper::delegate_attribute_map;
use crate::target::Rule;
use crate::target::RuleOrMacroInstance;
use crate::types::AttrValue;
use crate::types::LabelVisitor;
use crate::types::Type;

/// Java `RawAttributeMapper`.
// Java: com/google/devtools/build/lib/packages/RawAttributeMapper.java:33 RawAttributeMapper
#[derive(Clone, Copy)]
pub struct RawAttributeMapper<'a> {
    base: AbstractAttributeMapper<'a>,
}

delegate_attribute_map!(RawAttributeMapper);

impl<'a> RawAttributeMapper<'a> {
    // Java: com/google/devtools/build/lib/packages/RawAttributeMapper.java:39 RawAttributeMapper.of
    pub fn of(rule_or_macro_instance: &'a dyn RuleOrMacroInstance) -> Self {
        RawAttributeMapper {
            base: AbstractAttributeMapper::new(rule_or_macro_instance),
        }
    }

    pub fn base(&self) -> &AbstractAttributeMapper<'a> {
        &self.base
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:56 AbstractAttributeMapper.get
    fn get_impl(&self, attribute_name: &str, type_: Type) -> Option<AttrValue> {
        self.base.get_from_raw_attribute_value(
            self,
            self.base.rule.get_attr_typed(attribute_name, type_),
            attribute_name,
            type_,
        )
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:196 AbstractAttributeMapper.visitLabels
    fn visit_labels_of_attribute_impl(
        &self,
        attribute: &Attribute,
        type_: Type,
        visitor: &mut LabelVisitor<'_>,
    ) {
        self.base
            .visit_labels_of_attribute(self, attribute, type_, visitor)
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:169 AbstractAttributeMapper.visitLabels
    fn visit_labels_with_filter_impl(
        &self,
        filter: &DependencyFilter,
        consumer: &mut dyn FnMut(&Attribute, &Label),
    ) {
        self.base.visit_labels_list(
            &self.base.all_attributes(),
            filter,
            consumer,
            &mut |attr, type_, visitor| self.visit_labels_of_attribute_impl(attr, type_, visitor),
        )
    }

    /// For a configurable list attribute, the values of all branches merged (duplicates removed,
    /// first occurrence kept) as a list of the same type; otherwise the attribute's value.
    // PORT: Java returns a Collection of elements; the merged elements are returned as an
    // AttrValue list of `type_`.
    // Java: com/google/devtools/build/lib/packages/RawAttributeMapper.java:62 RawAttributeMapper.getMergedValues
    pub fn get_merged_values(&self, attribute_name: &str, type_: Type) -> Option<AttrValue> {
        blaze_util::check_state!(type_.is_list_type());
        if !self.is_configurable(attribute_name) {
            return self.get(attribute_name, type_);
        }

        let selector_list = self
            .base
            .get_selector_list(attribute_name, type_)
            .expect("NullPointerException: selector list");
        let mut strings: IndexSet<String> = IndexSet::new();
        let mut ints: IndexSet<i32> = IndexSet::new();
        let mut labels: IndexSet<Label> = IndexSet::new();
        for selector in selector_list.get_selectors() {
            selector.for_each(|_label, value| match value {
                None => {}
                Some(AttrValue::StringList(l)) => strings.extend(l.iter().cloned()),
                Some(AttrValue::IntList(l)) => ints.extend(l.iter().copied()),
                Some(AttrValue::LabelList(l)) => labels.extend(l.iter().cloned()),
                Some(other) => panic!("ClassCastException: {other:?} is not a list"),
            });
        }
        Some(match type_ {
            Type::StringList => AttrValue::StringList(strings.into_iter().collect()),
            Type::IntegerList => AttrValue::IntList(ints.into_iter().collect()),
            _ => AttrValue::LabelList(labels.into_iter().collect()),
        })
    }

    /// The configuration keys of a configurable attribute; empty otherwise.
    // Java: com/google/devtools/build/lib/packages/RawAttributeMapper.java:84 RawAttributeMapper.getConfigurabilityKeys
    pub fn get_configurability_keys(&self, attribute_name: &str, type_: Type) -> Vec<Label> {
        let Some(selector_list) = self.base.get_selector_list(attribute_name, type_) else {
            return Vec::new();
        };
        let mut builder = Vec::new();
        for selector in selector_list.get_selectors() {
            selector.for_each(|label, _value| builder.push(label.clone()));
        }
        builder
    }

    /// The raw value (possibly a select or computed default) of the named attribute of `rule`.
    // Java: com/google/devtools/build/lib/packages/RawAttributeMapper.java:103 RawAttributeMapper.getRawAttributeValue
    pub fn get_raw_attribute_value(&self, rule: &Rule, attr_name: &str) -> Option<AttrValue> {
        let attr = match self.get_attribute_definition(attr_name) {
            Some(a) => a,
            None => panic!("NullPointerException: {rule} {attr_name}"),
        };
        self.get_raw_attribute_value_of(rule, attr)
    }

    /// The raw value of `attr`: the visibility's declared labels, a select, a computed default,
    /// or the value of the attribute's type.
    // Java: com/google/devtools/build/lib/packages/RawAttributeMapper.java:125 RawAttributeMapper.getRawAttributeValue
    pub fn get_raw_attribute_value_of(&self, rule: &Rule, attr: &Attribute) -> Option<AttrValue> {
        // This special case for the visibility attribute is needed because its value is replaced
        // with an empty list during package loading if it is public or private in order not to
        // visit the package called 'visibility'.
        if attr.get_name() == "visibility" {
            return Some(AttrValue::LabelList(
                RuleOrMacroInstance::get_visibility_declared_labels(rule),
            ));
        }

        // If the attribute value contains one or more select(...) expressions, then return
        // the SelectorList object representing those expressions.
        if let Some(selector_list) = self
            .base
            .get_selector_list(attr.get_name(), attr.get_type())
        {
            return Some(AttrValue::SelectorList(selector_list));
        }

        // If the attribute value is not explicitly declared, and the rule class provides a
        // computed default value for it, then we should return the ComputedDefault object.
        if let Some(computed_default) = self
            .base
            .get_computed_default(attr.get_name(), attr.get_type())
        {
            return Some(AttrValue::ComputedDefault(computed_default));
        }
        self.get(attr.get_name(), attr.get_type())
    }
}
