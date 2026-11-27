//! The tests of Java's `ConfiguredTargetQuerySemanticsTest`.
//!
//! Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java

use super::*;

#[allow(unused_imports)]
use blaze_query_engine::Setting;

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:71 ConfiguredTargetQuerySemanticsTest.testConfigurationRespected
pub fn test_configuration_respected(t: &mut AbstractQueryTest) {
    t.write_build_files_with_configurable_attributes_unconditionally();
    assert!(
        t.eval("deps(//configurable:main) ^ //configurable:adep")
            .is_empty(),
        "{:?}",
        t.eval("deps(//configurable:main) ^ //configurable:adep")
    );
    assert_eq!(
        t.eval("deps(//configurable:main) ^ //configurable:defaultdep")
            .len(),
        1,
        "{:?}",
        t.eval("deps(//configurable:main) ^ //configurable:defaultdep")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:412 ConfiguredTargetQuerySemanticsTest.testConfig_nullConfig
pub fn test_config_null_config(t: &mut AbstractQueryTest) {
    t.write_file(
        "test/BUILD",
        &["load(\"@rules_java//java:defs.bzl\", \"java_library\")
java_library(
    name = \"my_java\",
    srcs = [\"foo.java\"],
)
"],
    );

    let result = t.eval_result("config(//test:foo.java,null)");
    assert_eq!(result.labels.len(), 1, "{:?}", result.labels);
    assert!(t.get_configuration(&result, 0).is_none());
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:428 ConfiguredTargetQuerySemanticsTest.testConfig_configHash
pub fn test_config_config_hash(t: &mut AbstractQueryTest) {
    t.create_config_transitioning_rule_class();
    t.write_file(
        "test/BUILD",
        &["load(\"//test:rules.bzl\", \"rule_with_deps_transition\", \"simple_rule\", \"string_flag\")

string_flag(
    name = \"my_flag\",
    build_setting_default = \"\",
)

rule_with_deps_transition(
    name = \"buildme\",
    deps = [\":mydep\"],
)

simple_rule(name = \"mydep\")
"],
    );

    // If we don't set --universe_scope=//test:buildme, cquery builds both //test:buildme and
    // //test:mydep as top-level targets. That means //test:mydep will have two configured
    // targets: one under the transitioned configuration and one under the top-level
    // configuration. By setting --universe_scope we ensure only the transitioned version exists.
    t.helper.set_universe_scope("//test:buildme");
    t.helper
        .set_query_settings(&[Setting::OnlyTargetDeps, Setting::NoImplicitDeps]);
    let correction = t.get_dependency_correction();
    let result = t.eval_result(&format!("deps(//test:buildme, 1){correction}"));
    assert_eq!(result.labels.len(), 2, "{:?}", result.labels);

    let my_dep_index = if result.labels[0] == "//test:mydep" {
        0
    } else {
        1
    };
    let my_dep_config = t
        .get_configuration(&result, my_dep_index)
        .expect("a configuration");
    let string_flag_config = t
        .get_configuration(&result, 1 - my_dep_index)
        .expect("a configuration");

    // Note: eval() resets the universe scope after each call. We have to xplicitly set it again.
    t.helper.set_universe_scope("//test:buildme");
    let r = t.eval_result(&format!(
        "config(//test:mydep, {})",
        my_dep_config.checksum()
    ));
    assert_eq!(r.labels.len(), 1, "{:?}", r.labels);

    t.helper.set_universe_scope("//test:buildme");
    let e = t.eval_query_exception(&format!(
        "config(//test:mydep, {})",
        string_flag_config.checksum()
    ));
    assert!(
        e.message.contains(
            "No target (in) //test:mydep could be found in the configuration with checksum"
        ),
        "{}",
        e.message
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:512 ConfiguredTargetQuerySemanticsTest.testConfig_exprArgumentFailure
pub fn test_config_expr_argument_failure(t: &mut AbstractQueryTest) {
    t.write_file(
        "test/BUILD",
        &["load(\"@rules_java//java:defs.bzl\", \"java_library\")
java_library(
    name = \"my_java\",
    srcs = [\"foo.java\"],
)
"],
    );

    let eval_throws_result = t.eval_throws(
        "config(filter(\"??not-a-valid-regex\", //test:foo.java), null)",
        /* unconditionally_throws= */ true,
    );
    assert!(
        eval_throws_result
            .message
            .starts_with("illegal 'filter' pattern regexp '??not-a-valid-regex'"),
        "{}",
        eval_throws_result.message
    );
    assert_eq!(
        query_code(&eval_throws_result.failure_detail),
        Some(blaze_protos::failure_details::query::Code::SyntaxError as i32)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:543 ConfiguredTargetQuerySemanticsTest.testRecursiveTargetPatternNeverThrowsError
pub fn test_recursive_target_pattern_never_throws_error(t: &mut AbstractQueryTest) {
    t.write_file("parent/BUILD", &["filegroup(name = 'parent')"]);
    let parent = t.helper.get_root_directory().get_relative_str("parent");
    let child = parent.get_relative_str("child");
    child.create_directory().expect("create child");
    let bad_build = child.get_relative_str("BUILD");
    bad_build
        .create_symbolic_link(&bad_build)
        .expect("create symlink");

    t.helper.set_keep_going(true);
    assert_eq!(t.eval("//parent:all"), t.eval("//parent:parent"));

    t.helper.set_keep_going(false);
    t.helper.turn_off_fail_fast();
    let e = t.eval_throws_target_parsing_exception("//parent/...");
    assert_eq!(
        e.message,
        format!(
            "error loading package under directory 'parent': no such package 'parent/child': \
             Symlink cycle detected while trying to find BUILD file {}/parent/child/BUILD",
            t.helper.get_root_directory().get_path_string()
        )
    );
    assert_eq!(
        package_loading_code(&e.failure_detail),
        Some(
            blaze_protos::failure_details::package_loading::Code::SymlinkCycleOrInfiniteExpansion
                as i32
        )
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:573 ConfiguredTargetQuerySemanticsTest.testRecursiveTargetPatternOutsideOfScopeFailsGracefully
pub fn test_recursive_target_pattern_outside_of_scope_fails_gracefully(t: &mut AbstractQueryTest) {
    t.write_file("testA/BUILD", &["filegroup(name = 'testA')"]);
    t.write_file("testB/BUILD", &["filegroup(name = 'testB')"]);
    t.write_file("testB/testC/BUILD", &["filegroup(name = 'testC')"]);
    t.helper.set_universe_scope("//testA");
    let e = t.eval_query_exception("//testB/...");
    assert_eq!(
        query_code(&e.failure_detail),
        Some(blaze_protos::failure_details::query::Code::TargetNotInUniverseScope as i32)
    );
    assert!(
        e.message.contains("package is not in scope"),
        "{}",
        e.message
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:586 ConfiguredTargetQuerySemanticsTest.testMultipleTopLevelConfigurations_nullConfigs
pub fn test_multiple_top_level_configurations_null_configs(t: &mut AbstractQueryTest) {
    t.write_file(
        "test/BUILD",
        &["load(\"@rules_java//java:defs.bzl\", \"java_library\")
java_library(
    name = \"my_java\",
    srcs = [\"foo.java\"],
)
"],
    );

    let result = t.eval_result("//test:my_java+//test:foo.java");

    assert_eq!(result.labels.len(), 2, "{:?}", result.labels);

    if result.labels[0] == "//test:foo.java" {
        assert!(t.get_configuration(&result, 0).is_none());
        assert!(t.get_configuration(&result, 1).is_some());
    } else {
        assert!(t.get_configuration(&result, 0).is_some());
        assert!(t.get_configuration(&result, 1).is_none());
    }
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:613 ConfiguredTargetQuerySemanticsTest.testSomePath_depInCustomConfiguration
pub fn test_some_path_dep_in_custom_configuration(t: &mut AbstractQueryTest) {
    t.create_config_transitioning_rule_class();
    t.write_file(
        "test/BUILD",
        &["load(\"//test:rules.bzl\", \"rule_with_deps_transition\", \"simple_rule\", \"string_flag\")

string_flag(
    name = \"my_flag\",
    build_setting_default = \"\",
)

rule_with_deps_transition(
    name = \"buildme\",
    deps = [\":mydep\"],
)

simple_rule(name = \"mydep\")
"],
    );

    // If we don't set --universe_scope=//test:buildme, then cquery builds both //test:buildme and
    // //test:mydep as top-level targets. That means //test:mydep will have two configured
    // targets: one under the transitioned configuration and one under the top-level
    // configuration. In these cases cquery prefers the top-level configured one, which won't
    // produce a match since that's not the one down this dependency path.
    t.helper.set_universe_scope("//test:buildme");
    let result = t.eval_result("somepath(//test:buildme, //test:mydep)");
    assert!(
        result.labels.contains(&"//test:mydep".to_owned()),
        "{:?}",
        result.labels
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:712 ConfiguredTargetQuerySemanticsTest.testLabelExpressionsMatchesAllConfiguredTargetsWithLabel
pub fn test_label_expressions_matches_all_configured_targets_with_label(t: &mut AbstractQueryTest) {
    t.create_config_transitioning_rule_class();
    t.write_file(
        "test/BUILD",
        &["load(\"//test:rules.bzl\", \"rule_with_deps_transition\", \"simple_rule\", \"string_flag\")

string_flag(
    name = \"my_flag\",
    build_setting_default = \"\",
)

rule_with_deps_transition(
    name = \"transitioner\",
    deps = [\":simple\"],
)

simple_rule(name = \"simple\")
"],
    );

    t.helper
        .set_universe_scope("//test:transitioner,//test:simple");
    let result = t.eval_result("//test:simple");
    assert_eq!(result.labels.len(), 2, "{:?}", result.labels);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:738 ConfiguredTargetQuerySemanticsTest.testConfigFunctionRefinesMultipleMatches
pub fn test_config_function_refines_multiple_matches(t: &mut AbstractQueryTest) {
    // Peer to testLabelExpressionsMatchesAllConfiguredTargetsWithLabel. The point of that test is
    // to show "cquery //foo:bar" might return multiple configured targets. The point of this
    // test is to show that config() can refine the same query to a specific one.
    t.create_config_transitioning_rule_class();
    t.write_file(
        "test/BUILD",
        &["load(\"//test:rules.bzl\", \"rule_with_deps_transition\", \"simple_rule\", \"string_flag\")

string_flag(
    name = \"my_flag\",
    build_setting_default = \"\",
)

rule_with_deps_transition(
    name = \"transitioner\",
    deps = [\":simple\"],
)

simple_rule(name = \"simple\")
"],
    );

    t.helper
        .set_universe_scope("//test:transitioner,//test:simple");
    let result = t.eval_result("config(//test:simple, target)");
    assert_eq!(result.labels.len(), 1, "{:?}", result.labels);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:767 ConfiguredTargetQuerySemanticsTest.testAspectDepsAppearInCqueryDeps
pub fn test_aspect_deps_appear_in_cquery_deps(t: &mut AbstractQueryTest) {
    t.write_file(
        "donut/test.bzl",
        &[
            r#"TestAspectInfo = provider("TestAspectInfo", fields = ["info"])

def _test_aspect_impl(target, ctx):
    return [
        TestAspectInfo(
            info = depset([target.label]),
        ),
    ]

_test_aspect = aspect(
    implementation = _test_aspect_impl,
    attr_aspects = ["deps"],
    attrs = {
        "_test_attr": attr.label(
            allow_files = True,
            default = Label("//donut:test_filegroup"),
        ),
    },
    provides = [TestAspectInfo],
)

def _test_impl(ctx):
    pass

test_rule = rule(
    _test_impl,
    attrs = {
        "deps": attr.label_list(
            aspects = [_test_aspect],
        ),
    },
)
"#,
        ],
    );
    t.write_file(
        "donut/BUILD",
        &[r#"load(":test.bzl", "test_rule")

filegroup(
    name = "test_filegroup",
    srcs = ["test.bzl"],
)

test_rule(
    name = "test_rule_dep",
)

test_rule(
    name = "test_rule",
    deps = [":test_rule_dep"],
)
"#],
    );
    t.helper
        .set_query_settings(&[Setting::IncludeAspects, Setting::ExplicitAspects]);
    let result = t.eval_descriptions("filter(//donut, deps(//donut:test_rule))");
    assert_eq!(
        sorted(result),
        sorted(vec![
            String::from("//donut:test_rule"),
            String::from("//donut:test_rule_dep"),
            String::from("//donut:test.bzl%_test_aspect of //donut:test_rule_dep"),
            String::from("//donut:test.bzl"),
            String::from("//donut:test_filegroup")
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:839 ConfiguredTargetQuerySemanticsTest.testToolchainPropagatingAspectDepsAppearInCqueryDeps
pub fn test_toolchain_propagating_aspect_deps_appear_in_cquery_deps(t: &mut AbstractQueryTest) {
    t.write_file("donut_toolchains/test_toolchain.bzl", &["def _impl(ctx):\n    return [platform_common.ToolchainInfo()]\n\ntest_toolchain = rule(\n    implementation = _impl,\n)\n"]);
    t.write_file(
        "donut_toolchains/BUILD",
        &[
            r#"load("//donut_toolchains:test_toolchain.bzl", "test_toolchain")

toolchain_type(name = "toolchain_type_1")

test_toolchain(
    name = "foo",
)

toolchain(
    name = "foo_toolchain",
    toolchain = ":foo",
    toolchain_type = ":toolchain_type_1",
)
"#,
        ],
    );
    t.write_file(
        "donut/test.bzl",
        &[
            r#"TestAspectInfo = provider("TestAspectInfo", fields = ["info"])

def _test_aspect_impl(target, ctx):
    return [
        TestAspectInfo(
            info = depset([target.label]),
        ),
    ]

_test_aspect = aspect(
    implementation = _test_aspect_impl,
    toolchains_aspects = ["//donut_toolchains:toolchain_type_1"],
    attrs = {
        "_test_attr": attr.label(
            allow_files = True,
            default = Label("//donut:test_filegroup"),
        ),
    },
    provides = [TestAspectInfo],
)

def _test_impl(ctx):
    pass

test_rule = rule(
    _test_impl,
    attrs = {
        "deps": attr.label_list(
            aspects = [_test_aspect],
        ),
    },
)

rule_with_toolchain = rule(
    _test_impl,
    toolchains = ["//donut_toolchains:toolchain_type_1"],
)
"#,
        ],
    );
    t.write_file(
        "donut/BUILD",
        &[r#"load(":test.bzl", "test_rule", "rule_with_toolchain")

filegroup(
    name = "test_filegroup",
    srcs = ["test.bzl"],
)

rule_with_toolchain(
    name = "test_rule_dep",
)

test_rule(
    name = "test_rule",
    deps = [":test_rule_dep"],
)
"#],
    );
    t.helper
        .set_query_settings(&[Setting::IncludeAspects, Setting::ExplicitAspects]);
    t.helper
        .use_configuration(&["--extra_toolchains=//donut_toolchains:foo_toolchain"]);
    let result = t.eval_descriptions("filter(//donut, deps(//donut:test_rule))");
    assert_eq!(
        sorted(result),
        sorted(vec![
            String::from("//donut:test_rule"),
            String::from("//donut:test_rule_dep"),
            String::from("//donut:test.bzl%_test_aspect of //donut:test_rule_dep"),
            String::from("//donut:test.bzl"),
            String::from("//donut:test_filegroup"),
            String::from("//donut_toolchains:foo"),
            String::from("//donut_toolchains:toolchain_type_1"),
            String::from("//donut:test.bzl%_test_aspect of //donut_toolchains:foo")
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:949 ConfiguredTargetQuerySemanticsTest.testAspectOnAspectDepsAppearInCqueryDeps
pub fn test_aspect_on_aspect_deps_appear_in_cquery_deps(t: &mut AbstractQueryTest) {
    t.write_file(
        "donut/test.bzl",
        &[
            r#"TestAspectInfo = provider("TestAspectInfo", fields = ["info"])
TestAspectOnAspectInfo = provider("TestAspectOnAspectInfo", fields = ["info"])

def _test_aspect_impl(target, ctx):
    return [
        TestAspectInfo(
            info = depset([target.label]),
        ),
    ]

_test_aspect = aspect(
    implementation = _test_aspect_impl,
    attr_aspects = ["deps"],
    attrs = {
        "_test_attr": attr.label(
            allow_files = True,
            default = Label("//donut:test_aspect_filegroup"),
        ),
    },
    provides = [TestAspectInfo],
)

def _test_aspect_on_aspect_impl(target, ctx):
    return [
        TestAspectOnAspectInfo(
            info = depset(
                direct = [target.label],
                transitive = [target[TestAspectInfo].info],
            ),
        ),
    ]

_test_aspect_on_aspect = aspect(
    implementation = _test_aspect_on_aspect_impl,
    attr_aspects = ["deps"],
    attrs = {
        "_test_attr": attr.label(
            allow_files = True,
            default = Label("//donut:test_aspect_on_aspect_filegroup"),
        ),
    },
    required_aspect_providers = [TestAspectInfo],
    provides = [TestAspectOnAspectInfo],
)

def _test_impl(ctx):
    pass

test_rule = rule(
    _test_impl,
    attrs = {
        "deps": attr.label_list(
            aspects = [_test_aspect],
        ),
    },
)

def _test_aspect_on_aspect_rule_impl(ctx):
    pass

test_aspect_on_aspect_rule = rule(
    _test_aspect_on_aspect_rule_impl,
    attrs = {
        "deps": attr.label_list(
            aspects = [_test_aspect, _test_aspect_on_aspect],
        ),
    },
)
"#,
        ],
    );
    t.write_file("donut/test_aspect.file", &[]);
    t.write_file("donut/test_aspect_on_aspect.file", &[]);
    t.write_file(
        "donut/BUILD",
        &[
            r#"load(":test.bzl", "test_aspect_on_aspect_rule", "test_rule")

filegroup(
    name = "test_aspect_filegroup",
    srcs = ["test_aspect.file"],
)

filegroup(
    name = "test_aspect_on_aspect_filegroup",
    srcs = ["test_aspect_on_aspect.file"],
)

test_rule(
    name = "test_rule_dep",
)

test_rule(
    name = "test_rule",
    deps = [":test_rule_dep"],
)

test_aspect_on_aspect_rule(
    name = "test_aspect_on_aspect_rule",
    deps = ["test_rule"],
)
"#,
        ],
    );
    t.helper.set_universe_scope("//donut/...");
    t.helper
        .set_query_settings(&[Setting::IncludeAspects, Setting::ExplicitAspects]);
    let result = t.eval_descriptions("filter(//donut, deps(//donut:test_aspect_on_aspect_rule))");
    assert_eq!(
        sorted(result),
        sorted(vec![
            String::from(format!(
                "{}{}",
                "//donut:test.bzl%_test_aspect_on_aspect on top of",
                " [//donut:test.bzl%_test_aspect of //donut:test_rule_dep]"
            )),
            String::from(format!(
                "{}{}",
                "//donut:test.bzl%_test_aspect_on_aspect on top of",
                " [//donut:test.bzl%_test_aspect of //donut:test_rule]"
            )),
            String::from("//donut:test_rule_dep"),
            String::from("//donut:test_rule"),
            String::from("//donut:test.bzl%_test_aspect of //donut:test_rule_dep"),
            String::from("//donut:test.bzl%_test_aspect of //donut:test_rule"),
            String::from("//donut:test_aspect_on_aspect_rule"),
            String::from("//donut:test_aspect.file"),
            String::from("//donut:test_aspect_on_aspect_filegroup"),
            String::from("//donut:test_aspect_on_aspect.file"),
            String::from("//donut:test_aspect_filegroup")
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:1078 ConfiguredTargetQuerySemanticsTest.testAspectDepsAppearInCqueryRdeps
pub fn test_aspect_deps_appear_in_cquery_rdeps(t: &mut AbstractQueryTest) {
    t.write_file(
        "donut/test.bzl",
        &[
            r#"TestAspectInfo = provider("TestAspectInfo", fields = ["info"])

def _test_aspect_impl(target, ctx):
    return [
        TestAspectInfo(
            info = depset([target.label]),
        ),
    ]

_test_aspect = aspect(
    implementation = _test_aspect_impl,
    attr_aspects = ["deps"],
    attrs = {
        "_test_attr": attr.label(
            allow_files = True,
            default = Label("//donut:test_filegroup"),
        ),
    },
    provides = [TestAspectInfo],
)

def _test_impl(ctx):
    pass

test_rule = rule(
    _test_impl,
    attrs = {
        "deps": attr.label_list(
            aspects = [_test_aspect],
        ),
    },
)
"#,
        ],
    );
    t.write_file(
        "donut/BUILD",
        &[r#"load(":test.bzl", "test_rule")

filegroup(
    name = "test_filegroup",
    srcs = ["test.bzl"],
)

test_rule(
    name = "test_rule_dep",
)

test_rule(
    name = "test_rule",
    deps = [":test_rule_dep"],
)
"#],
    );
    t.helper
        .set_query_settings(&[Setting::IncludeAspects, Setting::ExplicitAspects]);
    let result = t.eval_descriptions("rdeps(//donut/..., //donut:test_filegroup)");
    assert_eq!(
        sorted(result),
        sorted(vec![
            String::from("//donut:test_filegroup"),
            String::from("//donut:test_rule"),
            String::from("//donut:test.bzl%_test_aspect of //donut:test_rule_dep")
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:1194 ConfiguredTargetQuerySemanticsTest.testOnlySelectedDormantDepsReturned
pub fn test_only_selected_dormant_deps_returned(t: &mut AbstractQueryTest) {
    t.write_file(
        "a/a.bzl",
        &[r#"ComponentInfo = provider(fields=["dormant"])

def _bin_impl(ctx):
  return [DefaultInfo()]

def _materializer(ctx):
  return [d for d in ctx.attr.dep[ComponentInfo].dormant if "yes" in str(d.label)]

bin = rule(
  implementation = _bin_impl,
  attrs = {
    "_materialized": attr.label_list(materializer=_materializer),
    "dep": attr.label(for_dependency_resolution = True) })

def _component_impl(ctx):
  return [ComponentInfo(dormant=ctx.attr.impl)]

component = rule(
  implementation = _component_impl,
  dependency_resolution_rule = True,
  attrs = { "impl": attr.dormant_label_list() })
"#],
    );

    t.write_file(
        "a/BUILD",
        &[r#"load("a.bzl", "bin", "component")

filegroup(name="a_yes")
filegroup(name="b_no")

bin(name="bin", dep=":c")
component(name="c", impl=[":a_yes", "b_no"])
"#],
    );

    t.helper.use_configuration(&["--experimental_dormant_deps"]);
    let deps = t.eval_to_list_of_strings("deps('//a:bin')");
    for label in ["//a:bin", "//a:c", "//a:a_yes"] {
        assert!(deps.contains(&label.to_owned()), "{label} not in {deps:?}");
    }
    assert!(!deps.contains(&"//a:b_no".to_owned()), "{deps:?}");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:1240 ConfiguredTargetQuerySemanticsTest.testMaterializerRuleQuery
pub fn test_materializer_rule_query(t: &mut AbstractQueryTest) {
    t.write_file(
        "defs.bzl",
        &[r#"# Component ######################################

ComponentInfo = provider(fields = ["output"])

def _component_impl(ctx):
   f = ctx.actions.declare_file(ctx.label.name + ".txt")
   ctx.actions.write(f, ctx.label.name)
   return ComponentInfo(output = f)

component = rule(
    implementation = _component_impl,
    provides = [ComponentInfo],
)

# Component selector #############################

def _component_selector_impl(ctx):
    selected = []
    for cd in ctx.attr.all_components_dormant:
        if "yes" in str(cd.label):
            selected.append(cd)
    return MaterializedDepsInfo(deps = selected)

component_selector = materializer_rule(
    implementation = _component_selector_impl,
    attrs = {
        "all_components_dormant": attr.dormant_label_list(),
    },
)

# Binary #########################################

def _binary_impl(ctx):
    return DefaultInfo()

binary = rule(
    implementation = _binary_impl,
    attrs = {
        "deps": attr.label_list(providers = [ComponentInfo]),
    },
)
"#],
    );
    t.write_file(
        "BUILD",
        &[
            r#"load(":defs.bzl", "component", "component_selector", "binary")

binary(
    name = "bin",
    deps = [
        ":aaa",
        ":component_selector",
        ":zzz",
    ],
)

component_selector(
    name = "component_selector",
    all_components_dormant = [":a_yes", ":b_yes", ":c_no", ":d_no"],
)

component(name = "aaa")
component(name = "a_yes")
component(name = "b_yes")
component(name = "c_no")
component(name = "d_no")
component(name = "zzz")
"#,
        ],
    );
    t.helper.use_configuration(&["--experimental_dormant_deps"]);
    let direct_deps = t.eval_to_list_of_strings("deps('//:bin', 1)");
    assert!(
        direct_deps.contains(&String::from("//:a_yes")),
        "{:?}",
        direct_deps
    );
    assert!(
        direct_deps.contains(&String::from("//:b_yes")),
        "{:?}",
        direct_deps
    );
    assert!(
        direct_deps.contains(&String::from("//:component_selector")),
        "{:?}",
        direct_deps
    );
    assert!(
        !direct_deps.contains(&String::from("//:c_no")),
        "{:?}",
        direct_deps
    );
    assert!(
        !direct_deps.contains(&String::from("//:d_no")),
        "{:?}",
        direct_deps
    );
    let all_deps = t.eval_to_list_of_strings("deps('//:bin')");
    assert!(
        all_deps.contains(&String::from("//:a_yes")),
        "{:?}",
        all_deps
    );
    assert!(
        all_deps.contains(&String::from("//:b_yes")),
        "{:?}",
        all_deps
    );
    assert!(
        all_deps.contains(&String::from("//:component_selector")),
        "{:?}",
        all_deps
    );
    assert!(
        !all_deps.contains(&String::from("//:c_no")),
        "{:?}",
        all_deps
    );
    assert!(
        !all_deps.contains(&String::from("//:d_no")),
        "{:?}",
        all_deps
    );
    let component_selector_deps = t.eval_to_list_of_strings("deps('//:component_selector', 1)");
    assert!(
        !component_selector_deps.contains(&String::from("//:a_yes")),
        "{:?}",
        component_selector_deps
    );
    assert!(
        !component_selector_deps.contains(&String::from("//:b_yes")),
        "{:?}",
        component_selector_deps
    );
    assert!(
        !component_selector_deps.contains(&String::from("//:c_no")),
        "{:?}",
        component_selector_deps
    );
    assert!(
        !component_selector_deps.contains(&String::from("//:d_no")),
        "{:?}",
        component_selector_deps
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:1335 ConfiguredTargetQuerySemanticsTest.testMaterializerRuleRealDepsQuery
pub fn test_materializer_rule_real_deps_query(t: &mut AbstractQueryTest) {
    t.write_file(
        "defs.bzl",
        &[r#"# Component ######################################

ComponentInfo = provider(fields = ["output", "info"])

def _component_impl(ctx):
    f = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.write(f, ctx.label.name)
    return ComponentInfo(output = f, info = ctx.attr.info)

component = rule(
    implementation = _component_impl,
    provides = [ComponentInfo],
    attrs = {
        "info": attr.string(),
    }
)

# Component selector #############################

def _component_selector_impl(ctx):
    selected = []
    for c in ctx.attr.all_components:
        if "yes" in c[ComponentInfo].info:
            selected.append(c)
    return MaterializedDepsInfo(deps = selected)

component_selector = materializer_rule(
    implementation = _component_selector_impl,
    allow_real_deps = True,
    attrs = {
        "all_components": attr.label_list(),
    },
)

# Binary #########################################

def _binary_impl(ctx):
    return DefaultInfo()

binary = rule(
    implementation = _binary_impl,
    attrs = {
        "deps": attr.label_list(providers = [ComponentInfo]),
    },
)
"#],
    );
    t.write_file(
        "BUILD",
        &[
            r#"load(":defs.bzl", "component", "component_selector", "binary")

binary(
    name = "bin",
    deps = [
        ":aaa",
        ":component_selector",
        ":zzz",
    ],
)

component_selector(
    name = "component_selector",
    all_components = [":a", ":b", ":c", ":d"],
)

component(name = "aaa")
component(name = "a", info = "yes")
component(name = "b", info = "yes")
component(name = "c", info = "no")
component(name = "d", info = "no")
component(name = "zzz")
"#,
        ],
    );
    t.helper.use_configuration(&["--experimental_dormant_deps"]);
    let direct_deps = t.eval_to_list_of_strings("deps('//:bin', 1)");
    assert!(
        direct_deps.contains(&String::from("//:a")),
        "{:?}",
        direct_deps
    );
    assert!(
        direct_deps.contains(&String::from("//:b")),
        "{:?}",
        direct_deps
    );
    assert!(
        direct_deps.contains(&String::from("//:component_selector")),
        "{:?}",
        direct_deps
    );
    assert!(
        !direct_deps.contains(&String::from("//:c")),
        "{:?}",
        direct_deps
    );
    assert!(
        !direct_deps.contains(&String::from("//:d")),
        "{:?}",
        direct_deps
    );
    let all_deps = t.eval_to_list_of_strings("deps('//:bin')");
    assert!(all_deps.contains(&String::from("//:a")), "{:?}", all_deps);
    assert!(all_deps.contains(&String::from("//:b")), "{:?}", all_deps);
    assert!(all_deps.contains(&String::from("//:c")), "{:?}", all_deps);
    assert!(all_deps.contains(&String::from("//:d")), "{:?}", all_deps);
    assert!(
        all_deps.contains(&String::from("//:component_selector")),
        "{:?}",
        all_deps
    );
    let component_selector_deps = t.eval_to_list_of_strings("deps('//:component_selector', 1)");
    assert!(
        component_selector_deps.contains(&String::from("//:a")),
        "{:?}",
        component_selector_deps
    );
    assert!(
        component_selector_deps.contains(&String::from("//:b")),
        "{:?}",
        component_selector_deps
    );
    assert!(
        component_selector_deps.contains(&String::from("//:c")),
        "{:?}",
        component_selector_deps
    );
    assert!(
        component_selector_deps.contains(&String::from("//:d")),
        "{:?}",
        component_selector_deps
    );
}
