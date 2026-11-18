//! Starlark casting helpers for the C++ natives: Java's `Starlark.errorf`, `Sequence.cast`,
//! `Depset.cast`, `Dict.cast`, `CcModule.nullIfNone`, `Structure.getValue(name, type)` and the
//! `BuiltinRestriction` / `CcModule` access checks, with Java's error texts.
//!
//! PORT: `blaze_analysis::starlark::util` has crate-private copies of some of these; the C++
//! natives live in another crate, so they are ported here again from the same Java.

use std::collections::BTreeSet;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::analysis::starlark::starlark_file::artifact_from_value;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_collect::NestedSet;
use blaze_packages::bzl_context::BazelModuleContext;
use blaze_starlark::depset::Depset;
use blaze_starlark::structs::StarlarkInfo;
use indexmap::IndexMap;
use starlark::eval::Evaluator;
use starlark::values::ValueLike;
use starlark::values::dict::DictRef;
use starlark::values::list::ListRef;
use starlark::values::tuple::TupleRef;
use starlark::values::{Heap, Value};

/// Java `Starlark.errorf(...)` / `new EvalException(msg)`.
pub fn errorf(msg: impl Into<String>) -> starlark::Error {
    let msg: String = msg.into();
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// Java `Starlark.type(x)` as Blaze spells Starlark type names.
pub fn starlark_type(x: Value) -> String {
    x.get_type().to_owned()
}

/// Java `x instanceof Sequence` (list, tuple, range).
pub fn is_sequence(v: Value) -> bool {
    matches!(v.get_type(), "list" | "tuple" | "range")
}

/// The elements of a list, tuple or range (Java `Sequence` iteration).
pub fn sequence_elements<'v>(v: Value<'v>, heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
    if let Some(l) = ListRef::from_value(v) {
        return Ok(l.content().to_vec());
    }
    if let Some(t) = TupleRef::from_value(v) {
        return Ok(t.content().to_vec());
    }
    Ok(v.iterate(heap)?.collect())
}

