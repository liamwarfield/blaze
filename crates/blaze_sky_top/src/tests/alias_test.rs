//! Port of `src/test/java/com/google/devtools/build/lib/rules/AliasTest.java`.

use crate::testing::{BuildViewTestCase, assert_contains_exactly, base_artifact_names};
use blaze_analysis::providers::file_provider::{FileProvider, RunfilesProvider};

// Java: AliasTest.smoke
#[test]
fn smoke() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "a",
    srcs = ["a.cc"],
)

alias(
    name = "b",
    actual = "a",
)
"#],
    );

    let b = t.get_configured_target("//a:b").expect("//a:b");
    let cc_info = cc_info_get(&b).expect("CcInfo");
    // assertThat(CcInfo.get(b).getCcCompilationContext()).isNotNull()
    starlark::values::Heap::temp(|heap| {
        // SAFETY: the value is used only while `cc_info` (which keeps its frozen heap alive) is
        // borrowed.
        let compilation_context = unsafe { cc_info.unchecked_frozen_value() }
            .to_value()
            .get_attr("compilation_context", heap)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(compilation_context.is_some_and(|v| !v.is_none()));
    });
}

/// The test's `CcInfo.get(target)` (rules/cpp/CcInfo.java in src/test/java): the
/// `@_builtins` CcInfo, else the `@rules_cc+` one.
fn cc_info_get(
    target: &blaze_analysis::configured_target::ConfiguredTarget,
) -> Option<starlark::values::OwnedFrozenValue> {
    let builtins = blaze_packages::rule_class::ProviderKey::Starlark {
        extension_label: blaze_cmdline::Label::parse_canonical_unchecked(
            "@_builtins//:common/cc/cc_info.bzl",
        ),
        exported_name: "CcInfo".to_string(),
    };
    target.get(&builtins).or_else(|| {
        BuildViewTestCase::get_starlark_provider(
            target,
            "@rules_cc+//cc/private:cc_info.bzl",
            "CcInfo",
        )
    })
}

// Java: AliasTest.aliasToInputFile
#[test]
fn alias_to_input_file() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &["exports_files([\"a\"])\n\nalias(\n    name = \"b\",\n    actual = \"a\",\n)\n"],
    );
    let b = t.get_configured_target("//a:b").unwrap();
    assert_contains_exactly(
        &base_artifact_names(crate::testing::get_files_to_build(&b).to_list().iter()),
        &["a"],
    );
}

// Java: AliasTest.visibilityIsOverriddenAndIsOkay
#[test]
fn visibility_is_overridden_and_is_okay() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &["filegroup(name='a', visibility=['//b:__pkg__'])"],
    );
    t.scratch.file(
        "b/BUILD",
        &["alias(name='b', actual='//a:a', visibility=['//visibility:public'])"],
    );
    t.scratch
        .file("c/BUILD", &["filegroup(name='c', srcs=['//b:b'])"]);
    t.get_configured_target("//c:c");
}

// Java: AliasTest.visibilityIsOverriddenAndIsError
#[test]
fn visibility_is_overridden_and_is_error() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &["filegroup(name='a', visibility=['//visibility:public'])"],
    );
    t.scratch.file(
        "b/BUILD",
        &["alias(name='b', actual='//a:a', visibility=['//visibility:private'])"],
    );
    t.scratch
        .file("c/BUILD", &["filegroup(name='c', srcs=['//b:b'])"]);

    t.remove_fail_fast_handler();
    t.get_configured_target("//c:c");
    t.assert_contains_event(
        "alias '//b:b' referring to target '//a:a' is not visible from\ntarget '//c:c'",
    );
}

// Java: AliasTest.visibilityIsOverriddenAndIsErrorAfterMultipleAliases
#[test]
fn visibility_is_overridden_and_is_error_after_multiple_aliases() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &["filegroup(name='a', visibility=['//visibility:public'])"],
    );
    t.scratch.file(
        "b/BUILD",
        &["alias(name='b', actual='//a:a', visibility=['//visibility:public'])"],
    );
    t.scratch.file(
        "c/BUILD",
        &["alias(name='c', actual='//b:b', visibility=['//visibility:private'])"],
    );
    t.scratch
        .file("d/BUILD", &["filegroup(name='d', srcs=['//c:c'])"]);

    t.remove_fail_fast_handler();
    t.get_configured_target("//d:d");
    t.assert_contains_event(
        "alias '//c:c' referring to target '//a:a' through '//b:b' is not visible from\ntarget '//d:d'",
    );
}

// Java: AliasTest.testAliasWithPrivateVisibilityAccessibleFromSamePackage
#[test]
fn test_alias_with_private_visibility_accessible_from_same_package() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file("a/BUILD", &["exports_files(['af'])"]);
    t.scratch.file(
        "b/BUILD",
        &[r#"package(default_visibility = ["//visibility:private"])

alias(
    name = "al",
    actual = "//a:af",
)

filegroup(
    name = "ta",
    srcs = [":al"],
)
"#],
    );
    t.get_configured_target("//b:ta");
}

