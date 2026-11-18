//! Small helpers shared by the ports in `crate::starlark`: Java's `Starlark.errorf`,
//! `Sequence.cast`, `Depset.cast`, `Dict.cast` and `BuiltinRestriction` checks as used by the
//! analysis-phase Starlark API.

use std::collections::BTreeMap;

use blaze_actions::Artifact;
use blaze_actions::analysis::starlark::starlark_file::artifact_from_value;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_starlark::depset::Depset;
use indexmap::IndexMap;
use starlark::eval::Evaluator;
use starlark::values::Value;
use starlark::values::dict::DictRef;
use starlark::values::list::ListRef;
use starlark::values::tuple::TupleRef;

/// Java `Starlark.errorf(...)` / `new EvalException(msg)`.
pub(crate) fn errorf(msg: impl Into<String>) -> starlark::Error {
    let msg: String = msg.into();
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// Java `x instanceof Sequence` (list, tuple, range).
pub(crate) fn is_sequence(v: Value) -> bool {
    matches!(v.get_type(), "list" | "tuple" | "range")
}

/// Java `x instanceof Iterable` for Starlark values (`StarlarkIterable`: list, tuple, range,
/// dict, set).
pub(crate) fn is_iterable(v: Value) -> bool {
    matches!(v.get_type(), "list" | "tuple" | "range" | "dict" | "set")
}

/// The elements of a list, tuple or range (Java `Sequence` iteration).
pub(crate) fn sequence_elements<'v>(
    v: Value<'v>,
    eval: &Evaluator<'v, '_, '_>,
) -> starlark::Result<Vec<Value<'v>>> {
    if let Some(l) = ListRef::from_value(v) {
        return Ok(l.content().to_vec());
    }
    if let Some(t) = TupleRef::from_value(v) {
        return Ok(t.content().to_vec());
    }
    Ok(v.iterate(eval.heap())?.collect())
}

/// `Sequence.cast(x, Artifact.class, what)`.
// Java: net/starlark/java/eval/Sequence.java:132 Sequence.cast
pub(crate) fn sequence_cast_artifacts<'v>(
    x: Value<'v>,
    what: &str,
    eval: &Evaluator<'v, '_, '_>,
) -> starlark::Result<Vec<Artifact>> {
    if !is_sequence(x) {
        return Err(errorf(format!(
            "for {what}, got {}, want sequence",
            x.get_type()
        )));
    }
    let mut result = Vec::new();
    for (i, elem) in sequence_elements(x, eval)?.into_iter().enumerate() {
        match artifact_from_value(elem) {
            Some(a) => result.push(a),
            None => {
                return Err(errorf(format!(
                    "at index {i} of {what}, got element of type {}, want File",
                    elem.get_type()
                )));
            }
        }
    }
    Ok(result)
}

/// `Sequence.cast(x, String.class, what)`.
// Java: net/starlark/java/eval/Sequence.java:132 Sequence.cast
pub(crate) fn sequence_cast_strings<'v>(
    x: Value<'v>,
    what: &str,
    eval: &Evaluator<'v, '_, '_>,
) -> starlark::Result<Vec<String>> {
    if !is_sequence(x) {
        return Err(errorf(format!(
            "for {what}, got {}, want sequence",
            x.get_type()
        )));
    }
    let mut result = Vec::new();
    for (i, elem) in sequence_elements(x, eval)?.into_iter().enumerate() {
        match elem.unpack_str() {
            Some(s) => result.push(s.to_owned()),
            None => {
                return Err(errorf(format!(
                    "at index {i} of {what}, got element of type {}, want string",
                    elem.get_type()
                )));
            }
        }
    }
    Ok(result)
}

/// `Depset.cast(x, Artifact.class, what)`: the depset's `NestedSet<Artifact>` with the same
/// structure (so the same `toList` order).
// Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:262 Depset.cast
pub(crate) fn depset_cast_artifacts(x: Value, what: &str) -> starlark::Result<NestedSet<Artifact>> {
    if Depset::from_value(x).is_none() {
        return Err(errorf(format!(
            "for {what}, got {}, want a depset of File",
            x.get_type()
        )));
    }
    depset_get_set_artifacts(x).map_err(|e| errorf(format!("for '{what}', {e}")))
}