/// `Sequence.cast(x, Artifact.class, what)`.
// Java: net/starlark/java/eval/Sequence.java:132 Sequence.cast
pub fn sequence_cast_artifacts<'v>(
    x: Value<'v>,
    what: &str,
    heap: Heap<'v>,
) -> starlark::Result<Vec<Artifact>> {
    if !is_sequence(x) {
        return Err(errorf(format!(
            "for {what}, got {}, want sequence",
            x.get_type()
        )));
    }
    let mut result = Vec::new();
    for (i, elem) in sequence_elements(x, heap)?.into_iter().enumerate() {
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
pub fn sequence_cast_strings<'v>(
    x: Value<'v>,
    what: &str,
    heap: Heap<'v>,
) -> starlark::Result<Vec<String>> {
    if !is_sequence(x) {
        return Err(errorf(format!(
            "for {what}, got {}, want sequence",
            x.get_type()
        )));
    }
    let mut result = Vec::new();
    for (i, elem) in sequence_elements(x, heap)?.into_iter().enumerate() {
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
/// structure.
// Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:262 Depset.cast
pub fn depset_cast_artifacts(x: Value, what: &str) -> starlark::Result<NestedSet<Artifact>> {
    blaze_analysis::rule::depsets::depset_to_artifacts(x, what)
}

/// `Depset.noneableCast(x, Artifact.class, what)`: `None` for Starlark `None`.
// Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:275 Depset.noneableCast
pub fn depset_noneable_cast_artifacts(
    x: Value,
    what: &str,
) -> starlark::Result<Option<NestedSet<Artifact>>> {
    if x.is_none() {
        return Ok(None);
    }
    depset_cast_artifacts(x, what).map(Some)
}

/// `Depset.getSet(Artifact.class)` of a value known to be a depset, as Java's `TypeException`
/// message on failure.
// Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:207 Depset.getSet
pub fn depset_get_set_artifacts(x: Value) -> Result<NestedSet<Artifact>, String> {
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

/// `Depset.getSet(String.class)` of a value known to be a depset.
// Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:207 Depset.getSet
pub fn depset_get_set_strings(x: Value) -> Result<NestedSet<String>, String> {
    let d = Depset::from_value(x).expect("depset");
    if !d.is_empty() && d.element_type() != "string" {
        return Err(format!(
            "got a depset of '{}', expected a depset of 'string'",
            d.element_type()
        ));
    }
    d.to_nested_set(&mut |v: Value| -> starlark::Result<String> {
        v.unpack_str()
            .map(str::to_owned)
            .ok_or_else(|| errorf("depset element is not a string"))
    })
    .map_err(|e| e.to_string())
}

/// `Dict.cast(x, String.class, String.class, what)`, keeping the dict's order.
// Java: net/starlark/java/eval/Dict.java:659 Dict.cast
pub fn dict_cast_string_string(x: Value, what: &str) -> starlark::Result<IndexMap<String, String>> {
    let Some(dict) = DictRef::from_value(x) else {
        return Err(errorf(format!(
            "got {} for '{what}', want dict",
            x.get_type()
        )));
    };
    let mut result = IndexMap::new();
    for (k, v) in dict.iter() {
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

/// `Dict.cast(x, String.class, Object.class, what)`, keeping the dict's order.
// Java: net/starlark/java/eval/Dict.java:659 Dict.cast
pub fn dict_cast_string_object<'v>(
    x: Value<'v>,
    what: &str,
) -> starlark::Result<Vec<(String, Value<'v>)>> {
    let Some(dict) = DictRef::from_value(x) else {
        return Err(errorf(format!(
            "got {} for '{what}', want dict",
            x.get_type()
        )));
    };
    let mut result = Vec::new();
    for (k, v) in dict.iter() {
        let Some(ks) = k.unpack_str() else {
            return Err(errorf(format!(
                "got dict<{}, {}> for '{what}', want dict<string, Object>",
                k.get_type(),
                v.get_type()
            )));
        };
        result.push((ks.to_owned(), v));
    }
    Ok(result)
}

/// `CcModule.nullIfNone(object, Artifact.class)`: `None` for Starlark `None`, else the
/// artifact (Java's `Class.cast` throws `ClassCastException` for anything else).
// Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:185 CcModule.nullIfNone
pub fn null_if_none_artifact(x: Value) -> Option<Artifact> {
    if x.is_none() {
        return None;
    }
    match artifact_from_value(x) {
        Some(a) => Some(a),
        None => panic!(
            "ClassCastException: Cannot cast {} to com.google.devtools.build.lib.actions.Artifact",
            x.get_type()
        ),
    }
}

/// The `Artifact` of a `File` value, or Java's parameter type error.
pub fn artifact_param(x: Value, function: &str, param: &str) -> starlark::Result<Artifact> {
    artifact_from_value(x).ok_or_else(|| {
        errorf(format!(
            "in call to {function}(), parameter '{param}' got value of type '{}', want 'File'",
            x.get_type()
        ))
    })
}

/// A struct-like value (`StarlarkInfo`, Java `Structure`), or Java's parameter type error.
pub fn struct_param<'v>(
    x: Value<'v>,
    function: &str,
    param: &str,
    want: &str,
) -> starlark::Result<&'v StarlarkInfo<'v>> {
    StarlarkInfo::from_value(x).ok_or_else(|| {
        errorf(format!(
            "in call to {function}(), parameter '{param}' got value of type '{}', want '{want}'",
            x.get_type()
        ))
    })
}

/// The `BuildConfigurationValue` behind a `configuration` value (Java
/// `(BuildConfigurationValue) x`).
pub fn configuration_param(
    x: Value,
    function: &str,
    param: &str,
) -> starlark::Result<Arc<BuildConfigurationValue>> {
    x.downcast_ref::<blaze_analysis::starlark::starlark_configuration::StarlarkConfiguration>()
        .map(|c| c.0.clone())
        .ok_or_else(|| {
            errorf(format!(
                "in call to {function}(), parameter '{param}' got value of type '{}', want \
                 'configuration'",
                x.get_type()
            ))
        })
}

/// Java `Structure.getValue(name, type)`'s cast failure message.
// Java: com/google/devtools/build/lib/packages/StructImpl.java:49 StructImpl.getValue
pub fn field_type_error(field: &str, got: Value, want: &str) -> starlark::Error {
    errorf(format!(
        "for {field} field, got {}, want {want}",
        got.get_type()
    ))
}

// -------------------------------------------------------------------------------------------
// Access checks

/// Java `BuiltinRestriction.INTERNAL_STARLARK_API_ALLOWLIST` as `(apparent repo name, package
/// prefix)` entries.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:36 BuiltinRestriction.INTERNAL_STARLARK_API_ALLOWLIST
// PORT: same table as `blaze_analysis::starlark::util::INTERNAL_STARLARK_API_ALLOWLIST`
// (crate-private there).
const INTERNAL_STARLARK_API_ALLOWLIST: &[(&str, &str)] = &[
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

/// `CcModule.checkPrivateStarlarkificationAllowlist(thread)`, i.e.
/// `BuiltinRestriction.failIfCalledOutsideDefaultAllowlist(thread)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:189 CcModule.checkPrivateStarlarkificationAllowlist
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:181 BuiltinRestriction.failIfCalledOutsideDefaultAllowlist
pub fn check_private_starlarkification_allowlist(eval: &Evaluator) -> starlark::Result<()> {
    blaze_packages::starlark_defs::rule_class_functions::fail_if_called_outside_allowlist(
        eval,
        INTERNAL_STARLARK_API_ALLOWLIST,
    )
}

/// `CcModule.isCalledFromStarlarkCcCommon(thread)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:202 CcModule.isCalledFromStarlarkCcCommon
pub fn is_called_from_starlark_cc_common(eval: &Evaluator) -> starlark::Result<()> {
    // Java: BazelModuleContext.ofInnermostBzlOrThrow(thread): Preconditions.checkNotNull.
    let Some(module) = BazelModuleContext::of_innermost_bzl(eval) else {
        panic!("NullPointerException: BazelModuleContext.ofInnermostBzlOrThrow");
    };
    blaze_cc_features::cc_module::is_called_from_starlark_cc_common(
        &module.label().get_canonical_form(),
    )
    .map_err(|e| errorf(e.message))
}

/// Java `BuiltinRestriction.AllowlistEntry.allows`.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:124 BuiltinRestriction.AllowlistEntry.allows
// PORT: same logic as `blaze_packages::builtins::stubs::allowlist_entry_allows` (crate-private
// there), which `cc_internal.check_private_api` used while loading.
pub fn allowlist_entry_allows(
    apparent_repo_name: &str,
    package_prefix: &str,
    label: &blaze_cmdline::Label,
    repo_mapping: &blaze_cmdline::RepositoryMapping,
) -> bool {
    // Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:129 BuiltinRestriction.AllowlistEntry.reposMatch
    let given_name = label.get_repository();
    let repos_match = if given_name.is_main() {
        // The main repository may be one of the allowlisted rulesets, in which case we need to
        // fall back to interpreting allowedName as the apparent repo name.
        apparent_repo_name == blaze_cmdline::repository_name::MAIN.get_name()
            || repo_mapping.get(apparent_repo_name).is_main()
    } else if *given_name == *blaze_cmdline::repository_name::BAZEL_TOOLS {
        apparent_repo_name == blaze_cmdline::repository_name::BAZEL_TOOLS.get_name()
    } else {
        // allowedName is a module name and givenName is a real canonical repo name, so it
        // belongs to any version of that module if and only if it contains <allowedName>+ as a
        // prefix.
        given_name
            .get_name()
            .starts_with(&format!("{}+", apparent_repo_name))
    };
    repos_match
        && label
            .get_package_fragment()
            .starts_with(&blaze_vfs::PathFragment::create(package_prefix))
}

// -------------------------------------------------------------------------------------------
// Java HashMap iteration order

/// Java `String.hashCode()` (over UTF-16 code units).
// Java: java/lang/String.java String.hashCode
pub fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for unit in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(unit as i32);
    }
    h
}

/// The iteration order of a `java.util.HashMap<String, V>` (default capacity 16, load factor
/// 0.75) after inserting `keys` in order: buckets ascending, insertion order within a bucket
/// (resizes keep the relative order of entries that stay in one bucket).
// PORT: java-hash-order. Java: java/util/HashMap.java HashMap.hash, HashMap.resize
pub fn java_hash_map_order<K: AsRef<str> + Clone>(keys: &[K]) -> Vec<K> {
    let mut distinct: Vec<K> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for k in keys {
        if seen.insert(k.as_ref().to_owned()) {
            distinct.push(k.clone());
        }
    }
    let mut capacity: usize = 16;
    while distinct.len() as f64 > capacity as f64 * 0.75 {
        capacity *= 2;
    }
    let spread = |s: &str| -> u32 {
        let h = java_string_hash_code(s) as u32;
        h ^ (h >> 16)
    };
    let mut indexed: Vec<(usize, usize, K)> = distinct
        .into_iter()
        .enumerate()
        .map(|(i, k)| ((spread(k.as_ref()) as usize) & (capacity - 1), i, k))
        .collect();
    indexed.sort_by_key(|(bucket, i, _)| (*bucket, *i));
    indexed.into_iter().map(|(_, _, k)| k).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_string_hash_code_matches_java() {
        // "abc".hashCode() == 96354, "".hashCode() == 0.
        assert_eq!(java_string_hash_code("abc"), 96354);
        assert_eq!(java_string_hash_code(""), 0);
        // "no-remote".hashCode() (Java) == -724727342.
        assert_eq!(java_string_hash_code("no-remote"), -724727342);
    }

    #[test]
    fn java_hash_map_order_single_key() {
        assert_eq!(java_hash_map_order(&["local"]), vec!["local"]);
    }
}