// Java: AliasTest.testAliasCycle
#[test]
fn test_alias_cycle() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[r#"alias(
    name = "a",
    actual = ":b",
)

alias(
    name = "b",
    actual = ":c",
)

alias(
    name = "c",
    actual = ":a",
)

filegroup(
    name = "d",
    srcs = [":c"],
)
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//a:d");
    t.assert_contains_event("cycle in dependency graph");
}

// Java: AliasTest.testAliasedInvalidDependency
#[test]
fn test_aliased_invalid_dependency() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "a",
    deps = [":b"],
)

alias(
    name = "b",
    actual = ":c",
)

filegroup(name = "c")
"#],
    );

    t.remove_fail_fast_handler();
    t.get_configured_target("//a:a");
    t.assert_contains_event("alias '//a:b' referring to filegroup rule '//a:c' is misplaced here");
}

// Java: AliasTest.testAspectPropagation
#[test]
#[ignore = "not ported yet: needs writeConfigTransitionTestFiles and top-level aspects (BuildView.createTopLevelAspectKeys)"]
fn test_aspect_propagation() {}

// Java: AliasTest.assertNoLicensesAttribute
#[test]
fn assert_no_licenses_attribute() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[r#"filegroup(name = "a")

alias(
    name = "b",
    actual = ":a",
    licenses = ["unencumbered"],
)
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//a:b");
    t.assert_contains_event("no such attribute 'licenses' in 'alias' rule");
}

// Java: AliasTest.passesTargetTypeCheck
#[test]
fn passes_target_type_check() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "a",
    srcs = ["a.cc"],
    deps = [":b"],
)

alias(
    name = "b",
    actual = ":c",
)

cc_library(
    name = "c",
    srcs = ["c.cc"],
)
"#],
    );

    t.get_configured_target("//a:a");
}

// Java: AliasTest.packageGroupInAlias
#[test]
fn package_group_in_alias() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[r#"package_group(
    name = "a",
    packages = ["//a"],
)

alias(
    name = "b",
    actual = ":a",
)

filegroup(
    name = "c",
    srcs = [":b"],
)
"#],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//a:c");
    t.assert_contains_event(
        "in actual attribute of alias rule //a:b: package group '//a:a' is misplaced here",
    );
}

// Java: AliasTest.aliasedFile
#[test]
fn aliased_file() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[r#"exports_files(["a"])

alias(
    name = "b",
    actual = "a",
)

filegroup(
    name = "c",
    srcs = [":b"],
)
"#],
    );
    let c = t.get_configured_target("//a:c").unwrap();
    assert_contains_exactly(
        &base_artifact_names(
            c.get_provider::<FileProvider>()
                .unwrap()
                .get_files_to_build()
                .to_list()
                .iter(),
        ),
        &["a"],
    );
}

// Java: AliasTest.aliasedConfigSetting
#[test]
fn aliased_config_setting() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[r#"filegroup(
    name = "a",
    srcs = select({
        ":b": ["f1"],
        "//conditions:default": ["f2"],
    }),
)

alias(
    name = "b",
    actual = ":c",
)

config_setting(
    name = "c",
    values = {"define": "foo=bar"},
)
"#],
    );
    t.use_configuration(&["--define=foo=bar"]);
    t.get_configured_target("//a");
}

// Java: AliasTest.aliasedTestSuiteDep
#[test]
fn aliased_test_suite_dep() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[
            "load('//test_defs:foo_test.bzl', 'foo_test')",
            "foo_test(name='a', srcs=['a.sh'])",
        ],
    );
    t.scratch.file(
        "b/BUILD",
        &[r#"alias(
    name = "b",
    testonly = 1,
    actual = "//a:a",
)

test_suite(
    name = "c",
    tests = [":b"],
)
"#],
    );
    let c = t.get_configured_target("//b:c").unwrap();
    let runfiles = c
        .get_provider::<RunfilesProvider>()
        .unwrap()
        .get_data_runfiles()
        .get_all_artifacts();
    assert!(
        base_artifact_names(runfiles.to_list().iter()).contains(&"a.sh".to_string()),
        "{:?}",
        base_artifact_names(runfiles.to_list().iter())
    );
}

// Java: AliasTest.testRedirectChasing
#[test]
fn test_redirect_chasing() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &[
            "load('@rules_cc//cc:cc_library.bzl', 'cc_library')",
            &format!(
                "alias(name='cc', actual='{}')",
                crate::testing::analysis_mock::PLATFORM_LABEL
            ),
            "cc_library(name='a', srcs=['a.cc'])",
        ],
    );

    t.use_configuration(&["--platforms=//a:cc"]);
    t.get_configured_target("//a:a");
}

// Java: AliasTest.testNoActual
#[test]
fn test_no_actual() {
    let mut t = BuildViewTestCase::new();
    t.check_error(
        "a",
        "a",
        "missing value for mandatory attribute 'actual'",
        &["alias(name='a')"],
    );
}
