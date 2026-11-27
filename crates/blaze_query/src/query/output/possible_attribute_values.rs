//! Port of `query2/query/output/PossibleAttributeValues.java`.

use blaze_packages::attribute::{AggregatingAttributeMapper, Attribute, AttributeMap};
use blaze_packages::target::Rule;
use blaze_packages::types::{AttrValue, Type};

// PORT: skipped, a module of free functions has no instance: PossibleAttributeValues.<init>

/// Returns the possible values of the specified attribute in the specified rule. For
/// configurable attributes, multiple values may be possible (one per branch of the select, or
/// the concatenations of the selects of a list-typed attribute); `None` elements are Java's
/// nulls.
///
/// If `may_treat_multiple_as_none` is true, a configurable attribute with more than one value
/// may be returned as empty.
// Java: com/google/devtools/build/lib/query2/query/output/PossibleAttributeValues.java:61 PossibleAttributeValues.forRuleAndAttribute
pub fn for_rule_and_attribute(
    rule: &Rule,
    attr: &Attribute,
    may_treat_multiple_as_none: bool,
) -> Vec<Option<AttrValue>> {
    let attribute_map = AggregatingAttributeMapper::of(rule);
    if attr.get_type() == Type::LabelList && attribute_map.base().is_configurable(attr.get_name()) {
        // TODO(b/203330807): figure out what to do when the configurable attribute has multiple
        // values.
        return vec![Some(AttrValue::LabelList(
            attribute_map
                .get_reachable_labels(attr.get_name(), /* include_select_keys= */ false)
                .into_iter()
                .collect(),
        ))];
    }
    let concatenated_selects_value = attribute_map
        .get_concatenated_selector_lists_of_list_type(attr.get_name(), attr.get_type());
    if let Some(concatenated_selects_value) = concatenated_selects_value {
        return concatenated_selects_value.into_iter().map(Some).collect();
    }
    attribute_map.visit_attribute_with(attr.get_name(), attr.get_type(), may_treat_multiple_as_none)
}
