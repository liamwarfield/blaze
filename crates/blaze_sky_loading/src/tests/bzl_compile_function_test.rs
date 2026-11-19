//! Port of BzlCompileFunctionTest.
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/BzlCompileFunctionTest.java.

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::{EventKind, StoredEventHandler};
use blaze_skyframe::SkyKey;
use blaze_starlark::semantics::Utf8EnforcementMode;
use blaze_vfs::{DigestHashFunction, Root};

use crate::bzl_compile::{BzlCompileKey, BzlCompileValue};
use crate::test_util::{FakeRepos, TestSetup, evaluate, value_of};

fn compile_value(
    setup: &TestSetup,
    key: BzlCompileKey,
) -> (Arc<BzlCompileValue>, Arc<StoredEventHandler>) {
    let evaluator = setup.evaluator();
    let events = Arc::new(StoredEventHandler::new());
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(key.clone())],
        false,
        events.clone(),
    );
    assert!(!result.has_error(), "{:?}", result);
    (result.get(&key).unwrap(), events)
}

#[test]
#[ignore = "needs PACKAGE and a mock file system that throws on stat"]
fn test_io_exception_occurs_during_reading() {}

#[test]
fn test_load_from_file_in_remote_repo() {
    let setup = TestSetup::new(FakeRepos::default());
    let repo_path = setup.scratch.dir("a_remote_repo");
    setup.scratch.file("a_remote_repo/REPO.bazel", "");
    setup.scratch.file("a_remote_repo/remote_pkg/BUILD", "");
    setup.scratch.file(
        "a_remote_repo/remote_pkg/foo.bzl",
        "load(':bar.bzl', 'CONST')",
    );
    setup
        .scratch
        .file("a_remote_repo/remote_pkg/bar.bzl", "CONST = 17");

    // PORT: Java parses "@a_remote_repo//..." with the main repo mapping of its test setup,
    // which maps it to the canonical name "a_remote_repo".
    let key = BzlCompileKey::key(
        Root::from_path(repo_path),
        Label::parse_canonical_unchecked("@@a_remote_repo//remote_pkg:foo.bzl"),
    );
    let (value, _) = compile_value(&setup, key);
    let loads: Vec<&str> = value
        .get_program()
        .loads
        .iter()
        .map(|l| l.module.as_str())
        .collect();
    assert_eq!(loads, vec![":bar.bzl"]);
}

#[test]
fn test_load_of_nonexistent_file() {
    let setup = TestSetup::new(FakeRepos::default());
    let key = BzlCompileKey::key(
        Root::from_path(setup.workspace().clone()),
        Label::parse_canonical_unchecked("//pkg:foo.bzl"),
    );
    let evaluator = setup.evaluator();
    let value = value_of(&evaluator, key);
    assert!(!value.lookup_successful());
    assert!(
        value
            .get_error()
            .contains("cannot load '//pkg:foo.bzl': no such file")
    );
}

#[test]
fn test_big_integer_literals() {
    // This test ensures that numerical literals with values that can't be expressed as Java
    // longs can be compiled. Regression test for b/217548647.
    let setup = TestSetup::new(FakeRepos::default());
    let key = BzlCompileKey::key(
        Root::from_path(setup.workspace().clone()),
        Label::parse_canonical_unchecked("//pkg:bigint.bzl"),
    );
    setup.scratch.file(&setup.ws("pkg/BUILD"), "");
    setup.scratch.file(
        &setup.ws("pkg/bigint.bzl"),
        "[-9223372036854775809, 9223372036854775808]",
    );
    let (value, _) = compile_value(&setup, key);
    assert!(value.lookup_successful());
    // PORT: Java also executes the program and checks its value; execution is BZL_LOAD's
    // (blaze_starlark) concern.
}

#[test]
fn test_invalid_utf8_enforcement_off() {
    let mut setup = TestSetup::new(FakeRepos::default());
    setup.semantics.incompatible_enforce_starlark_utf8 = Utf8EnforcementMode::Off;
    setup.scratch.file(&setup.ws("pkg/BUILD"), "");
    setup
        .scratch
        .file_bytes(&setup.ws("pkg/foo.bzl"), &[b'#', b' ', 0x80]);

    let key = BzlCompileKey::key(
        Root::from_path(setup.workspace().clone()),
        Label::parse_canonical_unchecked("//pkg:foo.bzl"),
    );
    let (value, events) = compile_value(&setup, key);
    assert!(value.lookup_successful());
    assert!(events.get_events().is_empty(), "{:?}", events.get_events());
}

