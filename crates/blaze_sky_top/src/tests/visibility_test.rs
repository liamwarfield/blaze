//! Port of `src/test/java/com/google/devtools/build/lib/analysis/VisibilityTest.java` (an
//! `AnalysisTestCase`).

use crate::testing::UpdateError;
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
fn test_verbose_diagnostics_rule_implicit_dep() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file(
        "tool/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "tool",
    visibility = ["//visibility:private"],
)
"#],
    );
    t.scratch.file("rule/BUILD", &[]);
    t.scratch.file(
        "rule/defs.bzl",
        &[r#"def _impl(ctx):
    pass

my_rule = rule(
    implementation = _impl,
    attrs = {"_implicit_dep": attr.label(default="//tool:tool")},
)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("//rule:defs.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event("* The dependency is an implicit dependency of the consuming target's rule, my_rule, which is defined in //rule:defs.bzl. Since that file's package, //rule, does not match");
}

// Java: VisibilityTest.testVerboseDiagnostics_aspectImplicitDep
#[test]
fn test_verbose_diagnostics_aspect_implicit_dep() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file(
        "tool/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "tool",
    visibility = ["//visibility:private"],
)
"#],
    );
    t.scratch.file("aspect/BUILD", &[]);
    t.scratch.file(
        "aspect/defs.bzl",
        &[r#"def _impl(ctx):
    pass

my_aspect = aspect(
    implementation = _impl,
    attrs = {"_implicit_dep": attr.label(default="//tool:tool")},
)
"#],
    );
    t.scratch.file("rule/BUILD", &[]);
    t.scratch.file(
        "rule/defs.bzl",
        &[r#"load("//aspect:defs.bzl", "my_aspect")

def _impl(ctx):
    pass

my_rule = rule(
    implementation = _impl,
    attrs = {"deps": attr.label_list(aspects=[my_aspect])},
)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
load("//rule:defs.bzl", "my_rule")

cc_library(name = "dep")

my_rule(
    name = "foo",
    deps = [":dep"],
)
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event("* The dependency is an implicit dependency of the consuming target's aspect, my_aspect, which is defined in //aspect:defs.bzl. Since that file's package, //aspect, does not");
}

// Java: VisibilityTest.testVerboseDiagnostics_consumingLocation_isNotInMacro
#[test]
fn test_verbose_diagnostics_consuming_location_is_not_in_macro() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file(
        "tool/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "tool",
    visibility = ["//visibility:private"],
)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "foo",
    deps = ["//tool:tool"],
)
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event(
        "* The location being checked is the package where the consuming target lives, //pkg.",
    );
}

// Java: VisibilityTest.testVerboseDiagnostics_consumingLocation_isInMacro
#[test]
fn test_verbose_diagnostics_consuming_location_is_in_macro() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file(
        "tool/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "tool",
    visibility = ["//visibility:private"],
)
"#],
    );
    t.scratch.file("macro/BUILD", &[]);
    t.scratch.file(
        "macro/defs.bzl",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
def _impl(name, visibility):
    cc_library(
        name = name + "_bar",
        deps = ["//tool:tool"],
    )

my_macro = macro(implementation = _impl)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("//macro:defs.bzl", "my_macro")

my_macro(name = "foo")
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo_bar"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event("* Because the consuming target was declared in the body of the symbolic macro my_macro defined in //macro:defs.bzl, the location being checked is this file's package, //macro.");
}

