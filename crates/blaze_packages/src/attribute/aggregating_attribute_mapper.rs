//! Port of `packages/AggregatingAttributeMapper.java`: visits all possible values of
//! configurable attributes (query output and dependency visitation use it), and
//! `packages/DelegatingAttributeMapper.java` (the base of the map-backed attribute map).

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::sync::Arc;

use blaze_cmdline::Label;
use indexmap::IndexMap;
use indexmap::IndexSet;

use crate::attribute::Attribute;
use crate::attribute::AttributeMap;
use crate::attribute::DependencyFilter;
use crate::attribute::abstract_attribute_mapper::AbstractAttributeMapper;
use crate::attribute::abstract_attribute_mapper::RuleInfoProvider;
use crate::attribute::abstract_attribute_mapper::delegate_attribute_map;
use crate::package::PackageArgs;
use crate::target::RuleOrMacroInstance;
use crate::types::AttrValue;
use crate::types::BuildTypeSelector;
use crate::types::BuildTypeSelectorList;
use crate::types::LabelClass;
use crate::types::LabelVisitor;
use crate::types::Type;

/// Java `AggregatingAttributeMapper`.
// Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:49 AggregatingAttributeMapper
#[derive(Clone, Copy)]
pub struct AggregatingAttributeMapper<'a> {
    base: AbstractAttributeMapper<'a>,
}

delegate_attribute_map!(AggregatingAttributeMapper);

/// Java `(rawVal instanceof Collection) && ((Collection<?>) rawVal).isEmpty()`.
fn is_empty_collection(value: &AttrValue) -> bool {
    match value {
        AttrValue::StringList(l) => l.is_empty(),
        AttrValue::StringSet(s) => s.is_empty(),
        AttrValue::IntList(l) => l.is_empty(),
        AttrValue::LabelList(l) => l.is_empty(),
        _ => false,
    }
}

/// Java `CollectionUtils.duplicatedElementsOf(list)`.
// PORT: java-hash-order. Java collects the duplicates in a HashSet; they are kept here in order
// of second occurrence.
// Java: com/google/devtools/build/lib/collect/CollectionUtils.java:37 CollectionUtils.duplicatedElementsOf
fn duplicated_elements_of(input: &[Label]) -> IndexSet<Label> {
    let mut duplicates = IndexSet::new();
    if input.len() < 2 {
        return duplicates;
    }
    let mut element_set = std::collections::HashSet::with_capacity(input.len());
    for el in input {
        if !element_set.insert(el) {
            duplicates.insert(el.clone());
        }
    }
    duplicates
}

impl<'a> AggregatingAttributeMapper<'a> {
    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:55 AggregatingAttributeMapper.of
    pub fn of(rule: &'a dyn RuleOrMacroInstance) -> Self {
        AggregatingAttributeMapper {
            base: AbstractAttributeMapper::new(rule),
        }
    }

