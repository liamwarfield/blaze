//! Not a Java test: after every command, only the kept Skyframe graph holds its evaluator.
//!
//! A server keeps its graph between commands only when nothing but the graph holds the
//! evaluator once the command's executors are gone (blaze_sky_top `incremental`). Every new
//! strong holder (an `Arc` cycle through a graph value, a function or the command's services:
//! the package root lookup, the rewound-action synchronizer and the action executor's graph of
//! 2026-11-26) leaks the evaluator and makes the next command start from an empty graph. The
//! dispatcher checks it after every command (`SkyframeIncrementalState::check_kept_graph_unshared`,
//! a panic in debug builds); this test runs commands that analyze, execute actions (a genrule,
//! a Starlark action, a tree artifact, a test with a runfiles tree) and query, and asserts the
//! count itself, so a holder added in any crate fails here.

use std::sync::Arc;

use blaze_buildtool_testing::BuildIntegrationTestCase;

const RULES: &[&str] = &[
    "def _t_impl(ctx):",
    "    out = ctx.actions.declare_file(ctx.label.name + '.sh')",
    "    ctx.actions.write(out, '#!/bin/sh\\nexit 0\\n', is_executable = True)",
    "    return [DefaultInfo(executable = out, runfiles = ctx.runfiles(files = ctx.files.data))]",
    "",
    "t_test = rule(_t_impl, test = True, attrs = {'data': attr.label_list(allow_files = True)})",
    "",
    "def _tree_impl(ctx):",
    "    d = ctx.actions.declare_directory(ctx.label.name)",
    "    ctx.actions.run_shell(outputs = [d], command = 'echo x > %s/f.txt' % d.path)",
    "    return [DefaultInfo(files = depset([d]))]",
    "",
    "tree = rule(_tree_impl)",
];

const BUILD: &[&str] = &[
    "load(':rules.bzl', 't_test', 'tree')",
    "genrule(name = 'g', srcs = ['in.txt'], outs = ['g.out'], cmd = 'cp $< $@')",
    "tree(name = 'tree')",
    "t_test(name = 't', data = [':g', ':tree'])",
];

/// The kept graph after the last command, and the assertion that the graph (plus the clone this
/// test takes) is its only holder.
fn assert_only_the_kept_graph_holds_the_evaluator(t: &BuildIntegrationTestCase, after: &str) {
    let evaluator = t
        .get_kept_evaluator()
        .unwrap_or_else(|| panic!("no graph kept after {after}"));
    assert_eq!(
        Arc::strong_count(&evaluator),
        2,
        "after {after}: something besides the kept graph holds its evaluator"
    );
}

#[test]
fn only_the_kept_graph_holds_the_evaluator_after_each_command() {
    let mut t = BuildIntegrationTestCase::new();
    t.write("pkg/rules.bzl", RULES);
    t.write("pkg/BUILD", BUILD);
    t.write("pkg/in.txt", &["in"]);

    let built = t.build_target(&["//pkg:all"]).is_success();
    assert!(built, "{}", t.out_err().err_as_latin1());
    assert_only_the_kept_graph_holds_the_evaluator(&t, "a build that executed actions");

    let tested = t.run_command("test", &["//pkg:t"]).is_success();
    assert!(tested, "{}", t.out_err().err_as_latin1());
    assert_only_the_kept_graph_holds_the_evaluator(&t, "a test command");

    t.write("pkg/in.txt", &["changed"]);
    let rebuilt = t.build_target(&["//pkg:all"]).is_success();
    assert!(rebuilt, "{}", t.out_err().err_as_latin1());
    assert_only_the_kept_graph_holds_the_evaluator(&t, "an incremental build");

    let queried = t.run_command("cquery", &["//pkg:all"]).is_success();
    assert!(queried, "{}", t.out_err().err_as_latin1());
    assert_only_the_kept_graph_holds_the_evaluator(&t, "a cquery");
}

#[test]
fn only_the_kept_graph_holds_the_evaluator_after_a_failed_build() {
    let mut t = BuildIntegrationTestCase::new();
    t.write(
        "pkg/BUILD",
        &["genrule(name = 'bad', outs = ['bad.out'], cmd = 'exit 1')"],
    );
    let built = t.build_target(&["//pkg:bad"]).is_success();
    assert!(!built, "the genrule must fail");
    assert_only_the_kept_graph_holds_the_evaluator(&t, "a build whose action failed");
}
