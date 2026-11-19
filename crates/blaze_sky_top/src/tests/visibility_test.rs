//! Port of `src/test/java/com/google/devtools/build/lib/analysis/VisibilityTest.java` (an
//! `AnalysisTestCase`).

use crate::testing::{AnalysisTestCase, BuildViewTestCase};

fn new_test() -> AnalysisTestCase {
    BuildViewTestCase::analysis_test_case()
}

/// `hasErrors(getConfiguredTarget(label))` after a successful `update`.
fn assert_no_errors(t: &mut AnalysisTestCase, label: &str) {
    let ct = t.get_configured_target(label);
    assert!(
        !BuildViewTestCase::has_errors(ct.as_ref()),
        "{label} has errors"
    );
}

// Java: VisibilityTest.setupArgsScenario
fn setup_args_scenario(t: &AnalysisTestCase) {
    t.scratch.file(
        "tool/tool.sh",
        &["#!/bin/sh", "echo Hello > $2", "cat $1 >> $2"],
    );
    t.scratch.file("rule/BUILD", &[]);
    t.scratch.file(
        "rule/rule.bzl",
        &[r#"def _impl(ctx):
    output = ctx.actions.declare_file(ctx.label.name + ".out")
    ctx.actions.run(
        inputs = ctx.files._tool + ctx.files.data,
        executable = ctx.files._tool[0].path,
        arguments = [f.path for f in ctx.files.data] + [output.path],
        outputs = [output],
    )

greet = rule(
    implementation = _impl,
    attrs = {
        "data": attr.label(allow_files = True),
        "_tool": attr.label(
            cfg = "exec",
            allow_files = True,
            default = Label("//tool:tool.sh"),
        ),
    },
    outputs = {"out": "%{name}.out"},
)
"#],
    );
    t.scratch.file("data/data.txt", &["World"]);
    t.scratch.file(
        "use/BUILD",
        &[r#"load("//rule:rule.bzl", "greet")

greet(
    name = "world",
    data = "//data:data.txt",
)
"#],
    );
}

// Java: VisibilityTest.testToolVisibilityRuleCheckAtRule
#[test]
fn test_tool_visibility_rule_check_at_rule() {
    let mut t = new_test();
    setup_args_scenario(&t);
    t.scratch.file(
        "data/BUILD",
        &["exports_files(['data.txt'], visibility=['//visibility:public'])"],
    );
    t.scratch.file(
        "tool/BUILD",
        &["exports_files(['tool.sh'], visibility=['//rule:__pkg__'])"],
    );
    t.update(&["//use:world"]).unwrap();
    assert_no_errors(&mut t, "//use:world");
}

// Java: VisibilityTest.testToolVisibilityUseCheckAtUse
#[test]
fn test_tool_visibility_use_check_at_use() {
    let mut t = new_test();
    setup_args_scenario(&t);
    t.scratch.file(
        "data/BUILD",
        &["exports_files(['data.txt'], visibility=['//visibility:public'])"],
    );
    t.scratch.file(
        "tool/BUILD",
        &["exports_files(['tool.sh'], visibility=['//use:__pkg__'])"],
    );
    t.update(&["//use:world"]).unwrap();
    assert_no_errors(&mut t, "//use:world");
}

// Java: VisibilityTest.testToolVisibilityUseCheckAtRule_fallbackToUse
#[test]
fn test_tool_visibility_use_check_at_rule_fallback_to_use() {
    let mut t = new_test();
    setup_args_scenario(&t);
    t.scratch.file(
        "data/BUILD",
        &["exports_files(['data.txt'], visibility=['//visibility:public'])"],
    );
    t.scratch.file(
        "tool/BUILD",
        &["exports_files(['tool.sh'], visibility=['//use:__pkg__'])"],
    );

    t.update(&["//use:world"]).unwrap();

    assert_no_errors(&mut t, "//use:world");
}

// Java: VisibilityTest.testToolVisibilityPrivateCheckAtUse
#[test]
fn test_tool_visibility_private_check_at_use() {
    let mut t = new_test();
    setup_args_scenario(&t);
    t.scratch.file(
        "data/BUILD",
        &["exports_files(['data.txt'], visibility=['//visibility:public'])"],
    );
    t.scratch.file(
        "tool/BUILD",
        &["exports_files(['tool.sh'], visibility=['//visibility:private'])"],
    );

    t.remove_fail_fast_handler();
    assert!(t.update(&["//use:world"]).is_err());
}

// Java: VisibilityTest.testToolVisibilityPrivate
#[test]
fn test_tool_visibility_private() {
    let mut t = new_test();
    setup_args_scenario(&t);
    t.scratch.file(
        "data/BUILD",
        &["exports_files(['data.txt'], visibility=['//visibility:public'])"],
    );
    t.scratch.file(
        "tool/BUILD",
        &["exports_files(['tool.sh'], visibility=['//visibility:private'])"],
    );

    t.remove_fail_fast_handler();
    assert!(t.update(&["//use:world"]).is_err());
}

// Java: VisibilityTest.testDataVisibilityUseCheckPrivateAtRule
#[test]
fn test_data_visibility_use_check_private_at_rule() {
    let mut t = new_test();
    setup_args_scenario(&t);
    t.scratch.file(
        "data/BUILD",
        &["exports_files(['data.txt'], visibility=['//use:__pkg__'])"],
    );
    t.scratch.file(
        "tool/BUILD",
        &["exports_files(['tool.sh'], visibility=['//visibility:public'])"],
    );
    t.update(&["//use:world"]).unwrap();
    assert_no_errors(&mut t, "//use:world");
}

// Java: VisibilityTest.testDataVisibilityPrivate
#[test]
fn test_data_visibility_private() {
    let mut t = new_test();
    setup_args_scenario(&t);
    t.scratch.file(
        "data/BUILD",
        &["exports_files(['data.txt'], visibility=['//visibility:private'])"],
    );
    t.scratch.file(
        "tool/BUILD",
        &["exports_files(['tool.sh'], visibility=['//visibility:public'])"],
    );

    t.remove_fail_fast_handler();
    assert!(t.update(&["//use:world"]).is_err());
}

// Java: VisibilityTest.testConfigSettingVisibilityAlwaysCheckedAtUse
#[test]
fn test_config_setting_visibility_always_checked_at_use() {
    let mut t = new_test();
    t.scratch.file(
        "BUILD",
        &[
            "load('//build_defs:defs.bzl', 'my_rule')",
            "my_rule(",
            "    name = 'my_target',",
            "    value = select({",
            "        '//config_setting:my_setting': 'foo',",
            "        '//conditions:default': 'bar',",
            "    }),",
            ")",
        ],
    );
    t.scratch.file("build_defs/BUILD", &[]);
    t.scratch.file(
        "build_defs/defs.bzl",
        &[r#"def _my_rule_impl(ctx):
    pass

my_rule = rule(
    implementation = _my_rule_impl,
    attrs = {
        "value": attr.string(mandatory = True),
    },
)
"#],
    );
    t.scratch.file(
        "config_setting/BUILD",
        &[r#"config_setting(
    name = "my_setting",
    values = {"compilation_mode": "dbg"},
    visibility = ["//:__pkg__"],
)
"#],
    );

    t.update(&["//:my_target"]).unwrap();
    assert_no_errors(&mut t, "//:my_target");
}

// Java: VisibilityTest.testImplicitDependency_samePackageAsDefinition_visible
#[test]
#[ignore = "needs aspects (attr.label(aspects = [...]); the aspects wave ports them)"]
fn test_implicit_dependency_same_package_as_definition_visible() {}

// Java: VisibilityTest.testAspectImplicitDependencyCheckedAtDefinition_visible
#[test]
#[ignore = "needs aspects (attr.label(aspects = [...]); the aspects wave ports them)"]
fn test_aspect_implicit_dependency_checked_at_definition_visible() {}

// Java: VisibilityTest.testAspectImplicitDependencyCheckedAtDefinition_visibleWithNameCollision
#[test]
#[ignore = "needs aspects (attr.label(aspects = [...]); the aspects wave ports them)"]
fn test_aspect_implicit_dependency_checked_at_definition_visible_with_name_collision() {}

// Java: VisibilityTest.testAspectImplicitDependencyCheckedAtDefinition_outerAspectToolNotVisible
#[test]
#[ignore = "needs aspects (attr.label(aspects = [...]); the aspects wave ports them)"]
fn test_aspect_implicit_dependency_checked_at_definition_outer_aspect_tool_not_visible() {}

// Java: VisibilityTest.testAspectImplicitDependencyCheckedAtDefinition_innerAspectToolNotVisible
#[test]
#[ignore = "needs aspects (attr.label(aspects = [...]); the aspects wave ports them)"]
fn test_aspect_implicit_dependency_checked_at_definition_inner_aspect_tool_not_visible() {}

// Java: VisibilityTest.testAspectImplicitDependencyCheckedAtDefinition_ruleToolNotVisible
#[test]
#[ignore = "needs aspects (attr.label(aspects = [...]); the aspects wave ports them)"]
fn test_aspect_implicit_dependency_checked_at_definition_rule_tool_not_visible() {}

// Java: VisibilityTest.setupFilesScenario
fn setup_files_scenario(t: &AnalysisTestCase, want_read: &str) {
    t.scratch.file("src/source.txt", &["source"]);
    t.scratch.file(
        "src/BUILD",
        &["exports_files(['source.txt'], visibility=['//pkg:__pkg__'])"],
    );
    t.scratch.file("pkg/foo.txt", &["foo"]);
    t.scratch.file("pkg/bar.txt", &["bar"]);
    t.scratch.file("pkg/groupfile.txt", &["groupfile"]);
    t.scratch.file("pkg/unused.txt", &["unused"]);
    t.scratch.file("pkg/exported.txt", &["exported"]);
    t.scratch.file(
        "pkg/BUILD",
        &[r#"package(default_visibility = ["//visibility:public"])

exports_files(["exported.txt"])

genrule(
    name = "foobar",
    srcs = [
        "foo.txt",
        "bar.txt",
    ],
    outs = ["foobar.txt"],
    cmd = "cat $(SRCS) > $@",
)

filegroup(
    name = "remotegroup",
    srcs = ["//src:source.txt"],
)

filegroup(
    name = "localgroup",
    srcs = [":groupfile.txt"],
)
"#],
    );
    t.scratch.file(
        "otherpkg/BUILD",
        &[
            "genrule(",
            "  name = 'it',",
            &format!("  srcs = ['//pkg:{want_read}'],"),
            "  outs = ['it.xt'],",
            "  cmd = 'cp $< $@',",
            ")",
        ],
    );
}

/// The body of the file scenario tests that expect success.
fn files_scenario_succeeds(want_read: &str, flag: &str) {
    let mut t = new_test();
    setup_files_scenario(&t, want_read);
    t.use_configuration(&[flag]);
    t.update(&["//otherpkg:it"]).unwrap();
    assert_no_errors(&mut t, "//otherpkg:it");
}

/// The body of the file scenario tests that expect a `ViewCreationFailedException`.
fn files_scenario_fails(want_read: &str, flag: &str) {
    let mut t = new_test();
    setup_files_scenario(&t, want_read);
    t.use_configuration(&[flag]);

    t.remove_fail_fast_handler();
    assert!(t.update(&["//otherpkg:it"]).is_err());
}

// Java: VisibilityTest.testTargetImplicitExport
#[test]
fn test_target_implicit_export() {
    files_scenario_succeeds("foobar", "--noincompatible_no_implicit_file_export");
}

// Java: VisibilityTest.testTargetNoImplicitExport
#[test]
fn test_target_no_implicit_export() {
    files_scenario_succeeds("foobar", "--incompatible_no_implicit_file_export");
}

// Java: VisibilityTest.testLocalFilegroupImplicitExport
#[test]
fn test_local_filegroup_implicit_export() {
    files_scenario_succeeds("localgroup", "--noincompatible_no_implicit_file_export");
}

// Java: VisibilityTest.testLocalFilegroupNoImplicitExport
#[test]
fn test_local_filegroup_no_implicit_export() {
    files_scenario_succeeds("localgroup", "--incompatible_no_implicit_file_export");
}

// Java: VisibilityTest.testRemoteFilegroupImplicitExport
#[test]
fn test_remote_filegroup_implicit_export() {
    files_scenario_succeeds("remotegroup", "--noincompatible_no_implicit_file_export");
}

// Java: VisibilityTest.testRemoteFilegroupNoImplicitExport
#[test]
fn test_remote_filegroup_no_implicit_export() {
    files_scenario_succeeds("remotegroup", "--incompatible_no_implicit_file_export");
}

// Java: VisibilityTest.testExportedImplicitExport
#[test]
fn test_exported_implicit_export() {
    files_scenario_succeeds("exported.txt", "--noincompatible_no_implicit_file_export");
}

// Java: VisibilityTest.testExportedNoImplicitExport
#[test]
fn test_exported_no_implicit_export() {
    files_scenario_succeeds("exported.txt", "--incompatible_no_implicit_file_export");
}

// Java: VisibilityTest.testUnusedImplicitExport
#[test]
fn test_unused_implicit_export() {
    files_scenario_fails("unused.txt", "--noincompatible_no_implicit_file_export");
}

// Java: VisibilityTest.testUnusedNoImplicitExport
#[test]
fn test_unused_no_implicit_export() {
    files_scenario_fails("unused.txt", "--incompatible_no_implicit_file_export");
}

// Java: VisibilityTest.testSourcefileImplicitExport
#[test]
fn test_sourcefile_implicit_export() {
    files_scenario_succeeds("foo.txt", "--noincompatible_no_implicit_file_export");
}

// Java: VisibilityTest.testSourcefileNoImplicitExport
#[test]
fn test_sourcefile_no_implicit_export() {
    files_scenario_fails("foo.txt", "--incompatible_no_implicit_file_export");
}

// Java: VisibilityTest.testVerboseDiagnostics_ruleImplicitDep
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_rule_implicit_dep() {}

// Java: VisibilityTest.testVerboseDiagnostics_aspectImplicitDep
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_aspect_implicit_dep() {}

// Java: VisibilityTest.testVerboseDiagnostics_consumingLocation_isNotInMacro
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_consuming_location_is_not_in_macro() {}

// Java: VisibilityTest.testVerboseDiagnostics_consumingLocation_isInMacro
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_consuming_location_is_in_macro() {}

// Java: VisibilityTest.testVerboseDiagnostics_consumingLocation_isDelegatedFromPackage
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_consuming_location_is_delegated_from_package() {}

// Java: VisibilityTest.testVerboseDiagnostics_consumingLocation_isDelegatedFromMacro
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_consuming_location_is_delegated_from_macro() {}

// Java: VisibilityTest.testVerboseDiagnostics_aliasDisclaimer_shownForAlias
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_alias_disclaimer_shown_for_alias() {}

// Java: VisibilityTest.testVerboseDiagnostics_aliasDisclaimer_notShownForNonAlias
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_alias_disclaimer_not_shown_for_non_alias() {}

// Java: VisibilityTest.testVerboseDiagnostics_samePackageDisclaimer
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_same_package_disclaimer() {}

// Java: VisibilityTest.testVerboseDiagnostics_samePackageDisclaimer_shownForImplicitDepOfMacro
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_same_package_disclaimer_shown_for_implicit_dep_of_macro() {}

// Java: VisibilityTest.testVerboseDiagnostics_samePackageDisclaimer_notShownForImplicitDepOfRule
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_same_package_disclaimer_not_shown_for_implicit_dep_of_rule() {}

// Java: VisibilityTest.testVerboseDiagnostics_moreDelegationNeeded_fromAncestorMacro
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_more_delegation_needed_from_ancestor_macro() {}

// Java: VisibilityTest.testVerboseDiagnostics_moreDelegationNeeded_fromBuildFile
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_more_delegation_needed_from_build_file() {}

// Java: VisibilityTest.testVerboseDiagnostics_moreDelegationNeeded_incompleteDelegation
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_more_delegation_needed_incomplete_delegation() {}

// Java: VisibilityTest.testVerboseDiagnostics_editVisibilitySuggestion_forRuleTarget
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_edit_visibility_suggestion_for_rule_target() {}

// Java: VisibilityTest.testVerboseDiagnostics_editVisibilitySuggestion_forFileTarget
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_verbose_diagnostics_edit_visibility_suggestion_for_file_target() {}
