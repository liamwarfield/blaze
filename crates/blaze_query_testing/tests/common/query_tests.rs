//! The tests of Java's `QueryTest`.
//!
//! Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java

use super::*;

#[allow(unused_imports)]
use blaze_query_engine::Setting;

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:65 QueryTest.testFindsAllTargets_nativeRuleMacro
pub fn test_finds_all_targets_native_rule_macro(t: &mut AbstractQueryTest) {
    t.write_file(
        "test/starlark/extension.bzl",
        &[r#"def macro(name):
    native.genrule(name = name, outs = [name + ".txt"], cmd = "echo hi >$@")
"#],
    );
    t.write_file(
        "test//starlark/BUILD",
        &[r#"load("//test/starlark:extension.bzl", "macro")

macro(name = "rule1")

macro(name = "rule2")
"#],
    );
    assert_eq!(
        t.eval("//test/starlark:*"),
        labels(&[
            "//test/starlark:rule1",
            "//test/starlark:rule2",
            "//test/starlark:BUILD",
            "//test/starlark:rule1.txt",
            "//test/starlark:rule2.txt"
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:93 QueryTest.testFindsAllTargets_starlarkRuleMacro
pub fn test_finds_all_targets_starlark_rule_macro(t: &mut AbstractQueryTest) {
    t.write_file("test//starlark/extension.bzl", &["def impl(ctx):\n    return None\n\nstarlark_rule = rule(implementation = impl)\n\ndef macro(name):\n    starlark_rule(name = name)\n"]);
    t.write_file(
        "test//starlark/BUILD",
        &[r#"load("//test/starlark:extension.bzl", "macro")

macro(name = "rule1")

macro(name = "rule2")
"#],
    );
    assert_eq!(
        t.eval("//test/starlark:*"),
        labels(&[
            "//test/starlark:rule1",
            "//test/starlark:rule2",
            "//test/starlark:BUILD"
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:121 QueryTest.testFindsAllTargets_symbolicMacro
pub fn test_finds_all_targets_symbolic_macro(t: &mut AbstractQueryTest) {
    t.write_file(
        "test//starlark/extension.bzl",
        &[r#"def _rule_impl(ctx):
    return None

starlark_rule = rule(implementation = _rule_impl)

def _macro_impl(name, visibility):
    starlark_rule(name = name, visibility = visibility)
    native.genrule(name = name + "_gen", outs = [name + "_gen.txt"], cmd = "echo hi >$@")

symbolic_macro = macro(implementation = _macro_impl)
"#],
    );
    t.write_file(
        "test//starlark/BUILD",
        &[r#"load("//test/starlark:extension.bzl", "symbolic_macro")

symbolic_macro(name = "foo")
"#],
    );
    assert_eq!(
        t.eval("//test/starlark:*"),
        labels(&[
            "//test/starlark:BUILD",
            "//test/starlark:foo",
            "//test/starlark:foo_gen",
            "//test/starlark:foo_gen.txt"
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:188 QueryTest.testBuildfiles_starlarkDep
pub fn test_buildfiles_starlark_dep(t: &mut AbstractQueryTest) {
    t.write_file(
        "test//starlark/extension.bzl",
        &[r#"def macro(name):
    native.genrule(name = name, outs = [name + ".txt"], cmd = "echo hi >$@")
"#],
    );
    t.write_file(
        "test//starlark/BUILD",
        &[r#"load("//test/starlark:extension.bzl", "macro")

macro(name = "rule1")
"#],
    );
    assert_eq!(
        t.eval("buildfiles(//test/starlark:BUILD)"),
        labels(&["//test/starlark:extension.bzl", "//test/starlark:BUILD"])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:231 QueryTest.testLoadfiles_starlarkDep
pub fn test_loadfiles_starlark_dep(t: &mut AbstractQueryTest) {
    t.write_file(
        "test//starlark/extension.bzl",
        &[r#"def macro(name):
    native.genrule(name = name, outs = [name + ".txt"], cmd = "echo hi >$@")
"#],
    );
    t.write_file(
        "test//starlark/BUILD",
        &[r#"load("//test/starlark:extension.bzl", "macro")

macro(name = "rule1")
"#],
    );
    assert_eq!(
        t.eval("loadfiles(//test/starlark:BUILD)"),
        labels(&["//test/starlark:extension.bzl"])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:252 QueryTest.testLoadfiles_sclDep
pub fn test_loadfiles_scl_dep(t: &mut AbstractQueryTest) {
    t.write_bzl_and_scl_files();
    assert_eq!(
        t.eval("loadfiles(//foo:BUILD)"),
        labels(&[
            "//bar:direct.scl",
            "//bar:indirect.scl",
            "//bar:intermediate.bzl",
            "//test_defs:foo_library.bzl"
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:277 QueryTest.testDeps_labelKeyedStringDictDeps
pub fn test_deps_label_keyed_string_dict_deps(t: &mut AbstractQueryTest) {
    t.write_file(
        "test//starlark/rule.bzl",
        &[r#"def _impl(ctx):
    return

my_rule = rule(
    implementation = _impl,
    attrs = {
        "value_dict": attr.label_keyed_string_dict(allow_files = True),
    },
)
"#],
    );
    t.write_file("test//starlark/dep.cc", &[]);
    t.write_file(
        "test//starlark/BUILD",
        &[r#"load("//test/starlark:rule.bzl", "my_rule")

filegroup(
    name = "group",
    srcs = ["dep.cc"],
)

my_rule(
    name = "rule",
    value_dict = {":group": "queried"},
)
"#],
    );
    assert_eq!(
        t.eval("deps(//test/starlark:rule)"),
        labels(&[
            "//test/starlark:rule",
            "//test/starlark:group",
            "//test/starlark:dep.cc"
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:313 QueryTest.testBuildfiles_transitiveStarlarkDeps
pub fn test_buildfiles_transitive_starlark_deps(t: &mut AbstractQueryTest) {
    t.write_file(
        "test//starlark/extension1.bzl",
        &[r#"def macro(name):
    native.genrule(name = name, outs = [name + ".txt"], cmd = "echo hi >$@")
"#],
    );
    t.write_file(
        "test//starlark/extension2.bzl",
        &[r#"load("//test/starlark:extension1.bzl", "macro")

def func(name):
    macro(name)
"#],
    );
    t.write_file(
        "test//starlark/BUILD",
        &[r#"load("//test/starlark:extension2.bzl", "func")

func(name = "rule1")
"#],
    );
    assert_eq!(
        t.eval("buildfiles(//test/starlark:BUILD)"),
        labels(&[
            "//test/starlark:extension1.bzl",
            "//test/starlark:extension2.bzl",
            "//test/starlark:BUILD"
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:346 QueryTest.testBuildfiles_diamondStarlarkDeps
pub fn test_buildfiles_diamond_starlark_deps(t: &mut AbstractQueryTest) {
    t.write_file(
        "test//starlark/extension1.bzl",
        &[r#"my_constant = "rule1"

def macro(name):
    native.genrule(name = name, outs = [name + ".txt"], cmd = "echo hi >$@")
"#],
    );
    t.write_file(
        "test//starlark/extension2.bzl",
        &[r#"load("//test/starlark:extension1.bzl", "macro")

def func(name):
    macro(name)
"#],
    );
    t.write_file(
        "test//starlark/extension3.bzl",
        &[r#"load("//test/starlark:extension1.bzl", "my_constant")

my_rule_name = my_constant
"#],
    );
    t.write_file(
        "test//starlark/extension4.bzl",
        &[r#"load("//test/starlark:extension2.bzl", "func")
load("//test/starlark:extension3.bzl", "my_rule_name")

my_dummy_name = my_rule_name
"#],
    );
    t.write_file(
        "test//starlark/BUILD",
        &[r#"load("//test/starlark:extension2.bzl", "func")
load("//test/starlark:extension3.bzl", "my_rule_name")
load("//test/starlark:extension4.bzl", "my_dummy_name")

func(name = my_rule_name + "-" + my_dummy_name)
"#],
    );
    assert_eq!(
        t.eval("buildfiles(//test/starlark:BUILD)"),
        labels(&[
            "//test/starlark:extension1.bzl",
            "//test/starlark:extension2.bzl",
            "//test/starlark:extension3.bzl",
            "//test/starlark:extension4.bzl",
            "//test/starlark:BUILD"
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:402 QueryTest.testBuildfiles_starlarkDepPackageBuildfileIncluded
pub fn test_buildfiles_starlark_dep_package_buildfile_included(t: &mut AbstractQueryTest) {
    t.write_file("test//starlark2/BUILD", &[]);
    t.write_file("test//starlark2/extension.bzl", &["file_ext = '.txt'"]);
    t.write_file("test//starlark1/BUILD", &[]);
    t.write_file(
        "test//starlark1/extension.bzl",
        &[r#"load("//test/starlark2:extension.bzl", "file_ext")

def macro(name):
    native.genrule(name = name, outs = [name + file_ext], cmd = "echo hi >$@")
"#],
    );
    t.write_file(
        "test/pkg/BUILD",
        &[r#"load("//test/starlark1:extension.bzl", "macro")

macro(name = "rule1")
"#],
    );
    assert_eq!(
        t.eval("buildfiles(//test/pkg:BUILD)"),
        labels(&[
            "//test/pkg:BUILD",
            "//test/starlark1:extension.bzl",
            "//test/starlark1:BUILD",
            "//test/starlark2:extension.bzl",
            "//test/starlark2:BUILD"
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:448 QueryTest.testQueryTimeLoadingWhenPackageIsMalformed
pub fn test_query_time_loading_when_package_is_malformed(t: &mut AbstractQueryTest) {
    t.write_file(
        "a/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'a') BUT WAIT THERE'S MORE",
        ],
    );
    let s = t.eval_throws("//a:a", false).message;
    let _ = &s;
    t.assert_contains_event("unclosed string literal");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:466 QueryTest.testQueryTimeLoadingOfSymlinkCyclePackage
pub fn test_query_time_loading_of_symlink_cycle_package(t: &mut AbstractQueryTest) {
    t.helper.ensure_symbolic_link("a/BUILD", "a/BUILD");
    let s = t.eval_throws("//a:*", false).message;
    let _ = &s;
    t.assert_contains_event("circular symlinks detected");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:481 QueryTest.boundedDepsQueryWithError
pub fn bounded_deps_query_with_error(t: &mut AbstractQueryTest) {
    t.write_file(
        "foo/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name ='foo', deps = ['//bar'])",
        ],
    );
    t.write_file(
        "bar/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name ='bar')",
        ],
    );
    t.write_file(
        "errorparent",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'errorparent', deps = ['//error'])",
        ],
    );
    t.write_file("error", &["has errors"]);
    t.eval_throws("deps(//foo:all + //errorparent:all, 25)", false);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:546 QueryTest.testQueryStarlarkComputedDefault
pub fn test_query_starlark_computed_default(t: &mut AbstractQueryTest) {
    t.write_starlark_defined_rule_class_bzl_file();
    t.write_file(
        "test//starlark/BUILD",
        &[r#"load("//test/starlark:extension.bzl", "custom_rule")

custom_rule(
    name = "custom",
    prefix = "a",
    suffix = "b",
)
"#],
    );
    let targets = t.eval("//test/starlark:*");
    assert_eq!(
        targets,
        labels(&[
            "//test/starlark:BUILD",
            "//test/starlark:custom",
            "//test/starlark:a01b",
            "//test/starlark:a02b"
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:682 QueryTest.testFileTargetLiteralInSubdirectory
pub fn test_file_target_literal_in_subdirectory(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["exports_files(glob(['**/*.txt']))"]);
    t.write_file("foo/bar/file1.txt", &[]);
    t.write_file("foo/bar/file2.txt", &[]);
    let targets = t.eval("foo/bar/file1.txt + foo/bar/file2.txt");
    assert_eq!(
        targets,
        labels(&["//foo:bar/file1.txt", "//foo:bar/file2.txt"])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:696 QueryTest.testShorthandTargetLiteralUnion
pub fn test_shorthand_target_literal_union(t: &mut AbstractQueryTest) {
    t.write_file(
        "foo/bar/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'bar')",
        ],
    );
    t.write_file(
        "foo/baz/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'baz')",
        ],
    );
    let targets = t.eval("foo/bar + foo/baz");
    assert_eq!(targets, labels(&["//foo/bar:bar", "//foo/baz:baz"]));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:713 QueryTest.testShorthandAbsoluteTargetLiteralUnion
pub fn test_shorthand_absolute_target_literal_union(t: &mut AbstractQueryTest) {
    t.write_file(
        "foo/bar/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'bar')",
        ],
    );
    t.write_file(
        "foo/baz/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'baz')",
        ],
    );
    let targets = t.eval("//foo/bar + //foo/baz");
    assert_eq!(targets, labels(&["//foo/bar:bar", "//foo/baz:baz"]));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:730 QueryTest.testLoadfilesWithDuplicates
pub fn test_loadfiles_with_duplicates(t: &mut AbstractQueryTest) {
    t.write_file(
        "foo/BUILD",
        &[r#"load("//bar:bar.bzl", "B")
load('//test_defs:foo_library.bzl', 'foo_library')
foo_library(
    name = "foo",
    deps = ["//bar"],
)
"#],
    );
    t.write_file(
        "bar/BUILD",
        &[r#"load("//bar:bar.bzl", "B")
load('//test_defs:foo_library.bzl', 'foo_library')

foo_library(name = "bar")
"#],
    );
    t.write_file("bar/bar.bzl", &["B = []"]);
    assert_eq!(
        t.eval_to_string("loadfiles(deps(//foo))"),
        "//bar:bar.bzl //test_defs:foo_library.bzl"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:787 QueryTest.testRdepsUnboundedWithNonDefaultDependencyFilter
pub fn test_rdeps_unbounded_with_non_default_dependency_filter(t: &mut AbstractQueryTest) {
    t.run_test_rdeps_with_non_default_dependency_filter(
        "rdeps(//foo:all, //foo:a)",
        "//foo:a //foo:b",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:792 QueryTest.testRdepsBoundedWithNonDefaultDependencyFilter
pub fn test_rdeps_bounded_with_non_default_dependency_filter(t: &mut AbstractQueryTest) {
    t.run_test_rdeps_with_non_default_dependency_filter(
        "rdeps(//foo:all, //foo:a, 1)",
        "//foo:a //foo:b",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:799 QueryTest.testDefaultVisibilityOfOutputTarget_noImplicitDeps
pub fn test_default_visibility_of_output_target_no_implicit_deps(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["package(default_visibility = [':pg'])\ngenrule(name = 'gen', srcs = ['in'], outs = ['out'], cmd = 'doesntmatter')\npackage_group(name = 'pg', includes = [':other-pg'])\npackage_group(name = 'other-pg')\n"]);
    t.assert_equals_filtered(
        format!(
            "{}{}",
            "deps(//foo:gen) + //foo:out + //foo:pg + //foo:other-pg",
            t.get_dependency_correction_with_gen()
        ),
        format!(
            "{}{}",
            "deps(//foo:out)",
            t.get_dependency_correction_with_gen()
        ),
        Setting::NoImplicitDeps,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:816 QueryTest.testDormantDepsAreReturned
pub fn test_dormant_deps_are_returned(t: &mut AbstractQueryTest) {
    t.write_file(
        "a/a.bzl",
        &[r#"def _impl(*args):
  fail("should not be called")

r = rule(
  implementation = _impl,
  dependency_resolution_rule = True,
  attrs = { "dormant": attr.dormant_label(), "dormant_list": attr.dormant_label_list() })
"#],
    );
    t.write_file(
        "a/BUILD",
        &[r#"load(":a.bzl", "r")
filegroup(name="a")
filegroup(name="b1")
filegroup(name="b2")

r(name="r", dormant=":a", dormant_list=[":b1", ":b2"])
"#],
    );
    assert!(
        t.eval_to_list_of_strings("deps('//a:r')")
            .contains(&String::from("//a:r")),
        "{:?}",
        t.eval_to_list_of_strings("deps('//a:r')")
    );
    assert!(
        t.eval_to_list_of_strings("deps('//a:r')")
            .contains(&String::from("//a:a")),
        "{:?}",
        t.eval_to_list_of_strings("deps('//a:r')")
    );
    assert!(
        t.eval_to_list_of_strings("deps('//a:r')")
            .contains(&String::from("//a:b1")),
        "{:?}",
        t.eval_to_list_of_strings("deps('//a:r')")
    );
    assert!(
        t.eval_to_list_of_strings("deps('//a:r')")
            .contains(&String::from("//a:b2")),
        "{:?}",
        t.eval_to_list_of_strings("deps('//a:r')")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:845 QueryTest.testMaterializerRuleQuery
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
    # interleave these to make it more interesting
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
    files = [dep[ComponentInfo].output for dep in ctx.attr.deps]
    return DefaultInfo(files = depset(direct = files))

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
    assert!(
        t.eval_to_list_of_strings("deps('//:bin')")
            .contains(&String::from("//:aaa")),
        "{:?}",
        t.eval_to_list_of_strings("deps('//:bin')")
    );
    assert!(
        t.eval_to_list_of_strings("deps('//:bin')")
            .contains(&String::from("//:a_yes")),
        "{:?}",
        t.eval_to_list_of_strings("deps('//:bin')")
    );
    assert!(
        t.eval_to_list_of_strings("deps('//:bin')")
            .contains(&String::from("//:b_yes")),
        "{:?}",
        t.eval_to_list_of_strings("deps('//:bin')")
    );
    assert!(
        t.eval_to_list_of_strings("deps('//:bin')")
            .contains(&String::from("//:c_no")),
        "{:?}",
        t.eval_to_list_of_strings("deps('//:bin')")
    );
    assert!(
        t.eval_to_list_of_strings("deps('//:bin')")
            .contains(&String::from("//:d_no")),
        "{:?}",
        t.eval_to_list_of_strings("deps('//:bin')")
    );
    assert!(
        t.eval_to_list_of_strings("deps('//:bin')")
            .contains(&String::from("//:zzz")),
        "{:?}",
        t.eval_to_list_of_strings("deps('//:bin')")
    );
    assert!(
        t.eval_to_list_of_strings("deps('//:bin')")
            .contains(&String::from("//:component_selector")),
        "{:?}",
        t.eval_to_list_of_strings("deps('//:bin')")
    );
    let direct_deps = t.eval_to_list_of_strings("deps('//:bin', 1)");
    assert!(
        direct_deps.contains(&String::from("//:aaa")),
        "{:?}",
        direct_deps
    );
    assert!(
        direct_deps.contains(&String::from("//:component_selector")),
        "{:?}",
        direct_deps
    );
    assert!(
        direct_deps.contains(&String::from("//:zzz")),
        "{:?}",
        direct_deps
    );
    assert!(
        !direct_deps.contains(&String::from("//:a_yes")),
        "{:?}",
        direct_deps
    );
    assert!(
        !direct_deps.contains(&String::from("//:b_yes")),
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
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:941 QueryTest.testMaterializerRuleRealDepsQuery
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
    attrs = {
        "all_components": attr.label_list(),
    },
)

# Binary #########################################

def _binary_impl(ctx):
    files = [dep[ComponentInfo].output for dep in ctx.attr.deps]
    return DefaultInfo(files = depset(direct = files))

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
    let all_deps = t.eval_to_list_of_strings("deps('//:bin')");
    assert!(all_deps.contains(&String::from("//:aaa")), "{:?}", all_deps);
    assert!(all_deps.contains(&String::from("//:a")), "{:?}", all_deps);
    assert!(all_deps.contains(&String::from("//:b")), "{:?}", all_deps);
    assert!(all_deps.contains(&String::from("//:c")), "{:?}", all_deps);
    assert!(all_deps.contains(&String::from("//:d")), "{:?}", all_deps);
    assert!(all_deps.contains(&String::from("//:zzz")), "{:?}", all_deps);
    assert!(
        all_deps.contains(&String::from("//:component_selector")),
        "{:?}",
        all_deps
    );
    let direct_deps = t.eval_to_list_of_strings("deps('//:bin', 1)");
    assert!(
        direct_deps.contains(&String::from("//:aaa")),
        "{:?}",
        direct_deps
    );
    assert!(
        direct_deps.contains(&String::from("//:component_selector")),
        "{:?}",
        direct_deps
    );
    assert!(
        direct_deps.contains(&String::from("//:zzz")),
        "{:?}",
        direct_deps
    );
    assert!(
        !direct_deps.contains(&String::from("//:a")),
        "{:?}",
        direct_deps
    );
    assert!(
        !direct_deps.contains(&String::from("//:b")),
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
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:154 QueryTest.testFindsAllTargets_symbolicMacro_withLazyMacroExpansion
pub fn test_finds_all_targets_symbolic_macro_with_lazy_macro_expansion(t: &mut AbstractQueryTest) {
    // setLazyMacroExpansionPackages(LazyMacroExpansionPackages.ALL)
    t.helper.set_lazy_macro_expansion_packages("*");
    t.write_file(
        "test//starlark/extension.bzl",
        &["def _rule_impl(ctx):
    return None

starlark_rule = rule(implementation = _rule_impl)

def _macro_impl(name, visibility):
    starlark_rule(name = name, visibility = visibility)
    native.genrule(name = name + \"_gen\", outs = [name + \"_gen.txt\"], cmd = \"echo hi >$@\")

symbolic_macro = macro(implementation = _macro_impl)
"],
    );

    t.write_file(
        "test//starlark/BUILD",
        &["load(\"//test/starlark:extension.bzl\", \"symbolic_macro\")

symbolic_macro(name = \"foo\")
"],
    );

    assert_eq!(
        t.eval("//test/starlark:*"),
        labels(&[
            "//test/starlark:BUILD",
            "//test/starlark:foo",
            "//test/starlark:foo_gen",
            "//test/starlark:foo_gen.txt",
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:209 QueryTest.testBuildfiles_starlarkDep_withLazyMacroExpansion
pub fn test_buildfiles_starlark_dep_with_lazy_macro_expansion(t: &mut AbstractQueryTest) {
    t.helper.set_lazy_macro_expansion_packages("*");
    t.write_file(
        "test//starlark/extension.bzl",
        &["def macro(name):
    native.genrule(name = name, outs = [name + \".txt\"], cmd = \"echo hi >$@\")
"],
    );

    t.write_file(
        "test//starlark/BUILD",
        &["load(\"//test/starlark:extension.bzl\", \"macro\")

macro(name = \"rule1\")
"],
    );

    assert_eq!(
        t.eval("buildfiles(//test/starlark:BUILD)"),
        labels(&["//test/starlark:extension.bzl", "//test/starlark:BUILD"])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:264 QueryTest.testLoadfiles_sclDep_withLazyMacroExpansion
pub fn test_loadfiles_scl_dep_with_lazy_macro_expansion(t: &mut AbstractQueryTest) {
    t.helper.set_lazy_macro_expansion_packages("*");
    t.write_bzl_and_scl_files();

    assert_eq!(
        t.eval("loadfiles(//foo:BUILD)"),
        labels(&[
            "//bar:direct.scl",
            "//bar:indirect.scl",
            "//bar:intermediate.bzl",
            "//test_defs:foo_library.bzl",
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:434 QueryTest.testQueryTimeLoadingWhenPackageDoesNotExist
pub fn test_query_time_loading_when_package_does_not_exist(t: &mut AbstractQueryTest) {
    // Given a workspace containing a package "//a",
    t.write_file(
        "a/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'a')",
        ],
    );

    // When the query environment is queried for "//a/b:b" which doesn't exist,
    let non_existent_package = "a/b";
    let s = t
        .eval_throws(&format!("//{non_existent_package}:itsNotThere"), false)
        .message;

    // Then an exception is thrown that says that the specified package does not exist.
    let expected = format!("no such package '{non_existent_package}'");
    assert!(s.contains(&expected), "{s}");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:500 QueryTest.testIgnoredSubdirectories
pub fn test_ignored_subdirectories(t: &mut AbstractQueryTest) {
    t.use_reduced_set_of_rules();
    let ignored = t.helper.get_ignored_subdirectories_file();
    t.write_file(ignored.get_path_string(), &["a/b", "a/c"]);
    t.write_file("a/BUILD", &["filegroup(name = 'a')"]);
    t.write_file("b/BUILD", &["filegroup(name = 'b')"]);
    t.write_file("a/b/BUILD", &["filegroup(name = 'a_b')"]);
    t.write_file("a/c/BUILD", &["filegroup(name = 'a_c')"]);
    t.write_file("a/d/BUILD", &["filegroup(name = 'a_d')"]);
    t.write_file("a/e/BUILD", &["filegroup(name = 'a_e')"]);
    // Ensure that modified files are invalidated in the skyframe. If a file has
    // already been read prior to the test's writes, this forces the query to
    // pick up the modified versions.
    t.helper.maybe_handle_diffs();
    let result = t.eval("//...");
    for label in ["//a:a", "//b:b", "//a/d:a_d", "//a/e:a_e"] {
        assert!(result.contains(label), "{label} not in {result:?}");
    }
    for label in ["//a/b:a_b", "//a/c:a_c"] {
        assert!(!result.contains(label), "{label} in {result:?}");
    }
    let result = t.eval("//a/...");
    assert_eq!(result, labels(&["//a:a", "//a/d:a_d", "//a/e:a_e"]));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:570 QueryTest.testQueryStarlarkComputedDefaultWithConfigurableDeps
pub fn test_query_starlark_computed_default_with_configurable_deps(t: &mut AbstractQueryTest) {
    t.write_starlark_defined_rule_class_bzl_file();
    t.write_file(
        "test//starlark/BUILD",
        &[
            "load('//test/starlark:extension.bzl', 'custom_rule')",
            "",
            "config_setting(",
            "    name = 'cfg_a',",
            "    values = {'test_arg': 'something'})",
            "config_setting(",
            "    name = 'cfg_b',",
            "    values = {'test_arg': 'something_else'})",
            "",
            "custom_rule(",
            "    name = 'custom',",
            "    prefix = select({",
            "        ':cfg_a':'a',",
            "        ':cfg_b':'b',",
            "        '//conditions:default':'def'}),",
            "    suffix = select({",
            "        ':cfg_a':'a',",
            "        ':cfg_b':'b',",
            "        '//conditions:default':'def'}))",
        ],
    );

    let mut computed_labels = Vec::new();
    for prefix in ["a", "b", "def"] {
        for middle in ["01", "02"] {
            for suffix in ["a", "b", "def"] {
                computed_labels.push(format!("//test/starlark:{prefix}{middle}{suffix}"));
            }
        }
    }

    let targets = t.eval("//test/starlark:*");
    for label in [
        "//test/starlark:BUILD",
        "//test/starlark:cfg_a",
        "//test/starlark:cfg_b",
        "//test/starlark:custom",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain(computed_labels)
    {
        assert!(targets.contains(&label), "{label} not in {targets:?}");
    }
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:617 QueryTest.testQueryStarlarkComputedDefaultWithConfigurableDepsUsedTwice
pub fn test_query_starlark_computed_default_with_configurable_deps_used_twice(
    t: &mut AbstractQueryTest,
) {
    t.write_starlark_defined_rule_class_bzl_file();
    t.write_file(
        "test//starlark/BUILD",
        &[
            "load('//test/starlark:extension.bzl', 'custom_rule')",
            "",
            "config_setting(",
            "    name = 'cfg_a',",
            "    values = {'test_arg': 'something'})",
            "config_setting(",
            "    name = 'cfg_b',",
            "    values = {'test_arg': 'something_else'})",
            "",
            "custom_rule(",
            "    name = 'custom_one',",
            "    prefix = select({",
            "        ':cfg_a':'a_one',",
            "        ':cfg_b':'b_one',",
            "        '//conditions:default':'def_one'}),",
            "    suffix = select({",
            "        ':cfg_a':'a_one',",
            "        ':cfg_b':'b_one',",
            "        '//conditions:default':'def_one'}))",
            "custom_rule(",
            "    name = 'custom_two',",
            "    prefix = select({",
            "        ':cfg_a':'a_two',",
            "        ':cfg_b':'b_two',",
            "        '//conditions:default':'def_two'}),",
            "    suffix = select({",
            "        ':cfg_a':'a_two',",
            "        ':cfg_b':'b_two',",
            "        '//conditions:default':'def_two'}))",
        ],
    );

    let mut computed_labels = Vec::new();
    for (prefixes, suffixes) in [
        (["a_one", "b_one", "def_one"], ["a_one", "b_one", "def_one"]),
        (["a_two", "b_two", "def_two"], ["a_two", "b_two", "def_two"]),
    ] {
        for prefix in prefixes {
            for middle in ["01", "02"] {
                for suffix in suffixes {
                    computed_labels.push(format!("//test/starlark:{prefix}{middle}{suffix}"));
                }
            }
        }
    }
    let mut expected: TargetSet = [
        "//test/starlark:BUILD",
        "//test/starlark:cfg_a",
        "//test/starlark:cfg_b",
        "//test/starlark:custom_one",
        "//test/starlark:custom_two",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // containsExactlyElementsIn: the computed labels are distinct.
    let n = 5 + computed_labels.len();
    expected.extend(computed_labels);
    assert_eq!(expected.len(), n);

    assert_eq!(t.eval("//test/starlark:*"), expected);
}
