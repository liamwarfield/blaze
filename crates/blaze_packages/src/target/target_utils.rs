//! Ports `packages/TargetUtils.java` and the parts of `packages/TestTargetUtils.java` loading and
//! query use.

use std::collections::BTreeMap;
use std::collections::HashSet;

use blaze_cmdline::Label;
use blaze_starlark::Location;

use crate::attribute::AggregatingAttributeMapper;
use crate::attribute::Attribute;
use crate::attribute::AttributeMap;
use crate::attribute::DependencyFilter;
use crate::attribute::NonconfigurableAttributeMapper;
use crate::exceptions::NoSuchThingException;
use crate::target::Rule;
use crate::target::RuleOrMacroInstance;
use crate::target::Target;
use crate::types::AttrValue;
use crate::types::TriState;
use crate::types::Type;

// *_test / test_suite attribute that used to specify constraint keywords.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:51 TargetUtils.CONSTRAINTS_ATTR
const CONSTRAINTS_ATTR: &str = "tags";

/// Java `ExecutionRequirements.LOCAL`.
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java ExecutionRequirements.LOCAL
const EXECUTION_REQUIREMENTS_LOCAL: &str = "local";

/// Java `ExecutionRequirements.WORKER_KEY_MNEMONIC`.
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java ExecutionRequirements.WORKER_KEY_MNEMONIC
const EXECUTION_REQUIREMENTS_WORKER_KEY_MNEMONIC: &str = "worker-key-mnemonic";

