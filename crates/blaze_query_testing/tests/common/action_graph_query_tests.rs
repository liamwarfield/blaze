//! The tests of Java's `ActionGraphQueryTest`.
//!
//! Java: src/test/java/com/google/devtools/build/lib/query2/aquery/ActionGraphQueryTest.java

use super::*;

#[allow(unused_imports)]
use blaze_query_engine::Setting;

// Java: src/test/java/com/google/devtools/build/lib/query2/aquery/ActionGraphQueryTest.java:64 ActionGraphQueryTest.testMultipleTopLevelConfigurations_nullConfigs
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

// Java: src/test/java/com/google/devtools/build/lib/query2/aquery/ActionGraphQueryTest.java:92 ActionGraphQueryTest.testImplicitToolchainBinding_containsToolchainTarget
pub fn test_implicit_toolchain_binding_contains_toolchain_target(t: &mut AbstractQueryTest) {
    t.write_file(
        "q/BUILD",
        &["load(\":q.bzl\", \"r\", \"tc\")

genrule(
    name = \"gr\",
    srcs = [],
    outs = [\"gro\"],
    cmd = \"echo GRO > $@\",
)

tc(
    name = \"tc\",
    dep = \":gr\",
)

toolchain_type(name = \"type\")

toolchain(
    name = \"tc.toolchain\",
    toolchain = \":tc\",
    toolchain_type = \":type\",
)

r(name = \"r\")
"],
    );
    t.write_file(
        "q/q.bzl",
        &["def _r_impl(ctx):
    gro = ctx.toolchains[\"//q:type\"].gro
    o = ctx.actions.declare_file(ctx.label.name + \".output\")
    ctx.actions.run_shell(
        inputs = depset([gro]),
        outputs = [o],
        command = \"cp \" + gro.path + \" \" + o.path,
    )
    return DefaultInfo(files = depset([o]))

def _tc_impl(ctx):
    gro = ctx.files.dep[0]
    return [platform_common.ToolchainInfo(gro = gro)]

tc = rule(
    implementation = _tc_impl,
    attrs = {\"dep\": attr.label()},
)
r = rule(
    implementation = _r_impl,
    toolchains = [\"//q:type\"],
)
"],
    );
    t.overwrite_file("MODULE.bazel", &["register_toolchains('//q:tc.toolchain')"]);

    let result = t.eval_result("deps('//q:r')");

    t.assert_does_not_contain_event("Targets were missing from graph");
    // getConfiguredTarget().getOriginalLabel().getCanonicalForm()
    assert!(
        result.labels.contains(&"//q:tc".to_owned()),
        "{:?}",
        result.labels
    );
}