    pub fn base(&self) -> &AbstractAttributeMapper<'a> {
        &self.base
    }

    /// The select of a configurable attribute, or `None`.
    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:248 AbstractAttributeMapper.getSelectorList
    pub fn get_selector_list(
        &self,
        attribute_name: &str,
        type_: Type,
    ) -> Option<Arc<BuildTypeSelectorList>> {
        self.base.get_selector_list(attribute_name, type_)
    }

    /// All of this rule's attributes that are non-configurable. These are unconditionally
    /// available to computed defaults no matter what dependencies they've declared.
    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:63 AggregatingAttributeMapper.getNonConfigurableAttributes
    fn get_non_configurable_attributes(&self) -> &'a [String] {
        self.base
            .rule
            .get_attribute_provider()
            .get_non_configurable_attributes()
            .expect("NullPointerException: nonConfigurableAttributes")
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

    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:87 AggregatingAttributeMapper.visitLabels
    fn visit_labels_of_attribute_impl(
        &self,
        attribute: &Attribute,
        type_: Type,
        visitor: &mut LabelVisitor<'_>,
    ) {
        let index = self
            .base
            .rule_class()
            .get_attribute_index(attribute.get_name())
            .expect("NullPointerException: attribute index");
        self.visit_labels_5(visitor, attribute, type_, true, index);
    }

    /// Also visits the rule's configurable attribute keys (which are themselves labels),
    /// parsing each selector directly.
    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:75 AggregatingAttributeMapper.visitLabels
    fn visit_labels_with_filter_impl(
        &self,
        filter: &DependencyFilter,
        consumer: &mut dyn FnMut(&Attribute, &Label),
    ) {
        let mut visitor = |label: &Label, attribute: Option<&Attribute>| {
            consumer(
                attribute.expect("NullPointerException: visitor context"),
                label,
            )
        };
        self.visit_labels_with_label_visitor(filter, &mut visitor);
    }

    /// Java package-private `visitLabels(DependencyFilter, Type.LabelVisitor)`.
    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:96 AggregatingAttributeMapper.visitLabels
    pub fn visit_labels_with_label_visitor(
        &self,
        filter: &DependencyFilter,
        visitor: &mut LabelVisitor<'_>,
    ) {
        let info = RuleInfoProvider(self.base.rule);
        let attributes = self.base.rule_class().get_attributes();
        for (i, attr) in attributes.iter().enumerate() {
            let type_ = attr.get_type();
            if type_ != Type::Output
                && type_ != Type::OutputList
                && type_ != Type::NodepLabel
                && type_ != Type::NodepLabelList
                && filter.test(&info, attr)
            {
                self.visit_labels_5(visitor, attr, type_, true, i);
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:112 AggregatingAttributeMapper.visitLabels
    fn visit_labels_5(
        &self,
        visitor: &mut LabelVisitor<'_>,
        attr: &Attribute,
        type_: Type,
        include_select_keys: bool,
        i: usize,
    ) {
        let rule = self.base.rule;
        if type_.get_label_class() == LabelClass::None {
            // The only way for LabelClass.NONE to contain labels is in select keys.
            if include_select_keys && attr.is_configurable() {
                let raw_val = rule.attribute_container().get_attr_if_stored(i);
                if let Some(AttrValue::SelectorList(selector_list)) = raw_val {
                    Self::visit_labels_in_select(
                        selector_list,
                        attr,
                        type_,
                        visitor,
                        Some(rule),
                        /* include_keys= */ true,
                        /* include_values= */ false,
                    );
                }
            }
            return;
        }
        let mut raw_val = rule.attribute_container().get_attr_if_stored(i).cloned();
        if raw_val.is_none() {
            // Frozen rules don't store computed defaults.
            if !attr.has_computed_default() || rule.is_frozen() {
                raw_val = attr.get_default_value(Some(rule));
            }
        }
        if let Some(AttrValue::SelectorList(selector_list)) = &raw_val {
            Self::visit_labels_in_select(
                selector_list,
                attr,
                type_,
                visitor,
                Some(rule),
                /* include_keys= */ include_select_keys,
                /* include_values= */ true,
            );
            return;
        }
        if let Some(AttrValue::ComputedDefault(computed_default)) = &raw_val {
            // Computed defaults are a special pain: we have no choice but to iterate through
            // their (computed) values and look for labels.
            for value in computed_default
                .get_possible_values(type_, rule)
                .iter()
                .flatten()
            {
                type_.visit_labels(visitor, value, Some(attr));
            }
            return;
        }
        let raw_val = match raw_val {
            Some(AttrValue::LateBound(late_bound)) => late_bound.get_default(Some(rule)),
            Some(AttrValue::Materializer(materializer)) => materializer.get_default(),
            other => other,
        };
        let Some(raw_val) = raw_val else {
            return;
        };
        if is_empty_collection(&raw_val) {
            return;
        }
        type_.visit_labels(visitor, &raw_val, Some(attr));
    }

    /// Applies `visitor` to the labels in `selector_list` (keys and/or values). If `rule` is
    /// given, it interprets a possible late-bound default of the attribute.
    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:182 AggregatingAttributeMapper.visitLabelsInSelect
    pub fn visit_labels_in_select(
        selector_list: &BuildTypeSelectorList,
        attribute: &Attribute,
        type_: Type,
        visitor: &mut LabelVisitor<'_>,
        rule: Option<&dyn RuleOrMacroInstance>,
        include_keys: bool,
        include_values: bool,
    ) {
        for selector in selector_list.get_selectors() {
            let has_default = selector.has_default();
            let unconditional = selector.is_unconditional();
            selector.for_each(|key, val| {
                if include_keys
                    && !unconditional
                    && (!has_default || !BuildTypeSelector::is_default_condition_label(key))
                {
                    visitor(key, Some(attribute));
                }
                if include_values {
                    let value = if selector.is_value_set(key) {
                        val.cloned()
                    } else {
                        attribute.get_default_value(rule)
                    };
                    // PORT: Java visits a null value too; its label visitors drop null labels.
                    if let Some(value) = value {
                        type_.visit_labels(visitor, &value, Some(attribute));
                    }
                }
            });
        }
    }

    /// All labels reachable via the given attribute, with duplicate instances removed.
    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:229 AggregatingAttributeMapper.getReachableLabels
    pub fn get_reachable_labels(
        &self,
        attribute_name: &str,
        include_select_keys: bool,
    ) -> IndexSet<Label> {
        let rule_class = self.base.rule_class();
        let attribute_index = rule_class
            .get_attribute_index(attribute_name)
            .expect("NullPointerException: attribute index");
        let attribute = rule_class.get_attribute(attribute_index);
        let mut builder = IndexSet::new();
        self.visit_labels_5(
            &mut |label: &Label, _attr: Option<&Attribute>| {
                builder.insert(label.clone());
            },
            attribute,
            attribute.get_type(),
            include_select_keys,
            attribute_index,
        );
        builder
    }

    /// The labels that appear multiple times in the same attribute value.
    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:244 AggregatingAttributeMapper.checkForDuplicateLabels
    pub fn check_for_duplicate_labels(&self, attribute: &Attribute) -> IndexSet<Label> {
        let attr_type = attribute.get_type();
        if attr_type != Type::LabelList && attr_type != Type::LabelListDict {
            return IndexSet::new();
        }
        let attr_name = attribute.get_name();
        let raw_val = self
            .base
            .rule
            .get_attr_typed(attr_name, attribute.get_type());

        if attr_type == Type::LabelListDict {
            let mut duplicates: Option<IndexSet<Label>> = None;
            // For LABEL_LIST_DICT, independently check each value list for duplicates.
            if !matches!(raw_val, Some(AttrValue::SelectorList(_))) {
                // Plain old attribute (no selects).
                let possible_dicts =
                    self.visit_raw_non_configurable_attribute_value(raw_val, attr_name, attr_type);
                for dict in possible_dicts.iter().flatten() {
                    if let AttrValue::LabelListDict(dict) = dict {
                        for labels in dict.values() {
                            duplicates = add_duplicate_labels(duplicates, labels);
                        }
                    }
                }
                return duplicates.unwrap_or_default();
            }
            let Some(AttrValue::SelectorList(selector_list)) = &raw_val else {
                unreachable!()
            };
            for selector in selector_list.get_selectors() {
                for dict in selector.values_copy().iter().flatten() {
                    if let AttrValue::LabelListDict(dict) = dict {
                        for labels in dict.values() {
                            duplicates = add_duplicate_labels(duplicates, labels);
                        }
                    }
                }
            }
            return duplicates.unwrap_or_default();
        }

        // Plain old attribute (no selects).
        if !matches!(raw_val, Some(AttrValue::SelectorList(_))) {
            let possible =
                self.visit_raw_non_configurable_attribute_value(raw_val, attr_name, attr_type);
            return check_for_duplicate_labels_in(&label_lists(&possible));
        }
        let Some(AttrValue::SelectorList(selector_list)) = &raw_val else {
            unreachable!()
        };

        let selectors = selector_list.get_selectors();

        // "attr = select({...})" with just a single select.
        if selectors.len() == 1 {
            return check_for_duplicate_labels_in(&label_lists(&selectors[0].values_copy()));
        }

        // It's expensive to iterate over every possible permutation of values, so instead check
        // for duplicates within a single select branch. Then, after analysis we will check for
        // duplicates within only the used permutations.
        let mut duplicates: Option<IndexSet<Label>> = None;
        for selector in selectors {
            for labels_in_selector_value in label_lists(&selector.values_copy()) {
                // Duplicates within a single select branch are not okay.
                duplicates = add_duplicate_labels(duplicates, labels_in_selector_value);
            }
        }
        duplicates.unwrap_or_default()
    }

    /// For a configurable list attribute, one element per select: the concatenation of all of
    /// that select's values. `None` if the attribute isn't of that form.
    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:351 AggregatingAttributeMapper.getConcatenatedSelectorListsOfListType
    pub fn get_concatenated_selector_lists_of_list_type(
        &self,
        attribute_name: &str,
        type_: Type,
    ) -> Option<Vec<AttrValue>> {
        let selector_list = self.base.get_selector_list(attribute_name, type_);
        if let Some(selector_list) = selector_list
            && type_.is_list_type()
        {
            let mut select_list = Vec::new();
            for selector in selector_list.get_selectors() {
                let mut values = Vec::with_capacity(selector.get_num_entries());
                selector.for_each(|_label, value| {
                    values.push(
                        value
                            .cloned()
                            .expect("NullPointerException: select branch value"),
                    )
                });
                select_list.push(
                    type_
                        .concat(values)
                        .expect("NullPointerException: concatenation"),
                );
            }
            return Some(select_list);
        }
        None
    }

    /// All possible values of the attribute: every select combination; computed defaults over
    /// all their dependencies' values. **Efficiency warning**: exponential in the number of
    /// selects; prefer [`AggregatingAttributeMapper::get_reachable_labels`].
    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:403 AggregatingAttributeMapper.visitAttribute
    pub fn visit_attribute(&self, attribute_name: &str, type_: Type) -> Vec<Option<AttrValue>> {
        self.visit_attribute_with(attribute_name, type_, false)
    }

    /// Like [`AggregatingAttributeMapper::visit_attribute`]; with `may_treat_multiple_as_none`,
    /// computation may stop (returning nothing) once a second value is found.
    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:416 AggregatingAttributeMapper.visitAttribute
    pub fn visit_attribute_with(
        &self,
        attribute_name: &str,
        type_: Type,
        may_treat_multiple_as_none: bool,
    ) -> Vec<Option<AttrValue>> {
        let raw_val = self.base.rule.get_attr_typed(attribute_name, type_);

        // If this attribute value is configurable, visit all possible values.
        if let Some(AttrValue::SelectorList(selector_list)) = &raw_val {
            return get_all_values(
                selector_list.get_selectors(),
                type_,
                may_treat_multiple_as_none,
            )
            .into_iter()
            .map(Some)
            .collect();
        }

        self.visit_raw_non_configurable_attribute_value(raw_val, attribute_name, type_)
    }

    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:428 AggregatingAttributeMapper.visitRawNonConfigurableAttributeValue
    fn visit_raw_non_configurable_attribute_value(
        &self,
        raw_val: Option<AttrValue>,
        attribute_name: &str,
        type_: Type,
    ) -> Vec<Option<AttrValue>> {
        // If this attribute is a computed default, feed it all possible value combinations of
        // its declared dependencies and return all computed results.
        if let Some(AttrValue::ComputedDefault(computed_default)) = &raw_val {
            return computed_default.get_possible_values(type_, self.base.rule);
        }

        if attribute_name == "visibility" && type_ == Type::NodepLabelList {
            // This special case for the visibility attribute is needed because its value is
            // replaced with an empty list during package loading if it is public or private in
            // order not to visit the package called 'visibility'.
            return vec![Some(AttrValue::LabelList(
                self.base.rule.get_visibility_declared_labels(),
            ))];
        }

        // For any other attribute, just return its direct value.
        match self
            .base
            .get_from_raw_attribute_value(self, raw_val, attribute_name, type_)
        {
            None => Vec::new(),
            Some(value) => vec![Some(value)],
        }
    }

    /// For the given attributes, an `{attrName -> attrValue}` map for every possible combination
    /// of their values. `limiter` is called with the running combination count and may abort
    /// (Java `ComputationLimiter`; the error is its exception message).
    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:470 AggregatingAttributeMapper.visitAttributes
    pub fn visit_attributes(
        &self,
        attributes: &[String],
        limiter: &mut dyn FnMut(i32) -> Result<(), String>,
    ) -> Result<Vec<HashMap<String, Option<AttrValue>>>, String> {
        let mut dep_maps = Vec::new();
        let mut combinations_so_far = 0;
        self.visit_attributes_inner(
            attributes,
            &mut dep_maps,
            &mut HashMap::with_capacity(attributes.len()),
            &mut combinations_so_far,
            limiter,
        )?;
        Ok(dep_maps)
    }

    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:495 AggregatingAttributeMapper.visitAttributesInner
    fn visit_attributes_inner(
        &self,
        attributes: &[String],
        mappings: &mut Vec<HashMap<String, Option<AttrValue>>>,
        current_map: &mut HashMap<String, Option<AttrValue>>,
        combinations_so_far: &mut i32,
        limiter: &mut dyn FnMut(i32) -> Result<(), String>,
    ) -> Result<(), String> {
        if attributes.is_empty() {
            // Because this method uses exponential time/space on the number of inputs, we may
            // limit the total number of method calls.
            *combinations_so_far += 1;
            limiter(*combinations_so_far)?;
            // Recursive base case: snapshot and store whatever's already been populated in
            // currentMap.
            mappings.push(current_map.clone());
            return Ok(());
        }

        // Take the first attribute in the dependency list and iterate over all its values. For
        // each value x, update currentMap with the additional entry { firstAttrName: x }, then
        // feed this recursively into a subcall over all remaining dependencies.
        let current_attribute = &attributes[0];
        let attribute_type = self
            .get_attribute_type(current_attribute)
            .expect("NullPointerException: attribute type");
        let first_attribute_possible_values =
            self.visit_attribute(current_attribute, attribute_type);
        let rest_of_attrs = &attributes[1..];
        for value in first_attribute_possible_values {
            // Overwrite each time.
            current_map.insert(current_attribute.clone(), value);
            self.visit_attributes_inner(
                rest_of_attrs,
                mappings,
                current_map,
                combinations_so_far,
                limiter,
            )?;
        }
        Ok(())
    }

    /// An [`AttributeMap`] that delegates to this mapper except for `get` of configurable
    /// attributes, which are looked up in `direct_map` (missing ones panic, as Java's
    /// IllegalArgumentException).
    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:533 AggregatingAttributeMapper.createMapBackedAttributeMap
    pub fn create_map_backed_attribute_map(
        &self,
        direct_map: HashMap<String, Option<AttrValue>>,
    ) -> Box<dyn AttributeMap + 'a> {
        Box::new(MapBackedAttributeMap {
            owner: *self,
            direct_map,
        })
    }
}

/// The label lists among possible values of a `LABEL_LIST` attribute.
fn label_lists(values: &[Option<AttrValue>]) -> Vec<&Vec<Label>> {
    values
        .iter()
        .map(|v| match v {
            Some(AttrValue::LabelList(l)) => l,
            other => panic!("ClassCastException: {other:?} is not a label list"),
        })
        .collect()
}

// Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:306 AggregatingAttributeMapper.checkForDuplicateLabels
fn check_for_duplicate_labels_in(possible_labels: &[&Vec<Label>]) -> IndexSet<Label> {
    match possible_labels.len() {
        0 => IndexSet::new(),
        1 => duplicated_elements_of(possible_labels[0]),
        _ => {
            let mut duplicates: Option<IndexSet<Label>> = None;
            for labels in possible_labels {
                duplicates = add_duplicate_labels(duplicates, labels);
            }
            duplicates.unwrap_or_default()
        }
    }
}

// Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:326 AggregatingAttributeMapper.addDuplicateLabels
fn add_duplicate_labels(
    builder: Option<IndexSet<Label>>,
    labels: &[Label],
) -> Option<IndexSet<Label>> {
    let duplicates = duplicated_elements_of(labels);
    if duplicates.is_empty() {
        return builder;
    }
    let mut builder = builder.unwrap_or_default();
    builder.extend(duplicates);
    Some(builder)
}

/// A node in the logical DAG of combinations of selectors' values (Java
/// `AggregatingAttributeMapper.ConfigurableAttrVisitationNode`).
// Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:572 AggregatingAttributeMapper.ConfigurableAttrVisitationNode
struct ConfigurableAttrVisitationNode {
    /// Offset into the list of selectors being combined.
    offset: usize,
    /// Key of the selector taken.
    bound_key: Label,
    /// Accumulated value through this node.
    value_so_far: Option<AttrValue>,
}

/// A path previously taken through a previous selector (Java
/// `AggregatingAttributeMapper.BoundKeyAndOffset`).
// Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:594 AggregatingAttributeMapper.BoundKeyAndOffset
struct BoundKeyAndOffset {
    /// Key chosen from associated select.
    key: Label,
    /// Offset into the list of selectors where this key was bound.
    offset: usize,
}

/// All possible values of a configurable attribute.
// Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:619 AggregatingAttributeMapper.getAllValues
pub(crate) fn get_all_values(
    selectors: &[BuildTypeSelector],
    type_: Type,
    may_treat_multiple_as_none: bool,
) -> Vec<AttrValue> {
    if selectors.is_empty() {
        return Vec::new();
    }

    if selectors.len() == 1 {
        // Optimize for common case.
        let mut result_builder = Vec::new();
        selectors[0].for_each(|_key, value| {
            if let Some(value) = value {
                result_builder.push(value.clone());
            }
        });
        return result_builder;
    }

    let selector_maps: Vec<IndexMap<Label, Option<AttrValue>>> =
        selectors.iter().map(BuildTypeSelector::map_copy).collect();

    // Java ArrayDeque push/pop: a stack.
    let mut nodes: Vec<ConfigurableAttrVisitationNode> = Vec::new();
    // Track per selector key set when we started visiting a specific key.
    let mut bound_keys_and_offsets: HashMap<BTreeSet<Label>, BoundKeyAndOffset> = HashMap::new();
    let mut result = Vec::new();

    let key_set =
        |m: &IndexMap<Label, Option<AttrValue>>| -> BTreeSet<Label> { m.keys().cloned().collect() };

    // Seed visitation.
    for (key, value) in &selector_maps[0] {
        nodes.push(ConfigurableAttrVisitationNode {
            offset: 0,
            bound_key: key.clone(),
            value_so_far: value.clone(),
        });
    }

    let mut found_results = false;
    while let Some(node) = nodes.pop() {
        let next_offset = node.offset + 1;
        if next_offset >= selectors.len() {
            // Null values arise when a None is used as the value of a Selector for a type
            // without a default value.
            if let Some(value_so_far) = node.value_so_far {
                if found_results && may_treat_multiple_as_none {
                    // Caller wanted one value or none at all, this is the second, so bail.
                    return Vec::new();
                }
                found_results = true;
                result.push(value_so_far);
            }
            continue;
        }

        let next_selector_entries = &selector_maps[next_offset];
        let next_keys = key_set(next_selector_entries);
        if let Some(bound_key_and_offset) = bound_keys_and_offsets.get(&next_keys)
            && bound_key_and_offset.offset < node.offset
        {
            // We've seen this select key set before along this path and chosen this key.
            let key = bound_key_and_offset.key.clone();
            let next = next_selector_entries.get(&key).cloned().flatten();
            nodes.push(ConfigurableAttrVisitationNode {
                offset: next_offset,
                value_so_far: concat(type_, node.value_so_far, next),
                bound_key: key,
            });
            continue;
        }

        let current_keys = key_set(&selector_maps[node.offset]);
        // Record that we've descended along node.boundKey starting at this offset.
        bound_keys_and_offsets.insert(
            current_keys.clone(),
            BoundKeyAndOffset {
                key: node.bound_key.clone(),
                offset: node.offset,
            },
        );

        if current_keys == next_keys {
            let next = next_selector_entries
                .get(&node.bound_key)
                .cloned()
                .flatten();
            nodes.push(ConfigurableAttrVisitationNode {
                offset: next_offset,
                value_so_far: concat(type_, node.value_so_far, next),
                bound_key: node.bound_key,
            });
            continue;
        }

        for (key, value) in next_selector_entries {
            nodes.push(ConfigurableAttrVisitationNode {
                offset: next_offset,
                bound_key: key.clone(),
                value_so_far: concat(type_, node.value_so_far.clone(), value.clone()),
            });
        }
    }

    result
}

/// `type.concat(ImmutableList.of(lhs, rhs))`; Java's `ImmutableList.of` throws on null.
// Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:708 AggregatingAttributeMapper.concat
fn concat(type_: Type, lhs: Option<AttrValue>, rhs: Option<AttrValue>) -> Option<AttrValue> {
    let (Some(lhs), Some(rhs)) = (lhs, rhs) else {
        panic!("NullPointerException: ImmutableList.of with a null element");
    };
    type_.concat(vec![lhs, rhs])
}

/// The map-backed attribute map of computed defaults (Java: an anonymous subclass of
/// `DelegatingAttributeMapper` in `createMapBackedAttributeMap`).
// Java: com/google/devtools/build/lib/packages/DelegatingAttributeMapper.java:30 DelegatingAttributeMapper
struct MapBackedAttributeMap<'a> {
    owner: AggregatingAttributeMapper<'a>,
    direct_map: HashMap<String, Option<AttrValue>>,
}

