//! Port of BzlLoadFunctionTest (the cases that need no rule-definition builtins).
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/BzlLoadFunctionTest.java. Builtins
//! injection is disabled (`--experimental_builtins_bzl_path=`) unless a test says otherwise, so
//! that .bzl files evaluate in blaze_starlark's minimal environment.

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::StoredEventHandler;
use blaze_skyframe::SkyKey;
use blaze_util::Fingerprint;
use blaze_vfs::DigestHashFunction;

use crate::bzl_load::{BzlLoadFailedException, BzlLoadKey};
use crate::starlark_builtins::{StarlarkBuiltinsKey, exports_entrypoint_key};
use crate::test_util::{FakeRepos, TestSetup, evaluate};

fn setup() -> TestSetup {
    let mut setup = TestSetup::new(FakeRepos::default());
    setup.semantics.experimental_builtins_bzl_path = String::new();
    setup
}

fn key(label: &str) -> BzlLoadKey {
    BzlLoadKey::key_for_build(Label::parse_canonical_unchecked(label))
}

fn file(setup: &TestSetup, rel: &str, contents: &str) {
    setup.scratch.file(&setup.ws(rel), contents);
}

// Java: BzlLoadFunctionTest.checkSuccessfulLookup
fn check_successful_lookup(setup: &TestSetup, label: &str) {
    let evaluator = setup.evaluator();
    let k = key(label);
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(k.clone())],
        false,
        Arc::new(StoredEventHandler::new()),
    );
    assert!(!result.has_error(), "{:?}", result);
    // Ensure that the file has been processed by checking its Module for the label field.
    assert_eq!(
        result.get(&k).unwrap().get_module_context().label,
        Label::parse_canonical_unchecked(label)
    );
}

// Java: BzlLoadFunctionTest.checkFailingLookup
fn check_failing_lookup(
    setup: &TestSetup,
    label: &str,
    expected_message: &str,
) -> Arc<StoredEventHandler> {
    let evaluator = setup.evaluator();
    let k = key(label);
    let events = Arc::new(StoredEventHandler::new());
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(k.clone())],
        false,
        events.clone(),
    );
    assert!(result.has_error());
    let error = result.get_error(&SkyKey::new(k)).expect("error entry");
    let e = error
        .exception_as::<BzlLoadFailedException>()
        .expect("a BzlLoadFailedException");
    assert!(
        e.get_message().contains(expected_message),
        "{:?} does not contain {:?}",
        e.get_message(),
        expected_message
    );
    events
}

fn event_messages(events: &StoredEventHandler) -> Vec<String> {
    events
        .get_events()
        .iter()
        .map(|e| e.get_message().into_owned())
        .collect()
}

#[test]
fn test_load_relative_label() {
    let s = setup();
    file(&s, "pkg/BUILD", "");
    file(&s, "pkg/ext1.bzl", "a = 1");
    file(&s, "pkg/ext2.bzl", "load(':ext1.bzl', 'a')");
    check_successful_lookup(&s, "//pkg:ext2.bzl");
}

#[test]
fn test_load_absolute_label() {
    let s = setup();
    file(&s, "pkg2/BUILD", "");
    file(&s, "pkg3/BUILD", "");
    file(&s, "pkg2/ext.bzl", "b = 1");
    file(&s, "pkg3/ext.bzl", "load('//pkg2:ext.bzl', 'b')");
    check_successful_lookup(&s, "//pkg3:ext.bzl");
}

#[test]
fn test_load_from_same_absolute_label_twice() {
    let s = setup();
    file(&s, "pkg1/BUILD", "");
    file(&s, "pkg2/BUILD", "");
    file(&s, "pkg1/ext.bzl", "a = 1\nb = 2");
    file(
        &s,
        "pkg2/ext.bzl",
        "load('//pkg1:ext.bzl', 'a')\nload('//pkg1:ext.bzl', 'b')",
    );
    check_successful_lookup(&s, "//pkg2:ext.bzl");
}

