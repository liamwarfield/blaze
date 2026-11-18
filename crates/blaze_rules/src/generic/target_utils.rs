//! `packages/TargetUtils.getExecutionInfo` in Java's iteration order.
//!
//! PORT: `blaze_packages::target::target_utils::get_execution_info` returns a sorted map; Java
//! returns `ImmutableMap.copyOf` of a `java.util.HashMap` filled in the order of the `tags`
//! attribute value (which is sorted: `tags` is order-independent, BuildType.java:249), and that
//! iteration order reaches action keys and aquery output wherever the caller keeps it
//! (`CppCompileActionBuilder`'s `LinkedHashMap`). The order is computed here.

use blaze_packages::attribute::AttributeMap;
use blaze_packages::attribute::nonconfigurable_attribute_mapper::NonconfigurableAttributeMapper;
use blaze_packages::target::{Rule, RuleOrMacroInstance};
use blaze_packages::types::{AttrValue, Type};
use indexmap::IndexMap;

use crate::cpp::util::java_hash_map_order;

// *_test / test_suite attribute that used to specify constraint keywords.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:51 TargetUtils.CONSTRAINTS_ATTR
const CONSTRAINTS_ATTR: &str = "tags";

/// Returns the execution info from the tags declared on the target. These include only some tags
/// (`legalExecInfoKeys`) as keys with empty values.
// PORT: java-hash-order.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:243 TargetUtils.getExecutionInfo
pub fn get_execution_info(rule: &Rule) -> IndexMap<String, String> {
    // Java: `legalExecInfoKeys(tag)`; private in blaze_packages, whose (sorted) result holds
    // exactly the legal tags.
    let legal = blaze_packages::target::target_utils::get_execution_info(rule);
    // tags may contain duplicate values.
    let tags: Vec<String> =
        match NonconfigurableAttributeMapper::of(rule as &dyn RuleOrMacroInstance)
            .get(CONSTRAINTS_ATTR, Type::StringList)
        {
            Some(AttrValue::StringList(tags)) => tags
                .iter()
                .filter(|tag| legal.contains_key(tag.as_str()))
                .cloned()
                .collect(),
            // blaze_packages' `get_execution_info` treats any other value as "no tags".
            _ => Vec::new(),
        };
    execution_info_of_legal_tags(&tags)
}

/// `map.put(tag, "")` for each (legal) tag in order, then `ImmutableMap.copyOf(map)`.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:245 TargetUtils.getExecutionInfo
fn execution_info_of_legal_tags(tags: &[String]) -> IndexMap<String, String> {
    java_hash_map_order(tags)
        .into_iter()
        .map(|tag| (tag, String::new()))
        .collect()
}

/// Returns the execution info from the tags declared on the target, or nothing unless
/// `allow_tags_propagation`.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:264 TargetUtils.getExecutionInfo
pub fn get_execution_info_with_propagation(
    rule: &Rule,
    allow_tags_propagation: bool,
) -> IndexMap<String, String> {
    if allow_tags_propagation {
        get_execution_info(rule)
    } else {
        IndexMap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(tags: &[&str]) -> Vec<String> {
        let tags: Vec<String> = tags.iter().map(|t| (*t).to_owned()).collect();
        execution_info_of_legal_tags(&tags).into_keys().collect()
    }

    /// The `executionInfo` order Bazel 9.3.2 prints (raw `aquery --output=jsonproto`) for the
    /// CppCompile actions of scripts/w4/scenarios/cc-tags. `tags` is an order-independent
    /// attribute, so its value arrives sorted (BuildType.java:249): the inputs below are the
    /// attribute values of `tags = ["no-remote-exec", "no-remote"]` (and the reverse),
    /// `["no-sandbox", "no-asan"]`, `["no-sandbox", "requires-network", "no-sandbox"]` and
    /// `["no-cache", "exclusive"]` without the tag that is not a legal key (analysis review
    /// S1-6).
    #[test]
    fn execution_info_keeps_java_hash_map_order() {
        assert_eq!(
            keys(&["no-remote", "no-remote-exec"]),
            vec!["no-remote", "no-remote-exec"]
        );
        assert_eq!(
            keys(&["no-asan", "no-sandbox"]),
            vec!["no-asan", "no-sandbox"]
        );
        assert_eq!(
            keys(&["no-sandbox", "no-sandbox", "requires-network"]),
            vec!["requires-network", "no-sandbox"]
        );
        assert_eq!(keys(&["no-cache"]), vec!["no-cache"]);
    }
}