// We don't want to pollute the execution info with random things, and we also need to reserve
// some internal tags that we don't allow to be set on targets. We also don't want to
// exhaustively enumerate all the legal values here. Right now, only a ~small set of tags is
// recognized by Bazel.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:58 TargetUtils.legalExecInfoKeys
// PORT: skipped, the private constructor of a static-only class: TargetUtils.<init>
fn legal_exec_info_keys(tag: &str) -> bool {
    tag.starts_with("block-")
        || tag.starts_with("requires-")
        || tag.starts_with("no-")
        || tag.starts_with("supports-")
        || tag.starts_with("disable-")
        || tag.starts_with("cpu:")
        || tag == EXECUTION_REQUIREMENTS_LOCAL
        || tag == EXECUTION_REQUIREMENTS_WORKER_KEY_MNEMONIC
        || tag.starts_with("resources:")
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:72 TargetUtils.isTestRuleName
pub fn is_test_rule_name(name: &str) -> bool {
    name.ends_with("_test")
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:76 TargetUtils.isTestSuiteRuleName
pub fn is_test_suite_rule_name(name: &str) -> bool {
    name == "test_suite"
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:80 TargetUtils.isExecutableNonTestRule
pub fn is_executable_non_test_rule(target: &Target) -> bool {
    match target {
        Target::Rule(rule) => rule.is_executable() && !is_test_rule(target),
        _ => false,
    }
}

/// True iff `target` is a `*_test` rule; excludes `test_suite`.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:88 TargetUtils.isTestRule
pub fn is_test_rule(target: &Target) -> bool {
    match target {
        Target::Rule(rule) => is_test_rule_name(rule.get_rule_class()),
        _ => false,
    }
}

/// True iff `target` is a `test_suite` rule.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:95 TargetUtils.isTestSuiteRule
pub fn is_test_suite_rule(target: &Target) -> bool {
    match target {
        Target::Rule(rule) => is_test_suite_rule_name(rule.get_rule_class()),
        _ => false,
    }
}

/// True iff `target` is a `*_test` or `test_suite`.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:102 TargetUtils.isTestOrTestSuiteRule
pub fn is_test_or_test_suite_rule(target: &Target) -> bool {
    is_test_rule(target) || is_test_suite_rule(target)
}

/// True if `target` has "manual" in the tags attribute.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:110 TargetUtils.hasManualTag
pub fn has_manual_tag(target: &Target) -> bool {
    match target {
        Target::Rule(rule) => has_constraint(rule, "manual"),
        _ => false,
    }
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:122 TargetUtils.isExclusiveTestRule
pub fn is_exclusive_test_rule(rule: &Rule) -> bool {
    has_constraint(rule, "exclusive")
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:133 TargetUtils.isExclusiveIfLocalTestRule
pub fn is_exclusive_if_local_test_rule(rule: &Rule) -> bool {
    has_constraint(rule, "exclusive-if-local")
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:145 TargetUtils.isLocalTestRule
pub fn is_local_test_rule(rule: &Rule) -> bool {
    has_constraint(rule, "local")
        || NonconfigurableAttributeMapper::of(rule)
            .get("local", Type::Boolean)
            .and_then(|v| v.as_bool())
            .expect("NullPointerException: local")
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:158 TargetUtils.isExternalTestRule
pub fn is_external_test_rule(rule: &Rule) -> bool {
    has_constraint(rule, "external")
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:168 TargetUtils.isNoTestloasdTestRule
pub fn is_no_testloasd_test_rule(rule: &Rule) -> bool {
    has_constraint(rule, "no-testloasd")
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:172 TargetUtils.getStringListAttr
pub fn get_string_list_attr(target: &Target, attr_name: &str) -> Vec<String> {
    let Target::Rule(rule) = target else {
        panic!("IllegalArgumentException");
    };
    match NonconfigurableAttributeMapper::of(&**rule).get(attr_name, Type::StringList) {
        Some(AttrValue::StringList(list)) => list,
        other => panic!("ClassCastException: {attr_name} value {other:?}"),
    }
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:177 TargetUtils.getStringAttr
pub fn get_string_attr(target: &Target, attr_name: &str) -> Option<String> {
    let Target::Rule(rule) = target else {
        panic!("IllegalArgumentException");
    };
    NonconfigurableAttributeMapper::of(&**rule)
        .get(attr_name, Type::String)
        .map(|v| match v {
            AttrValue::String(s) => s,
            other => panic!("ClassCastException: {attr_name} value {other:?}"),
        })
}

/// The possible values of the attribute as strings (`None` is Java null).
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:182 TargetUtils.getAttrAsString
pub fn get_attr_as_string(target: &Target, attr_name: &str) -> Vec<Option<String>> {
    let Target::Rule(rule) = target else {
        panic!("IllegalArgumentException");
    };
    let mut values = Vec::new(); // May hold null values.
    if let Some(attribute) = rule.get_attribute_definition(attr_name) {
        let attribute_type = attribute.get_type();
        for attr_value in AggregatingAttributeMapper::of(&**rule)
            .visit_attribute(attribute.get_name(), attribute_type)
        {
            values.push(convert_attribute_value(attribute_type, attr_value.as_ref()));
        }
    }
    values
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:199 TargetUtils.convertAttributeValue
pub fn convert_attribute_value(
    attribute_type: Type,
    attr_value: Option<&AttrValue>,
) -> Option<String> {
    // Ugly hack to maintain backward 'attr' query compatibility for BOOLEAN and TRISTATE
    // attributes. These are internally stored as actual Boolean or TriState objects but were
    // historically queried as integers. To maintain compatibility, we inspect their actual
    // value and return the integer equivalent represented as a String. This code is the
    // opposite of the code in BooleanType and TriStateType respectively.
    if attribute_type == Type::Boolean {
        let value = attr_value
            .and_then(AttrValue::as_bool)
            .expect("ClassCastException: boolean");
        Some(if value { "1" } else { "0" }.to_owned())
    } else if attribute_type == Type::Tristate {
        let value = attr_value
            .and_then(AttrValue::as_tri_state)
            .expect("ClassCastException: tristate");
        Some(
            match value {
                TriState::Auto => "-1",
                TriState::No => "0",
                TriState::Yes => "1",
            }
            .to_owned(),
        )
    } else {
        attr_value.map(java_to_string)
    }
}

/// Java `Object.toString()` of a converted attribute value (Java collections print as
/// `[a, b]` and `{k=v}`).
fn java_to_string(value: &AttrValue) -> String {
    fn list<T: std::fmt::Display>(items: impl Iterator<Item = T>) -> String {
        let items: Vec<String> = items.map(|i| i.to_string()).collect();
        format!("[{}]", items.join(", "))
    }
    fn dict<K: std::fmt::Display, V>(
        items: impl Iterator<Item = (K, V)>,
        value: impl Fn(V) -> String,
    ) -> String {
        let items: Vec<String> = items.map(|(k, v)| format!("{}={}", k, value(v))).collect();
        format!("{{{}}}", items.join(", "))
    }
    match value {
        AttrValue::Int(i) => i.to_string(),
        AttrValue::String(s) => s.clone(),
        AttrValue::Bool(b) => b.to_string(),
        AttrValue::Label(l) => l.to_string(),
        AttrValue::StringList(l) => list(l.iter()),
        AttrValue::StringSet(l) => list(l.iter()),
        AttrValue::IntList(l) => list(l.iter()),
        AttrValue::LabelList(l) => list(l.iter()),
        AttrValue::StringDict(d) => dict(d.iter(), |v| v.clone()),
        AttrValue::StringListDict(d) => dict(d.iter(), |v| list(v.iter())),
        AttrValue::LabelDictUnary(d) => dict(d.iter(), |v| v.to_string()),
        AttrValue::LabelKeyedStringDict(d) => dict(d.iter(), |v| v.clone()),
        AttrValue::LabelListDict(d) => dict(d.iter(), |v| list(v.iter())),
        AttrValue::License(l) => l.to_string(),
        AttrValue::TriState(_)
        | AttrValue::SelectorList(_)
        | AttrValue::ComputedDefault(_)
        | AttrValue::StarlarkComputedDefaultTemplate(_)
        | AttrValue::LateBound(_)
        | AttrValue::Materializer(_) => {
            blaze_util::unported!("TargetUtils.convertAttributeValue toString", "{value:?}")
        }
    }
}

/// The `deprecation` value of a rule, or `None` if unset or not a rule.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:222 TargetUtils.getDeprecation
pub fn get_deprecation(target: &Target) -> Option<String> {
    let Target::Rule(rule) = target else {
        return None;
    };
    if rule.is_attr_defined("deprecation", Type::String) {
        NonconfigurableAttributeMapper::of(&**rule)
            .get("deprecation", Type::String)
            .and_then(|v| v.as_str().map(str::to_owned))
    } else {
        None
    }
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:239 TargetUtils.hasConstraint
fn has_constraint(rule: &Rule, keyword: &str) -> bool {
    match NonconfigurableAttributeMapper::of(rule).get(CONSTRAINTS_ATTR, Type::StringList) {
        Some(AttrValue::StringList(tags)) => tags.iter().any(|t| t == keyword),
        other => panic!("ClassCastException: tags value {other:?}"),
    }
}

/// The execution info from the rule's tags (legal keys only, empty values).
// PORT: Java returns ImmutableMap.copyOf(HashMap): hash order. Sorted here; callers in Java sort
// (getFilteredExecutionInfo) or use it as a map.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:249 TargetUtils.getExecutionInfo
pub fn get_execution_info(rule: &Rule) -> BTreeMap<String, String> {
    // tags may contain duplicate values.
    let mut map = BTreeMap::new();
    if let Some(AttrValue::StringList(tags)) =
        NonconfigurableAttributeMapper::of(rule).get(CONSTRAINTS_ATTR, Type::StringList)
    {
        for tag in tags {
            if legal_exec_info_keys(&tag) {
                map.insert(tag, String::new());
            }
        }
    }
    map
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:270 TargetUtils.getExecutionInfo
pub fn get_execution_info_with_propagation(
    rule: &Rule,
    allow_tags_propagation: bool,
) -> BTreeMap<String, String> {
    if allow_tags_propagation {
        get_execution_info(rule)
    } else {
        BTreeMap::new()
    }
}

/// The language part of a rule class name (e.g. "foo" for foo_test).
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:330 TargetUtils.getRuleLanguage
pub fn get_rule_language(rule_class: &str) -> &str {
    // Chop off "_binary" or "_test".
    match rule_class.rfind('_') {
        Some(index) => &rule_class[..index],
        None => rule_class,
    }
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:336 TargetUtils.isExplicitDependency
fn is_explicit_dependency(rule: &Rule, label: &Label) -> bool {
    if rule.get_visibility_dependency_labels().contains(label) {
        return true;
    }
    let mapper = AggregatingAttributeMapper::of(rule);
    let mut found = false;
    // PORT: Java stops the visitation with an exception; the rest is visited without effect.
    mapper.visit_labels_with_filter(
        &DependencyFilter::no_implicit_deps(),
        &mut |_, dep_label| {
            if label == dep_label {
                found = true;
            }
        },
    );
    found
}

/// A test tag filter: accepts tests matching all required tags and none of the excluded ones.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:363 TargetUtils.tagFilter
pub fn tag_filter(tag_filter_list: &[String]) -> impl Fn(&Target) -> bool + use<> {
    let (required_tags, excluded_tags) = sort_tags_by_sense(tag_filter_list);
    move |input: &Target| {
        if required_tags.is_empty() && excluded_tags.is_empty() {
            return true;
        }
        let Target::Rule(rule) = input else {
            return required_tags.is_empty();
        };
        // Note that test_tags are those originating from the XX_test rule, whereas the
        // requiredTags and excludedTags originate from the command line or test_suite rule.
        let tags: HashSet<String> = rule.get_rule_tags().into_iter().collect();
        test_matches_filters(&tags, &required_tags, &excluded_tags, false)
    }
}

/// The location of a rule or input file target, else `None`.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:384 TargetUtils.getLocationMaybe
pub fn get_location_maybe(target: &Target) -> Option<Location> {
    match target {
        Target::Rule(_) | Target::InputFile(_) => Some(target.get_location()),
        _ => None,
    }
}

/// The message that `label`, referenced from `target`, does not exist because of `e`.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:392 TargetUtils.formatMissingEdge
pub fn format_missing_edge_with_attribute(
    target: Option<&Target>,
    label: &Label,
    e: &NoSuchThingException,
    attr: Option<&Attribute>,
) -> String {
    match target {
        Some(Target::Rule(rule)) => {
            if is_explicit_dependency(rule, label) {
                format!("{} and referenced by '{}'", e, rule.get_label())
            } else {
                let mut additional_info = String::new();
                if let Some(attr) = attr
                    && let Some(doc) = attr.get_doc().filter(|d| !d.is_empty())
                {
                    additional_info = format!(
                        "\nDocumentation for implicit attribute {} of rules of type {}:\n{}",
                        attr.get_public_name(),
                        rule.get_rule_class(),
                        doc
                    );
                }
                // N.B. If you see this error message in one of our integration tests during
                // development of a change that adds a new implicit dependency when running Blaze,
                // maybe you forgot to add a new mock target to the integration test's setup.
                format!(
                    "every rule of type {} implicitly depends upon the target '{}', but this \
                     target could not be found because of: {}{}",
                    rule.get_rule_class(),
                    label,
                    e,
                    additional_info
                )
            }
        }
        Some(Target::InputFile(_)) => format!(
            "{e} (this is usually caused by a missing package group in the package-level \
             visibility declaration)"
        ),
        Some(target) => format!(
            "in target '{}', no such label '{}': {}",
            target.get_label(),
            label,
            e
        ),
        None => e.to_string(),
    }
}

// Java: com/google/devtools/build/lib/packages/TargetUtils.java:430 TargetUtils.formatMissingEdge
pub fn format_missing_edge(
    target: Option<&Target>,
    label: &Label,
    e: &NoSuchThingException,
) -> String {
    format_missing_edge_with_attribute(target, label, e, None)
}

/// Java `TestTargetUtils.testMatchesFilters(testTags, requiredTags, excludedTags,
/// mustMatchAllPositive)`.
// Java: com/google/devtools/build/lib/packages/TestTargetUtils.java:33 TestTargetUtils.testMatchesFilters
pub fn test_matches_filters(
    test_tags: &HashSet<String>,
    required_tags: &HashSet<String>,
    excluded_tags: &HashSet<String>,
    must_match_all_positive: bool,
) -> bool {
    for tag in excluded_tags {
        if test_tags.contains(tag) {
            return false;
        }
    }

    // Check required tags, if there are any.
    if required_tags.is_empty() {
        true
    } else if must_match_all_positive {
        // Require all tags to be present.
        for tag in required_tags {
            if !test_tags.contains(tag) {
                return false;
            }
        }
        true
    } else {
        // Require at least one positive tag. If the two collections are not disjoint, then they
        // have at least one element in common.
        !required_tags.is_disjoint(test_tags)
    }
}

/// Splits tags into required (positive) and excluded (negative) tags; `manual` is ignored.
// Java: com/google/devtools/build/lib/packages/TestTargetUtils.java:121 TestTargetUtils.sortTagsBySense
pub fn sort_tags_by_sense<S: AsRef<str>>(tag_list: &[S]) -> (HashSet<String>, HashSet<String>) {
    let mut required_tags = HashSet::new();
    let mut excluded_tags = HashSet::new();

    for tag in tag_list {
        let tag = tag.as_ref();
        if let Some(t) = tag.strip_prefix('-') {
            excluded_tags.insert(t.to_owned());
        } else if let Some(t) = tag.strip_prefix('+') {
            required_tags.insert(t.to_owned());
        } else if tag != "manual" {
            // Ignore manual attribute because it is an exception: it is not a filter but a
            // property of test_suite.
            required_tags.insert(tag.to_owned());
        }
    }
    (required_tags, excluded_tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/TestTargetUtilsTest.java:193 TestTargetUtilsTest.testSortTagsBySenseSeparatesTagsNaively
    #[test]
    fn test_sort_tags_by_sense_separates_tags_naively() {
        // Contrived, but intentional.
        let (required, excluded) =
            sort_tags_by_sense(&["tag1", "tag2", "tag3", "-tag1", "+tag2", "-tag3"]);
        assert_eq!(required, set(&["tag1", "tag2", "tag3"]));
        assert_eq!(excluded, set(&["tag1", "tag3"]));
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/TargetUtilsTest.java:35 TargetUtilsTest.getRuleLanguage
    #[test]
    fn get_rule_language() {
        assert_eq!(super::get_rule_language("java_binary"), "java");
        assert_eq!(super::get_rule_language("foobar"), "foobar");
        assert_eq!(super::get_rule_language(""), "");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/TargetUtilsTest.java:42 TargetUtilsTest.testFilterByTag
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase)"]
    fn test_filter_by_tag() {
        unimplemented!("scratch BUILD with tagged sh_binary targets; tagFilter combinations")
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/TargetUtilsTest.java:100 TargetUtilsTest.testExecutionInfo
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase)"]
    fn test_execution_info() {
        unimplemented!("scratch BUILD; getExecutionInfo of tagged rules")
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/TestTargetUtilsTest.java:105 TestTargetUtilsTest.testFilterBySize
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase)"]
    fn test_filter_by_size() {
        unimplemented!("scratch BUILD with tests of each size; test_suite filtering")
    }
}
