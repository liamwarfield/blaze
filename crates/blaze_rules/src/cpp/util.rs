//! Starlark casting helpers for the C++ natives: Java's `Starlark.errorf`, `Sequence.cast`,
//! `Depset.cast`, `Dict.cast`, `CcModule.nullIfNone`, `Structure.getValue(name, type)` and the
//! `BuiltinRestriction` / `CcModule` access checks, with Java's error texts.
//!
//! PORT: `blaze_analysis::starlark::util` has crate-private copies of some of these; the C++
//! natives live in another crate, so they are ported here again from the same Java.

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::analysis::starlark::starlark_file::artifact_from_value;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_collect::NestedSet;
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
    let module = blaze_analysis::rule::builtin_restriction::of_innermost_bzl_or_throw(eval);
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

/// The iteration order of a `java.util.HashMap<String, V>` created with `new HashMap<>()`
/// after `put`ting `keys` in order: a simulation of `HashMap.putVal` and `HashMap.resize` (table
/// of 16 buckets allocated on the first put, load factor 0.75, bucket index
/// `(n - 1) & (h ^ (h >>> 16))`, new entries appended to their bucket's chain, a put of a present
/// key changing nothing, a resize splitting each chain into its "lo" and "hi" halves in order,
/// and the resize `treeifyBin` does instead of treeifying while the table is smaller than 64).
/// Iteration walks the buckets in ascending order and each chain in order.
///
/// A chain of more than 8 entries in a table of 64 or more buckets becomes a red-black tree
/// (`TreeNode`), whose iteration order is not ported: `unported!`.
// PORT: java-hash-order.
// Java: java/util/HashMap.java HashMap.hash, HashMap.putVal, HashMap.resize, HashMap.treeifyBin
pub fn java_hash_map_order<K: AsRef<str> + Clone>(keys: &[K]) -> Vec<K> {
    // Java: java/util/HashMap.java HashMap.TREEIFY_THRESHOLD
    const TREEIFY_THRESHOLD: usize = 8;
    // Java: java/util/HashMap.java HashMap.MIN_TREEIFY_CAPACITY
    const MIN_TREEIFY_CAPACITY: usize = 64;
    // Java: java/util/HashMap.java HashMap.DEFAULT_INITIAL_CAPACITY
    const DEFAULT_INITIAL_CAPACITY: usize = 16;

    // Java: java/util/HashMap.java HashMap.hash
    fn hash(s: &str) -> u32 {
        let h = java_string_hash_code(s) as u32;
        h ^ (h >> 16)
    }

    // Java: java/util/HashMap.java HashMap.resize
    fn resize<K>(table: &mut Vec<Vec<(u32, K)>>, threshold: &mut usize) {
        let old_cap = table.len();
        if old_cap == 0 {
            // zero initial threshold signifies using defaults
            *threshold = DEFAULT_INITIAL_CAPACITY * 3 / 4;
            table.resize_with(DEFAULT_INITIAL_CAPACITY, Vec::new);
            return;
        }
        let new_cap = old_cap << 1;
        // double threshold
        *threshold <<= 1;
        let old_tab = std::mem::take(table);
        table.resize_with(new_cap, Vec::new);
        for (j, bin) in old_tab.into_iter().enumerate() {
            // preserve order
            for (h, k) in bin {
                if (h as usize) & old_cap == 0 {
                    table[j].push((h, k));
                } else {
                    table[j + old_cap].push((h, k));
                }
            }
        }
    }

    let mut table: Vec<Vec<(u32, K)>> = Vec::new();
    let mut threshold: usize = 0;
    let mut size: usize = 0;
    for key in keys {
        let h = hash(key.as_ref());
        if table.is_empty() {
            resize(&mut table, &mut threshold);
        }
        let n = table.len();
        let i = (n - 1) & (h as usize);
        if table[i]
            .iter()
            .any(|(eh, ek)| *eh == h && ek.as_ref() == key.as_ref())
        {
            // existing mapping for key: only the value is replaced
            continue;
        }
        table[i].push((h, key.clone()));
        // Java: `if (binCount >= TREEIFY_THRESHOLD - 1) treeifyBin(tab, hash);` when the new node
        // is appended behind 8 or more others.
        if table[i].len() > TREEIFY_THRESHOLD {
            if n < MIN_TREEIFY_CAPACITY {
                resize(&mut table, &mut threshold);
            } else {
                blaze_util::unported!(
                    "HashMap.treeifyBin (iteration order of a treeified bucket)",
                    "{}",
                    key.as_ref()
                );
            }
        }
        size += 1;
        if size > threshold {
            resize(&mut table, &mut threshold);
        }
    }
    table
        .into_iter()
        .flat_map(|bin| bin.into_iter().map(|(_, k)| k))
        .collect()
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

    // The expected orders below are `new HashMap<String, String>()` key sets printed by the JDK
    // (OpenJDK, a `java` single-file program) after `put`ting the keys in the given order.

    /// Keys of one bucket keep their insertion order.
    #[test]
    fn java_hash_map_order_same_bucket_keeps_insertion_order() {
        assert_eq!(
            java_hash_map_order(&["no-remote-exec", "no-remote"]),
            vec!["no-remote-exec", "no-remote"]
        );
        assert_eq!(
            java_hash_map_order(&["no-remote", "no-remote-exec"]),
            vec!["no-remote", "no-remote-exec"]
        );
        assert_eq!(
            java_hash_map_order(&["no-sandbox", "no-asan"]),
            vec!["no-sandbox", "no-asan"]
        );
    }

    #[test]
    fn java_hash_map_order_different_buckets() {
        assert_eq!(
            java_hash_map_order(&["no-sandbox", "requires-network"]),
            vec!["requires-network", "no-sandbox"]
        );
    }

    #[test]
    fn java_hash_map_order_duplicate_keys() {
        assert_eq!(
            java_hash_map_order(&["no-remote-exec", "no-remote", "no-remote-exec"]),
            vec!["no-remote-exec", "no-remote"]
        );
    }

    /// More than 12 keys: the table is resized to 32 buckets.
    #[test]
    fn java_hash_map_order_after_resize() {
        assert_eq!(
            java_hash_map_order(&[
                "no-a", "no-b", "no-c", "no-d", "no-e", "no-f", "no-g", "no-h", "no-i", "no-j",
                "no-k", "no-l", "no-m", "no-n", "no-o", "no-p", "no-q",
            ]),
            vec![
                "no-c", "no-b", "no-a", "no-g", "no-f", "no-e", "no-d", "no-k", "no-j", "no-i",
                "no-h", "no-o", "no-n", "no-m", "no-l", "no-q", "no-p",
            ]
        );
        assert_eq!(
            java_hash_map_order(&[
                "requires-x",
                "no-cache",
                "supports-workers",
                "block-network",
                "no-remote",
                "local",
                "cpu:4",
                "no-sandbox",
                "disable-foo",
                "no-remote-exec",
                "requires-gpu",
                "no-remote-cache",
                "resources:x:1",
                "no-a",
            ]),
            vec![
                "requires-gpu",
                "no-sandbox",
                "supports-workers",
                "cpu:4",
                "no-a",
                "no-cache",
                "requires-x",
                "no-remote-cache",
                "local",
                "no-remote-exec",
                "block-network",
                "disable-foo",
                "resources:x:1",
                "no-remote",
            ]
        );
    }

    /// Nine keys of one bucket of the 16-bucket table: `treeifyBin` resizes instead of
    /// treeifying.
    #[test]
    fn java_hash_map_order_treeify_bin_resizes_small_table() {
        assert_eq!(
            java_hash_map_order(&[
                "no-17", "no-28", "no-39", "no-100", "no-111", "no-122", "no-133", "no-144",
                "no-155",
            ]),
            vec![
                "no-100", "no-111", "no-122", "no-133", "no-144", "no-155", "no-17", "no-28",
                "no-39",
            ]
        );
    }
}