impl AttributeMap for MapBackedAttributeMap<'_> {
    // Java: com/google/devtools/build/lib/packages/DelegatingAttributeMapper.java:40 DelegatingAttributeMapper.getLabel
    fn get_label(&self) -> &Label {
        self.owner.get_label()
    }

    // Java: com/google/devtools/build/lib/packages/DelegatingAttributeMapper.java:105 DelegatingAttributeMapper.has
    fn has(&self, attr_name: &str) -> bool {
        self.owner.has(attr_name)
    }

    // Java: com/google/devtools/build/lib/packages/DelegatingAttributeMapper.java:110 DelegatingAttributeMapper.has
    fn has_typed(&self, attr_name: &str, type_: Type) -> bool {
        self.owner.has_typed(attr_name, type_)
    }

    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:539 AggregatingAttributeMapper.createMapBackedAttributeMap.get
    fn get(&self, attribute_name: &str, type_: Type) -> Option<AttrValue> {
        self.owner.base.check_type(attribute_name, type_);
        if self
            .owner
            .get_non_configurable_attributes()
            .iter()
            .any(|a| a == attribute_name)
        {
            return self.owner.get(attribute_name, type_);
        }

        match self.direct_map.get(attribute_name) {
            Some(Some(val)) => Some(val.clone()),
            Some(None) => None,
            None => panic!(
                "IllegalArgumentException: attribute \"{attribute_name}\" isn't available in this \
                 computed default context"
            ),
        }
    }

    // Java: com/google/devtools/build/lib/packages/DelegatingAttributeMapper.java:50 DelegatingAttributeMapper.isConfigurable
    fn is_configurable(&self, attribute_name: &str) -> bool {
        self.owner.is_configurable(attribute_name)
    }

    // Java: com/google/devtools/build/lib/packages/AggregatingAttributeMapper.java:557 AggregatingAttributeMapper.createMapBackedAttributeMap.getAttributeNames
    fn get_attribute_names(&self) -> Vec<String> {
        let non_configurable_attributes = self.owner.get_non_configurable_attributes();
        let mut names =
            Vec::with_capacity(self.direct_map.len() + non_configurable_attributes.len());
        // PORT: java-hash-order. Java iterates a HashMap's key set; callers only build maps from
        // the names.
        names.extend(self.direct_map.keys().cloned());
        names.extend(non_configurable_attributes.iter().cloned());
        names
    }

    // Java: com/google/devtools/build/lib/packages/DelegatingAttributeMapper.java:60 DelegatingAttributeMapper.getAttributeType
    fn get_attribute_type(&self, attr_name: &str) -> Option<Type> {
        self.owner.get_attribute_type(attr_name)
    }

    // Java: com/google/devtools/build/lib/packages/DelegatingAttributeMapper.java:66 DelegatingAttributeMapper.getAttributeDefinition
    fn get_attribute_definition(&self, attr_name: &str) -> Option<&Attribute> {
        self.owner
            .base
            .rule_class()
            .get_attribute_by_name_maybe(attr_name)
    }

    // Java: com/google/devtools/build/lib/packages/DelegatingAttributeMapper.java:71 DelegatingAttributeMapper.isAttributeValueExplicitlySpecified
    fn is_attribute_value_explicitly_specified(&self, attribute_name: &str) -> bool {
        self.owner
            .is_attribute_value_explicitly_specified(attribute_name)
    }

    // Java: com/google/devtools/build/lib/packages/DelegatingAttributeMapper.java:76 DelegatingAttributeMapper.visitAllLabels
    fn visit_all_labels(&self, consumer: &mut dyn FnMut(&Attribute, &Label)) {
        self.owner.visit_all_labels(consumer)
    }

    // Java: com/google/devtools/build/lib/packages/DelegatingAttributeMapper.java:81 DelegatingAttributeMapper.visitLabels
    fn visit_labels(&self, attribute_name: &str, consumer: &mut dyn FnMut(&Label)) {
        self.owner.visit_labels(attribute_name, consumer)
    }

    // Java: com/google/devtools/build/lib/packages/DelegatingAttributeMapper.java:86 DelegatingAttributeMapper.visitLabels
    fn visit_labels_with_filter(
        &self,
        filter: &DependencyFilter,
        consumer: &mut dyn FnMut(&Attribute, &Label),
    ) {
        self.owner.visit_labels_with_filter(filter, consumer)
    }

    // Java: com/google/devtools/build/lib/packages/DelegatingAttributeMapper.java:91 DelegatingAttributeMapper.getPackageArgs
    fn get_package_args(&self) -> PackageArgs {
        self.owner.get_package_args()
    }
}
