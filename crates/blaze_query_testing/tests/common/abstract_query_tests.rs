//! The tests of Java's `AbstractQueryTest` (generic over the suite).
//!
//! Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java

use super::*;

#[allow(unused_imports)]
use blaze_query_engine::Setting;

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:298 AbstractQueryTest.testTargetLiterals
pub fn test_target_literals(t: &mut AbstractQueryTest) {
    t.write_build_files1();
    assert_eq!(t.eval_to_string("a/b:*"), AB_FILES);
    assert_eq!(t.eval_to_string("a/...:*"), A_AB_FILES);
    assert_eq!(t.eval_to_string("a:*"), A_FILES);
    assert_eq!(t.eval_to_string("//a:x"), "//a:x");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:307 AbstractQueryTest.testBadTargetLiterals
pub fn test_bad_target_literals(t: &mut AbstractQueryTest) {
    let result = t.eval_throws("bad:*:*", false);
    t.check_resultof_bad_target_literals(result.message, result.failure_detail);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:319 AbstractQueryTest.testAlgebraicSetOperations
pub fn test_algebraic_set_operations(t: &mut AbstractQueryTest) {
    t.write_build_files1();
    assert_eq!(t.eval_to_string("a/...:* intersect a/b/...:*"), AB_FILES);
    assert_eq!(t.eval_to_string("a/b/...:* intersect a/...:*"), AB_FILES);
    assert_eq!(
        t.eval_to_string("//a:x union a/b/...:*"),
        format!("{}{}", AB_FILES, " //a:x")
    );
    assert_eq!(
        t.eval_to_string("a/b/...:* union //a:x"),
        format!("{}{}", AB_FILES, " //a:x")
    );
    assert_eq!(t.eval_to_string("a/...:* except a/b/...:*"), A_FILES);
    assert_eq!(t.eval_to_string("a/b/...:* except a/...:*"), "");
    assert_eq!(
        t.eval_to_string("(a/...:* union a/b/...:*) except //a/b:p"),
        format!("{}{}", "//a/b:BUILD //a/b:q ", A_FILES)
    );
    assert_eq!(
        t.eval_to_string("a/...:* union (a/b/...:* except //a/b:p)"),
        A_AB_FILES
    );
    assert_eq!(
        t.eval_to_string("a/...:* + (a/b/...:* - //a/b:p)"),
        A_AB_FILES
    );
    assert_eq!(t.eval_to_string("a/...:* ^ a/b/...:*"), AB_FILES);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:338 AbstractQueryTest.testAlgebraicSetOperations_manyOperands
pub fn test_algebraic_set_operations_many_operands(t: &mut AbstractQueryTest) {
    t.write_build_files1();
    assert_eq!(
        t.eval_to_string("//a:BUILD + //a:x + //a:y + //a:z + //a/b:BUILD + //a/b:p + //a/b:q"),
        A_AB_FILES
    );
    assert_eq!(
        t.eval_to_string(
            "a/...:* - //a:BUILD - //a:x - //a:y - //a:z - //a/b:BUILD - //a/b:p - //a/b:q"
        ),
        ""
    );
    assert_eq!(
        t.eval_to_string(
            "(//a:x + //a:y) ^ (//a:x + //a:z) ^ (//a:x + //a/b:p) ^ (//a:x + //a/b:q)"
        ),
        "//a:x"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:366 AbstractQueryTest.testKindOperator
pub fn test_kind_operator(t: &mut AbstractQueryTest) {
    t.write_build_files2();
    assert_eq!(
        t.eval_to_string("c:*"),
        format!(
            "{}{}",
            "//c:BUILD //c:c //c:d //c:d.dwp //c:d.stripped //c:e.cc //c:f //c:f.dwp //c:f.stripped",
            " //c:g.cc //c:p //c:q //c:r //c:s"
        )
    );
    assert_eq!(t.eval_to_string("kind(rule, c:*)"), "//c:c //c:d //c:f");
    assert_eq!(t.eval_to_string("kind(genrule, c:*)"), "//c:c");
    assert_eq!(t.eval_to_string("kind(cc.*, c:*)"), "//c:d //c:f");
    assert_eq!(
        t.eval_to_string("kind(file, c:*)"),
        format!(
            "{}{}",
            "//c:BUILD //c:d.dwp //c:d.stripped //c:e.cc //c:f.dwp //c:f.stripped //c:g.cc //c:p",
            " //c:q //c:r //c:s"
        )
    );
    assert_eq!(
        t.eval_to_string("kind(gener.*, c:*)"),
        "//c:d.dwp //c:d.stripped //c:f.dwp //c:f.stripped //c:r //c:s"
    );
    assert_eq!(
        t.eval_to_string("kind(gen.*, c:*)"),
        "//c:c //c:d.dwp //c:d.stripped //c:f.dwp //c:f.stripped //c:r //c:s"
    );
    assert_eq!(
        t.eval_to_string("kind(source, c:*)"),
        "//c:BUILD //c:e.cc //c:g.cc //c:p //c:q"
    );
    assert_eq!(
        t.eval_to_string("kind('source file', c:*)"),
        "//c:BUILD //c:e.cc //c:g.cc //c:p //c:q"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:390 AbstractQueryTest.testFilterOperator
pub fn test_filter_operator(t: &mut AbstractQueryTest) {
    t.write_build_files2();
    assert_eq!(
        t.eval_to_string("c:*"),
        format!(
            "{}{}",
            "//c:BUILD //c:c //c:d //c:d.dwp //c:d.stripped //c:e.cc //c:f //c:f.dwp //c:f.stripped",
            " //c:g.cc //c:p //c:q //c:r //c:s"
        )
    );
    assert_eq!(t.eval_to_string("filter(BUILD, c:*)"), "//c:BUILD");
    assert_eq!(
        t.eval_to_string(r#"filter('\.cc$', c:*)"#),
        "//c:e.cc //c:g.cc"
    );
    assert_eq!(
        t.eval_to_string("filter(//c.*cc$, c:*)"),
        "//c:e.cc //c:g.cc"
    );
    assert_eq!(
        t.eval_to_string("filter(:.$, c:*)"),
        "//c:c //c:d //c:f //c:p //c:q //c:r //c:s"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:404 AbstractQueryTest.testAttrOperatorOnName
pub fn test_attr_operator_on_name(t: &mut AbstractQueryTest) {
    t.write_build_files2();
    t.write_build_files_with_configurable_attributes();
    assert_eq!(
        t.eval_to_string("attr(name, '.*', '//c:*')"),
        "//c:c //c:d //c:f"
    );
    assert_eq!(
        t.eval_to_string("attr(name, '.+', '//c:*')"),
        "//c:c //c:d //c:f"
    );
    assert_eq!(t.eval_to_string("attr(name, '.*d.*', '//c:*')"), "//c:d");
    assert_eq!(t.eval_to_string("attr(name, '.*e.*', '//c:*')"), "");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:416 AbstractQueryTest.testAttrOperator
pub fn test_attr_operator(t: &mut AbstractQueryTest) {
    t.write_build_files2();
    t.write_build_files_with_configurable_attributes();
    assert_eq!(
        t.eval_to_string("c:*"),
        format!(
            "{}{}",
            "//c:BUILD //c:c //c:d //c:d.dwp //c:d.stripped //c:e.cc //c:f //c:f.dwp //c:f.stripped",
            " //c:g.cc //c:p //c:q //c:r //c:s"
        )
    );
    assert_eq!(t.eval_to_string("attr(cmd,':', c:*)"), "//c:c");
    assert_eq!(t.eval_to_string("attr(cmd,'', c:*)"), "//c:c");
    assert_eq!(t.eval_to_string("attr(linkshared, 0, c:*)"), "//c:d //c:f");
    assert_eq!(t.eval_to_string("attr('data', 'r', c:*)"), "//c:d");
    assert_eq!(
        t.eval_to_string(r#"attr('deps', '\[\]', c:*)"#),
        "//c:d //c:f"
    );
    assert_eq!(t.eval_to_string("attr('deps', '^..$', c:*)"), "//c:d //c:f");
    assert_eq!(
        t.eval_to_string(r#"attr('srcs', '\[[^,]+\]', c:*)"#),
        "//c:d //c:f"
    );
    if t.test_configurable_attributes() {
        assert_eq!(
            t.eval_to_string("attr('deps', 'bdep', //configurable/...)"),
            "//configurable:main"
        );
        assert_eq!(
            t.eval_to_string("attr('deps', 'nomatch', //configurable/...)"),
            ""
        );
    }
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:445 AbstractQueryTest.testNullAttrOperator
pub fn test_null_attr_operator(t: &mut AbstractQueryTest) {
    t.write_build_files2();
    assert_eq!(t.eval_to_string("attr(deprecation, ' ', c:*)"), "");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:451 AbstractQueryTest.testAttrOperatorOnBooleans
pub fn test_attr_operator_on_booleans(t: &mut AbstractQueryTest) {
    t.write_file(
        "t/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(name='t', srcs=['t.cc'], data=['r'], testonly=0)
cc_library(name='t_test', srcs=['t.cc'], data=['r'], testonly=1)
"#],
    );
    assert_eq!(t.eval_to_string("attr(testonly, 0, t:*)"), "//t:t");
    assert_eq!(t.eval_to_string("attr(testonly, 1, t:*)"), "//t:t_test");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:494 AbstractQueryTest.testGenqueryScope
pub fn test_genquery_scope(t: &mut AbstractQueryTest) {
    t.run_genquery_scope_test(false);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:499 AbstractQueryTest.testAttrOnPackageDefaultVisibility
pub fn test_attr_on_package_default_visibility(t: &mut AbstractQueryTest) {
    t.write_file(
        "t/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
package(default_visibility=['//visibility:public'])
cc_library(name='t', srcs=['t.cc'])
"#],
    );
    assert_eq!(t.eval_to_string("attr(visibility, public, t:*)"), "//t:t");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:512 AbstractQueryTest.testSomeOperator_noCountParameter
pub fn test_some_operator_no_count_parameter(t: &mut AbstractQueryTest) {
    t.write_build_files2();
    assert_eq!(t.eval("some(c:*)").len(), 1, "{:?}", t.eval("some(c:*)"));
    assert_contains(t.eval("c:*"), t.eval("some(c:*)"));
    assert_eq!(t.eval_to_string("some(//c:q)"), "//c:q");
    let result = t.eval_throws("some(//c:q intersect //c:p)", true);
    assert_eq!(result.message, "argument set is empty");
    assert_eq!(
        query_code(&result.failure_detail),
        Some(blaze_protos::failure_details::query::Code::ArgumentsMissing as i32)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:524 AbstractQueryTest.testSomeOperator_countParameterNotEqualActualCount
pub fn test_some_operator_count_parameter_not_equal_actual_count(t: &mut AbstractQueryTest) {
    t.write_build_files2();
    assert_eq!(
        t.eval("some(//c:p + //c:q, 5)").len(),
        2,
        "{:?}",
        t.eval("some(//c:p + //c:q, 5)")
    );
    assert_eq!(t.eval_to_string("some(//c:p + //c:q, 5)"), "//c:p //c:q");
    assert_eq!(
        t.eval("some(//c:c + //c:d + //c:p + //c:q + //c:r + //c:s, 3)")
            .len(),
        3,
        "{:?}",
        t.eval("some(//c:c + //c:d + //c:p + //c:q + //c:r + //c:s, 3)")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:534 AbstractQueryTest.testSomeOperator_nestedSomeTest
pub fn test_some_operator_nested_some_test(t: &mut AbstractQueryTest) {
    t.write_build_files2();
    assert_eq!(
        t.eval("some(some(//c:p + //c:q, 2) + some(//c:p + //c:s + //c:q, 3), 5)")
            .len(),
        3,
        "{:?}",
        t.eval("some(some(//c:p + //c:q, 2) + some(//c:p + //c:s + //c:q, 3), 5)")
    );
    assert_eq!(
        t.eval_to_string("some(some(//c:p + //c:q, 2) + some(//c:p + //c:s + //c:q, 3), 5)"),
        "//c:p //c:q //c:s"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:670 AbstractQueryTest.testAllPathsOperator
pub fn test_all_paths_operator(t: &mut AbstractQueryTest) {
    t.write_build_files3();
    t.write_build_files_with_configurable_attributes();
    assert_eq!(t.eval("somepath(//a, //a:a2)"), empty_set());
    assert_eq!(t.eval("somepath(//d, //a)"), empty_set());
    let allpaths_ato_d = t.eval("allpaths(//a, //d)");
    assert!(allpaths_ato_d.is_superset(&t.eval("//a + //b + //c + //d")));
    if t.test_configurable_attributes() {
        assert_eq!(
            t.eval("allpaths(//configurable:main, //configurable:bdep.cc)"),
            t.eval("//configurable:main + //configurable:bdep + //configurable:bdep.cc")
        );
    }
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:688 AbstractQueryTest.testPathOperatorsWithOutputFile
pub fn test_path_operators_with_output_file(t: &mut AbstractQueryTest) {
    t.write_file("a/BUILD", &["genrule(name='a', outs=['out'], cmd=':')"]);
    assert_eq!(t.eval("somepath(//a, //a:out)"), empty_set());
    assert_eq!(t.eval("allpaths(//a, //a:out)"), empty_set());
    assert_eq!(t.eval("somepath(//a:out, //a)"), t.eval("//a + //a:out"));
    assert_eq!(t.eval("allpaths(//a:out, //a)"), t.eval("//a + //a:out"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:758 AbstractQueryTest.testDepsDoesNotIncludeBuildFiles
pub fn test_deps_does_not_include_build_files(t: &mut AbstractQueryTest) {
    t.write_file(
        "deps/BUILD",
        &["exports_files(['build_def', 'starlark.bzl'])"],
    );
    t.write_file("deps/starlark.bzl", &["def macro():\n  native.genrule(name = 'dep2', outs = ['dep2.txt'], cmd = 'echo Hi >$@')\n"]);
    t.write_file("s/BUILD", &["load('//deps:starlark.bzl', 'macro')\nmacro()\ngenrule(name = 'my_rule',\n        outs = ['my.txt'],\n        srcs = [':dep1.txt', ':dep2.txt'],\n        cmd = 'echo $(SRCS) >$@')\n"]);
    let result = t.eval_to_list_of_strings("deps(//s:my_rule)");
    assert!(result.contains(&String::from("//s:dep2")), "{:?}", result);
    assert!(
        result.contains(&String::from("//s:dep1.txt")),
        "{:?}",
        result
    );
    assert!(
        result.contains(&String::from("//s:dep2.txt")),
        "{:?}",
        result
    );
    assert!(
        result.contains(&String::from("//s:my_rule")),
        "{:?}",
        result
    );
    assert!(
        !result.contains(&String::from("//deps:BUILD")),
        "{:?}",
        result
    );
    assert!(
        !result.contains(&String::from("//deps:build_def")),
        "{:?}",
        result
    );
    assert!(
        !result.contains(&String::from("//deps:starlark.bzl")),
        "{:?}",
        result
    );
    assert!(!result.contains(&String::from("//s:BUILD")), "{:?}", result);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:823 AbstractQueryTest.testAspectOnRuleWithoutDeclaredProviders
pub fn test_aspect_on_rule_without_declared_providers(t: &mut AbstractQueryTest) {
    t.helper.set_query_settings(&[Setting::IncludeAspects]);
    t.write_aspect_definition("{'_extra_deps' : attr.label(default = Label('//test:z'))}");
    t.write_file("test/BUILD", &["load('//test:aspect.bzl', 'aspect_rule', 'plain_rule')\naspect_rule(name='a', attr=[':b'])\nplain_rule(name='b')\nplain_rule(name='z')\n"]);
    assert!(
        t.eval("deps(//test:a)")
            .is_superset(&t.eval("//test:b + //test:z"))
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:839 AbstractQueryTest.testQueryStarlarkAspects
pub fn test_query_starlark_aspects(t: &mut AbstractQueryTest) {
    t.helper.set_query_settings(&[Setting::IncludeAspects]);
    t.write_aspect_definition("{'_extra_deps' : attr.label(default = Label('//prod:zzz'))}");
    t.write_file("test/BUILD", &["load('//test:aspect.bzl', 'aspect_rule', 'plain_rule')\nplain_rule(\n     name = 'yyy',\n)\naspect_rule(\n     name = 'xxx',\n     attr = [':yyy'],\n)\naspect_rule(\n     name = 'qqq',\n     attr = ['//test:yyy'],\n)\n"]);
    assert!(
        t.eval("deps(//test:xxx)")
            .is_superset(&t.eval("//prod:zzz + //test:yyy"))
    );
    assert!(
        t.eval("deps(//test:qqq)")
            .is_superset(&t.eval("//prod:zzz + //test:yyy"))
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:864 AbstractQueryTest.testQueryStarlarkAspectWithParameters
pub fn test_query_starlark_aspect_with_parameters(t: &mut AbstractQueryTest) {
    t.helper.set_query_settings(&[Setting::IncludeAspects]);
    t.write_aspect_definition(
        (format!(
            "{}{}",
            "{'_extra_deps' : attr.label(default = Label('//prod:zzz')),",
            "'param' : attr.string(values=['a', 'b']) }"
        ))
        .as_str(),
    );
    t.write_file("test/BUILD", &["load('//test:aspect.bzl', 'aspect_rule', 'plain_rule')\nplain_rule(\n     name = 'yyy',\n)\naspect_rule(\n     name = 'xxx',\n     attr = [':yyy'],\n     param = 'a',\n)\naspect_rule(\n     name = 'qqq',\n     attr = ['//test:yyy'],\n     param = 'b',\n)\n"]);
    assert!(
        t.eval("deps(//test:xxx)")
            .is_superset(&t.eval("//prod:zzz + //test:yyy"))
    );
    assert!(
        t.eval("deps(//test:qqq)")
            .is_superset(&t.eval("//prod:zzz + //test:yyy"))
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:893 AbstractQueryTest.testQueryStarlarkAspectsNoImplicitDeps
pub fn test_query_starlark_aspects_no_implicit_deps(t: &mut AbstractQueryTest) {
    t.helper.set_query_settings(&[Setting::IncludeAspects]);
    t.write_aspect_definition("{'_extra_deps':attr.label(default = Label('//prod:zzz'))}");
    t.write_file("test/BUILD", &["load('//test:aspect.bzl', 'aspect_rule', 'plain_rule')\nplain_rule(\n     name = 'yyy',\n)\naspect_rule(\n     name = 'xxx',\n     attr = [':yyy'],\n)\n"]);
    t.helper.set_query_settings(&[Setting::NoImplicitDeps]);
    assert!(
        t.eval("deps(//test:xxx)")
            .is_disjoint(&t.eval("//prod:zzz"))
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:914 AbstractQueryTest.testStarlarkDiamondEquality
pub fn test_starlark_diamond_equality(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//foo:a.bzl', 'A')\nload('//foo:b.bzl', 'B')\nload('//foo:checker.bzl', 'check')\nload('//test_defs:foo_library.bzl', 'foo_library')\ncheck(A.c, B.c)\ncheck(B.a, A)\nfoo_library(name = 'foo')\n"]);
    t.write_file("foo/a.bzl", &["load('//foo:c.bzl', 'C')\nA = struct(c = C)\n# comment to make sure this formats properly\n"]);
    t.write_file(
        "foo/b.bzl",
        &["load('//foo:a.bzl', 'A')\nload('//foo:c.bzl', 'C')\nB = struct(a = A, c = C)\n"],
    );
    t.write_file("foo/c.bzl", &["C = struct()"]);
    t.write_file("foo/checker.bzl", &["def check(arg1, arg2):\n  if arg1 != arg2:\n    fail('Long error message just saying that the two args passed in were not equal')\n"]);
    assert_eq!(t.eval_to_string("//foo:foo"), "//foo:foo");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:953 AbstractQueryTest.testRdeps
pub fn test_rdeps(t: &mut AbstractQueryTest) {
    t.write_build_files3();
    t.write_build_files_with_configurable_attributes();
    assert_eq!(
        t.eval((format!("{}{}", "rdeps(//a, //d)", t.get_dependency_correction())).as_str()),
        t.eval("//a union //b union //c union //d")
    );
    assert_eq!(
        t.eval((format!("{}{}", "rdeps(//b, //d)", t.get_dependency_correction())).as_str()),
        t.eval("//b union //d")
    );
    assert_eq!(
        t.eval(
            (format!(
                "{}{}",
                "rdeps(//b union //c, //d)",
                t.get_dependency_correction()
            ))
            .as_str()
        ),
        t.eval("//b union //c union //d")
    );
    assert_eq!(
        t.eval(
            (format!(
                "{}{}",
                "rdeps(//a union //c, //b)",
                t.get_dependency_correction()
            ))
            .as_str()
        ),
        t.eval("//a union //b")
    );
    assert_eq!(
        t.eval(
            (format!(
                "{}{}",
                "rdeps(//a:out union //c:out, //b)",
                t.get_dependency_correction()
            ))
            .as_str()
        ),
        t.eval("//a union //a:out union //b")
    );
    assert_eq!(
        t.eval((format!("{}{}", "rdeps(//d, //a)", t.get_dependency_correction())).as_str()),
        empty_set()
    );
    assert_eq!(
        t.eval((format!("{}{}", "rdeps(//a, //d, 1)", t.get_dependency_correction())).as_str()),
        t.eval("//b union //c union //d")
    );
    assert_eq!(
        t.eval((format!("{}{}", "rdeps(//a, //d, 0)", t.get_dependency_correction())).as_str()),
        t.eval("//d")
    );
    if t.test_configurable_attributes() {
        assert_eq!(
            t.eval("rdeps(//configurable:all, //configurable:adep.cc)"),
            t.eval("//configurable:main + //configurable:adep + //configurable:adep.cc")
        );
        assert_eq!(
            t.eval("rdeps(//configurable:all, //configurable:bdep.cc)"),
            t.eval("//configurable:main + //configurable:bdep + //configurable:bdep.cc")
        );
        assert_eq!(
            t.eval("rdeps(//configurable:all, //configurable:defaultdep.cc)"),
            t.eval(
                (format!(
                    "{}{}",
                    "//configurable:main + //configurable:defaultdep + ",
                    "//configurable:defaultdep.cc"
                ))
                .as_str()
            )
        );
    }
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:989 AbstractQueryTest.testLet
pub fn test_let(t: &mut AbstractQueryTest) {
    t.write_build_files3();
    t.helper.set_query_settings(&[Setting::NoImplicitDeps]);
    assert_contains(
        t.eval("//b + //c + //d"),
        t.eval(
            (format!(
                "{}{}",
                "let x = //a in deps($x) except $x",
                t.get_dependency_correction_with_gen()
            ))
            .as_str(),
        ),
    );
    let result = t.eval_throws("$undefined", true);
    assert_eq!(result.message, "undefined variable 'undefined'");
    assert_eq!(
        query_code(&result.failure_detail),
        Some(blaze_protos::failure_details::query::Code::VariableUndefined as i32)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1022 AbstractQueryTest.testSubdirSymlinkCycle
pub fn test_subdir_symlink_cycle(t: &mut AbstractQueryTest) {
    t.write_build_files1();
    t.helper.ensure_symbolic_link("a/s", "s");
    assert_eq!(t.eval_to_string("a/...:*"), A_AB_FILES);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1029 AbstractQueryTest.testCycleInSubpackage
pub fn test_cycle_in_subpackage(t: &mut AbstractQueryTest) {
    t.write_file("a/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'a', deps = [':dep'])\nfoo_library(name = 'dep')\n"]);
    t.write_file(
        "a/subdir/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'cycletarget', deps = ['cycletarget'])",
        ],
    );
    assert_eq!(
        sorted(t.eval_to_list_of_strings("deps(//a:a)")),
        sorted(vec![String::from("//a:a"), String::from("//a:dep")])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1071 AbstractQueryTest.testCycleInStarlarkParentDir
pub fn test_cycle_in_starlark_parent_dir(t: &mut AbstractQueryTest) {
    t.setup_cycle_in_starlark_parent_dir();
    assert_eq!(
        sorted(t.eval_to_list_of_strings("//a/subdir:all")),
        sorted(vec![String::from("//a/subdir:subdir")])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1077 AbstractQueryTest.testNestedLetExpressions
pub fn test_nested_let_expressions(t: &mut AbstractQueryTest) {
    t.write_file("a/BUILD", &["exports_files(['f1', 'f2'])"]);
    t.write_file("b/BUILD", &["exports_files(['f1', 'f2'])"]);
    let let_query = format!(
        "{}{}",
        format!(
            "{}{}",
            format!(
                "{}{}",
                format!("{}{}", "let x1 = //a:f1 in ", "let x2 = //a:f2 in "),
                "let x1 = //b:f1 in "
            ),
            "let x2 = //b:f2 in "
        ),
        "$x1 + $x2"
    );
    assert_eq!(t.eval((let_query).as_str()), t.eval("//b:f1 + //b:f2"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1090 AbstractQueryTest.testBuildFiles
pub fn test_build_files(t: &mut AbstractQueryTest) {
    t.write_build_files3();
    assert_eq!(t.eval("//a ^ //b"), empty_set());
    assert_eq!(t.eval("buildfiles(//a ^ //b)"), empty_set());
    assert_eq!(t.eval("buildfiles(//a)"), t.eval("//a:BUILD"));
    assert_eq!(t.eval("buildfiles(//b)"), t.eval("//b:BUILD"));
    assert_eq!(
        t.eval("buildfiles(//a + //b)"),
        t.eval("//a:BUILD + //b:BUILD")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1100 AbstractQueryTest.testBuildFilesDoesNotReturnVisibilityOfRule
pub fn test_build_files_does_not_return_visibility_of_rule(t: &mut AbstractQueryTest) {
    t.write_file(
        "fruit/BUILD",
        &["filegroup(name='fruit', visibility=['//fruit/lemon:lemon'])"],
    );
    t.write_file(
        "fruit/lemon/BUILD",
        &["package_group(name='lemon', packages=['//fruit/...'])"],
    );
    assert_eq!(t.eval("buildfiles(//fruit:all)"), t.eval("//fruit:BUILD"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1107 AbstractQueryTest.testBuildFilesDoesNotReturnVisibilityOfBUILD
pub fn test_build_files_does_not_return_visibility_of_build(t: &mut AbstractQueryTest) {
    t.write_file("fruit/BUILD", &["filegroup(name='fruit', srcs=['fruit.sh'])\nexports_files(['BUILD'], visibility=['//fruit/lemon:lemon'])\n"]);
    t.write_file(
        "fruit/lemon/BUILD",
        &["package_group(name='lemon', packages=['//fruit/...'])"],
    );
    assert_eq!(t.eval("buildfiles(//fruit:all)"), t.eval("//fruit:BUILD"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1193 AbstractQueryTest.testNodepDeps_defaultIsTrue
pub fn test_nodep_deps_default_is_true(t: &mut AbstractQueryTest) {
    t.run_nodep_deps_test(true, &[]);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1198 AbstractQueryTest.testNodepDeps_false
pub fn test_nodep_deps_false(t: &mut AbstractQueryTest) {
    t.run_nodep_deps_test(false, &[Setting::NoNodepDeps]);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1203 AbstractQueryTest.testCycleInStarlark
pub fn test_cycle_in_starlark(t: &mut AbstractQueryTest) {
    t.run_cycle_in_starlark_test(true);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1238 AbstractQueryTest.testLabelsOperator
pub fn test_labels_operator(t: &mut AbstractQueryTest) {
    t.write_build_files3();
    t.write_build_files_with_configurable_attributes();
    t.write_build_files_with_implicit_attribute();
    assert_eq!(t.eval("labels(srcs, //a)"), t.eval("//b + //c"));
    assert_eq!(t.eval("labels(srcs, //b)"), t.eval("//d"));
    assert_eq!(t.eval("labels(srcs, //b + //a)"), t.eval("//b + //c + //d"));
    assert_eq!(t.eval("labels(outs, //a)"), t.eval("//a:out"));
    assert_eq!(t.eval("labels(outs, //b)"), t.eval("//b:out"));
    assert_eq!(t.eval("labels(outs, //d)"), empty_set());
    assert_eq!(t.eval("labels(data, //b + //a)"), empty_set());
    assert_eq!(t.eval("labels(no_such_attr, //b)"), empty_set());
    assert_eq!(t.eval("labels(srcs, //k)"), t.eval("//k:k.txt"));
    assert_eq!(t.eval("labels('$implicit', //k)"), t.eval("//k:implicit"));
    if t.test_configurable_attributes() {
        assert_eq!(
            t.eval(r#"labels("deps", //configurable:main)"#),
            t.eval("//configurable:adep + //configurable:bdep + //configurable:defaultdep")
        );
    }
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1300 AbstractQueryTest.testTestsOperatorExpandsTestsAndExcludesNonTests
pub fn test_tests_operator_expands_tests_and_excludes_non_tests(t: &mut AbstractQueryTest) {
    t.write_file(
        "a/BUILD",
        &[
            "load('//test_defs:foo_binary.bzl', 'foo_binary')",
            "load('//test_defs:foo_test.bzl', 'foo_test')",
            "test_suite(name='a')",
            "foo_test(name='foo_test', srcs=['foo_test.sh'])",
            "foo_binary(name='cc_binary')",
        ],
    );
    assert_eq!(t.eval("tests(//a)"), t.eval("//a:foo_test"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1312 AbstractQueryTest.testTestsOperatorFiltersByTagSizeAndEnv
pub fn test_tests_operator_filters_by_tag_size_and_env(t: &mut AbstractQueryTest) {
    t.write_file(
        "b/BUILD",
        &[
            "load('//test_defs:foo_test.bzl', 'foo_test')",
            "test_suite(name='large_tests', tags=['large'])",
            "test_suite(name='prod_tests', tags=['prod'])",
            "test_suite(name='foo_tests', tags=['foo'])",
            "foo_test(name='large_test', size='large', srcs=['foo_test.sh'])",
            "foo_test(name='prod_test', tags=['prod'], srcs=['py_test.py'])",
            "foo_test(name='foo_test', tags=['foo'])",
        ],
    );
    assert_eq!(t.eval("tests(//b:large_tests)"), t.eval("//b:large_test"));
    assert_eq!(t.eval("tests(//b:prod_tests)"), t.eval("//b:prod_test"));
    assert_eq!(t.eval("tests(//b:foo_tests)"), t.eval("//b:foo_test"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1348 AbstractQueryTest.testTestsOperatorCrossesPackages
pub fn test_tests_operator_crosses_packages(t: &mut AbstractQueryTest) {
    t.write_file("c/BUILD", &["test_suite(name='c', tests=['//d:suite'])"]);
    t.write_file("d/BUILD", &["load('//test_defs:foo_test.bzl', 'foo_test')\ntest_suite(name='suite')\nfoo_test(name='foo_test', srcs=['foo_test.sh'])\n"]);
    assert_eq!(t.eval("tests(//c)"), t.eval("//d:foo_test"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1362 AbstractQueryTest.testTestsOperatorHandlesCyclesGracefully
pub fn test_tests_operator_handles_cycles_gracefully(t: &mut AbstractQueryTest) {
    t.write_file("c/BUILD", &["test_suite(name='c', tests=['//d'])"]);
    t.write_file("d/BUILD", &["test_suite(name='d', tests=['//c'])"]);
    assert_eq!(t.eval("tests(//c)"), empty_set());
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1370 AbstractQueryTest.testTestSuiteInTestsAttributeAndViceVersa
pub fn test_test_suite_in_tests_attribute_and_vice_versa(t: &mut AbstractQueryTest) {
    t.write_file("cherry/BUILD", &["load('//test_defs:foo_test.bzl', 'foo_test')\ntest_suite(name='cherry', tests=[':suite', ':direct'])\ntest_suite(name='suite', tests=[':indirect'])\nfoo_test(name='direct', srcs=['direct.sh'])\nfoo_test(name='indirect', srcs=['indirect.sh'])\n"]);
    assert_eq!(
        t.eval("tests(//cherry:cherry)"),
        t.eval("//cherry:direct + //cherry:indirect")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1386 AbstractQueryTest.testTestsOperatorReportsMissingTargets
pub fn test_tests_operator_reports_missing_targets(t: &mut AbstractQueryTest) {
    t.write_file("c/BUILD", &["test_suite(name='c', tests=['//d'])"]);
    t.write_file("d/BUILD", &[]);
    let result = t.eval_throws("tests(//c)", false);
    t.assert_starts_with(
        format!(
            "{}{}",
            "couldn't expand 'tests' attribute of test_suite //c:c: ", "no such target '//d:d'"
        ),
        result.message,
    );
    assert_eq!(
        package_loading_code(&result.failure_detail),
        Some(blaze_protos::failure_details::package_loading::Code::TargetMissing as i32)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1398 AbstractQueryTest.testDotDotDotWithUnrelatedCycle
pub fn test_dot_dot_dot_with_unrelated_cycle(t: &mut AbstractQueryTest) {
    t.write_file(
        "a/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'a')",
        ],
    );
    t.write_file("cycle/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'cycle1', deps = ['cycle2'])\nfoo_library(name = 'cycle2', deps = ['cycle1'])\n"]);
    assert_eq!(t.eval("//a:a"), t.eval("//a/..."));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1412 AbstractQueryTest.testDotDotDotWithCycle
pub fn test_dot_dot_dot_with_cycle(t: &mut AbstractQueryTest) {
    t.write_file(
        "a/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'a')",
        ],
    );
    t.write_file("a/b/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'cycle1', deps = ['cycle2'])\nfoo_library(name = 'cycle2', deps = ['cycle1'])\n"]);
    assert_eq!(
        t.eval("//a:a + //a/b:cycle1 + //a/b:cycle2"),
        t.eval("//a/...")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1428 AbstractQueryTest.testExecutablesQuery
pub fn test_executables_query(t: &mut AbstractQueryTest) {
    t.write_file(
        "donut/BUILD",
        &[r#"load('//test_defs:foo_binary.bzl', 'foo_binary')
load("@rules_cc//cc:cc_library.bzl", "cc_library")
load("@rules_cc//cc:cc_test.bzl", "cc_test")

foo_binary(
    name = "bin",
    srcs = ["thief.sh"],
)

cc_test(
    name = "test",
    srcs = ["shop.cc"],
)

cc_library(
    name = "lib",
    srcs = ["shop.cc"],
)
"#],
    );
    assert_eq!(t.eval("executables(//donut:all)"), t.eval("//donut:bin"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1458 AbstractQueryTest.testSet
pub fn test_set(t: &mut AbstractQueryTest) {
    t.write_build_files3();
    assert_eq!(t.eval("set()"), empty_set());
    assert_eq!(t.eval("set(\t//a\n//b )"), t.eval("//a + //b"));
    assert_eq!(
        t.eval("set(//a //b //c //d)"),
        t.eval("//a + //b + //c + //d")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1483 AbstractQueryTest.testDefaultVisibilityReturnedInDeps
pub fn test_default_visibility_returned_in_deps(t: &mut AbstractQueryTest) {
    t.write_file("kiwi/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\npackage(default_visibility=['//mango:mango'])\nfoo_library(name='kiwi')\n"]);
    t.write_file("mango/BUILD", &["package_group(name='mango', packages=[])"]);
    let result =
        t.eval((format!("{}{}", "deps(//kiwi:kiwi)", t.get_dependency_correction())).as_str());
    assert_eq!(result, t.eval("//mango:mango + //kiwi:kiwi"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1498 AbstractQueryTest.testDefaultVisibilityReturnedInDeps_nonEmptyDependencyFilter
pub fn test_default_visibility_returned_in_deps_non_empty_dependency_filter(
    t: &mut AbstractQueryTest,
) {
    t.write_file("kiwi/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\npackage(default_visibility=['//mango:mango'])\nfoo_library(name='kiwi')\n"]);
    t.write_file("mango/BUILD", &["package_group(name='mango', packages=[])"]);
    t.helper.set_query_settings(&[Setting::OnlyTargetDeps]);
    let result =
        t.eval((format!("{}{}", "deps(//kiwi:kiwi)", t.get_dependency_correction())).as_str());
    assert_eq!(result, t.eval("//mango:mango + //kiwi:kiwi"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1515 AbstractQueryTest.testDefaultVisibilityReturnedInDepsForInputFiles
pub fn test_default_visibility_returned_in_deps_for_input_files(t: &mut AbstractQueryTest) {
    t.write_file("kiwi/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\npackage(default_visibility=['//mango:mango'])\nfoo_library(name='kiwi', srcs=['kiwi.sh'])\n"]);
    t.write_file("mango/BUILD", &["package_group(name='mango', packages=[])"]);
    let result = t.eval("deps(//kiwi:kiwi.sh)");
    assert_eq!(result, t.eval("//mango:mango + //kiwi:kiwi.sh"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1532 AbstractQueryTest.testIncludesReturnedInDeps
pub fn test_includes_returned_in_deps(t: &mut AbstractQueryTest) {
    t.write_file("peach/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\npackage_group(name='peach',\n              includes=[':seed'])\npackage_group(name='seed',\n              includes=[':cyanide'])\npackage_group(name='cyanide',\n              packages=['//hydrogen', '//nitrogen', '//carbon'])\nfoo_library(name='dessert',\n           visibility=[':peach'])\n"]);
    let result = t.eval(
        (format!(
            "{}{}",
            "deps(//peach:dessert)",
            t.get_dependency_correction()
        ))
        .as_str(),
    );
    assert_eq!(
        result,
        t.eval("//peach:peach + //peach:seed + //peach:cyanide + //peach:dessert")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1555 AbstractQueryTest.testRegression1267510
pub fn test_regression1267510(t: &mut AbstractQueryTest) {
    t.write_file("x/BUILD", &[]);
    t.write_file("y/BUILD", &[]);
    assert_eq!(
        t.eval("somepath(x:BUILD, y:BUILD) + x:BUILD"),
        t.eval("x:BUILD")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1566 AbstractQueryTest.testRegression1309697
pub fn test_regression1309697(t: &mut AbstractQueryTest) {
    t.write_file(
        "x/BUILD",
        &[
            "load('@rules_cc//cc:cc_library.bzl', 'cc_library')",
            "cc_library(name='x', srcs=['a.cc', 'a.cc'])",
        ],
    );
    let expected_error = "Label '//x:a.cc' is duplicated in the 'srcs' attribute of rule 'x'";
    if t.helper.is_keep_going() {
        assert!(
            t.eval_throws("//x:all", false)
                .message
                .contains(expected_error),
            "{}",
            t.eval_throws("//x:all", false).message
        );
    } else {
        t.eval_throws("//x:all", false);
        t.assert_contains_event(&expected_error);
    }
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1639 AbstractQueryTest.testWildcardsDontLoadUnnecessaryPackages
pub fn test_wildcards_dont_load_unnecessary_packages(t: &mut AbstractQueryTest) {
    t.write_file(
        "x/BUILD",
        &[
            "load('@rules_cc//cc:cc_library.bzl', 'cc_library')",
            "cc_library(name='x', deps=['//y'])",
        ],
    );
    t.write_file("y/BUILD", &[]);
    t.eval("//x:*");
    t.helper.assert_package_not_loaded("y");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1652 AbstractQueryTest.testRegression1352570
pub fn test_regression1352570(t: &mut AbstractQueryTest) {
    t.write_file(
        "x/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(name='x', deps=['z'])
cc_library(name='y', deps=['z'])
cc_library(name='z')
"#],
    );
    let result = t.eval("deps(//x:x + //x:y, 2) intersect //x:*");
    assert_eq!(result, t.eval("//x:x + //x:y + //x:z"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1666 AbstractQueryTest.testTestSuiteWithFile
pub fn test_test_suite_with_file(t: &mut AbstractQueryTest) {
    t.write_file("x/BUILD", &["test_suite(name='a', tests=['a.txt'])"]);
    assert!(
        t.eval("tests(//x:a)").is_empty(),
        "{:?}",
        t.eval("tests(//x:a)")
    );
    assert_eq!(t.eval("deps(//x:a)"), t.eval("//x:a + //x:a.txt"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1674 AbstractQueryTest.testStrictTestSuiteWithFile
pub fn test_strict_test_suite_with_file(t: &mut AbstractQueryTest) {
    t.helper
        .set_query_settings(&[Setting::TestsExpressionStrict]);
    t.write_file("x/BUILD", &["test_suite(name='a', tests=['a.txt'])"]);
    let result = t.eval_throws("tests(//x:a)", false);
    assert_eq!(
        result.message,
        format!(
            "{}{}",
            "The label '//x:a.txt' in the test_suite '//x:a' does not refer to a test or ",
            "test_suite rule!"
        )
    );
    assert_eq!(
        query_code(&result.failure_detail),
        Some(blaze_protos::failure_details::query::Code::InvalidLabelInTestSuite as i32)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1686 AbstractQueryTest.testAmbiguousAllResolvesToTestSuiteNamedAll
pub fn test_ambiguous_all_resolves_to_test_suite_named_all(t: &mut AbstractQueryTest) {
    t.helper
        .set_query_settings(&[Setting::TestsExpressionStrict]);
    t.write_file(
        "x/BUILD",
        &[r#"load("@rules_cc//cc:cc_test.bzl", "cc_test")
cc_test(name='one')
cc_test(name='two')
test_suite(name='all', tests=[':one'])
"#],
    );
    assert_eq!(t.eval("tests(//x:all)"), t.eval("//x:one"));
    t.assert_contains_event(
        (format!(
            "{}{}",
            "The target pattern '//x:all' is ambiguous: ':all' is both a wildcard, and ",
            "the name of an existing test_suite rule; using the latter interpretation"
        ))
        .as_str(),
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1717 AbstractQueryTest.testSlashSlashDotDotDot
pub fn test_slash_slash_dot_dot_dot(t: &mut AbstractQueryTest) {
    t.use_reduced_set_of_rules();
    t.write_file("MODULE.bazel", &[]);
    t.write_file("a/BUILD", &["filegroup(name = 'a', srcs = ['a.sh'])"]);
    assert_eq!(t.eval("//..."), t.eval("//a"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1725 AbstractQueryTest.testQueryTimeLoadingOfTargetPatternHappyPath
pub fn test_query_time_loading_of_target_pattern_happy_path(t: &mut AbstractQueryTest) {
    t.write_file(
        "a/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'a')",
        ],
    );
    t.write_file(
        "a/b/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'b')",
        ],
    );
    let query_time_loaded_pattern = t.eval("//a/b:b");
    assert_eq!(
        query_time_loaded_pattern.len(),
        1,
        "{:?}",
        query_time_loaded_pattern
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1742 AbstractQueryTest.testQueryTimeLoadingOfTargetsBelowPackageHappyPath
pub fn test_query_time_loading_of_targets_below_package_happy_path(t: &mut AbstractQueryTest) {
    t.write_file(
        "a/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'a')",
        ],
    );
    t.write_file(
        "a/b/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'b')",
        ],
    );
    t.write_file(
        "a/b/c/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'c')",
        ],
    );
    let query_time_loaded_pattern = t.eval("//a/b/...");
    assert_eq!(
        query_time_loaded_pattern.len(),
        2,
        "{:?}",
        query_time_loaded_pattern
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1781 AbstractQueryTest.testQueryTimeLoadingTargetsBelowNonPackageDirectory
pub fn test_query_time_loading_targets_below_non_package_directory(t: &mut AbstractQueryTest) {
    t.write_file(
        "a/b/c/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'c')",
        ],
    );
    t.write_file(
        "a/b/c/d/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'd')",
        ],
    );
    let query_time_loaded_pattern = t.eval("//a/b/...");
    assert_eq!(
        query_time_loaded_pattern.len(),
        2,
        "{:?}",
        query_time_loaded_pattern
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1873 AbstractQueryTest.testNoDepsOnAspectAttributeWhenAspectMissing
pub fn test_no_deps_on_aspect_attribute_when_aspect_missing(t: &mut AbstractQueryTest) {
    t.use_extended_set_of_rules();
    t.write_file("a/BUILD", &["aspect(name='a', foo=[':b'])\nhonest(name='b', foo=[])\nextra_attribute_aspect_requiring_provider(name='c', foo=[':d'])\nsimple(name='d', foo=[])\n"]);
    t.write_file("extra/BUILD", &["honest(name='extra', foo=[])"]);
    assert!(
        !t.eval_to_string("deps(//a:a)").contains("//extra:extra"),
        "{}",
        t.eval_to_string("deps(//a:a)")
    );
    assert!(
        !t.eval_to_string("deps(//a:c)").contains("//extra:extra"),
        "{}",
        t.eval_to_string("deps(//a:c)")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1919 AbstractQueryTest.testVisible_simple_public
pub fn test_visible_simple_public(t: &mut AbstractQueryTest) {
    t.simple_visibility_test("//visibility:public", true);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1924 AbstractQueryTest.testVisible_simple_private
pub fn test_visible_simple_private(t: &mut AbstractQueryTest) {
    t.simple_visibility_test("//visibility:private", false);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1929 AbstractQueryTest.testVisible_simple_package
pub fn test_visible_simple_package(t: &mut AbstractQueryTest) {
    t.simple_visibility_test("//a:__pkg__", true);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1934 AbstractQueryTest.testVisible_simple_subpackages
pub fn test_visible_simple_subpackages(t: &mut AbstractQueryTest) {
    t.simple_visibility_test("//a:__subpackages__", true);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1939 AbstractQueryTest.testVisible_simple_different_subpackages
pub fn test_visible_simple_different_subpackages(t: &mut AbstractQueryTest) {
    t.simple_visibility_test("//c:__subpackages__", false);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1944 AbstractQueryTest.testVisible_private_same_package
pub fn test_visible_private_same_package(t: &mut AbstractQueryTest) {
    t.use_reduced_set_of_rules();
    t.write_file("MODULE.bazel", &[]);
    t.write_file("a/BUILD", &["filegroup(name = 'a', srcs = [':b'], visibility = ['//visibility:private'])\nfilegroup(name = 'b', srcs = ['b.txt'], visibility = ['//visibility:private'])\n"]);
    assert_eq!(
        t.eval_to_string("visible(//a:a, somepath(//a:a, //a:b))"),
        "//a:a //a:b"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1957 AbstractQueryTest.testVisible_package_group
pub fn test_visible_package_group(t: &mut AbstractQueryTest) {
    t.use_reduced_set_of_rules();
    t.write_file("MODULE.bazel", &[]);
    t.write_file("a/BUILD", &["filegroup(name = 'a', srcs = ['//b:b'])"]);
    t.write_file("b/BUILD", &["package_group(name = 'friends', packages = ['//a', '//b'])\nfilegroup(name = 'b', srcs = ['b.txt'], visibility = [':friends'])\n"]);
    assert_eq!(
        t.eval_to_string("visible(//a, somepath(//a, //b))"),
        "//a:a //b:b"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1971 AbstractQueryTest.testVisible_package_group_invisible
pub fn test_visible_package_group_invisible(t: &mut AbstractQueryTest) {
    t.use_reduced_set_of_rules();
    t.write_file("MODULE.bazel", &[]);
    t.write_file("a/BUILD", &["filegroup(name = 'a', srcs = ['//b:b'])"]);
    t.write_file("b/BUILD", &["package_group(name = 'friends', packages = ['//c'])\nfilegroup(name = 'b', srcs = ['b.txt'], visibility = [':friends'])\n"]);
    t.write_file("c/BUILD", &[]);
    assert_eq!(
        t.eval_to_string("visible(//a, somepath(//a, //b))"),
        "//a:a"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1986 AbstractQueryTest.testVisible_package_group_include
pub fn test_visible_package_group_include(t: &mut AbstractQueryTest) {
    t.use_reduced_set_of_rules();
    t.write_file("MODULE.bazel", &[]);
    t.write_file("a/BUILD", &["filegroup(name = 'a', srcs = ['//b:b'])"]);
    t.write_file("b/BUILD", &["package_group(name = 'friends', packages = ['//c'], includes = [':friends_of_friends'])\npackage_group(name = 'friends_of_friends', packages = ['//a'])\nfilegroup(name = 'b', srcs = ['b.txt'], visibility = [':friends'])\n"]);
    t.write_file("c/BUILD", &[]);
    assert_eq!(
        t.eval_to_string("visible(//a, somepath(//a, //b))"),
        "//a:a //b:b"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2002 AbstractQueryTest.testVisible_java_javatests
pub fn test_visible_java_javatests(t: &mut AbstractQueryTest) {
    t.use_reduced_set_of_rules();
    t.write_file("MODULE.bazel", &[]);
    t.write_file(
        "java/com/google/a/BUILD",
        &["filegroup(name = 'a', srcs = ['a.txt'], visibility = ['//visibility:private'])"],
    );
    t.write_file(
        "javatests/com/google/a/BUILD",
        &[(format!(
            "{}{}",
            "filegroup(name = 'a', srcs = ['//java/com/google/a:a'],",
            " visibility = ['//visibility:private'])"
        ))
        .as_str()],
    );
    assert_eq!(
        t.eval_to_string(
            (format!(
                "{}{}",
                "visible(//javatests/com/google/a,",
                " somepath(//javatests/com/google/a, //java/com/google/a))"
            ))
            .as_str()
        ),
        "//java/com/google/a:a //javatests/com/google/a:a"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2020 AbstractQueryTest.testVisible_java_javatests_different_package
pub fn test_visible_java_javatests_different_package(t: &mut AbstractQueryTest) {
    t.use_reduced_set_of_rules();
    t.write_file("MODULE.bazel", &[]);
    t.write_file(
        "java/com/google/a/BUILD",
        &["filegroup(name = 'a', srcs = ['a.txt'], visibility = ['//visibility:private'])"],
    );
    t.write_file(
        "javatests/com/google/b/BUILD",
        &[(format!(
            "{}{}",
            "filegroup(name = 'b', srcs = ['//java/com/google/a:a'],",
            " visibility = ['//visibility:private'])"
        ))
        .as_str()],
    );
    assert_eq!(
        t.eval_to_string(
            (format!(
                "{}{}",
                "visible(//javatests/com/google/b,",
                " somepath(//javatests/com/google/b, //java/com/google/a))"
            ))
            .as_str()
        ),
        "//javatests/com/google/b:b"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2039 AbstractQueryTest.testVisible_javatests_java
pub fn test_visible_javatests_java(t: &mut AbstractQueryTest) {
    t.use_reduced_set_of_rules();
    t.write_file("MODULE.bazel", &[]);
    t.write_file(
        "javatests/com/google/a/BUILD",
        &["filegroup(name = 'a', srcs = ['a.txt'], visibility = ['//visibility:private'])"],
    );
    t.write_file(
        "java/com/google/a/BUILD",
        &[(format!(
            "{}{}",
            "filegroup(name = 'a', srcs = ['//javatests/com/google/a:a'],",
            " visibility = ['//visibility:private'])"
        ))
        .as_str()],
    );
    assert_eq!(
        t.eval_to_string(
            (format!(
                "{}{}",
                "visible(//java/com/google/a,",
                " somepath(//java/com/google/a, //javatests/com/google/a))"
            ))
            .as_str()
        ),
        "//java/com/google/a:a"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2057 AbstractQueryTest.testVisible_default_private
pub fn test_visible_default_private(t: &mut AbstractQueryTest) {
    t.use_reduced_set_of_rules();
    t.write_file("MODULE.bazel", &[]);
    t.write_file("a/BUILD", &["filegroup(name = 'a', srcs = ['//b'])"]);
    t.write_file("b/BUILD", &["package(default_visibility = ['//visibility:private'])\nfilegroup(name = 'b', srcs = ['b.txt'])\n"]);
    assert_eq!(
        t.eval_to_string("visible(//a, somepath(//a, //b))"),
        "//a:a"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2071 AbstractQueryTest.testVisible_default_public
pub fn test_visible_default_public(t: &mut AbstractQueryTest) {
    t.use_reduced_set_of_rules();
    t.write_file("MODULE.bazel", &[]);
    t.write_file("a/BUILD", &["filegroup(name = 'a', srcs = ['//b'])"]);
    t.write_file("b/BUILD", &["package(default_visibility = ['//visibility:public'])\nfilegroup(name = 'b', srcs = ['b.txt'])\n"]);
    assert_eq!(
        t.eval_to_string("visible(//a, somepath(//a, //b))"),
        "//a:a //b:b"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2085 AbstractQueryTest.testPackageGroupAllBeneath
pub fn test_package_group_all_beneath(t: &mut AbstractQueryTest) {
    t.use_reduced_set_of_rules();
    t.write_file("MODULE.bazel", &[]);
    t.write_file("a/BUILD", &["filegroup(name = 'a', srcs = ['//b:b'])"]);
    t.write_file("b/BUILD", &["package_group(name = 'friends', packages = ['//a/...'])\nfilegroup(name = 'b', srcs = ['b.txt'], visibility = [':friends'])\n"]);
    assert_eq!(
        t.eval_to_string("visible(//a, somepath(//a, //b))"),
        "//a:a //b:b"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2099 AbstractQueryTest.testBuildfilesWithDuplicates
pub fn test_buildfiles_with_duplicates(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//baz:baz.bzl', 'x')\nload('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'foo', deps = ['//baz'])\n"]);
    t.write_file("bar/BUILD", &["load('//baz:baz.bzl', 'x')\nload('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'bar', deps = ['//baz'])\n"]);
    t.write_file("baz/BUILD", &["load('//baz:baz.bzl', 'x')\nload('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'baz')\n"]);
    t.write_file("baz/baz.bzl", &["x = 2"]);
    assert_eq!(
        t.eval_to_string("buildfiles(deps(//foo)) + buildfiles(deps(//bar))"),
        format!(
            "{}{}",
            "//bar:BUILD //baz:BUILD //baz:baz.bzl //foo:BUILD //test_defs:BUILD",
            " //test_defs:foo_library.bzl"
        )
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2129 AbstractQueryTest.bzlPackageBadDueToBrokenLoad
pub fn bzl_package_bad_due_to_broken_load(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//bar:bar.bzl', 'sym')"]);
    t.write_file("bar/BUILD", &["load('//noexist:noexist.bzl', 'bad')"]);
    t.write_file("bar/bar.bzl", &["sym = 0"]);
    assert_eq!(
        sorted(t.eval_to_list_of_strings("buildfiles(//foo:BUILD)")),
        sorted(vec![
            String::from("//foo:BUILD"),
            String::from("//bar:bar.bzl"),
            String::from("//bar:BUILD")
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2138 AbstractQueryTest.bzlPackageBadDueToBrokenSyntax
pub fn bzl_package_bad_due_to_broken_syntax(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//bar:bar.bzl', 'sym')"]);
    t.write_file("bar/BUILD", &["malformed syntax"]);
    t.write_file("bar/bar.bzl", &["sym = 0"]);
    assert_eq!(
        sorted(t.eval_to_list_of_strings("buildfiles(//foo:BUILD)")),
        sorted(vec![
            String::from("//foo:BUILD"),
            String::from("//bar:bar.bzl"),
            String::from("//bar:BUILD")
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2147 AbstractQueryTest.testBuildfilesContainingScl
pub fn test_buildfiles_containing_scl(t: &mut AbstractQueryTest) {
    t.write_bzl_and_scl_files();
    assert_eq!(
        t.eval_to_string("buildfiles(deps(//foo))"),
        format!(
            "{}{}",
            "//bar:BUILD //bar:direct.scl //bar:indirect.scl //bar:intermediate.bzl //foo:BUILD",
            " //test_defs:BUILD //test_defs:foo_library.bzl"
        )
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2157 AbstractQueryTest.badRuleInDeps
pub fn bad_rule_in_deps(t: &mut AbstractQueryTest) {
    t.run_bad_rule_in_deps(ExpectedCode::PackageLoading(
        blaze_protos::failure_details::package_loading::Code::StarlarkEvalError,
    ));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2185 AbstractQueryTest.buildfilesBazel
pub fn buildfiles_bazel(t: &mut AbstractQueryTest) {
    t.write_file("bar/BUILD.bazel", &[]);
    t.write_file("bar/bar.bzl", &["sym = 0"]);
    t.write_file("foo/BUILD.bazel", &["load('//bar:bar.bzl', 'sym')"]);
    assert_eq!(
        sorted(t.eval_to_list_of_strings("buildfiles(foo:*)")),
        sorted(vec![
            String::from("//foo:BUILD.bazel"),
            String::from("//bar:bar.bzl"),
            String::from("//bar:BUILD.bazel")
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2194 AbstractQueryTest.testTargetsFromBuildfilesAndRealTargets
pub fn test_targets_from_buildfiles_and_real_targets(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//baz:baz.bzl', 'x')\nload('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'foo', deps = ['//baz'])\n"]);
    t.write_file("baz/BUILD", &["load('//baz:baz.bzl', 'x')\nload('//test_defs:foo_library.bzl', 'foo_library')\nexports_files(['baz.bzl'])\nfoo_library(name = 'baz')\n"]);
    t.write_file("baz/baz.bzl", &["x = 2"]);
    assert_eq!(
        t.eval_to_string("buildfiles(deps(//foo)) + //baz:BUILD + //baz:baz.bzl"),
        "//baz:BUILD //baz:baz.bzl //foo:BUILD //test_defs:BUILD //test_defs:foo_library.bzl"
    );
    assert_eq!(
        t.eval_to_string("buildfiles(deps(//foo)) ^ //baz:BUILD"),
        "//baz:BUILD"
    );
    assert_eq!(
        t.eval_to_string("buildfiles(deps(//foo)) ^ //baz:baz.bzl"),
        "//baz:baz.bzl"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2219 AbstractQueryTest.testBuildfilesOfBuildfiles
pub fn test_buildfiles_of_buildfiles(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//baz:baz.bzl', 'x')\nload('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'foo')\n"]);
    t.write_file("baz/BUILD", &["load('//bar:bar.bzl', 'x')"]);
    t.write_file("baz/baz.bzl", &["x = 1"]);
    t.write_file("bar/BUILD", &[]);
    t.write_file("bar/bar.bzl", &["x = 2"]);
    assert_eq!(
        t.eval_to_string("buildfiles(//foo)"),
        "//baz:BUILD //baz:baz.bzl //foo:BUILD //test_defs:BUILD //test_defs:foo_library.bzl"
    );
    assert_eq!(
        t.eval_to_string("buildfiles(buildfiles(//foo))"),
        "//baz:BUILD //baz:baz.bzl //foo:BUILD //test_defs:BUILD //test_defs:foo_library.bzl"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2240 AbstractQueryTest.testBoundedDepsStreaming
pub fn test_bounded_deps_streaming(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'a', deps = [':b'])\nfoo_library(name = 'b', deps = [':c'])\nfoo_library(name = 'c', deps = [':d'])\nfoo_library(name = 'd')\n"]);
    assert_eq!(
        t.eval_to_string(
            (format!(
                "{}{}",
                "deps(//foo:a + //foo:b, 1)",
                t.get_dependency_correction()
            ))
            .as_str()
        ),
        "//foo:a //foo:b //foo:c"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2255 AbstractQueryTest.testBoundedRdepsStreaming
pub fn test_bounded_rdeps_streaming(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'a', deps = [':b'])\nfoo_library(name = 'b', deps = [':c'])\nfoo_library(name = 'c', deps = [':d'])\nfoo_library(name = 'd')\n"]);
    assert_eq!(
        t.eval_to_string(
            (format!(
                "{}{}",
                "rdeps(//foo:a, //foo:d + //foo:c, 1)",
                t.get_dependency_correction()
            ))
            .as_str()
        ),
        "//foo:b //foo:c //foo:d"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2270 AbstractQueryTest.boundedDepsWithError
pub fn bounded_deps_with_error(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'foo', deps = [':dep'])\nfoo_library(name = 'dep', deps = ['//bar:missing'])\n"]);
    assert_eq!(
        sorted(t.eval_to_list_of_strings("deps(//foo:foo, 1)")),
        sorted(vec![String::from("//foo:foo"), String::from("//foo:dep")])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2284 AbstractQueryTest.boundedRdepsWithError
pub fn bounded_rdeps_with_error(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'foo', deps = [':dep'])\nfoo_library(name = 'dep', deps = ['//bar:missing'])\n"]);
    assert!(
        t.eval_throws("rdeps(//foo:foo, //foo:dep, 1)", false)
            .message
            .contains("preloading transitive closure failed: no such package 'bar':"),
        "{}",
        t.eval_throws("rdeps(//foo:foo, //foo:dep, 1)", false)
            .message
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2316 AbstractQueryTest.testSiblings_simple
pub fn test_siblings_simple(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'a')\nfoo_library(name = 'b')\nfoo_library(name = 'c')\nfoo_library(name = 'd')\n"]);
    assert_eq!(
        t.eval_to_string("siblings(//foo:a)"),
        "//foo:BUILD //foo:a //foo:b //foo:c //foo:d"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2331 AbstractQueryTest.testSiblings_duplicatePackages
pub fn test_siblings_duplicate_packages(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'a')\nfoo_library(name = 'b')\nfoo_library(name = 'c')\nfoo_library(name = 'd')\n"]);
    assert_eq!(
        t.eval_to_string("siblings(//foo:a + //foo:b + //foo:c + //foo:d)"),
        "//foo:BUILD //foo:a //foo:b //foo:c //foo:d"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2346 AbstractQueryTest.testSiblings_samePackageRdeps
pub fn test_siblings_same_package_rdeps(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'a', deps = [':b'])\nfoo_library(name = 'b', deps = [':c', ':d'])\nfoo_library(name = 'c', deps = [':d'])\nfoo_library(name = 'd')\n"]);
    t.write_file("bar/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'e', deps = ['//foo:d'])\nfoo_library(name = 'f', deps = ['//foo:d'])\n"]);
    assert_eq!(
        t.eval_to_string("rdeps(//foo:* + //bar:*, //foo:d, 1)"),
        "//bar:e //bar:f //foo:b //foo:c //foo:d"
    );
    assert_eq!(
        t.eval_to_string("rdeps(siblings(//foo:d), //foo:d, 1)"),
        "//foo:b //foo:c //foo:d"
    );
    assert_eq!(
        t.eval_to_string("same_pkg_direct_rdeps(//foo:d)"),
        "//foo:b //foo:c"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2373 AbstractQueryTest.testSiblings_matchesTargetNamedAll
pub fn test_siblings_matches_target_named_all(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["# NOTE: target named 'all' collides with, takes precedence over the ':all' wildcard\nload('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'all')\nfoo_library(name = 'ball')\nfoo_library(name = 'call')\nfoo_library(name = 'doll')\n"]);
    assert_eq!(t.eval_to_string("//foo:all"), "//foo:all");
    assert_eq!(
        t.eval_to_string("kind(' rule', siblings(//foo:BUILD))"),
        "//foo:all //foo:ball //foo:call //foo:doll"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2395 AbstractQueryTest.testSiblings_withBuildfiles
pub fn test_siblings_with_buildfiles(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//bar:bar.bzl', 'x')\nload('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'foo')\n"]);
    t.write_file(
        "bar/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'bar')",
        ],
    );
    t.write_file("bar/bar.bzl", &["x = 42"]);
    assert_eq!(
        t.eval_to_string("siblings(buildfiles(//foo:foo))"),
        "//foo:BUILD //foo:foo"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2412 AbstractQueryTest.testSamePackageRdeps_simple
pub fn test_same_package_rdeps_simple(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'a', srcs = ['A.java'])\nfoo_library(name = 'b', srcs = ['B.java'], deps = [':a'])\nfoo_library(name = 'c', srcs = ['C.java'], deps = [':b'])\n"]);
    assert_eq!(
        t.eval_to_string("same_pkg_direct_rdeps(//foo:A.java)"),
        "//foo:a"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2425 AbstractQueryTest.testSamePackageRdeps_duplicate
pub fn test_same_package_rdeps_duplicate(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'a', srcs = ['A.java'])\nfoo_library(name = 'b', srcs = ['B.java'], deps = [':a'])\nfoo_library(name = 'c', srcs = ['C.java'], deps = [':b'])\n"]);
    assert_eq!(
        t.eval_to_string("same_pkg_direct_rdeps(//foo:A.java + //foo:A.java)"),
        "//foo:a"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2439 AbstractQueryTest.testSamePackageRdeps_two
pub fn test_same_package_rdeps_two(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'a', deps = [':b'])\nfoo_library(name = 'b', deps = [':c', ':d'])\nfoo_library(name = 'c', deps = [':d'])\nfoo_library(name = 'd')\n"]);
    t.write_file("bar/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'e', deps = ['//foo:d'])\nfoo_library(name = 'f', deps = ['//foo:d'])\n"]);
    assert_eq!(
        t.eval_to_string("kind(rule, same_pkg_direct_rdeps(//foo:d))"),
        "//foo:b //foo:c"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2461 AbstractQueryTest.testSamePackageRdeps_twoPackages
pub fn test_same_package_rdeps_two_packages(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'a', srcs = ['A.java'])\nfoo_library(name = 'b', srcs = ['B.java'], deps = [':a'])\nfoo_library(name = 'c', srcs = ['C.java'], deps = [':b'])\n"]);
    t.write_file(
        "bar/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'd', srcs = ['D.java'], deps = ['//foo:a'])",
        ],
    );
    assert_eq!(
        t.eval_to_string("kind(rule, same_pkg_direct_rdeps(//foo:a))"),
        "//foo:b"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2479 AbstractQueryTest.testSamePackageRdeps_crissCross
pub fn test_same_package_rdeps_criss_cross(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'a', srcs = ['A.java'])\nfoo_library(name = 'b', srcs = ['B.java'], deps = ['//bar:a'])\n"]);
    t.write_file("bar/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'a', srcs = ['A.java'])\nfoo_library(name = 'b', srcs = ['B.java'], deps = ['//foo:a'])\n"]);
    assert_eq!(
        t.eval_to_string("kind(rule, same_pkg_direct_rdeps(//foo:a + //bar:a))"),
        ""
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2498 AbstractQueryTest.testVisibleWithNonPackageGroupVisibility
pub fn test_visible_with_non_package_group_visibility(t: &mut AbstractQueryTest) {
    t.write_file(
        "foo/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'foo', visibility = ['//bar:bar'])",
        ],
    );
    t.write_file(
        "bar/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'bar')",
        ],
    );
    assert_eq!(t.eval_to_string("visible(//bar:bar, //foo:foo)"), "");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2511 AbstractQueryTest.testVisibleWithPackageGroupWithNonPackageGroupIncludes
pub fn test_visible_with_package_group_with_non_package_group_includes(t: &mut AbstractQueryTest) {
    t.write_file("foo/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'foo', visibility = [':pg'])\npackage_group(name = 'pg', includes = ['//bar:bar'])\n"]);
    t.write_file(
        "bar/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'bar')",
        ],
    );
    assert_eq!(t.eval_to_string("visible(//bar:bar, //foo:foo)"), "");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2543 AbstractQueryTest.testUnsuccessfulInnerFutureInNestedLetTransformAsyncFastPath
pub fn test_unsuccessful_inner_future_in_nested_let_transform_async_fast_path(
    t: &mut AbstractQueryTest,
) {
    t.write_file(
        "foo/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'foo')",
        ],
    );
    let result = t.eval_throws("let x = let y = //foo in $nope in $x", true);
    assert!(
        result.message.contains("undefined variable 'nope'"),
        "{}",
        result.message
    );
    assert!(
        !result.message.contains("java.lang.IllegalStateException"),
        "{}",
        result.message
    );
    assert_eq!(
        query_code(&result.failure_detail),
        Some(blaze_protos::failure_details::query::Code::VariableUndefined as i32)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2558 AbstractQueryTest.testUnconditionalQueryException
pub fn test_unconditional_query_exception(t: &mut AbstractQueryTest) {
    t.write_file(
        "foo/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'foo')",
        ],
    );
    let result = t.eval_throws("some(//foo - //foo) + //foo", true);
    assert_eq!(result.message, "argument set is empty");
    assert_eq!(
        query_code(&result.failure_detail),
        Some(blaze_protos::failure_details::query::Code::ArgumentsMissing as i32)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2672 AbstractQueryTest.testExternalRepo_allTargetsInPackage
pub fn test_external_repo_all_targets_in_package(t: &mut AbstractQueryTest) {
    t.write_bzlmod_build_files();
    assert_eq!(t.eval_to_string("@my_repo//a/b:*"), REPO_AB_ALL);
    assert_eq!(t.eval_to_string("@my_repo//a:*"), REPO_A_ALL);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2679 AbstractQueryTest.testExternalRepo_allTargetsBelow
pub fn test_external_repo_all_targets_below(t: &mut AbstractQueryTest) {
    t.write_bzlmod_build_files();
    assert_eq!(t.eval_to_string("@my_repo//...:*"), REPO_A_AB_ALL);
    assert_eq!(t.eval_to_string("@my_repo//a/..."), REPO_A_AB_RULES);
    assert_eq!(t.eval_to_string("@my_repo//a/b/..."), REPO_AB_RULES);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2687 AbstractQueryTest.testLabelFlagDefaultAppearsInDepsQuery
pub fn test_label_flag_default_appears_in_deps_query(t: &mut AbstractQueryTest) {
    t.write_file("donut/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'thief', srcs = ['thief.sh'])\nlabel_flag(name = 'myflag', build_setting_default = ':thief')\n"]);
    assert_eq!(
        t.eval_to_string(
            (format!(
                "{}{}",
                "deps(//donut:myflag, 1)",
                t.get_dependency_correction_with_gen()
            ))
            .as_str()
        ),
        "//donut:myflag //donut:thief"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2701 AbstractQueryTest.testLabelSettingDefaultAppearsInDepsQuery
pub fn test_label_setting_default_appears_in_deps_query(t: &mut AbstractQueryTest) {
    t.write_file("donut/BUILD", &["load('//test_defs:foo_library.bzl', 'foo_library')\nfoo_library(name = 'thief', srcs = ['thief.sh'])\nlabel_setting(name = 'mysetting', build_setting_default = ':thief')\n"]);
    assert_eq!(
        t.eval_to_string(
            (format!(
                "{}{}",
                "deps(//donut:mysetting, 1)",
                t.get_dependency_correction_with_gen()
            ))
            .as_str()
        ),
        "//donut:mysetting //donut:thief"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2715 AbstractQueryTest.testStarlarkRuleToolchainDeps
pub fn test_starlark_rule_toolchain_deps(t: &mut AbstractQueryTest) {
    t.overwrite_file("MODULE.bazel", &["register_toolchains('//bar:all')"]);
    t.write_file(
        "foo/BUILD",
        &[r#"load(":foo.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );
    t.write_file("foo/foo.bzl", &["def noop(ctx):\n  pass\n\nmy_rule = rule(\n  implementation = noop,\n  toolchains = ['//bar:bar_type'],\n)\n"]);
    t.write_file(
        "bar/BUILD",
        &[r#"load(":bar.bzl", "test_toolchain")

toolchain_type(name = "bar_type")
toolchain_type(name = "other_type")
test_toolchain(name = "bar_impl")
test_toolchain(name = "other_impl")

toolchain(
    name = "bar_toolchain",
    toolchain = ":bar_impl",
    toolchain_type = ":bar_type",
)

toolchain(
    name = "other_toolchain",
    toolchain = ":other_impl",
    toolchain_type = ":other_type",
)
"#],
    );
    t.write_file("bar/bar.bzl", &["def _impl(ctx):\n    toolchain = platform_common.ToolchainInfo()\n    return [toolchain]\n\ntest_toolchain = rule(\n    implementation = _impl,\n)\n"]);
    assert!(
        t.eval_to_string("deps(//foo, 1)")
            .contains("//bar:bar_type"),
        "{}",
        t.eval_to_string("deps(//foo, 1)")
    );
    assert!(
        t.eval_to_string("deps(//foo)").contains("//bar:bar_type"),
        "{}",
        t.eval_to_string("deps(//foo)")
    );
    t.helper.set_query_settings(&[Setting::NoImplicitDeps]);
    assert!(
        !t.eval_to_string("deps(//foo, 1)")
            .contains("//bar:bar_type"),
        "{}",
        t.eval_to_string("deps(//foo, 1)")
    );
    assert!(
        !t.eval_to_string("deps(//foo)").contains("//bar:bar_type"),
        "{}",
        t.eval_to_string("deps(//foo)")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2782 AbstractQueryTest.testNativeRuleToolchainDeps
pub fn test_native_rule_toolchain_deps(t: &mut AbstractQueryTest) {
    t.write_file(
        "foo/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(name = "cclib")
"#],
    );
    assert!(
        t.eval_to_string("deps(//foo:cclib)")
            .contains("//tools/cpp:toolchain_type"),
        "{}",
        t.eval_to_string("deps(//foo:cclib)")
    );
    t.helper.set_query_settings(&[Setting::NoImplicitDeps]);
    assert!(
        !t.eval_to_string("deps(//foo:cclib)")
            .contains("//tools/cpp:toolchain_type"),
        "{}",
        t.eval_to_string("deps(//foo:cclib)")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:275 AbstractQueryTest.testTargetLiteralWithMissingTargets
pub fn test_target_literal_with_missing_targets(t: &mut AbstractQueryTest) {
    t.write_file("a/BUILD", &[]);
    let eval_throws_result = t.eval_throws("//a:b", false);
    // TestUtils.createMissingTargetAssertionString("b", "a", rootDirectory, ""); the pattern has
    // no regex metacharacters but '.', which matches itself.
    let expected = format!(
        "no such target '//a:b': target 'b' not declared in package 'a' defined by {}/a/BUILD",
        t.helper.get_root_directory().get_path_string()
    );
    assert_eq!(eval_throws_result.message, expected);
    assert_eq!(
        package_loading_code(&eval_throws_result.failure_detail),
        Some(blaze_protos::failure_details::package_loading::Code::TargetMissing as i32)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:625 AbstractQueryTest.testSomePathOperator
pub fn test_some_path_operator(t: &mut AbstractQueryTest) {
    t.write_build_files3();
    t.write_build_files_with_configurable_attributes();

    assert_eq!(t.eval("somepath(//a, //a:a2)"), empty_set()); // no path
    assert_eq!(t.eval("somepath(//d, //a)"), empty_set()); // no path

    let somepath_a_to_d = t.eval("somepath(//a, //d)");
    let a = t.eval("//a");
    assert!(somepath_a_to_d.is_superset(&a));
    let a_and_b = t.eval("//a + //b");
    // Contains one of {//b, //c}:
    assert!(!somepath_a_to_d.is_disjoint(&a_and_b));
    let d = t.eval("//d");
    assert_contains(somepath_a_to_d, d);

    // Configurable attributes:
    if t.test_configurable_attributes() {
        assert_eq!(
            t.eval("somepath(//configurable:main, //configurable:bdep.cc)"),
            t.eval("//configurable:main + //configurable:bdep + //configurable:bdep.cc")
        );
    }
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:647 AbstractQueryTest.testSomePathOperatorOrdering
pub fn test_some_path_operator_ordering(t: &mut AbstractQueryTest) {
    t.write_file(
        "a/BUILD",
        &[
            "genrule(name='a1', srcs=['//b', '//c'], outs=['out1'], cmd=':')
genrule(name='a0', srcs=[':a1'], outs=['out0'], cmd=':')
",
        ],
    );
    t.write_file(
        "b/BUILD",
        &["genrule(name='b', srcs=['//d'], outs=['out'], cmd=':')"],
    );
    t.write_file(
        "c/BUILD",
        &["genrule(name='c', srcs=['//d'], outs=['out'], cmd=':')"],
    );
    t.write_file("d/BUILD", &["exports_files(['d'])"]);

    let path_list1: Vec<String> = ["//a:a0", "//a:a1", "//b:b", "//d:d"]
        .map(String::from)
        .to_vec();
    let path_list2: Vec<String> = ["//a:a0", "//a:a1", "//c:c", "//d:d"]
        .map(String::from)
        .to_vec();

    let somepath_a_to_d = t.eval_to_list_of_strings("somepath(//a:a0, //d)");
    if somepath_a_to_d.contains(&"//b:b".to_owned()) {
        assert_eq!(path_list1, somepath_a_to_d);
    } else {
        assert_eq!(somepath_a_to_d, path_list2);
    }
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:699 AbstractQueryTest.testDeps
pub fn test_deps(t: &mut AbstractQueryTest) {
    t.write_build_files3();
    t.write_build_files_with_configurable_attributes();
    let with_gen = t.get_dependency_correction_with_gen();

    assert_eq!(t.eval("deps(//d)"), t.eval("//d"));
    assert_eq!(
        t.eval(&format!("deps(//c){with_gen}")),
        t.eval("//c union //d")
    );
    assert_eq!(
        t.eval(&format!("deps(//b){with_gen}")),
        t.eval("//b union //d")
    );
    assert_eq!(
        t.eval(&format!("deps(//a){with_gen}")),
        t.eval("//a union //b union //c union //d")
    );

    assert_eq!(t.eval("deps(//c:out)"), t.eval("deps(//c) union //c:out"));
    assert_eq!(t.eval("deps(//b:out)"), t.eval("deps(//b) union //b:out"));
    assert_eq!(t.eval("deps(//a:out)"), t.eval("deps(//a) union //a:out"));

    // Test depth-bounded variant:
    assert_eq!(t.eval(&format!("deps(//a, 0){with_gen}")), t.eval("//a"));
    assert_eq!(
        t.eval(&format!("deps(//a, 1){with_gen}")),
        t.eval("//a union //b union //c")
    );
    assert_eq!(
        t.eval(&format!("deps(//a, 2){with_gen}")),
        t.eval("//a + //b + //c + //d")
    );

    // Regression coverage for bug #1561800:
    // "blaze query 'deps(<output file>, 1)' returns the output file,
    // not its generating rule"
    assert_eq!(t.eval("deps(//a:out, 0)"), t.eval("//a:out"));
    assert_eq!(
        t.eval(&format!("deps(//a:out, 1){with_gen}")),
        t.eval("//a:out + //a")
    );
    assert_eq!(
        t.eval(&format!("deps(//a:out, 2){with_gen}")),
        t.eval("//a:out + //a + //b + //c")
    );

    // Configurable attributes:
    if t.test_configurable_attributes() {
        let tools = t.get_tools_repository();
        // analysisMock.isThisBazel()
        let implicit_deps = format!(" + {tools}//tools/def_parser:def_parser");
        let mut expected_dependencies = format!(
            "{tools}//tools/cpp:link_extra_lib + {tools}//tools/cpp:malloc + //configurable:main + \
             //configurable:main.cc + //configurable:adep + //configurable:bdep + \
             //configurable:defaultdep + //conditions:a + //conditions:b {implicit_deps}"
        );
        if t.include_cpp_toolchain_dependencies() {
            expected_dependencies
                .push_str(" + //tools/cpp:toolchain_type + //tools/cpp:current_cc_toolchain");
        }
        assert_eq!(
            t.eval(&format!(
                "deps(//configurable:main, 1){CC_DEPENDENCY_CORRECTION}"
            )),
            t.eval(&expected_dependencies)
        );
    }
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1002 AbstractQueryTest.testScopeOfLetExpressions
pub fn test_scope_of_let_expressions(t: &mut AbstractQueryTest) {
    let num_targets = 1000;
    let mut files_builder = String::from("'0'");
    for i in 1..num_targets {
        files_builder.push_str(&format!(", '{i}'"));
    }
    t.write_file("a/BUILD", &[&format!("exports_files([{files_builder}])")]);

    let mut let_query_builder = String::from("(let x = //a:0 in $x)");
    for i in 1..num_targets {
        let_query_builder.push_str(&format!(" + (let x = //a:{i} in $x)"));
    }
    assert_eq!(t.eval(&let_query_builder), t.eval("//a:* - //a:BUILD"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1120 AbstractQueryTest.testNoImplicitDeps
pub fn test_no_implicit_deps(t: &mut AbstractQueryTest) {
    t.write_file(
        "x/BUILD",
        &[
            "load('@rules_cc//cc:cc_binary.bzl', 'cc_binary')",
            "cc_binary(name='x', srcs=['x.cc'])",
        ],
    );
    let tools = t.get_tools_repository();
    // Implicit dependencies:
    let host_deps_expr = format!(
        "{tools}//tools/cpp:malloc + {tools}//tools/cpp:link_extra_lib + {tools}//tools/cpp:linkextra.cc"
    );
    // analysisMock.isThisBazel()
    let implicit_deps_expr = format!(
        " + {tools}//tools/def_parser:def_parser + {tools}//tools/def_parser:def_parser.exe"
    );
    let target_deps_expr = "//x:x + //x:x.cc";
    let toolchain_deps_expr = "//tools/cpp:toolchain_type + //tools/cpp:current_cc_toolchain";

    // Test all combinations of --[no]host_deps and --[no]implicit_deps on //x:x
    let mut expected = format!("{target_deps_expr} + {host_deps_expr}{implicit_deps_expr}");
    if t.include_cpp_toolchain_dependencies() {
        expected.push_str(&format!(" + {toolchain_deps_expr}"));
    }
    t.assert_equals_filtered(
        &expected,
        &format!("deps(//x){CC_DEPENDENCY_CORRECTION}"),
        &[],
    );
    t.assert_equals_filtered(
        &format!("{target_deps_expr} + {host_deps_expr}"),
        &format!("deps(//x){CC_DEPENDENCY_CORRECTION}"),
        &[Setting::OnlyTargetDeps],
    );
    t.assert_equals_filtered(target_deps_expr, "deps(//x)", &[Setting::NoImplicitDeps]);
    t.assert_equals_filtered(
        target_deps_expr,
        "deps(//x)",
        &[Setting::OnlyTargetDeps, Setting::NoImplicitDeps],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1329 AbstractQueryTest.testTestsOperatorFiltersByNegativeTag
pub fn test_tests_operator_filters_by_negative_tag(t: &mut AbstractQueryTest) {
    let py_load = blaze_sky_top::testing::mock_java_python_support::get_py_load("py_test");
    t.write_file(
        "b/BUILD",
        &[
            &py_load,
            "load('@rules_cc//cc:cc_test.bzl', 'cc_test')",
            "test_suite(name='foo_tests', tags=['foo'])",
            "test_suite(name='bar_tests', tags=['bar'])",
            "test_suite(name='foo_notbar_tests', tags=['foo', '-bar'])",
            "py_test(name='py_test', tags=['blah', 'prod'], srcs=['py_test.py'])",
            "cc_test(name='cc_test', tags=['foo'])",
            "cc_test(name='cc_test2', tags=['bar'])",
            "cc_test(name='cc_test3', tags=['foo', 'bar'])",
        ],
    );

    assert_eq!(t.eval("tests(//b:foo_notbar_tests)"), t.eval("//b:cc_test"));
    assert_eq!(
        t.eval("tests(//b:foo_tests)"),
        t.eval("//b:cc_test + //b:cc_test3")
    );
    assert_eq!(
        t.eval("tests(//b:bar_tests)"),
        t.eval("//b:cc_test2 + //b:cc_test3")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1471 AbstractQueryTest.testRuleOutputAmbiguityIsntFatal
pub fn test_rule_output_ambiguity_isnt_fatal(t: &mut AbstractQueryTest) {
    t.write_file("x/BUILD", &["genrule(name='x', outs=['x'], cmd='')"]);
    let result = t.eval("allpaths(x:*, //x)"); // doesn't crash
    // result = { genrule(//x) }
    assert_eq!(result.len(), 1, "{result:?}");
    let r = result.into_iter().next().unwrap();
    assert_eq!(r, "//x:x");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1590 AbstractQueryTest.testGraphOrderOfWildcards
pub fn test_graph_order_of_wildcards(t: &mut AbstractQueryTest) {
    use blaze_query::query::blaze_query_environment::{TargetDigraph, TargetNode};
    t.write_file(
        "x/BUILD",
        &["genrule(name='x', srcs=['y'], outs=['x.out'], cmd=':')
genrule(name='y', outs=['y.out'], cmd=':')
"],
    );
    t.helper.set_ordered_results(true); // This query needs a graph.

    let result_and_targets = t.evaluate_query("//x:*");
    let results: Vec<String> = result_and_targets
        .get_result_set()
        .iter()
        .map(|target| t.helper.get_label(target))
        .collect();
    let graph = result_and_targets.get_graph();
    let in_results = |n: &TargetNode| result_and_targets.get_result_set().contains(&n.0);
    let subgraph = graph.borrow().extract_subgraph(&in_results);

    // one(label): the single target of the result set with that label.
    let one = |label: &str| -> TargetNode {
        let found: Vec<_> = result_and_targets
            .get_result_set()
            .iter()
            .filter(|target| target.get_label().to_string() == label)
            .cloned()
            .collect();
        assert_eq!(found.len(), 1, "{label}");
        TargetNode(found.into_iter().next().unwrap())
    };
    let x_build = one("//x:BUILD");
    let xx = one("//x:x");
    let xxout = one("//x:x.out");
    let xy = one("//x:y");
    let xyout = one("//x:y.out");

    assert_eq!(
        sorted(results),
        sorted(
            ["//x:BUILD", "//x:x", "//x:x.out", "//x:y", "//x:y.out"]
                .map(String::from)
                .to_vec()
        )
    );

    let mut expected = TargetDigraph::new();
    expected.add_edge(xyout, xy.clone());
    expected.add_edge(xx.clone(), xy);
    expected.add_edge(xxout, xx);
    expected.create_node(x_build);
    assert!(
        expected.equals(&subgraph),
        "Expected:\n{:?}\nWas:\n{:?}",
        graph_edges(&expected),
        graph_edges(&subgraph)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1705 AbstractQueryTest.testBigExpression
pub fn test_big_expression(t: &mut AbstractQueryTest) {
    t.write_build_files3();

    let mut query = String::from("//a");
    for _ in 1..10000 {
        query.push_str("+ //b");
    }
    assert_eq!(t.eval(&query), t.eval("//a + //b"));
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1763 AbstractQueryTest.testQueryTimeLoadingTargetsBelowMissingPackage
pub fn test_query_time_loading_targets_below_missing_package(t: &mut AbstractQueryTest) {
    // Given a workspace containing one package, "//a",
    t.write_file(
        "a/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'a')",
        ],
    );

    // When the query environment is queried for targets belonging to packages beneath the
    // package "a/b", which doesn't exist,
    let missing_package = "a/b";
    let result = t.eval_throws(&format!("//{missing_package}/..."), false);
    let s = &result.message;

    // Then an exception is thrown that says that the pattern matched nothing.
    let expected = format!("no targets found beneath '{missing_package}'");
    assert!(s.contains(&expected), "{s}");
    assert_eq!(
        target_patterns_code(&result.failure_detail),
        Some(blaze_protos::failure_details::target_patterns::Code::TargetsMissing as i32)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1859 AbstractQueryTest.testHaveDepsOnAspectsAttributes
pub fn test_have_deps_on_aspects_attributes(t: &mut AbstractQueryTest) {
    t.use_extended_set_of_rules();
    t.write_file(
        "a/BUILD",
        &[
            "extra_attribute_aspect_requiring_provider(name='a', foo=[':b'])
honest(name='b', foo=[])
",
        ],
    );
    t.write_file("extra/BUILD", &["honest(name='extra', foo=[])"]);

    let s = t.eval_to_string("deps(//a:a)");
    assert!(s.contains("//extra:extra"), "{s}");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1890 AbstractQueryTest.testNoDepsOnAspectAttributeWithNoImpicitDeps
pub fn test_no_deps_on_aspect_attribute_with_no_impicit_deps(t: &mut AbstractQueryTest) {
    t.use_extended_set_of_rules();
    t.helper.set_query_settings(&[Setting::NoImplicitDeps]);
    t.write_file(
        "a/BUILD",
        &[
            "extra_attribute_aspect_requiring_provider(name='a', foo=[':b'])
honest(name='b', foo=[])
",
        ],
    );
    t.write_file("extra/BUILD", &["honest(name='extra', foo=[])"]);

    let s = t.eval_to_string("deps(//a:a)");
    assert!(!s.contains("//extra:extra"), "{s}");
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2299 AbstractQueryTest.testEqualityOfOrderedThreadSafeImmutableSet
pub fn test_equality_of_ordered_thread_safe_immutable_set(t: &mut AbstractQueryTest) {
    t.write_file(
        "foo/BUILD",
        &["load('//test_defs:foo_library.bzl', 'foo_library')
foo_library(name = 'a')
foo_library(name = 'b')
"],
    );

    let targets = t.evaluate_query("//foo:a + //foo:b").targets;
    let env = t.helper.get_query_environment();
    let mutable_set = env.as_query_environment().create_thread_safe_mutable_set();
    mutable_set.add_all(&targets);
    // Set equality: the same elements.
    let mut in_set: Vec<String> = mutable_set
        .to_vec()
        .iter()
        .map(|target| target.get_label().to_string())
        .collect();
    in_set.sort();
    let mut expected: Vec<String> = targets
        .iter()
        .map(|target| target.get_label().to_string())
        .collect();
    expected.sort();
    assert_eq!(expected, in_set);
    env.close();
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2527 AbstractQueryTest.testDeepNestedLet
pub fn test_deep_nested_let(t: &mut AbstractQueryTest) {
    t.write_file(
        "foo/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'foo')",
        ],
    );

    // We used to get a StackOverflowError at this depth. We're still vulnerable to stack
    // overflows at higher depths, due to how the query engine works.
    let nesting_depth = 500;
    let query_string = vec!["let x = //foo:foo in $x"; nesting_depth].join(" + ");

    assert_eq!(t.eval_to_string(&query_string), "//foo:foo");
}
