//! Tests for the S1 findings of docs/reviews/2026-11-18-loading.md that have no Java test of
//! their own in a ported test class; the expected behaviour is the Java cited at each test (and,
//! for `--deleted_packages`, the oracle run recorded in the review response).

use blaze_cmdline::PackageIdentifier;
use blaze_options::Foreign;

use crate::tests::package_loading_test_case::PackageLoadingTestCase;

// S1-1. Java: BzlCompileFunction.java:249 shouldUseTypeSyntax (true for every non-prelude,
// non-.scl file by default: FlagConstants.java:36 DEFAULT_EXPERIMENTAL_STARLARK_TYPE_SYNTAX)
// and :212-214 (annotations parsed, not resolved, arbitrary expressions tolerated).
#[test]
fn bzl_type_annotations_are_accepted_by_default() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/defs.bzl",
        &[
            "def f(x: int, y: NotDefinedAnywhere[str] = 1) -> int:",
            "    z: int = x + y",
            "    return z",
            "X = f(1)",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &["load(':defs.bzl', 'X')", "filegroup(name = 'x' + str(X))"],
    );
    let pkg = t.get_package_without_errors("pkg");
    pkg.target("x2");
}

// S1-1. Java: BzlCompileFunction.shouldUseTypeSyntax with the flag off, and
// addSyntaxErrorsToListener's explanation.
#[test]
fn bzl_type_annotations_are_rejected_without_the_flag() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--noexperimental_starlark_type_syntax"]);
    t.scratch_file("pkg/defs.bzl", &["def f(x: int):", "    return x"]);
    t.scratch_file("pkg/BUILD", &["load(':defs.bzl', 'f')"]);
    let (result, events) = t.evaluate_package("pkg");
    assert!(result.has_error());
    let messages = crate::tests::package_loading_test_case::messages(&events);
    assert!(
        messages.iter().any(|m| m.contains(
            "Type annotations syntax can be enabled with --experimental_starlark_type_syntax \
             and/or --experimental_starlark_types_allowed_paths."
        )),
        "{messages:#?}"
    );
}

// S1-1. Java: shouldUseTypeSyntax's allowlist (`key.label.getCanonicalForm().startsWith(s)`).
#[test]
fn bzl_type_annotations_follow_the_allowed_paths() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_starlark_types_allowed_paths=//typed"]);
    t.scratch_file("typed/BUILD", &[]);
    t.scratch_file("typed/defs.bzl", &["def f(x: int):", "    return x"]);
    t.scratch_file("untyped/BUILD", &[]);
    t.scratch_file("untyped/defs.bzl", &["def f(x: int):", "    return x"]);
    t.scratch_file("ok/BUILD", &["load('//typed:defs.bzl', 'f')"]);
    t.scratch_file("bad/BUILD", &["load('//untyped:defs.bzl', 'f')"]);
    t.get_package_without_errors("ok");
    let (result, _) = t.evaluate_package("bad");
    assert!(result.has_error());
}

// S1-3. Java: SkyframeExecutor.java:3060 setDeletedPackages(packageOptions.getDeletedPackages())
// and PackageLookupFunction / PackageFunction's subpackage check: with `a/b` deleted,
// `a/b/BUILD` is an ordinary file of package `a`.
#[test]
fn deleted_packages_are_not_packages() {
    let mut t = PackageLoadingTestCase::new();
    t.scratch_file("a/BUILD", &["exports_files(['b/BUILD', 'b/x.txt'])"]);
    t.scratch_file("a/b/BUILD", &["filegroup(name = 'inner')"]);
    t.scratch_file("a/b/x.txt", &[]);

    // Without the flag `a/b` is a package and the labels cross its boundary.
    let pkg = t.get_package("a");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event("is invalid because 'a/b' is a subpackage");

    t.set_package_options(|o| {
        o.deleted_packages = vec![Foreign(PackageIdentifier::create_in_main_repo_str("a/b"))];
    });
    let pkg = t.get_package_without_errors("a");
    pkg.target("b/BUILD");
    pkg.target("b/x.txt");
    let ex = t.get_package_exception("a/b");
    assert!(
        ex.get_message()
            .contains("Package is considered deleted due to --deleted_packages"),
        "{}",
        ex.get_message()
    );
}