// Java: VisibilityTest.testVerboseDiagnostics_consumingLocation_isDelegatedFromPackage
#[test]
fn test_verbose_diagnostics_consuming_location_is_delegated_from_package() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file(
        "tool/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "tool",
    visibility = ["//visibility:private"],
)
"#],
    );
    t.scratch.file("submacro/BUILD", &[]);
    t.scratch.file(
        "submacro/defs.bzl",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
def _impl(name, visibility, deps):
    cc_library(
        name = name + "_baz",
        deps = deps,
    )

my_submacro = macro(
    implementation = _impl,
    attrs = {"deps": attr.label_list()},
)
"#],
    );
    t.scratch.file("macro/BUILD", &[]);
    t.scratch.file(
        "macro/defs.bzl",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
load("//submacro:defs.bzl", "my_submacro")

def _impl(name, visibility, deps, use_submacro):
    callable = my_submacro if use_submacro else cc_library
    callable(
        name = name + "_bar",
        deps = deps,
    )

my_macro = macro(
    implementation = _impl,
    attrs = {
        "deps": attr.label_list(),
        "use_submacro": attr.bool(configurable=False),
    },
)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("//macro:defs.bzl", "my_macro")

my_macro(
    name = "foo",
    deps = ["//tool:tool"],
    use_submacro = False,
)

my_macro(
    name = "foo2",
    deps = ["//tool:tool"],
    use_submacro = True,
)
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo_bar"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event("* Because the dependency was passed to the consuming target from an attribute of the symbolic macro //pkg:foo, the location being checked is the place where this macro is declared: package //pkg.");
    t.event_collector.clear();
    assert!(matches!(
        t.update(&["//pkg:foo2_bar_baz"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    // Should say "transitive", and should still identify outer macro (foo2), not inner macro
    // (foo2_bar).
    t.assert_contains_event("* Because the dependency was transitively passed to the consuming target from an attribute of the symbolic macro //pkg:foo2, the location being checked is the place where this macro is declared: package //pkg.");
}

// Java: VisibilityTest.testVerboseDiagnostics_consumingLocation_isDelegatedFromMacro
#[test]
fn test_verbose_diagnostics_consuming_location_is_delegated_from_macro() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file(
        "tool/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "tool",
    visibility = ["//visibility:private"],
)
"#],
    );
    t.scratch.file("subsubmacro/BUILD", &[]);
    t.scratch.file(
        "subsubmacro/defs.bzl",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
def _impl(name, visibility, deps):
    cc_library(
        name = name + "_qux",
        deps = deps,
    )

my_subsubmacro = macro(
    implementation = _impl,
    attrs = {"deps": attr.label_list()},
)
"#],
    );
    t.scratch.file("submacro/BUILD", &[]);
    t.scratch.file(
        "submacro/defs.bzl",
        &[r#"load("//subsubmacro:defs.bzl", "my_subsubmacro")

def _impl(name, visibility, deps):
    my_subsubmacro(
        name = name + "_baz",
        deps = deps,
    )

my_submacro = macro(
    implementation = _impl,
    attrs = {"deps": attr.label_list()},
)
"#],
    );
    t.scratch.file("macro/BUILD", &[]);
    t.scratch.file(
        "macro/defs.bzl",
        &[r#"load("//submacro:defs.bzl", "my_submacro")
load("//subsubmacro:defs.bzl", "my_subsubmacro")

def _impl(name, visibility, extra_level_deep):
    callable = my_submacro if extra_level_deep else my_subsubmacro
    callable(
        name = name + "_bar",
        deps = ["//tool:tool"],
    )

my_macro = macro(
    implementation = _impl,
    attrs = {"extra_level_deep": attr.bool(configurable=False)},
)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("//macro:defs.bzl", "my_macro")

my_macro(
    name = "foo",
    extra_level_deep = False,
)

my_macro(
    name = "foo2",
    extra_level_deep = True,
)
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo_bar_qux"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event("* Because the dependency was passed to the consuming target from an attribute of the symbolic macro //pkg:foo_bar, the location being checked is the place where this macro is declared: the body of the calling macro my_macro, defined in //macro:defs.bzl of package //macro.");
    t.event_collector.clear();
    assert!(matches!(
        t.update(&["//pkg:foo2_bar_baz_qux"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    // Should say "transitive", and should still identify outer macro (foo2_bar), not inner macro
    // (foo2_bar_baz).
    t.assert_contains_event("* Because the dependency was transitively passed to the consuming target from an attribute of the symbolic macro //pkg:foo2_bar, the location being checked is the place where this macro is declared: the body of the calling macro my_macro, defined in //macro:defs.bzl of package //macro.");
}

// Java: VisibilityTest.testVerboseDiagnostics_aliasDisclaimer_shownForAlias
#[test]
fn test_verbose_diagnostics_alias_disclaimer_shown_for_alias() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file(
        "tool/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "tool",
    visibility = ["//pkg:__pkg__"],
)

alias(
    name = "indirect",
    actual = ":tool",
    visibility = ["//visibility:private"],
)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "foo",
    deps = ["//tool:indirect"],
)
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event("* The dependency is an alias target. Note that it is the visibility of the alias we care about, not the visibility of the underlying target it refers to.
");
}

// Java: VisibilityTest.testVerboseDiagnostics_aliasDisclaimer_notShownForNonAlias
#[test]
fn test_verbose_diagnostics_alias_disclaimer_not_shown_for_non_alias() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file(
        "tool/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "tool",
    visibility = ["//visibility:private"],
)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "foo",
    deps = ["//tool:tool"],
)
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_does_not_contain_event("The dependency is an alias");
}

// Java: VisibilityTest.testVerboseDiagnostics_samePackageDisclaimer
#[test]
fn test_verbose_diagnostics_same_package_disclaimer() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file("macro/BUILD", &[]);
    t.scratch.file(
        "macro/defs.bzl",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
def _impl(name, visibility):
    cc_library(
        name = name,
        deps = ["//pkg:tool"],
    )

my_macro = macro(implementation = _impl)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
load("//macro:defs.bzl", "my_macro")

cc_library(
    name = "tool",
    visibility = ["//visibility:private"],
)

my_macro(
    name = "foo",
)
"#],
    );
    t.scratch.file(
        "pkg2/BUILD",
        &[r#"load("//macro:defs.bzl", "my_macro")

my_macro(
    name = "foo",
)
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event("* Although both targets live in the same package, they cannot automatically see each other because they are declared by different symbolic macros.");
    t.event_collector.clear();
    assert!(matches!(
        t.update(&["//pkg2:foo"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_does_not_contain_event("both targets live in the same package");
}

// Java: VisibilityTest.testVerboseDiagnostics_samePackageDisclaimer_shownForImplicitDepOfMacro
#[test]
fn test_verbose_diagnostics_same_package_disclaimer_shown_for_implicit_dep_of_macro() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file("macro/BUILD", &[]);
    t.scratch.file(
        "macro/defs.bzl",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
def _impl(name, visibility, _implicit_dep):
    cc_library(
        name = name,
        deps = [_implicit_dep],
    )

my_macro = macro(
    implementation = _impl,
    attrs = {"_implicit_dep": attr.label(default="//pkg:tool", configurable=False)},
)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
load("//macro:defs.bzl", "my_macro")

cc_library(
    name = "tool",
    visibility = ["//visibility:private"],
)

my_macro(
    name = "foo",
)
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event("* Although both targets live in the same package, they cannot automatically see each other because they are declared by different symbolic macros.");
}

// Java: VisibilityTest.testVerboseDiagnostics_samePackageDisclaimer_notShownForImplicitDepOfRule
#[test]
fn test_verbose_diagnostics_same_package_disclaimer_not_shown_for_implicit_dep_of_rule() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file("rule/BUILD", &[]);
    t.scratch.file(
        "rule/defs.bzl",
        &[r#"def _impl(ctx):
    pass

my_rule = rule(
    implementation = _impl,
    attrs = {"_implicit_dep": attr.label(default="//pkg:tool")},
)
"#],
    );
    t.scratch.file("macro/BUILD", &[]);
    t.scratch.file(
        "macro/defs.bzl",
        &[r#"load("//rule:defs.bzl", "my_rule")

def _impl(name, visibility):
    my_rule(name = name)

my_macro = macro(implementation = _impl)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
load("//macro:defs.bzl", "my_macro")

cc_library(
    name = "tool",
    visibility = ["//visibility:private"],
)

my_macro(name = "foo")
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_does_not_contain_event("both targets live in the same package");
}

// Java: VisibilityTest.testVerboseDiagnostics_moreDelegationNeeded_fromAncestorMacro
#[test]
fn test_verbose_diagnostics_more_delegation_needed_from_ancestor_macro() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file(
        "tool/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "tool",
    visibility = ["//macro:__pkg__"],
)
"#],
    );
    t.scratch.file("subsubmacro/BUILD", &[]);
    t.scratch.file(
        "subsubmacro/defs.bzl",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
def _impl(name, visibility):
    cc_library(
        name = name + "_qux",
        deps = ["//tool:tool"],
    )

my_subsubmacro = macro(implementation = _impl)
"#],
    );
    t.scratch.file("submacro/BUILD", &[]);
    t.scratch.file(
        "submacro/defs.bzl",
        &[r#"load("//subsubmacro:defs.bzl", "my_subsubmacro")

def _impl(name, visibility):
    my_subsubmacro(name = name + "_baz")

my_submacro = macro(implementation = _impl)
"#],
    );
    t.scratch.file("macro/BUILD", &[]);
    t.scratch.file(
        "macro/defs.bzl",
        &[r#"load("//submacro:defs.bzl", "my_submacro")
load("//subsubmacro:defs.bzl", "my_subsubmacro")

def _impl(name, visibility, extra_level_deep):
    callable = my_submacro if extra_level_deep else my_subsubmacro
    callable(name = name + "_bar")

my_macro = macro(
    implementation = _impl,
    attrs = {"extra_level_deep": attr.bool(configurable=False)},
)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("//macro:defs.bzl", "my_macro")

my_macro(
    name = "foo",
    extra_level_deep = False,
)

my_macro(
    name = "foo2",
    extra_level_deep = True,
)
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo_bar_qux"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event("* Although the dependency is not visible to the location being checked, it is visible to this location's caller, //pkg:foo, a my_macro macro defined in //macro. (Perhaps the caller needs to pass in the dependency as an argument?)");
    t.event_collector.clear();
    assert!(matches!(
        t.update(&["//pkg:foo2_bar_baz_qux"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    // Should say "transitive", and should still identify outer macro (foo2_bar), not inner macro
    // (foo2_bar_baz).
    t.assert_contains_event("* Although the dependency is not visible to the location being checked, it is visible to this location's transitive caller, //pkg:foo2, a my_macro macro defined in //macro. (Perhaps this caller, or an intermediate caller, needs to pass in the dependency as an argument?)");
}

// Java: VisibilityTest.testVerboseDiagnostics_moreDelegationNeeded_fromBuildFile
#[test]
fn test_verbose_diagnostics_more_delegation_needed_from_build_file() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file(
        "tool/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "tool",
    visibility = ["//pkg:__pkg__"],
)
"#],
    );
    t.scratch.file("submacro/BUILD", &[]);
    t.scratch.file(
        "submacro/defs.bzl",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
def _impl(name, visibility):
    cc_library(
        name = name + "_baz",
        deps = ["//tool:tool"],
    )

my_submacro = macro(implementation = _impl)
"#],
    );
    t.scratch.file("macro/BUILD", &[]);
    t.scratch.file(
        "macro/defs.bzl",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
load("//submacro:defs.bzl", "my_submacro")

def _impl(name, visibility, extra_level_deep):
    if extra_level_deep:
        my_submacro(name = name + "_bar")
    else:
        cc_library(
            name = name + "_bar",
            deps = ["//tool:tool"],
        )

my_macro = macro(
    implementation = _impl,
    attrs = {"extra_level_deep": attr.bool(configurable=False)},
)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("//macro:defs.bzl", "my_macro")

my_macro(
    name = "foo",
    extra_level_deep = False,
)

my_macro(
    name = "foo2",
    extra_level_deep = True,
)
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo_bar"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event("* Although the dependency is not visible to the location being checked, it is visible to this location's caller, the BUILD file of package //pkg. (Perhaps the caller needs to pass in the dependency as an argument?)");
    t.event_collector.clear();
    assert!(matches!(
        t.update(&["//pkg:foo2_bar_baz"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    // Should say "transitive".
    t.assert_contains_event("* Although the dependency is not visible to the location being checked, it is visible to this location's transitive caller, the BUILD file of package //pkg. (Perhaps this caller, or an intermediate caller, needs to pass in the dependency as an argument?)");
}

// Java: VisibilityTest.testVerboseDiagnostics_moreDelegationNeeded_incompleteDelegation
#[test]
fn test_verbose_diagnostics_more_delegation_needed_incomplete_delegation() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file(
        "tool/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "tool",
    visibility = ["//pkg:__pkg__"],
)
"#],
    );
    t.scratch.file("submacro/BUILD", &[]);
    t.scratch.file(
        "submacro/defs.bzl",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
def _impl(name, visibility, deps):
    if not deps:
        deps = ["//tool:tool"]
    cc_library(
        name = name + "_baz",
        deps = deps,
    )

my_submacro = macro(
    implementation = _impl,
    attrs = {"deps": attr.label_list(configurable=False)},
)
"#],
    );
    t.scratch.file("macro/BUILD", &[]);
    t.scratch.file(
        "macro/defs.bzl",
        &[r#"load("//submacro:defs.bzl", "my_submacro")

def _impl(name, visibility, deps, pass_in_tool):
    my_submacro(
        name = name + "_bar",
        deps = ["//tool:tool"] if pass_in_tool else [],
    )

my_macro = macro(
    implementation = _impl,
    attrs = {
        "deps": attr.label_list(),
        "pass_in_tool": attr.bool(configurable=False),
    },
)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("//macro:defs.bzl", "my_macro")

my_macro(
    name = "foo",
    pass_in_tool = True,
)

my_macro(
    name = "foo2",
    deps = ["//tool:tool"],
    pass_in_tool = False,
)
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo_bar_baz"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    // my_macro passed it to my_submacro, but BUILD didn't pass it to my_macro.
    t.assert_contains_event("* Although the dependency is not visible to the location being checked, it is visible to this location's caller, the BUILD file of package //pkg. (Perhaps the caller needs to pass in the dependency as an argument?)");
    t.event_collector.clear();
    assert!(matches!(
        t.update(&["//pkg:foo2_bar_baz"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    // BUILD passed it to my_macro, but my_macro didn't pass it to my_submacro.
    t.assert_contains_event("* Although the dependency is not visible to the location being checked, it is visible to this location's transitive caller, the BUILD file of package //pkg. (Perhaps this caller, or an intermediate caller, needs to pass in the dependency as an argument?)");
}

// Java: VisibilityTest.testVerboseDiagnostics_editVisibilitySuggestion_forRuleTarget
#[test]
fn test_verbose_diagnostics_edit_visibility_suggestion_for_rule_target() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file(
        "dep/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "dep",
    visibility = ["//visibility:private"],
)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "foo",
    deps = ["//dep:dep"],
)
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event("* If you think the dependency is legitimate, consider updating its visibility declaration.");
}

// Java: VisibilityTest.testVerboseDiagnostics_editVisibilitySuggestion_forFileTarget
#[test]
fn test_verbose_diagnostics_edit_visibility_suggestion_for_file_target() {
    let mut t = new_test();
    t.use_configuration(&["--verbose_visibility_errors"]);
    t.remove_fail_fast_handler();
    t.scratch.file(
        "dep/BUILD",
        &[r#"exports_files(
    ["dep"],
    visibility = ["//visibility:private"],
)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "foo",
    deps = ["//dep:dep"],
)
"#],
    );
    assert!(matches!(
        t.update(&["//pkg:foo"]),
        Err(UpdateError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event("* If you think the dependency on this source file is legitimate, consider updating its visibility declaration using exports_files().");
}
