//! Not a Java test (review query-2 round 3, P63-1): every reader of a Starlark dict in the Rust
//! tree either records the keys of a key-tracking dict (`ctx.var`, Java's
//! `ImmutableKeyTrackingDict`, whose `entrySet()`/`keySet()`/`iterator()` record every key) or is
//! listed below with the reason it cannot read a key-tracking dict, or reads it as Java reads it
//! without `entrySet()` (`instanceof`, `isEmpty`, `get`).
//!
//! A reader is a line with `DictRef::from_value(`, `DictMut::from_value(`, a `DictRef<'v>` or
//! `UnpackDictEntries`/`DictType` parameter, in `crates/*/src` outside test files. It records when
//! its enclosing function calls `dict_record_all_keys_accessed` before it (the Blaze helpers
//! `dict_cast` and `Type` conversion in blaze_packages, the `Dict.cast` helpers of blaze_analysis
//! and blaze_rules, `json.encode`, `proto.encode_text`, `select`). A new reader that is neither
//! fails this test; an entry of the list that no longer names a reader fails it too.

use std::path::{Path, PathBuf};

/// (path under `crates/`, enclosing function or `*` for the whole file, why it need not record).
const NOT_RECORDING: &[(&str, &str, &str)] = &[
    // Phases without `ctx.var`: repository rules and module extensions, MODULE.bazel,
    // REPO.bazel, PROJECT.scl, the builtins injection, loading-time computed defaults, Starlark
    // transitions, rule and macro definitions, the build-info translation callback (execution).
    (
        "blaze_repo_rules/src/",
        "*",
        "repository rules and module extensions have no ctx.var",
    ),
    (
        "blaze_bzlmod/src/module_file/module_file_globals.rs",
        "*",
        "MODULE.bazel has no ctx.var",
    ),
    (
        "blaze_sky_loading/src/project_function.rs",
        "*",
        "PROJECT.scl has no ctx.var",
    ),
    (
        "blaze_sky_loading/src/repo_file.rs",
        "*",
        "REPO.bazel has no ctx.var",
    ),
    (
        "blaze_sky_loading/src/starlark_builtins.rs",
        "get_dict",
        "the builtins' exported dicts, read at injection",
    ),
    (
        "blaze_packages/src/attribute/computed_default.rs",
        "cast_callback_result",
        "a Starlark computed default runs at loading",
    ),
    (
        "blaze_build_config/src/starlark_transition.rs",
        "*",
        "a transition implementation gets settings and attr, no ctx",
    ),
    (
        "blaze_actions/src/analysis/actions/build_info_file_write_action.rs",
        "call_translation_callback",
        "the translation callback runs at execution, no ctx",
    ),
    (
        "blaze_packages/src/starlark_defs/rule_class_functions.rs",
        "rule",
        "a type check of rule()'s attrs (Java instanceof Dict)",
    ),
    (
        "blaze_packages/src/starlark_defs/attr_module.rs",
        "is_java_dict",
        "a type check (Java instanceof Dict)",
    ),
    (
        "blaze_starlarkdocextract/src/label_renderer.rs",
        "repr_with",
        "documentation extraction of loaded .bzl values",
    ),
    // Type checks and reads that do not go through entrySet()/keySet() in Java.
    (
        "blaze_analysis/src/starlark/starlark_rule_context.rs",
        "is_non_empty_dict",
        "Java isNonEmptyDict: instanceof Dict and isEmpty() (reads contents)",
    ),
    (
        "blaze_analysis/src/starlark/starlark_rule_context.rs",
        "runfiles",
        "the type check of symlinks/root_symlinks; the read is a hooked cast",
    ),
    (
        "blaze_analysis/src/starlark/starlark_rule_context.rs",
        "check_label_dict",
        "a dict with string keys fails here (Java checkLabelDict), so the target is in error",
    ),
    (
        "blaze_analysis/src/starlark/starlark_action_factory.rs",
        "is_dict_or_none",
        "a type check (Java instanceof Dict)",
    ),
    (
        "blaze_analysis/src/starlark/starlark_action_factory.rs",
        "register_starlark_action",
        "the type check of resource_set (Java instanceof Dict)",
    ),
    (
        "blaze_rules/src/cpp/cc_starlark_internal.rs",
        "cc_toolchain_variables",
        "the type check of vars; the read is the hooked cast",
    ),
    (
        "blaze_rules/src/cpp/cc_starlark_internal.rs",
        "freeze",
        "the type check before Dict.immutableCopyOf",
    ),
    (
        "blaze_packages/src/build/select.rs",
        "select_impl",
        "takes the DictRef of select(), which records before calling it",
    ),
    (
        "blaze_starlark/src/dict.rs",
        "dict_immutable_copy_of",
        "Dict.immutableCopyOf returns an immutable dict (ctx.var) itself; copies only mutable ones",
    ),
    (
        "blaze_starlark/src/structs.rs",
        "java_hash_code",
        "ImmutableMap.hashCode/Dict.hashCode read contents",
    ),
    (
        "blaze_packages/src/starlark_defs/util.rs",
        "dict_entries",
        "the raw reader under dict_cast (which records); its other callers run at loading \
         (macro attrs, provider fields) or in query output",
    ),
];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crates directory")
        .to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.expect("a directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n == "tests" || n == "testing")
            {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs")
            && !path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains("test"))
        {
            out.push(path);
        }
    }
}