#[test]
fn test_load_from_same_relative_label_twice() {
    let s = setup();
    file(&s, "pkg/BUILD", "");
    file(&s, "pkg/ext1.bzl", "a = 1\nb = 2");
    file(
        &s,
        "pkg/ext2.bzl",
        "load(':ext1.bzl', 'a')\nload(':ext1.bzl', 'b')",
    );
    check_successful_lookup(&s, "//pkg:ext2.bzl");
}

#[test]
fn test_load_from_relative_label_in_subdir() {
    let s = setup();
    file(&s, "pkg/BUILD", "");
    file(&s, "pkg/subdir/ext1.bzl", "a = 1");
    file(&s, "pkg/subdir/ext2.bzl", "load(':subdir/ext1.bzl', 'a')");
    check_successful_lookup(&s, "//pkg:subdir/ext2.bzl");
}

#[test]
fn test_load_bad_extension_scl_disabled() {
    let mut s = setup();
    s.semantics.experimental_enable_scl_dialect = false;
    file(&s, "pkg/BUILD", "");
    file(&s, "pkg/ext.bzl", "load(':foo.garbage', 'a')");
    let events = check_failing_lookup(&s, "//pkg:ext.bzl", "has invalid load statements");
    let messages = event_messages(&events);
    assert!(
        messages
            .iter()
            .any(|m| m.contains("The label must reference a file with extension \".bzl\"")),
        "{:?}",
        messages
    );
    assert!(
        !messages.iter().any(|m| m.contains(".scl")),
        "{:?}",
        messages
    );
}

#[test]
fn test_load_bad_extension_scl_enabled() {
    let mut s = setup();
    s.semantics.experimental_enable_scl_dialect = true;
    file(&s, "pkg/BUILD", "");
    file(&s, "pkg/ext.bzl", "load(':foo.garbage', 'a')");
    let events = check_failing_lookup(&s, "//pkg:ext.bzl", "has invalid load statements");
    let messages = event_messages(&events);
    assert!(
        messages
            .iter()
            .any(|m| m
                .contains("The label must reference a file with extension \".bzl\" or \".scl\"")),
        "{:?}",
        messages
    );
}

#[test]
fn test_loading_scl_requires_experimental_flag() {
    let mut s = setup();
    s.semantics.experimental_enable_scl_dialect = false;
    file(&s, "pkg/BUILD", "");
    file(&s, "pkg/ext.scl", "");
    check_failing_lookup(
        &s,
        "//pkg:ext.scl",
        "loading .scl files requires setting --experimental_enable_scl_dialect",
    );
}

#[test]
fn test_scl_cannot_load_non_scl_files() {
    let mut s = setup();
    s.semantics.experimental_enable_scl_dialect = true;
    file(&s, "pkg/BUILD", "");
    file(&s, "pkg/ext.scl", "load('//pkg:other.bzl', 'a')");
    file(&s, "pkg/other.bzl", "a = 1");
    let events = check_failing_lookup(&s, "//pkg:ext.scl", "has invalid load statements");
    let messages = event_messages(&events);
    assert!(
        messages.iter().any(|m| m.contains(
            "The label must reference a file with extension \".scl\" (.scl files cannot load \
             .bzl files)"
        )),
        "{:?}",
        messages
    );
}

#[test]
fn test_bzl_load_no_build_file() {
    let s = setup();
    file(&s, "pkg/ext.bzl", "");
    check_failing_lookup(
        &s,
        "//pkg:ext.bzl",
        "Every .bzl file must have a corresponding package, but '//pkg:ext.bzl' does not",
    );
}

#[test]
fn test_bzl_load_no_build_file_for_load() {
    let s = setup();
    file(&s, "pkg2/BUILD", "");
    file(&s, "pkg1/ext.bzl", "a = 1");
    file(&s, "pkg2/ext.bzl", "load('//pkg1:ext.bzl', 'a')");
    check_failing_lookup(
        &s,
        "//pkg:ext.bzl",
        "Every .bzl file must have a corresponding package",
    );
}