// S1-3. Java: SkyframeExecutor.java:1712 setDefaultVisibility(packageOptions.defaultVisibility).
#[test]
fn default_visibility_option_sets_the_package_default() {
    use blaze_packages::package::RuleVisibility;

    let mut t = PackageLoadingTestCase::new();
    t.scratch_file("pkg/BUILD", &["filegroup(name = 'x')"]);
    let pkg = t.get_package_without_errors("pkg");
    assert_eq!(pkg.target("x").get_visibility(), RuleVisibility::Private);

    t.set_package_options(|o| {
        o.default_visibility = blaze_optdefs::command::rule_visibility::RuleVisibility::PUBLIC;
    });
    let pkg = t.get_package_without_errors("pkg");
    assert_eq!(pkg.target("x").get_visibility(), RuleVisibility::Public);
}

// S1-3. Java: BlazeWorkspace.getOrCreatePackageLocatorForCommand / PathPackageLocator.create:
// `--package_path` entries in order, `%workspace%` replaced, missing entries dropped.
#[test]
fn package_path_option_sets_the_package_locator() {
    let mut t = PackageLoadingTestCase::new();
    t.scratch_file("pkg/BUILD", &["filegroup(name = 'from_workspace')"]);
    t.setup.scratch.file(
        "other_root/pkg/BUILD",
        "filegroup(name = 'from_other_root')",
    );
    t.setup
        .scratch
        .file("other_root/only_here/BUILD", "filegroup(name = 'y')");
    let other_root = t.setup.scratch.path("other_root");

    t.set_package_options(|o| {
        o.package_path = vec![
            "%workspace%/does_not_exist".to_owned(),
            other_root.get_path_string().to_owned(),
            "%workspace%".to_owned(),
        ];
    });
    assert_eq!(
        t.setup.config.pkg_locator.read().get_path_entries().len(),
        2
    );
    // The first entry that has the package wins.
    t.get_package_without_errors("pkg")
        .target("from_other_root");
    t.get_package_without_errors("only_here").target("y");
}

// S1-5. Java: SelectorList.java:273 `@Param(name = "no_match_error", defaultValue = "''",
// named = true)`: positional-or-named.
#[test]
fn select_accepts_a_positional_no_match_error() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/BUILD",
        &[
            "filegroup(name = 'x', srcs = select({'//conditions:default': []}, 'no match'))",
            "filegroup(name = 'y', srcs = select({'//conditions:default': []}, no_match_error = 'no match'))",
        ],
    );
    let pkg = t.get_package_without_errors("pkg");
    pkg.target("x");
    pkg.target("y");
}

// S1-4. Java: RuleClass.java:2158 checkAspectAllowedValues with
// StarlarkDefinedAspect.java:299 getDefaultParametersExtractor: (a) a rule attribute with a
// select() adds no parameter, the aspect's default is checked and allowed; (b) a non-configurable
// rule attribute of the same name and type is the aspect's value even when the aspect attribute
// has a default.
#[test]
fn aspect_allowed_values_use_the_default_parameters_extractor() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/defs.bzl",
        &[
            "def _a(target, ctx):",
            "    return []",
            "my_aspect = aspect(_a, attrs = {'mode': attr.string(default = 'a', values = ['a', 'b'])})",
            "def _r(ctx):",
            "    pass",
            "r = rule(_r, attrs = {'deps': attr.label_list(aspects = [my_aspect]), 'mode': attr.string()})",
        ],
    );
    t.scratch_file("c/BUILD", &[]);
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(':defs.bzl', 'r')",
            "r(name = 'y', mode = select({'//c:1': 'a', '//c:2': 'b'}))",
            "r(name = 'ok', mode = 'b')",
        ],
    );
    t.get_package_without_errors("pkg");

    t.scratch_file(
        "bad/BUILD",
        &["load('//pkg:defs.bzl', 'r')", "r(name = 'x', mode = 'c')"],
    );
    let pkg = t.get_package("bad");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(
        "//bad:x: invalid value in 'mode' attribute: has to be one of 'a' or 'b' instead of 'c'",
    );
}

// S1-4. Java: RuleClass.java:2166-2177: the select() message is reported only when the checked
// value (here the aspect's own default, which no rule value replaced) is not allowed.
#[test]
fn aspect_allowed_values_report_select_only_when_the_value_is_not_allowed() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/defs.bzl",
        &[
            "def _a(target, ctx):",
            "    return []",
            "my_aspect = aspect(_a, attrs = {'mode': attr.string(values = ['a', 'b'])})",
            "def _r(ctx):",
            "    pass",
            "r = rule(_r, attrs = {'deps': attr.label_list(aspects = [my_aspect]), 'mode': attr.string()})",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(':defs.bzl', 'r')",
            "r(name = 'y', mode = select({'//c:1': 'a', '//c:2': 'b'}))",
        ],
    );
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(
        "//pkg:y: attribute 'mode' has a select() and aspect //pkg:defs.bzl%my_aspect also \
         declares '//pkg:y'. Aspect attributes don't currently support select().",
    );
}
