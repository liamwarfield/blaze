//! Port of `packages/NonconfiguredAttributeMapper.java`: reads attributes of a rule before
//! configuration; fails on attributes that currently have a select.

use blaze_cmdline::Label;

use crate::attribute::Attribute;
use crate::attribute::AttributeMap;
use crate::attribute::DependencyFilter;
use crate::attribute::abstract_attribute_mapper::AbstractAttributeMapper;
use crate::attribute::abstract_attribute_mapper::delegate_attribute_map;
use crate::target::Rule;
use crate::types::AttrValue;
use crate::types::LabelVisitor;
use crate::types::Type;

/// Java `NonconfiguredAttributeMapper`.
// Java: com/google/devtools/build/lib/packages/NonconfiguredAttributeMapper.java:30 NonconfiguredAttributeMapper
#[derive(Clone, Copy)]
pub struct NonconfiguredAttributeMapper<'a> {
    base: AbstractAttributeMapper<'a>,
}

delegate_attribute_map!(NonconfiguredAttributeMapper);

impl<'a> NonconfiguredAttributeMapper<'a> {
    // Java: com/google/devtools/build/lib/packages/NonconfiguredAttributeMapper.java:43 NonconfiguredAttributeMapper.of
    pub fn of(rule: &'a Rule) -> Self {
        NonconfiguredAttributeMapper {
            base: AbstractAttributeMapper::new(rule),
        }
    }

    // Java: com/google/devtools/build/lib/packages/NonconfiguredAttributeMapper.java:49 NonconfiguredAttributeMapper.get
    fn get_impl(&self, attribute_name: &str, type_: Type) -> Option<AttrValue> {
        let attribute = self.get_attribute_definition(attribute_name)?;
        if attribute.is_configurable() {
            blaze_util::check_state!(
                self.base.get_selector_list(attribute_name, type_).is_none(),
                "Attribute '{}' is configured - not allowed here",
                attribute_name
            );
        }
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
}