#[test]
fn test_load_from_subdir_in_same_package_is_ok() {
    let s = setup();
    file(&s, "a/BUILD", "");
    file(&s, "a/a.bzl", "load('//a:b/b.bzl', 'b')");
    file(&s, "a/b/b.bzl", "b = 42");
    check_successful_lookup(&s, "//a:a.bzl");
}

#[test]
fn test_load_must_respect_package_boundary_of_subpkg() {
    let s = setup();
    file(&s, "a/BUILD", "");
    file(&s, "a/a.bzl", "load('//a:b/b.bzl', 'b')");
    file(&s, "a/b/BUILD", "");
    file(&s, "a/b/b.bzl", "b = 42");
    check_failing_lookup(
        &s,
        "//a:a.bzl",
        "Label '//a:b/b.bzl' is invalid because 'a/b' is a subpackage; perhaps you meant to put \
         the colon here: '//a/b:b.bzl'?",
    );
}

#[test]
fn test_load_must_respect_package_boundary_of_subpkg_relative() {
    let s = setup();
    file(&s, "a/BUILD", "");
    file(&s, "a/a.bzl", "load('b/b.bzl', 'b')");
    file(&s, "a/b/BUILD", "");
    file(&s, "a/b/b.bzl", "b = 42");
    check_failing_lookup(
        &s,
        "//a:a.bzl",
        "Label '//a:b/b.bzl' is invalid because 'a/b' is a subpackage; perhaps you meant to put \
         the colon here: '//a/b:b.bzl'?",
    );
}

#[test]
fn test_load_from_non_existent_repository_produces_meaningful_error() {
    let s = setup();
    file(&s, "BUILD", "load(\"@repository//dir:file.bzl\", \"foo\")");
    check_failing_lookup(
        &s,
        "@repository//dir:file.bzl",
        "Unable to find package for @@repository//dir:file.bzl: The repository '@@repository' \
         could not be resolved: Repository '@@repository' is not defined.",
    );
}

// Not a Java test: the transitive digest is Java's Fingerprint of the file's digest followed by
// the transitive digests of its loads, in load order (no builtins digest with injection off).
#[test]
fn transitive_digest_matches_java_formula() {
    let s = setup();
    file(&s, "pkg/BUILD", "");
    let b = "b = 1\n";
    let a = "load(':b.bzl', 'b')\na = b\n";
    file(&s, "pkg/b.bzl", b);
    file(&s, "pkg/a.bzl", a);
    let evaluator = s.evaluator();
    let k = key("//pkg:a.bzl");
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(k.clone())],
        false,
        Arc::new(StoredEventHandler::new()),
    );
    assert!(!result.has_error(), "{:?}", result);
    let sha = |c: &str| {
        DigestHashFunction::Sha256
            .hash_reader(&mut c.as_bytes())
            .unwrap()
    };
    let digest_b = Fingerprint::new().add_bytes(&sha(b)).digest_and_reset();
    let digest_a = Fingerprint::new()
        .add_bytes(&sha(a))
        .add_bytes(&digest_b)
        .digest_and_reset();
    assert_eq!(result.get(&k).unwrap().get_transitive_digest(), digest_a);
    assert_eq!(
        format!("{:?}", k),
        "KeyForBuild{label=//pkg:a.bzl, isBuildPrelude=false}"
    );
}

#[test]
#[ignore = "needs agent B's .bzl builtins (provider(), ...) to evaluate @_builtins"]
fn builtins_exports_evaluate() {
    let s = TestSetup::new(FakeRepos::default());
    let evaluator = s.evaluator();
    let result = evaluate(
        &evaluator,
        vec![
            SkyKey::new(exports_entrypoint_key()),
            SkyKey::new(StarlarkBuiltinsKey::key(false)),
        ],
        false,
        Arc::new(StoredEventHandler::new()),
    );
    assert!(!result.has_error(), "{:?}", result);
}

macro_rules! ignored {
    ($($name:ident: $reason:literal,)*) => {
        $(
            #[test]
            #[ignore = $reason]
            fn $name() {}
        )*
    };
}

