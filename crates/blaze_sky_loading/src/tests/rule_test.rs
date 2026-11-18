//! Port of RuleTest (tests for `Rule`) on the PACKAGE harness.
//!
//! Java: src/test/java/com/google/devtools/build/lib/packages/RuleTest.java.
//!
//! Java adds three test-only native rules to `TestRuleClassProvider`: `fake_cc_library` (no
//! attributes), `fake_cc_binary` (`srcs`) and the test rule `fake_cc_test` (`srcs`, `deps` and
//! the test attributes). The rule class provider here is Bazel's, so they are Starlark rules of
//! the same names in `//fake_rules:defs.bzl` (a Starlark test rule has `size`, `timeout`,
//! `flaky`, `shard_count` and `local` implicitly), and BUILD files that use them start with
//! [`FAKE_RULES_LOAD`]. The rest of each BUILD file and every expected value is Java's.

use blaze_events::EventKind;
use blaze_packages::package::RuleVisibility;
use blaze_packages::target::target_utils;

use crate::tests::package_loading_test_case::{LoadedPackage, PackageLoadingTestCase};

/// The first line of a BUILD file that uses Java's fake rules.
const FAKE_RULES_LOAD: &str =
    "load('//fake_rules:defs.bzl', 'fake_cc_binary', 'fake_cc_library', 'fake_cc_test')";

/// A test case with `//fake_rules:defs.bzl` and Java's `--default_visibility=public`
/// (`PackageLoadingTestCase.parsePackageOptions`).
fn new_test_case() -> PackageLoadingTestCase {
    let mut t = PackageLoadingTestCase::new();
    t.use_public_default_visibility();
    t.scratch_file("fake_rules/BUILD", &[]);
    t.scratch_file(
        "fake_rules/defs.bzl",
        &[
            "def _impl(ctx):",
            "    pass",
            "fake_cc_library = rule(implementation = _impl)",
            "fake_cc_binary = rule(",
            "    implementation = _impl,",
            "    attrs = {'srcs': attr.label_list(allow_files = True)},",
            ")",
            "fake_cc_test = rule(",
            "    implementation = _impl,",
            "    test = True,",
            "    attrs = {",
            "        'srcs': attr.label_list(allow_files = True),",
            "        'deps': attr.label_list(allow_files = True),",
            "    },",
            ")",
        ],
    );
    t
}

/// Java `pkg.getRule(name).getVisibility().getDeclaredLabels()`, as label strings.
fn declared_visibility_labels(pkg: &LoadedPackage, rule: &str) -> Vec<String> {
    pkg.target(rule)
        .get_visibility()
        .get_declared_labels()
        .iter()
        .map(|label| label.to_string())
        .collect()
}

// Java: RuleTest.testOutputNameError
#[test]
fn test_output_name_error() {
    let t = new_test_case();
    t.scratch_file(
        "namecollide/BUILD",
        &[
            "genrule(",
            "    name = \"hello_world\",",
            "    srcs = [\"ignore_me.txt\"],",
            "    outs = [",
            "        \"message.txt\",",
            "        \"hello_world\",",
            "    ],",
            "    cmd = 'echo \"Hello, world.\" >$(location message.txt)',",
            ")",
        ],
    );
    let pkg = t.get_package("namecollide");
    let gen_rule = pkg
        .target("hello_world")
        .as_rule()
        .expect("hello_world is a rule");
    assert!(!gen_rule.contains_errors()); // TODO: assertTrue
    // Java: assertContainsEvent(message, ImmutableSet.of(EventKind.WARNING))
    let expected =
        "target 'hello_world' is both a rule and a file; please choose another name for the rule";
    assert!(
        pkg.events
            .iter()
            .any(|e| e.get_kind() == EventKind::Warning && e.to_string().contains(expected)),
        "no WARNING contains {expected:?}; events: {:#?}",
        pkg.messages()
    );
}

// Java: RuleTest.testIsLocalTestRuleForLocalEquals1
#[test]
fn test_is_local_test_rule_for_local_equals1() {
    let t = new_test_case();
    t.scratch_file(
        "x/BUILD",
        &[
            FAKE_RULES_LOAD,
            "fake_cc_test(",
            "    name = \"y\",",
            "    srcs = [\"a\"],",
            "    local = 0,",
            ")",
            "",
            "fake_cc_test(",
            "    name = \"z\",",
            "    srcs = [\"a\"],",
            "    local = 1,",
            ")",
        ],
    );
    let pkg = t.get_package_without_errors("x");
    let y = pkg.rule("y");
    assert!(!target_utils::is_local_test_rule(y));
    let z = pkg.rule("z");
    assert!(target_utils::is_local_test_rule(z));
}

// Java: RuleTest.testDeprecation
#[test]
fn test_deprecation() {
    let t = new_test_case();
    t.scratch_file(
        "x/BUILD",
        &[
            FAKE_RULES_LOAD,
            "fake_cc_test(name = \"y\")",
            "",
            "fake_cc_test(",
            "    name = \"z\",",
            "    deprecation = \"Foo\",",
            ")",
        ],
    );
    let pkg = t.get_package_without_errors("x");
    assert_eq!(target_utils::get_deprecation(pkg.target("y")), None);
    assert_eq!(
        target_utils::get_deprecation(pkg.target("z")),
        Some("Foo".to_owned())
    );
}