/// `Depset.getSet(Artifact.class)`; `Err` is the `TypeException` message.
// Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:207 Depset.getSet
pub(crate) fn depset_get_set_artifacts(x: Value) -> Result<NestedSet<Artifact>, String> {
    let d = Depset::from_value(x).expect("depset");
    if !d.is_empty() && d.element_type() != "File" {
        return Err(format!(
            "got a depset of '{}', expected a depset of 'File'",
            d.element_type()
        ));
    }
    d.to_nested_set(&mut |v: Value| -> starlark::Result<Artifact> {
        artifact_from_value(v).ok_or_else(|| errorf("depset element is not a File"))
    })
    .map_err(|e| e.to_string())
}

/// `Depset.cast(x, Object.class, what).toList()` for the elements of a depset of any type.
pub(crate) fn depset_to_list<'v>(x: Value<'v>) -> starlark::Result<Vec<Value<'v>>> {
    let d = Depset::from_value(x).expect("depset");
    d.to_list()
}

/// A `NestedSet<Artifact>` of `artifacts` in stable order (Java
/// `NestedSetBuilder.<Artifact>stableOrder().addAll(...).build()`).
pub(crate) fn stable_order_set(artifacts: Vec<Artifact>) -> NestedSet<Artifact> {
    let mut b = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
    b.add_all(artifacts);
    b.build()
}

/// `Dict.cast(x, String.class, String.class, what)`, keeping the dict's order.
// Java: net/starlark/java/eval/Dict.java Dict.cast
pub(crate) fn dict_cast_string_string(
    x: Value,
    what: &str,
) -> starlark::Result<IndexMap<String, String>> {
    let Some(d) = DictRef::from_value(x) else {
        return Err(errorf(format!(
            "for {what}, got {}, want dict",
            x.get_type()
        )));
    };
    let mut result = IndexMap::new();
    for (k, v) in d.iter() {
        let Some(ks) = k.unpack_str() else {
            return Err(errorf(format!(
                "got dict<{}, {}> for '{what}', want dict<string, string>",
                k.get_type(),
                v.get_type()
            )));
        };
        let Some(vs) = v.unpack_str() else {
            return Err(errorf(format!(
                "got dict<{}, {}> for '{what}', want dict<string, string>",
                k.get_type(),
                v.get_type()
            )));
        };
        result.insert(ks.to_owned(), vs.to_owned());
    }
    Ok(result)
}

/// `Dict.noneableCast(x, String.class, String.class, what)`.
// Java: net/starlark/java/eval/Dict.java Dict.noneableCast
pub(crate) fn dict_noneable_cast_string_string(
    x: Value,
    what: &str,
) -> starlark::Result<IndexMap<String, String>> {
    if x.is_none() {
        return Ok(IndexMap::new());
    }
    dict_cast_string_string(x, what)
}

/// `Dict.cast(x, String.class, Object.class, what)`.
// Java: net/starlark/java/eval/Dict.java Dict.cast
pub(crate) fn dict_cast_string_object<'v>(
    x: Value<'v>,
    what: &str,
) -> starlark::Result<IndexMap<String, Value<'v>>> {
    let Some(d) = DictRef::from_value(x) else {
        return Err(errorf(format!(
            "for {what}, got {}, want dict",
            x.get_type()
        )));
    };
    let mut result = IndexMap::new();
    for (k, v) in d.iter() {
        let Some(ks) = k.unpack_str() else {
            return Err(errorf(format!(
                "got dict<{}, {}> for '{what}', want dict<string, any>",
                k.get_type(),
                v.get_type()
            )));
        };
        result.insert(ks.to_owned(), v);
    }
    Ok(result)
}

/// Java `TargetUtils.legalExecInfoKeys`.
// PORT: move to blaze_packages target/target_utils.rs (private there).
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:58 TargetUtils.legalExecInfoKeys
fn legal_exec_info_keys(tag: &str) -> bool {
    tag.starts_with("block-")
        || tag.starts_with("requires-")
        || tag.starts_with("no-")
        || tag.starts_with("supports-")
        || tag.starts_with("disable-")
        || tag.starts_with("cpu:")
        || tag == blaze_actions::actions::execution_requirements::LOCAL
        || tag == blaze_actions::actions::execution_requirements::WORKER_KEY_MNEMONIC
        || tag.starts_with("resources:")
}

