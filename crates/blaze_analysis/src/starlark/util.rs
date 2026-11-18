//! Small helpers shared by the ports in `crate::starlark`: Java's `Starlark.errorf`,
//! `Sequence.cast`, `Depset.cast`, `Dict.cast` and `BuiltinRestriction` checks as used by the
//! analysis-phase Starlark API.

use std::collections::BTreeMap;

use blaze_actions::Artifact;
use blaze_actions::analysis::starlark::starlark_file::artifact_from_value;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_starlark::depset::Depset;
use indexmap::IndexMap;
use starlark::collections::SmallMap;
use starlark::eval::Evaluator;
use starlark::values::dict::{Dict, DictRef};
use starlark::values::list::ListRef;
use starlark::values::tuple::TupleRef;
use starlark::values::{Heap, Value};

/// Java `Starlark.errorf(...)` / `new EvalException(msg)`.
pub(crate) fn errorf(msg: impl Into<String>) -> starlark::Error {
    let msg: String = msg.into();
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// The interpreter's error for an argument whose class is not among the parameter's
/// `@Param(allowedTypes = ...)` (or, without `allowedTypes`, the Java parameter class).
/// `type_error_message` is Java `ParamDescriptor.getTypeErrorMessage()`: "a", "a or b",
/// "a, b, or c".
// Java: net/starlark/java/eval/BuiltinFunction.java:405 BuiltinFunction.checkParamValue
pub(crate) fn param_type_error(
    method_name: &str,
    param_name: &str,
    value: Value,
    type_error_message: &str,
) -> starlark::Error {
    errorf(format!(
        "in call to {method_name}(), parameter '{param_name}' got value of type '{}', want \
         '{type_error_message}'",
        value.get_type()
    ))
}

/// Java `x instanceof Boolean` for a parameter declared `Boolean` (no `allowedTypes`).
pub(crate) fn check_bool_param(
    method_name: &str,
    param_name: &str,
    value: Value,
) -> starlark::Result<bool> {
    match value.unpack_bool() {
        Some(b) => Ok(b),
        None => Err(param_type_error(method_name, param_name, value, "bool")),
    }
}

/// Java `StarlarkList.immutableCopyOf(elems)`: an immutable list of `elems` (the shared empty
/// list when there are none). Mutating it fails with "trying to mutate a frozen list value".
// Java: net/starlark/java/eval/StarlarkList.java:168 StarlarkList.immutableCopyOf
pub(crate) fn alloc_immutable_list<'v>(heap: Heap<'v>, elems: Vec<Value<'v>>) -> Value<'v> {
    starlark::values::list::alloc_immutable_list(heap, &elems)
}

/// Java `Dict.immutableCopyOf(map)` of a map filled with `entries` in order: a repeated key
/// keeps its first position and its last value. Mutating the result fails with "trying to
/// mutate a frozen dict value". Keys must be hashable (strings, `None` and `Target`s here).
// Java: net/starlark/java/eval/Dict.java:495 Dict.immutableCopyOf
pub(crate) fn alloc_immutable_dict<'v>(
    heap: Heap<'v>,
    entries: Vec<(Value<'v>, Value<'v>)>,
) -> Value<'v> {
    let mut content: SmallMap<Value<'v>, Value<'v>> = SmallMap::with_capacity(entries.len());
    for (key, value) in entries {
        let key = key
            .get_hashed()
            .unwrap_or_else(|e| panic!("IllegalArgumentException: unhashable dict key: {e}"));
        content.insert_hashed(key, value);
    }
    starlark::values::dict::alloc_immutable_dict(heap, Dict::new(content))
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

    // Expected texts: the oracle (Bazel 9.3.2) on scripts/w4/scenarios/ctx-errors, targets
    // runfiles_symlinks_list, runfiles_collect_default_none and run_env_list.
    #[test]
    fn param_type_error_matches_the_interpreter() {
        let list = starlark::values::FrozenValue::new_empty_list().to_value();
        assert_eq!(
            param_type_error("runfiles", "symlinks", list, "dict or depset").to_string(),
            "in call to runfiles(), parameter 'symlinks' got value of type 'list', want 'dict or \
             depset'"
        );
        assert_eq!(
            param_type_error("run", "env", list, "dict or NoneType").to_string(),
            "in call to run(), parameter 'env' got value of type 'list', want 'dict or NoneType'"
        );
    }

    #[test]
    fn check_bool_param_rejects_non_bool() {
        assert!(check_bool_param("runfiles", "collect_data", Value::new_bool(true)).unwrap());
        assert!(!check_bool_param("runfiles", "collect_data", Value::new_bool(false)).unwrap());
        assert_eq!(
            check_bool_param("runfiles", "collect_default", Value::new_none())
                .unwrap_err()
                .to_string(),
            "in call to runfiles(), parameter 'collect_default' got value of type 'NoneType', \
             want 'bool'"
        );
    }

    // Java: StarlarkList.immutableCopyOf / Dict.immutableCopyOf results reject mutation
    // ("trying to mutate a frozen dict value": the oracle on scripts/w4/scenarios/ctx-immutable,
    // target var_set).
    #[test]
    fn immutable_copies_are_immutable() {
        starlark::environment::Module::with_temp_heap(|module| {
            let heap = module.heap();
            let list = alloc_immutable_list(heap, vec![Value::new_none()]);
            assert_eq!(starlark::values::list::list_is_immutable(list), Some(true));
            assert_eq!(list.length().unwrap(), 1);
            let empty = alloc_immutable_list(heap, Vec::new());
            assert_eq!(starlark::values::list::list_is_immutable(empty), Some(true));

            let key = heap.alloc_str("k").to_value();
            let dict = alloc_immutable_dict(heap, vec![(key, Value::new_none())]);
            assert_eq!(starlark::values::dict::dict_is_immutable(dict), Some(true));
            let error = dict.set_at(key, Value::new_bool(true)).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("trying to mutate a frozen dict value"),
                "{error}"
            );
            let empty = alloc_immutable_dict(heap, Vec::new());
            assert_eq!(starlark::values::dict::dict_is_immutable(empty), Some(true));
        });
    }
}