ignored! {
    test_bzl_load_labels: "needs PACKAGE (loads of a BUILD file)",
    test_bzl_load_labels_alternative_root: "needs PACKAGE (loads of a BUILD file)",
    test_bzl_load_labels_multiple_build_files: "needs PACKAGE (loads of a BUILD file)",
    test_load_from_starlark_file_in_remote_repo: "needs Bzlmod (local_repository)",
    test_can_load_scl: "needs agent B's visibility() for .scl",
    test_can_load_scl_from_bzl_and_scl: "needs agent B's visibility() for .scl",
    test_scl_can_only_load_labels_relative_to_default_repo_root: "needs agent B's visibility() for .scl",
    test_scl_supports_struct_and_visibility: "needs agent B's visibility()",
    test_scl_does_not_support_other_bazel_symbols: "needs agent B's .bzl builtins",
    test_scl_disallows_non_ascii_string_literals: "needs blaze_starlark's .scl ASCII check message",
    test_bzl_load_filename_with_control_chars: "needs PACKAGE",
    test_load_must_respect_package_boundary_of_indirect_subpkg: "needs PACKAGE",
    test_load_must_respect_package_boundary_of_parent_pkg: "needs PACKAGE",
    test_bzl_visibility_disabled_without_flag: "needs agent B's visibility()",
    test_bzl_visibility_public_explicit: "needs agent B's visibility()",
    test_bzl_visibility_public_implicit: "needs agent B's visibility()",
    test_bzl_visibility_private_same_package: "needs agent B's visibility()",
    test_bzl_visibility_private_different_package: "needs agent B's visibility()",
    test_bzl_visibility_empty_list_means_private: "needs agent B's visibility()",
    test_bzl_visibility_public_list_element: "needs agent B's visibility()",
    test_bzl_visibility_private_list_element: "needs agent B's visibility()",
    test_bzl_visibility_failure_in_dependency: "needs agent B's visibility()",
    test_bzl_visibility_cannot_be_set_in_function: "needs agent B's visibility()",
    test_bzl_visibility_cannot_be_set_twice: "needs agent B's visibility()",
    test_bzl_visibility_enumerated_packages: "needs agent B's visibility()",
    test_bzl_visibility_single_enumerated_package_as_string: "needs agent B's visibility()",
    test_bzl_visibility_enumerated_packages_multiple_repos: "needs agent B's visibility()",
    test_bzl_visibility_can_use_public_private_regardless_of_flag: "needs agent B's visibility()",
    test_bzl_visibility_repo_root_subpackages_is_not_public_regardless_of_flag: "needs agent B's visibility()",
    test_bzl_visibility_disallows_subpackages_without_wildcard: "needs agent B's visibility()",
    test_bzl_visibility_allows_subpackages_with_wildcard: "needs agent B's visibility()",
    test_bzl_visibility_invalid_bad_type: "needs agent B's visibility()",
    test_bzl_visibility_invalid_bad_element_type: "needs agent B's visibility()",
    test_bzl_visibility_package_with_repo_works: "needs agent B's visibility()",
    test_bzl_visibility_invalid_negation_not_supported: "needs agent B's visibility()",
    test_bzl_visibility_errors_demoted_to_warning_when_break_glass_flag_is_set: "needs agent B's visibility()",
    test_load_bzl_file_from_bzlmod: "needs Bzlmod (registry)",
    test_builtins_injection_failure: "needs agent B's .bzl builtins to evaluate @_builtins",
    test_error_reading_bzl_file_is_transient_when_using_ast_inlining: "needs a mock file system that fails reads",
    test_error_reading_other_bzl_file_is_persistent_from_perspective_of_parent: "needs a mock file system that fails reads",
    test_error_stating_bzl_file_in_file_state_function_is_persistent: "needs a mock file system that fails stats",
    bzl_compile_cache_hit_from_node_with_other_key_kind_registers_file_dep: "needs Bzlmod keys evaluated by a module",
}
