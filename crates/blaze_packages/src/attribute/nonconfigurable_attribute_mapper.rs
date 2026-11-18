//! Port of `packages/NonconfigurableAttributeMapper.java`: reads attributes that must not be
//! configurable; fails on configurable ones.

use blaze_cmdline::Label;

use crate::attribute::Attribute;
use crate::attribute::AttributeMap;
use crate::attribute::DependencyFilter;
use crate::attribute::abstract_attribute_mapper::AbstractAttributeMapper;
use crate::attribute::abstract_attribute_mapper::delegate_attribute_map;
use crate::target::RuleOrMacroInstance;
use crate::types::AttrValue;
use crate::types::LabelVisitor;
use crate::types::Type;

/// Java `NonconfigurableAttributeMapper`.
// Java: com/google/devtools/build/lib/packages/NonconfigurableAttributeMapper.java:26 NonconfigurableAttributeMapper
#[derive(Clone, Copy)]
pub struct NonconfigurableAttributeMapper<'a> {
    base: AbstractAttributeMapper<'a>,
}

delegate_attribute_map!(NonconfigurableAttributeMapper);

impl<'a> NonconfigurableAttributeMapper<'a> {
    // Java: com/google/devtools/build/lib/packages/NonconfigurableAttributeMapper.java:39 NonconfigurableAttributeMapper.of
    pub fn of(rule: &'a dyn RuleOrMacroInstance) -> Self {
        NonconfigurableAttributeMapper {
            base: AbstractAttributeMapper::new(rule),
        }
    }

    // Java: com/google/devtools/build/lib/packages/NonconfigurableAttributeMapper.java:44 NonconfigurableAttributeMapper.get
    fn get_impl(&self, attribute_name: &str, type_: Type) -> Option<AttrValue> {
        let attr = self.base.get_from_raw_attribute_value(
            self,
            self.base.rule.get_attr_typed(attribute_name, type_),
            attribute_name,
            type_,
        );
        let definition = self
            .get_attribute_definition(attribute_name)
            .expect("NullPointerException: attribute definition");
        blaze_util::check_state!(
            !definition.is_configurable(),
            "Attribute '{}' is potentially configurable - not allowed here",
            attribute_name
        );
        attr
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

    /// The attribute's value, or `None` if the rule has no such attribute of `type_`.
    // Java: com/google/devtools/build/lib/packages/NonconfigurableAttributeMapper.java:52 NonconfigurableAttributeMapper.attributeOrNull
    pub fn attribute_or_null(
        rule: &dyn RuleOrMacroInstance,
        attr_name: &str,
        type_: Type,
    ) -> Option<AttrValue> {
        let mapper = NonconfigurableAttributeMapper::of(rule);
        if !mapper.has_typed(attr_name, type_) {
            return None;
        }
        mapper.get(attr_name, type_)
    }
}
