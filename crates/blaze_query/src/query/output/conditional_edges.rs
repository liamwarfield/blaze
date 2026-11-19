//! Port of `query2/query/output/ConditionalEdges.java`.

use std::collections::HashMap;

use blaze_cmdline::Label;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::attribute::raw_attribute_mapper::RawAttributeMapper;
use blaze_packages::target::{Rule, RuleOrMacroInstance};
use blaze_packages::types::attr_value::AttrValue;
use blaze_query_engine::JavaHashCode;
use blaze_query_engine::java_concurrent_hash_map::java_hash_map_order_by;

use crate::query::blaze_query_environment::TargetDigraph;

/// The table length of a value set of Guava's `HashMultimap.create()`:
/// `Sets.newHashSetWithExpectedSize(DEFAULT_VALUES_PER_KEY = 2)` is `new HashSet<>(3)`, whose
/// table has `tableSizeFor(3) = 4` slots.
const HASH_MULTIMAP_VALUE_SET_TABLE_SIZE: usize = 4;

/// The order in which a `HashSet` of labels made by `HashMultimap.create()` iterates, after
/// adding `labels` in order.
///
/// PORT: java-hash-order. Labels hash by value in Java, so the order is reproducible and reaches
/// `--output=graph` (the condition labels on an edge).
fn hash_multimap_value_order(labels: Vec<Label>) -> Vec<Label> {
    java_hash_map_order_by(
        HASH_MULTIMAP_VALUE_SET_TABLE_SIZE,
        labels.into_iter().map(|l| (l, ())).collect(),
        |l| l.java_hash_code(),
    )
    .into_iter()
    .map(|(l, ())| l)
    .collect()
}

/// Inserts `value` into a `HashMultimap`'s value set kept in insertion order (the iteration
/// order is computed when the set is read).
fn add_to_set(set: &mut Vec<Label>, value: Label) {
    if !set.contains(&value) {
        set.push(value);
    }
}

/// Class to represent all the conditional edges of a graph (edges coming from `select()`s).
// Java: com/google/devtools/build/lib/query2/query/output/ConditionalEdges.java:37 ConditionalEdges
#[derive(Default)]
pub struct ConditionalEdges {
    /// Map containing all the conditions for all the conditional edges in a graph: src node ->
    /// (dest node -> condition labels). Each value set keeps its insertion order; `get`
    /// returns it in Java's `HashSet` order.
    map: HashMap<Label, HashMap<Label, Vec<Label>>>,
}

impl ConditionalEdges {
    // Java: com/google/devtools/build/lib/query2/query/output/ConditionalEdges.java:43 ConditionalEdges.<init>
    pub fn new() -> Self {
        ConditionalEdges {
            map: HashMap::new(),
        }
    }

    /// Builds ConditionalEdges from given graph.
    // Java: com/google/devtools/build/lib/query2/query/output/ConditionalEdges.java:46 ConditionalEdges.<init>
    pub fn from_graph(graph: &TargetDigraph) -> Self {
        let mut edges = ConditionalEdges::new();

        for node in graph.get_nodes() {
            let Some(rule) = graph.get_label(node).0.get_associated_rule() else {
                // rule is null for source files and package groups. Skip them.
                continue;
            };

            let conditions = get_all_conditions(rule, &RawAttributeMapper::of(&**rule));
            if conditions.is_empty() {
                // bail early for most common case of no conditions in the rule.
                continue;
            }

            let node_label = graph.get_label(node).0.get_label().clone();
            for succ in graph.get_successors(node) {
                let successor_label = graph.get_label(*succ).0.get_label().clone();
                if let Some(conditions_of_successor) = conditions.get(&successor_label) {
                    // conditions.get(successorLabel): the HashMultimap's value set, in its order.
                    let ordered = hash_multimap_value_order(conditions_of_successor.clone());
                    edges.insert(node_label.clone(), successor_label, ordered);
                }
            }
        }
        edges
    }

    /// Inserts `conditions` for edge src --> dest.
    // Java: com/google/devtools/build/lib/query2/query/output/ConditionalEdges.java:75 ConditionalEdges.insert
    pub fn insert(&mut self, src: Label, dest: Label, conditions: Vec<Label>) {
        let set = self.map.entry(src).or_default().entry(dest).or_default();
        for condition in conditions {
            add_to_set(set, condition);
        }
    }

    /// Returns all conditions for edge src --> dest, if they exist. Does not return default
    /// conditions.
    // Java: com/google/devtools/build/lib/query2/query/output/ConditionalEdges.java:84 ConditionalEdges.get
    pub fn get(&self, src: &Label, dest: &Label) -> Option<Vec<Label>> {
        let set = self.map.get(src)?.get(dest)?;
        Some(hash_multimap_value_order(set.clone()))
    }
}

/// Returns map of dependency to list of condition-labels.
///
/// Example: For a rule like below,
///
/// ```text
///  some_rule(
///    ...
///    deps = [
///      ... default dependencies ...
///    ] + select ({
///      "//some:config1": [ "//some:a", "//some:common" ],
///      "//some:config2": [ "//other:a", "//some:common" ],
///      "//conditions:default": [ "//some:default" ],
///    })
///  )
/// ```
///
/// it returns following map:
///
/// ```text
///  {
///    "//some:a": ["//some:config1" ]
///    "//other:a": ["//some:config2" ]
///    "//some:common": ["//some:config1", "//some:config2" ]
///    "//some:default": [ "//conditions:default" ]
///  }
/// ```
///
/// Each value list is in insertion order (the `HashMultimap`'s value set before hashing).
// Java: com/google/devtools/build/lib/query2/query/output/ConditionalEdges.java:126 ConditionalEdges.getAllConditions
fn get_all_conditions(
    rule: &Rule,
    attribute_map: &RawAttributeMapper,
) -> HashMap<Label, Vec<Label>> {
    let mut conditions: HashMap<Label, Vec<Label>> = HashMap::new();
    for attr in rule.get_attributes() {
        // TODO(bazel-team): Handle the case where dependency exists through both configurable as
        // well as non-configurable attributes. Currently this prints such an edge as a
        // conditional one.
        if !attribute_map.base().is_configurable(attr.get_name()) {
            // skip non configurable attributes
            continue;
        }
        if matches!(
            rule.get_attr(attr.get_name()),
            Some(AttrValue::ComputedDefault(_))
        ) {
            // isConfigurable above checks that the attribute is either a `select()` or a
            // computed default. We don't currently handle the latter so skip it.
            continue;
        }

        let raw = attribute_map.get_raw_attribute_value_of(rule, attr);
        let selector_list = raw
            .as_ref()
            .and_then(|v| v.as_selector_list())
            .expect("ClassCastException: not a SelectorList");
        for selector in selector_list.get_selectors() {
            if selector.is_unconditional() {
                // skip unconditional selectors
                continue;
            }
            selector.for_each(|key, value| match value {
                Some(AttrValue::LabelList(deps)) => {
                    for label in deps {
                        add_to_set(conditions.entry(label.clone()).or_default(), key.clone());
                    }
                }
                Some(AttrValue::Label(label)) => {
                    add_to_set(conditions.entry(label.clone()).or_default(), key.clone());
                }
                _ => {}
            });
        }
    }
    conditions
}