// Java: RuleTest.testVisibilityValid
#[test]
fn test_visibility_valid() {
    let t = new_test_case();
    t.scratch_file(
        "x/BUILD",
        &[
            FAKE_RULES_LOAD,
            "fake_cc_binary(",
            "    name = \"pr\",",
            "    visibility = [\"//visibility:private\"],",
            ")",
            "",
            "fake_cc_binary(",
            "    name = \"pu\",",
            "    visibility = [\"//visibility:public\"],",
            ")",
            "",
            "fake_cc_binary(",
            "    name = \"cu\",",
            "    visibility = [\"//a:b\"],",
            ")",
        ],
    );
    let pkg = t.get_package_without_errors("x");
    assert_eq!(pkg.target("pu").get_visibility(), RuleVisibility::Public);
    assert_eq!(pkg.target("pr").get_visibility(), RuleVisibility::Private);
}

// Java: RuleTest.testVisibilityTypo_failsCleanly
#[test]
fn test_visibility_typo_fails_cleanly() {
    let t = new_test_case();
    t.scratch_file(
        "x/BUILD",
        &[
            FAKE_RULES_LOAD,
            "fake_cc_binary(",
            "    name = \"typo\",",
            "    visibility = [\"//visibility:none\"],",
            ")",
        ],
    );
    let pkg = t.get_package("x");
    pkg.assert_contains_event(
        "Invalid visibility label '//visibility:none'; did you mean //visibility:public or \
         //visibility:private?",
    );
    assert!(pkg.package.contains_errors());
}

// Java: RuleTest.testVisibilityTypo_whenVisibilityPackageExists_failsCleanly
#[test]
fn test_visibility_typo_when_visibility_package_exists_fails_cleanly() {
    let t = new_test_case();
    t.scratch_file(
        "visibility/BUILD",
        &[
            FAKE_RULES_LOAD,
            "fake_cc_binary(",
            "    name = \"none\",",
            ")",
        ],
    );
    t.scratch_file(
        "x/BUILD",
        &[
            FAKE_RULES_LOAD,
            "fake_cc_binary(",
            "    name = \"typo\",",
            "    visibility = [\"//visibility:none\"],",
            ")",
        ],
    );
    assert!(
        !t.get_package_without_errors("visibility")
            .package
            .contains_errors()
    );
    let pkg = t.get_package("x");
    pkg.assert_contains_event(
        "Invalid visibility label '//visibility:none'; did you mean //visibility:public or \
         //visibility:private?",
    );
    assert!(pkg.package.contains_errors());
}

// Java: RuleTest.testVisibilityPkgSubpackages_whenVisibilityPackageExists_succeeds
#[test]
fn test_visibility_pkg_subpackages_when_visibility_package_exists_succeeds() {
    let t = new_test_case();
    t.scratch_file(
        "visibility/BUILD",
        &[
            FAKE_RULES_LOAD,
            "fake_cc_binary(",
            "    name = \"none\",",
            ")",
        ],
    );
    t.scratch_file(
        "x/BUILD",
        &[
            FAKE_RULES_LOAD,
            "fake_cc_binary(",
            "    name = \"p\",",
            "    visibility = [\"//visibility:__pkg__\"],",
            ")",
            "",
            "fake_cc_binary(",
            "    name = \"s\",",
            "    visibility = [\"//visibility:__subpackages__\"],",
            ")",
        ],
    );
    assert!(
        !t.get_package_without_errors("visibility")
            .package
            .contains_errors()
    );
    let pkg = t.get_package_without_errors("x");
    assert!(!pkg.package.contains_errors());
    assert_eq!(
        declared_visibility_labels(&pkg, "p"),
        ["//visibility:__pkg__"]
    );
    assert_eq!(
        declared_visibility_labels(&pkg, "s"),
        ["//visibility:__subpackages__"]
    );
}

// Java: RuleTest.testVisibilityMisspelling
#[test]
fn test_visibility_misspelling() {
    let t = new_test_case();
    t.scratch_file(
        "x/BUILD",
        &[
            FAKE_RULES_LOAD,
            "fake_cc_binary(",
            "    name = \"is_this_public\",",
            "    visibility = [\"//visibility:plubic\"],",
            ")",
        ],
    );
    let pkg = t.get_package("x");
    pkg.assert_contains_event(
        "Invalid visibility label '//visibility:plubic'; did you mean //visibility:public or \
         //visibility:private?",
    );
    assert!(pkg.package.contains_errors());
}

// Java: RuleTest.testPublicAndPrivateVisibility
#[test]
fn test_public_and_private_visibility() {
    let t = new_test_case();
    t.scratch_file(
        "x/BUILD",
        &[
            FAKE_RULES_LOAD,
            "package(default_visibility = [\"//default:__pkg__\"])",
            "",
            "fake_cc_binary(",
            "    name = \"is_this_public\",",
            "    visibility = [\"//some:__pkg__\", \"//visibility:public\"],",
            ")",
            "",
            "fake_cc_binary(",
            "    name = \"is_private_dropped\",",
            "    visibility = [\"//some:__pkg__\", \"//visibility:private\"],",
            ")",
            "",
            "fake_cc_binary(",
            "    name = \"is_empty_visibility_private\",",
            "    visibility = [],",
            ")",
        ],
    );
    let pkg = t.get_package_without_errors("x");
    assert!(!pkg.package.contains_errors());
    assert_eq!(
        declared_visibility_labels(&pkg, "is_this_public"),
        ["//visibility:public"]
    );
    assert_eq!(
        declared_visibility_labels(&pkg, "is_private_dropped"),
        ["//some:__pkg__"]
    );
    assert_eq!(
        declared_visibility_labels(&pkg, "is_empty_visibility_private"),
        ["//visibility:private"]
    );
}

// Java: RuleTest.testReduceForSerialization
#[test]
#[ignore = "Target.reduceForSerialization (ReducedTargetData) is not ported in blaze_packages"]
fn test_reduce_for_serialization() {}
