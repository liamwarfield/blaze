//! The two cases of `PrepareDepsOfPatternsFunctionTest` that need `BuildViewTestCase` (the fake
//! registry, the mock `test_defs` rules and a graph kept between two evaluations); the others
//! are in `blaze_sky_loading/src/tests/prepare_deps_of_patterns_function_test.rs`.
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java

use blaze_cmdline::Label;
use blaze_sky_loading::prepare_deps::TargetPatternSequence;
use blaze_sky_loading::transitive_traversal::TransitiveTraversalKey;
use blaze_skyframe::{SkyKey, WalkableGraph};
use blaze_vfs::PathFragment;

use crate::testing::{BuildViewTestCase, create_module_key};

fn get_key_for_label(label: &str) -> SkyKey {
    SkyKey::new(TransitiveTraversalKey::key(
        Label::parse_canonical_unchecked(label),
    ))
}

fn patterns_key(pattern_sequence: &[&str]) -> SkyKey {
    SkyKey::new(TargetPatternSequence::key(
        pattern_sequence.iter().map(|p| (*p).to_owned()).collect(),
        PathFragment::empty_fragment(),
    ))
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:378 PrepareDepsOfPatternsFunctionTest.assertValidValue
fn assert_valid_value(graph: &dyn WalkableGraph, key: &SkyKey) {
    assert!(graph.get_value(key).is_some(), "{key:?}");
    assert!(graph.get_exception(key).is_none(), "{key:?}");
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:357 PrepareDepsOfPatternsFunctionTest.writeBzlmodFiles
fn write_bzlmod_files(t: &mut BuildViewTestCase) {
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &["bazel_dep(name= \"repo\", version=\"1.0\", repo_name=\"my_repo\")"],
    );
    t.scratch.overwrite_file(
        "rinne/BUILD",
        &[
            "genrule(",
            "    name = \"rinne\",",
            "    srcs = [\"@my_repo//a:x\"],",
            "    outs = [\"out.txt\"],",
            "    cmd = \"touch $@\",",
            ")",
        ],
    );
    t.registry().add_module(
        &create_module_key("repo", "1.0"),
        &["module(name = \"repo\", version = \"1.0\")"],
    );
    let module_root = t.module_root.clone();
    t.scratch.file(
        module_root
            .get_relative_str("repo+1.0/REPO.bazel")
            .get_path_string(),
        &[],
    );
    t.scratch.file(
        module_root
            .get_relative_str("repo+1.0/a/BUILD")
            .get_path_string(),
        &["exports_files(['x'])"],
    );
    t.invalidate_packages();
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:209 PrepareDepsOfPatternsFunctionTest.testFunctionLoadsTargetFromExternalRepo
#[test]
fn test_function_loads_target_from_external_repo() {
    let mut t = BuildViewTestCase::new();
    write_bzlmod_files(&mut t);

    // Given a target pattern sequence consisting of a single-target pattern for "//rinne",
    // when PrepareDepsOfPatternsFunction successfully completes evaluation,
    let result = t.evaluate(&[patterns_key(&["//rinne"])], /* keep_going= */ true);
    assert!(!result.has_error(), "{result:?}");
    let graph = t.get_evaluator();

    // Then the graph contains a value for the target "@//rinne:rinne" and the dep
    // "@@repo+//a:x",
    assert_valid_value(&*graph, &get_key_for_label("//rinne:rinne"));
    assert_valid_value(&*graph, &get_key_for_label("@@repo+//a:x"));
}

// Regression test for b/225877591 ("Unexpected missing value in PrepareDepsOfPatternsFunction
// when there's both a dep with a cached cycle and another dep with an error").
// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:227 PrepareDepsOfPatternsFunctionTest.testDepOnCachedPDOPNodeThatItselfDependsOnBothCycleAndError
#[test]
fn test_dep_on_cached_pdop_node_that_itself_depends_on_both_cycle_and_error() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "foo/BUILD",
        &[
            "load(\"//test_defs:foo_library.bzl\", \"foo_library\")",
            "foo_library(",
            "    name = \"t1\",",
            "    deps = [\"//foo:t2\"],",
            ")",
            "",
            "foo_library(",
            "    name = \"t2\",",
            "    deps = [",
            "        \"//foo:t1\",",
            "        \"//nope\",",
            "    ],",
            ")",
        ],
    );
    let _unused_just_want_side_effect_of_priming_graph =
        t.evaluate(&[patterns_key(&["//foo/..."])], /* keep_going= */ true);
    // The main property we're trying to test is that we don't crash (due to
    // PrepareDepsOfPatternsFunction's usage of BugReport.sendNonFatalBugReport). We get that by
    // virtue of control flow getting past this statement.
    let pdops_key = patterns_key(&["//foo/...", "//doesnt:matter"]);
    let evaluation_result = t.evaluate(std::slice::from_ref(&pdops_key), true);
    let error = evaluation_result
        .get_error(&pdops_key)
        .unwrap_or_else(|| panic!("an error entry for {pdops_key:?}: {evaluation_result:?}"));
    let exception = error.get_exception().expect("an exception");
    assert!(
        exception.to_string().contains("no such package 'nope'"),
        "{exception}"
    );
    assert_eq!(
        error.get_cycle_info()[0].get_cycle(),
        &[get_key_for_label("//foo:t1"), get_key_for_label("//foo:t2")]
    );
}