fn is_reader(line: &str) -> bool {
    let code = line.trim_start();
    if code.starts_with("//") {
        return false;
    }
    code.contains("DictRef::from_value(")
        || code.contains("DictMut::from_value(")
        || code.contains("UnpackDictEntries<")
        || code.contains("DictType<")
        || code.contains(": DictRef<'v>")
}

/// The name of the function a line is in: the nearest `fn name` above it.
fn enclosing_fn(lines: &[&str], index: usize) -> (usize, String) {
    for j in (0..=index).rev() {
        let line = lines[j].trim_start();
        if line.starts_with("//") {
            continue;
        }
        if let Some(pos) = line.find("fn ") {
            let before = &line[..pos];
            if before.is_empty() || before.ends_with(' ') || before.ends_with('(') {
                let name: String = line[pos + 3..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    return (j, name);
                }
            }
        }
    }
    (0, String::new())
}

#[test]
fn every_starlark_dict_reader_records_or_is_listed() {
    let crates = crates_dir();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates).expect("crates/") {
        let src = entry.expect("an entry").path().join("src");
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
    }
    files.sort();
    let mut used = vec![false; NOT_RECORDING.len()];
    let mut unlisted = Vec::new();
    for file in &files {
        let rel = file
            .strip_prefix(&crates)
            .expect("under crates/")
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(file).expect("a source file");
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !is_reader(line) {
                continue;
            }
            let (fn_line, fn_name) = enclosing_fn(&lines, i);
            if lines[fn_line..=i]
                .iter()
                .any(|l| l.contains("dict_record_all_keys_accessed"))
            {
                continue;
            }
            let listed = NOT_RECORDING.iter().position(|(path, func, _)| {
                rel.starts_with(path) && (*func == "*" || *func == fn_name)
            });
            match listed {
                Some(k) => used[k] = true,
                None => unlisted.push(format!("{rel}:{} in fn {fn_name}", i + 1)),
            }
        }
    }
    assert!(
        unlisted.is_empty(),
        "Starlark dict readers that neither call dict_record_all_keys_accessed first nor are \
         listed with a reason in NOT_RECORDING (Java's entrySet()/keySet()/iterator() of \
         ctx.var record every key):\n{}",
        unlisted.join("\n")
    );
    let stale: Vec<String> = NOT_RECORDING
        .iter()
        .zip(&used)
        .filter(|(_, used)| !**used)
        .map(|((path, func, _), _)| format!("{path} {func}"))
        .collect();
    assert!(
        stale.is_empty(),
        "NOT_RECORDING entries that name no reader any more:\n{}",
        stale.join("\n")
    );
}