/// `TargetUtils.getFilteredExecutionInfo(executionRequirementsUnchecked, rule,
/// allowTagsPropagation)`: the `ImmutableSortedMap` of the legal execution requirements, plus
/// the rule's tags when tags propagation is allowed.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:285 TargetUtils.getFilteredExecutionInfo
pub(crate) fn get_filtered_execution_info(
    execution_requirements_unchecked: Option<Value>,
    rule: &blaze_packages::target::Rule,
    allow_tags_propagation: bool,
) -> starlark::Result<BTreeMap<String, String>> {
    let mut execution_info: BTreeMap<String, String> = match execution_requirements_unchecked {
        None => BTreeMap::new(),
        Some(x) => dict_noneable_cast_string_string(x, "execution_requirements")?
            .into_iter()
            .filter(|(k, _)| legal_exec_info_keys(k))
            .collect(),
    };
    if allow_tags_propagation {
        let checked_tags = blaze_packages::target::target_utils::get_execution_info(rule);
        // merging filtered tags to the execution info map avoiding duplicates
        for (k, v) in checked_tags {
            execution_info.entry(k).or_insert(v);
        }
    }
    Ok(execution_info)
}

/// Java `BuiltinRestriction.INTERNAL_STARLARK_API_ALLOWLIST` as `(apparent repo name, package
/// prefix)` entries.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:36 BuiltinRestriction.INTERNAL_STARLARK_API_ALLOWLIST
pub(crate) const INTERNAL_STARLARK_API_ALLOWLIST: &[(&str, &str)] = &[
    // Testing
    ("", "test"),
    ("", "bazel_internal/test_rules"),
    // BuildInfo
    ("", "tools/build_defs/build_info"),
    ("bazel_tools", "tools/build_defs/build_info"),
    // Android rules
    ("", "bazel_internal/test_rules/cc"),
    ("", "tools/build_defs/android"),
    ("", "third_party/bazel_rules/rules_android"),
    ("rules_android", ""),
    ("build_bazel_rules_android", ""),
    // Apple rules
    ("", "third_party/apple_crosstool"),
    ("", "third_party/cpptoolchains/portable_llvm/build_defs"),
    ("", "third_party/bazel_rules/rules_apple"),
    ("rules_apple", ""),
    ("build_bazel_rules_apple", ""),
    // Cc rules
    ("", "third_party/bazel_rules/rules_cc"),
    ("", "tools/build_defs/cc"),
    ("rules_cc", ""),
    // Java rules
    ("", "third_party/bazel_rules/rules_java/java"),
    ("rules_java", "java"),
    // Rust rules
    ("", "third_party/bazel_rules/rules_rust/rust/private"),
    ("", "third_party/crubit"),
    ("rules_rust", "rust/private"),
    // CUDA rules
    ("", "third_party/gpus/cuda"),
    // Packaging rules
    ("", "tools/build_defs/packaging"),
    // Go rules
    ("", "tools/build_defs/go"),
    // Proto rules
    ("", "third_party/protobuf"),
    ("protobuf", ""),
    ("com_google_protobuf", ""),
    // Shell rules
    ("rules_shell", ""),
];

/// `BuiltinRestriction.failIfCalledOutsideDefaultAllowlist(thread)`.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:153 BuiltinRestriction.failIfCalledOutsideDefaultAllowlist
pub(crate) fn fail_if_called_outside_default_allowlist(eval: &Evaluator) -> starlark::Result<()> {
    blaze_packages::starlark_defs::rule_class_functions::fail_if_called_outside_allowlist(
        eval,
        INTERNAL_STARLARK_API_ALLOWLIST,
    )
}

/// Java `Starlark.truth(x)`.
pub(crate) fn truth(x: Value) -> bool {
    x.to_bool()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legal_exec_info_keys_matches_java() {
        assert!(legal_exec_info_keys("no-sandbox"));
        assert!(legal_exec_info_keys("requires-network"));
        assert!(legal_exec_info_keys("supports-workers"));
        assert!(legal_exec_info_keys("local"));
        assert!(legal_exec_info_keys("worker-key-mnemonic"));
        assert!(legal_exec_info_keys("cpu:4"));
        assert!(legal_exec_info_keys("resources:gpu:1"));
        assert!(legal_exec_info_keys("block-network"));
        assert!(legal_exec_info_keys("disable-foo"));
        assert!(!legal_exec_info_keys("manual"));
        assert!(!legal_exec_info_keys("exclusive"));
        assert!(!legal_exec_info_keys("flaky"));
    }
}
