//! The tests of Java's `PostAnalysisQueryTest` (generic over the cquery and aquery suites).
//!
//! Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java

use super::*;

#[allow(unused_imports)]
use blaze_query_engine::Setting;

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:278 PostAnalysisQueryTest.testImplicitDepsOnOutputFile
pub fn test_implicit_deps_on_output_file(t: &mut AbstractQueryTest) {
    t.write_file(
        "test/BUILD",
        &[r#"load(":defs.bzl", "my_dep", "my_rule")
my_rule(
    name = "buildme",
    explicit_deps = [":explicit_dep"],
    output = "foo.out",
)
my_dep(name = "explicit_dep")
my_dep(name = "implicit_dep")
"#],
    );
    t.write_file(
        "test/defs.bzl",
        &[r#"my_dep = rule(
    implementation = lambda ctx: [],
    attrs = {},
)

def _impl(ctx):
    ctx.actions.write(ctx.outputs.output, "hello!")

my_rule = rule(
    implementation = _impl,
    attrs = {
        "output": attr.output(),
        "explicit_deps": attr.label_list(),
        "_implicit_deps": attr.label_list(default = ["//test:implicit_dep"]),
    },
)
"#],
    );
    t.helper.set_query_settings(&[]);
    assert!(
        t.eval_to_list_of_strings("deps(//test:foo.out)")
            .contains(&String::from("//test:foo.out")),
        "{:?}",
        t.eval_to_list_of_strings("deps(//test:foo.out)")
    );
    assert!(
        t.eval_to_list_of_strings("deps(//test:foo.out)")
            .contains(&String::from("//test:buildme")),
        "{:?}",
        t.eval_to_list_of_strings("deps(//test:foo.out)")
    );
    assert!(
        t.eval_to_list_of_strings("deps(//test:foo.out)")
            .contains(&String::from("//test:explicit_dep")),
        "{:?}",
        t.eval_to_list_of_strings("deps(//test:foo.out)")
    );
    assert!(
        t.eval_to_list_of_strings("deps(//test:foo.out)")
            .contains(&String::from("//test:implicit_dep")),
        "{:?}",
        t.eval_to_list_of_strings("deps(//test:foo.out)")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:876 PostAnalysisQueryTest.testNonIdempotentRuleTransition
pub fn test_non_idempotent_rule_transition(t: &mut AbstractQueryTest) {
    t.write_file(
        "test/defs.bzl",
        &[r#"StringSettingInfo = provider(fields = ["value"])

def _string_impl(ctx):
    return StringSettingInfo(value = ctx.build_setting_value)

string_setting = rule(
    implementation = _string_impl,
    build_setting = config.string(),
)

def _transition_impl(settings, attr):
    return {
        k: v + "-transitioned"
        for k, v in settings.items()
    }

_transition = transition(
    implementation = _transition_impl,
    inputs = ["//test:string_setting"],
    outputs = ["//test:string_setting"],
)

def _custom_rule_impl(ctx):
    out = ctx.actions.declare_file(ctx.attr.name)
    ctx.actions.write(out, ctx.attr._string_setting[StringSettingInfo].value)
    return [DefaultInfo(files = depset([out]))]

custom_rule = rule(
    cfg = _transition,
    implementation = _custom_rule_impl,
    attrs = {
        "_string_setting": attr.label(default = "//test:string_setting"),
    },
 )
"#],
    );
    t.write_file(
        "test/BUILD",
        &[r#"load(":defs.bzl", "custom_rule", "string_setting")

string_setting(
    name = "string_setting",
    build_setting_default = "default",
)

custom_rule(name = "custom_rule_name")
"#],
    );
    let output = t.eval_to_list_of_strings("//test:custom_rule_name");
    assert_eq!(output.len(), 1, "{:?}", output);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:933 PostAnalysisQueryTest.testNonIdempotentRuleTransition_transitionedConfigIsAlsoToplevel
pub fn test_non_idempotent_rule_transition_transitioned_config_is_also_toplevel(
    t: &mut AbstractQueryTest,
) {
    t.write_file(
        "test/defs.bzl",
        &[r#"StringSettingInfo = provider(fields = ["value"])

def _string_impl(ctx):
    return StringSettingInfo(value = ctx.build_setting_value)

string_setting = rule(
    implementation = _string_impl,
    build_setting = config.string(),
)

def _transition_impl(settings, attr):
    return {
        k: v + "-transitioned"
        for k, v in settings.items()
    }

_transition = transition(
    implementation = _transition_impl,
    inputs = ["//test:string_setting"],
    outputs = ["//test:string_setting"],
)

def _custom_rule_impl(ctx):
    out = ctx.actions.declare_file(ctx.attr.name)
    ctx.actions.write(out, ctx.attr._string_setting[StringSettingInfo].value)
    return [DefaultInfo(files = depset([out]))]

custom_rule = rule(
    cfg = _transition,
    implementation = _custom_rule_impl,
    attrs = {
        "_string_setting": attr.label(default = "//test:string_setting"),
    },
 )

def _wrapper_impl(_):
    pass

wrapper = rule(
    implementation = _wrapper_impl,
    attrs = {
        "dep": attr.label(
            cfg = _transition,
        ),
    },
)
"#],
    );
    t.write_file(
        "test/BUILD",
        &[
            r#"load(":defs.bzl", "custom_rule", "string_setting", "wrapper")

string_setting(
    name = "string_setting",
    build_setting_default = "default",
)

custom_rule(name = "custom_rule_name")

wrapper(
    name = "wrapper_name",
    dep = ":custom_rule_name",
)
"#,
        ],
    );
    let output = t.eval_to_list_of_strings("//test:all intersect //test:custom_rule_name");
    assert_eq!(output.len(), 1, "{:?}", output);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:1030 PostAnalysisQueryTest.labelPointsToMultipleConfiguredTargets
pub fn label_points_to_multiple_configured_targets(_t: &mut AbstractQueryTest) {}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:1090 PostAnalysisQueryTest.testGenqueryScope
pub fn test_genquery_scope(t: &mut AbstractQueryTest) {
    t.run_genquery_scope_test(true);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:1101 PostAnalysisQueryTest.testEqualityOfOrderedThreadSafeImmutableSet
pub fn test_equality_of_ordered_thread_safe_immutable_set(_t: &mut AbstractQueryTest) {}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:1303 PostAnalysisQueryTest.testWildcardsDontLoadUnnecessaryPackages
pub fn test_wildcards_dont_load_unnecessary_packages(_t: &mut AbstractQueryTest) {}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:1307 PostAnalysisQueryTest.boundedDepsWithError
pub fn bounded_deps_with_error(_t: &mut AbstractQueryTest) {}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:1312 PostAnalysisQueryTest.testGraphOrderOfWildcards
pub fn test_graph_order_of_wildcards(_t: &mut AbstractQueryTest) {}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:1318 PostAnalysisQueryTest.testVisibleWithNonPackageGroupVisibility
pub fn test_visible_with_non_package_group_visibility(_t: &mut AbstractQueryTest) {}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:1324 PostAnalysisQueryTest.testVisibleWithPackageGroupWithNonPackageGroupIncludes
pub fn test_visible_with_package_group_with_non_package_group_includes(_t: &mut AbstractQueryTest) {
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:1329 PostAnalysisQueryTest.testNodepDeps_false
pub fn test_nodep_deps_false(_t: &mut AbstractQueryTest) {}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:1334 PostAnalysisQueryTest.testDefaultVisibilityReturnedInDeps_nonEmptyDependencyFilter
pub fn test_default_visibility_returned_in_deps_non_empty_dependency_filter(
    _t: &mut AbstractQueryTest,
) {
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:157 PostAnalysisQueryTest.testTargetLiteralWithMissingTargets
pub fn test_target_literal_with_missing_targets(t: &mut AbstractQueryTest) {
    t.helper.turn_off_fail_fast();
    // assertThrows(TargetParsingException.class, super::testTargetLiteralWithMissingTargets):
    // the super test throws from its evalThrows("//a:b", false).
    t.write_file("a/BUILD", &[]);
    let e = t.eval_throws_target_parsing_exception("//a:b");
    let expected = format!(
        "no such target '//a:b': target 'b' not declared in package 'a' defined by {}/a/BUILD",
        t.helper.get_root_directory().get_path_string()
    );
    assert_eq!(e.message, expected);
    assert_eq!(
        package_loading_code(&e.failure_detail),
        Some(blaze_protos::failure_details::package_loading::Code::TargetMissing as i32)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:175 PostAnalysisQueryTest.testBadTargetLiterals
pub fn test_bad_target_literals(t: &mut AbstractQueryTest) {
    t.helper.turn_off_fail_fast();
    // assertThrows(TargetParsingException.class, super::testBadTargetLiterals): the super test
    // throws from its evalThrows("bad:*:*", false).
    let e = t.eval_throws_target_parsing_exception("bad:*:*");
    t.check_resultof_bad_target_literals(e.message, e.failure_detail);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:236 PostAnalysisQueryTest.testNoImplicitDepsOnOutputFile
pub fn test_no_implicit_deps_on_output_file(t: &mut AbstractQueryTest) {
    t.write_file(
        "test/BUILD",
        &[r#"load(":defs.bzl", "my_dep", "my_rule")
my_rule(
    name = "buildme",
    explicit_deps = [":explicit_dep"],
    output = "foo.out",
)
my_dep(name = "explicit_dep")
my_dep(name = "implicit_dep")
"#],
    );

    t.write_file(
        "test/defs.bzl",
        &[r#"my_dep = rule(
    implementation = lambda ctx: [],
    attrs = {},
)

def _impl(ctx):
    ctx.actions.write(ctx.outputs.output, "hello!")

my_rule = rule(
    implementation = _impl,
    attrs = {
        "output": attr.output(),
        "explicit_deps": attr.label_list(),
        "_implicit_deps": attr.label_list(default = ["//test:implicit_dep"]),
    },
)
"#],
    );

    t.helper.set_query_settings(&[Setting::NoImplicitDeps]);
    let deps = t.eval_to_list_of_strings("deps(//test:foo.out)");
    for label in ["//test:foo.out", "//test:buildme", "//test:explicit_dep"] {
        assert!(deps.contains(&label.to_owned()), "{label} not in {deps:?}");
    }
    let deps = t.eval_to_list_of_strings("deps(//test:foo.out)");
    assert!(
        !deps.contains(&"//test:implicit_dep".to_owned()),
        "{deps:?}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:406 PostAnalysisQueryTest.testNoImplicitDeps_starlark_toolchains
pub fn test_no_implicit_deps_starlark_toolchains(t: &mut AbstractQueryTest) {
    t.write_simple_toolchain();
    t.write_file(
        "test/rule/rule.bzl",
        &["def _impl(ctx):
    return []

implicit_toolchain_deps_rule = rule(
    implementation = _impl,
    toolchains = [\"//test:toolchain_type\"],
)
"],
    );
    t.write_file(
        "test/rule/BUILD",
        &["load(\":rule.bzl\", \"implicit_toolchain_deps_rule\")

implicit_toolchain_deps_rule(
    name = \"my_rule\",
)
"],
    );
    t.helper
        .use_configuration(&["--extra_toolchains=//test:toolchain"]);

    let implicits = "//test:toolchain_impl";
    let explicits = "//test/rule:my_rule";

    // Check for implicit toolchain dependencies
    let deps = t.eval_to_list_of_strings("deps(//test/rule:my_rule)");
    for label in [explicits, implicits] {
        assert!(deps.contains(&label.to_owned()), "{label} not in {deps:?}");
    }

    t.helper.set_query_settings(&[Setting::NoImplicitDeps]);
    let filtered_deps = t.eval_to_list_of_strings("deps(//test/rule:my_rule)");
    assert!(
        filtered_deps.contains(&explicits.to_owned()),
        "{filtered_deps:?}"
    );
    assert!(
        !filtered_deps.contains(&implicits.to_owned()),
        "{filtered_deps:?}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:444 PostAnalysisQueryTest.testNoImplicitDeps_cc_toolchains
pub fn test_no_implicit_deps_cc_toolchains(t: &mut AbstractQueryTest) {
    t.write_file(
        "test/toolchain/toolchain_config.bzl",
        &["load(\"@rules_cc//cc/toolchains:cc_toolchain_config_info.bzl\", \"CcToolchainConfigInfo\")
load(\"@rules_cc//cc/common:cc_common.bzl\", \"cc_common\")
def _impl(ctx):
    return cc_common.create_cc_toolchain_config_info(
        ctx = ctx,
        toolchain_identifier = \"mock-llvm-toolchain-k8\",
        host_system_name = \"mock-system-name-for-k8\",
        target_system_name = \"mock-target-system-name-for-k8\",
        target_cpu = \"k8\",
        target_libc = \"mock-libc-for-k8\",
        compiler = \"mock-compiler-for-k8\",
        abi_libc_version = \"mock-abi-libc-for-k8\",
        abi_version = \"mock-abi-version-for-k8\",
    )

cc_toolchain_config = rule(
    implementation = _impl,
    attrs = {},
    provides = [CcToolchainConfigInfo],
)
"],
    );
    t.write_file(
        "test/toolchain/BUILD",
        &[
            "load('@rules_cc//cc/toolchains:cc_toolchain.bzl', 'cc_toolchain')",
            "load(':toolchain_config.bzl', 'cc_toolchain_config')",
            "cc_toolchain_config(name = 'some-cc-toolchain-config')",
            "filegroup(name = 'nothing', srcs = [])",
            "cc_toolchain(",
            "    name = 'some_cc_toolchain_impl',",
            "    all_files = ':nothing',",
            "    as_files = ':nothing',",
            "    compiler_files = ':nothing',",
            "    dwp_files = ':nothing',",
            "    linker_files = ':nothing',",
            "    objcopy_files = ':nothing',",
            "    strip_files = ':nothing',",
            "    toolchain_config = ':some-cc-toolchain-config',",
            ")",
            "toolchain(",
            "    name = 'some_cc_toolchain',",
            "    toolchain = ':some_cc_toolchain_impl',",
            // TestConstants.TOOLS_REPOSITORY
            "    toolchain_type = '@bazel_tools//tools/cpp:toolchain_type',",
            ")",
        ],
    );
    t.write_file(
        "test/BUILD",
        &["load(\"@rules_cc//cc:cc_library.bzl\", \"cc_library\")
cc_library(
    name = \"my_rule\",
    srcs = [\"whatever.cpp\"],
)
"],
    );
    t.helper
        .use_configuration(&["--extra_toolchains=//test/toolchain:some_cc_toolchain"]);

    let implicits = "//test/toolchain:some_cc_toolchain_impl";
    let explicits = "//test:my_rule";

    // Check for implicit toolchain dependencies
    let deps = t.eval_to_list_of_strings("deps(//test:my_rule)");
    let expected = t.eval_to_list_of_strings(&format!("{explicits}+{implicits}+{PLATFORM_LABEL}"));
    // containsAtLeastElementsIn(unique(...))
    for label in expected {
        assert!(deps.contains(&label), "{label} not in {deps:?}");
    }

    t.helper.set_query_settings(&[Setting::NoImplicitDeps]);
    let filtered_deps = t.eval_to_list_of_strings("deps(//test:my_rule)");
    assert!(
        filtered_deps.contains(&explicits.to_owned()),
        "{filtered_deps:?}"
    );
    assert!(
        !filtered_deps.contains(&implicits.to_owned()),
        "{filtered_deps:?}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:548 PostAnalysisQueryTest.testNoImplicitDepsFromAutoExecGroups_autoExecGroupsEnabled
pub fn test_no_implicit_deps_from_auto_exec_groups_auto_exec_groups_enabled(
    t: &mut AbstractQueryTest,
) {
    t.write_simple_toolchain();
    t.write_file(
        "test/aeg/defs.bzl",
        &["def _impl(ctx):
    return []

custom_rule = rule(
    implementation = _impl,
    toolchains = [\"//test:toolchain_type\"],
)
"],
    );
    t.write_file(
        "test/aeg/BUILD",
        &["load(\"//test/aeg:defs.bzl\", \"custom_rule\")

custom_rule(name = \"custom_rule_name\")
"],
    );
    t.helper.use_configuration(&[
        "--incompatible_auto_exec_groups",
        "--extra_toolchains=//test:all",
    ]);

    let implicits = "//test:toolchain_impl";
    let explicits = "//test/aeg:custom_rule_name";

    // Check for implicit toolchain dependencies
    let deps = t.eval_to_list_of_strings("deps(//test/aeg:custom_rule_name)");
    for label in [explicits, implicits] {
        assert!(deps.contains(&label.to_owned()), "{label} not in {deps:?}");
    }

    t.helper.set_query_settings(&[Setting::NoImplicitDeps]);
    let filtered_deps = t.eval_to_list_of_strings("deps(//test/aeg:custom_rule_name)");
    assert!(
        filtered_deps.contains(&explicits.to_owned()),
        "{filtered_deps:?}"
    );
    assert!(
        !filtered_deps.contains(&implicits.to_owned()),
        "{filtered_deps:?}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:586 PostAnalysisQueryTest.testNoImplicitDepsFromCustomExecGroups_autoExecGroupsEnabled
pub fn test_no_implicit_deps_from_custom_exec_groups_auto_exec_groups_enabled(
    t: &mut AbstractQueryTest,
) {
    t.write_simple_toolchain();
    t.write_file(
        "test/aeg/defs.bzl",
        &["def _impl(ctx):
    return []

custom_rule = rule(
    implementation = _impl,
    exec_groups = {
        \"custom_exec_group\": exec_group(
            toolchains = [\"//test:toolchain_type\"],
        ),
    },
)
"],
    );
    t.write_file(
        "test/aeg/BUILD",
        &["load(\"//test/aeg:defs.bzl\", \"custom_rule\")

custom_rule(name = \"custom_rule_name\")
"],
    );
    t.helper.use_configuration(&[
        "--incompatible_auto_exec_groups",
        "--extra_toolchains=//test:all",
    ]);

    let implicits = "//test:toolchain_impl";
    let explicits = "//test/aeg:custom_rule_name";

    // Check for implicit toolchain dependencies
    let deps = t.eval_to_list_of_strings("deps(//test/aeg:custom_rule_name)");
    for label in [explicits, implicits] {
        assert!(deps.contains(&label.to_owned()), "{label} not in {deps:?}");
    }

    t.helper.set_query_settings(&[Setting::NoImplicitDeps]);
    let filtered_deps = t.eval_to_list_of_strings("deps(//test/aeg:custom_rule_name)");
    assert!(
        filtered_deps.contains(&explicits.to_owned()),
        "{filtered_deps:?}"
    );
    assert!(
        !filtered_deps.contains(&implicits.to_owned()),
        "{filtered_deps:?}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:667 PostAnalysisQueryTest.testLet
pub fn test_let(t: &mut AbstractQueryTest) {
    t.helper.set_whole_test_universe_scope("//a,//b,//c,//d");
    abstract_query_tests::test_let(t);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:674 PostAnalysisQueryTest.testSet
pub fn test_set(t: &mut AbstractQueryTest) {
    t.helper
        .set_whole_test_universe_scope("//a:*,//b:*,//c:*,//d:*");
    abstract_query_tests::test_set(t);
}