#[test]
fn test_invalid_utf8_enforcement_warning() {
    let mut setup = TestSetup::new(FakeRepos::default());
    setup.semantics.incompatible_enforce_starlark_utf8 = Utf8EnforcementMode::Warning;
    setup.scratch.file(&setup.ws("pkg/BUILD"), "");
    let bzl = setup
        .scratch
        .file_bytes(&setup.ws("pkg/foo.bzl"), &[b'#', b' ', 0x80]);

    let key = BzlCompileKey::key(
        Root::from_path(setup.workspace().clone()),
        Label::parse_canonical_unchecked("//pkg:foo.bzl"),
    );
    let (value, events) = compile_value(&setup, key);
    assert!(value.lookup_successful());
    let events = events.get_events();
    assert!(
        events.iter().any(|e| {
            e.get_kind() == EventKind::Warning
                && e.get_location().map(|l| l.file().to_owned()) == Some(bzl.to_string())
                && e.get_message()
                    == "not a valid UTF-8 encoded file; this can lead to inconsistent behavior \
                        and will be disallowed in a future version of Bazel"
        }),
        "{events:?}"
    );
}

// Not a Java test: the program text of a file that is not UTF-8 is its bytes, one char each
// (`ParserInput.fromLatin1`), so a string literal holding the byte FF is the one char U+00FF and
// `é` (C3 A9) is two chars.
#[test]
fn non_utf8_file_compiles_with_latin1_text() {
    let setup = TestSetup::new(FakeRepos::default());
    setup.scratch.file(&setup.ws("pkg/BUILD"), "");
    setup
        .scratch
        .file_bytes(&setup.ws("pkg/foo.bzl"), b"X = '\xff'\nE = '\xc3\xa9'\n");
    let key = BzlCompileKey::key(
        Root::from_path(setup.workspace().clone()),
        Label::parse_canonical_unchecked("//pkg:foo.bzl"),
    );
    let (value, events) = compile_value(&setup, key);
    assert!(value.lookup_successful(), "{}", value.get_error());
    assert!(
        events
            .get_events()
            .iter()
            .any(|e| e.get_kind() == EventKind::Warning),
        "the default enforcement mode warns"
    );
    let digest = DigestHashFunction::Sha256
        .hash_reader(&mut &b"X = '\xff'\nE = '\xc3\xa9'\n"[..])
        .unwrap();
    assert_eq!(value.get_digest(), digest.as_slice());
}

#[test]
fn test_invalid_utf8_enforcement_error() {
    let mut setup = TestSetup::new(FakeRepos::default());
    setup.semantics.incompatible_enforce_starlark_utf8 = Utf8EnforcementMode::Error;
    setup.scratch.file(&setup.ws("pkg/BUILD"), "");
    let bzl = setup
        .scratch
        .file_bytes(&setup.ws("pkg/foo.bzl"), &[b'#', b' ', 0x80]);

    let key = BzlCompileKey::key(
        Root::from_path(setup.workspace().clone()),
        Label::parse_canonical_unchecked("//pkg:foo.bzl"),
    );
    let (value, events) = compile_value(&setup, key);
    assert!(!value.lookup_successful());
    assert_eq!(
        value.get_error(),
        format!("compilation of '{}' failed", bzl)
    );
    let events = events.get_events();
    assert!(events.iter().any(|e| {
        e.get_kind() == EventKind::Error
            && e.get_location().map(|l| l.file().to_owned()) == Some(bzl.to_string())
            && e.get_message()
                == "not a valid UTF-8 encoded file; this can lead to inconsistent behavior and \
                    will be disallowed in a future version of Bazel. For a temporary workaround, \
                    see the --incompatible_enforce_starlark_utf8 flag."
    }));
}

// Not a Java test: the digest of a successful compile is the SHA-256 of the file's bytes, as
// Java's `hashFunction.hashBytes(bytes)` (fast digests are unavailable on Linux file systems).
#[test]
fn digest_is_sha256_of_file_bytes() {
    let setup = TestSetup::new(FakeRepos::default());
    let contents = "X = 1\n";
    setup.scratch.file(&setup.ws("pkg/BUILD"), "");
    setup.scratch.file(&setup.ws("pkg/x.bzl"), contents);
    let key = BzlCompileKey::key(
        Root::from_path(setup.workspace().clone()),
        Label::parse_canonical_unchecked("//pkg:x.bzl"),
    );
    let (value, _) = compile_value(&setup, key.clone());
    let expected = DigestHashFunction::Sha256
        .hash_reader(&mut contents.as_bytes())
        .unwrap();
    assert_eq!(value.get_digest(), expected.as_slice());
    assert_eq!(
        format!("{:?}", key),
        format!("BZL_COMPILE:[{}]//pkg:x.bzl", setup.workspace())
    );
}
