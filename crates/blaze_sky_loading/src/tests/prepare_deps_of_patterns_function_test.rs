//! PREPARE_DEPS_OF_PATTERNS (SkyQuery's universe) and TRANSITIVE_TRAVERSAL.
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java
//! (the external-repository and cycle cases need the module registry harness and are ignored).

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::StoredEventHandler;
use blaze_packages::exceptions::{NoSuchPackageException, NoSuchTargetException};
use blaze_skyframe::{Evaluator, SkyKey, WalkableGraph};
use blaze_vfs::PathFragment;

use crate::prepare_deps::TargetPatternSequence;
use crate::test_util::{FakeRepos, TestSetup, evaluate};
use crate::transitive_traversal::TransitiveTraversalKey;

fn setup() -> TestSetup {
    TestSetup::with_bazel_rules(FakeRepos::default())
}

fn file(setup: &TestSetup, rel: &str, contents: &str) {
    setup.scratch.file(&setup.ws(rel), contents);
}

fn get_key_for_label(label: &str) -> SkyKey {
    SkyKey::new(TransitiveTraversalKey::key(
        Label::parse_canonical_unchecked(label),
    ))
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:304 PrepareDepsOfPatternsFunctionTest.getGraphFromPatternsEvaluation
fn get_graph_from_patterns_evaluation(
    setup: &TestSetup,
    pattern_sequence: &[&str],
    keep_going: bool,
) -> (Evaluator, Arc<StoredEventHandler>) {
    let evaluator = setup.evaluator();
    let events = Arc::new(StoredEventHandler::new());
    let key = TargetPatternSequence::key(
        pattern_sequence.iter().map(|p| (*p).to_owned()).collect(),
        PathFragment::empty_fragment(),
    );
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(key)],
        keep_going,
        events.clone(),
    );
    assert!(!result.has_error(), "{:?}", result);
    (evaluator, events)
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:317 PrepareDepsOfPatternsFunctionTest.createFooAndFoo2
fn create_foo_and_foo2(setup: &TestSetup, dependent: bool) {
    let dependency_if_any = if dependent { "srcs = [':foo2']," } else { "" };
    file(
        setup,
        "foo/BUILD",
        &format!(
            "genrule(name = 'foo',\n{dependency_if_any}\n    outs = ['out.txt'],\n    cmd = 'touch $@')\n\
             genrule(name = 'foo2',\n    outs = ['out2.txt'],\n    cmd = 'touch $@')\n"
        ),
    );
}

fn exists(key: &SkyKey, graph: &dyn WalkableGraph) -> bool {
    graph.get_value(key).is_some() || graph.get_exception(key).is_some()
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:378 PrepareDepsOfPatternsFunctionTest.assertValidValue
fn assert_valid_value(graph: &dyn WalkableGraph, key: &SkyKey, expect_transitive_exception: bool) {
    assert!(graph.get_value(key).is_some(), "{:?}", key);
    assert_eq!(
        graph.get_exception(key).is_some(),
        expect_transitive_exception,
        "{:?}",
        key
    );
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:398 PrepareDepsOfPatternsFunctionTest.assertException
fn assert_exception(graph: &dyn WalkableGraph, key: &SkyKey) -> blaze_skyframe::SkyError {
    assert!(graph.get_value(key).is_none(), "{:?}", key);
    graph.get_exception(key).expect("an exception")
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:64 PrepareDepsOfPatternsFunctionTest.testFunctionLoadsTargetAndNotUnspecifiedTargets
#[test]
fn test_function_loads_target_and_not_unspecified_targets() {
    let s = setup();
    create_foo_and_foo2(&s, false);
    let (graph, _) = get_graph_from_patterns_evaluation(&s, &["//foo"], true);
    assert_valid_value(&graph, &get_key_for_label("//foo:foo"), false);
    assert!(!exists(&get_key_for_label("//foo:foo2"), &graph));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:82 PrepareDepsOfPatternsFunctionTest.testFunctionLoadsTargetDependencies
#[test]
fn test_function_loads_target_dependencies() {
    let s = setup();
    create_foo_and_foo2(&s, true);
    let (graph, _) = get_graph_from_patterns_evaluation(&s, &["//foo"], true);
    assert_valid_value(&graph, &get_key_for_label("//foo:foo2"), false);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:98 PrepareDepsOfPatternsFunctionTest.testFunctionExpandsTargetPatterns
#[test]
fn test_function_expands_target_patterns() {
    let s = setup();
    create_foo_and_foo2(&s, false);
    let (graph, _) = get_graph_from_patterns_evaluation(&s, &["//foo:*"], true);
    assert_valid_value(&graph, &get_key_for_label("//foo:foo"), false);
    assert_valid_value(&graph, &get_key_for_label("//foo:foo2"), false);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:114 PrepareDepsOfPatternsFunctionTest.testTargetParsingException
#[test]
fn test_target_parsing_exception() {
    let s = setup();
    let (graph, _) = get_graph_from_patterns_evaluation(&s, &["//foo:foo"], true);
    assert!(!exists(&get_key_for_label("//foo:foo"), &graph));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:127 PrepareDepsOfPatternsFunctionTest.testDependencyTraversalNoSuchPackageException
#[test]
fn test_dependency_traversal_no_such_package_exception() {
    let s = setup();
    file(
        &s,
        "foo/BUILD",
        "genrule(\n    name = \"foo\",\n    srcs = [\"//bar\"],\n    outs = [\"out.txt\"],\n    cmd = \"touch $@\",\n)\n",
    );
    let (graph, _) = get_graph_from_patterns_evaluation(&s, &["//foo"], true);
    assert_valid_value(&graph, &get_key_for_label("//foo:foo"), true);
    let e = assert_exception(&graph, &get_key_for_label("//bar:bar"));
    assert!(e.downcast_ref::<NoSuchPackageException>().is_some(), "{e}");
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:150 PrepareDepsOfPatternsFunctionTest.testDependencyTraversalNoSuchTargetException
#[test]
fn test_dependency_traversal_no_such_target_exception() {
    let s = setup();
    file(
        &s,
        "foo/BUILD",
        "genrule(\n    name = \"foo\",\n    srcs = [\"//bar\"],\n    outs = [\"out.txt\"],\n    cmd = \"touch $@\",\n)\n",
    );
    file(&s, "bar/BUILD", "");
    let (graph, _) = get_graph_from_patterns_evaluation(&s, &["//foo"], true);
    assert_valid_value(&graph, &get_key_for_label("//foo:foo"), true);
    let e = assert_exception(&graph, &get_key_for_label("//bar:bar"));
    assert!(e.downcast_ref::<NoSuchTargetException>().is_some(), "{e}");
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:188 PrepareDepsOfPatternsFunctionTest.parsingProblem
fn parsing_problem(keep_going: bool) {
    let s = setup();
    create_foo_and_foo2(&s, false);
    let bogus_pattern = "//foo/....";
    let (graph, events) =
        get_graph_from_patterns_evaluation(&s, &[bogus_pattern, "//foo:foo"], keep_going);
    let messages: Vec<String> = events
        .get_events()
        .iter()
        .map(|e| e.get_message().to_string())
        .collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains(&format!("Skipping '{bogus_pattern}': "))),
        "{messages:?}"
    );
    assert!(exists(&get_key_for_label("//foo:foo"), &graph));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:173 PrepareDepsOfPatternsFunctionTest.testParsingProblemsKeepGoing
#[test]
fn test_parsing_problems_keep_going() {
    parsing_problem(true);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:184 PrepareDepsOfPatternsFunctionTest.testParsingProblemsNoKeepGoing
#[test]
fn test_parsing_problems_no_keep_going() {
    parsing_problem(false);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:209 PrepareDepsOfPatternsFunctionTest.testFunctionLoadsTargetFromExternalRepo
#[test]
#[ignore = "needs the bzlmod registry harness (FakeRegistry) in blaze_sky_loading's tests"]
fn test_function_loads_target_from_external_repo() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunctionTest.java:227 PrepareDepsOfPatternsFunctionTest.testDepOnCachedPDOPNodeThatItselfDependsOnBothCycleAndError
#[test]
#[ignore = "needs a cached PREPARE_DEPS_OF_PATTERN node over a cycle and an error (Java's \
            SkyframeExecutor test harness)"]
fn test_dep_on_cached_pdop_node_that_itself_depends_on_both_cycle_and_error() {}

/// Not a Java test: a recursive pattern requests COLLECT_PACKAGES_UNDER_DIRECTORY and loads the
/// targets of every package beneath the directory.
#[test]
fn recursive_pattern_loads_every_package_and_collects_packages() {
    let s = setup();
    create_foo_and_foo2(&s, false);
    file(&s, "foo/sub/BUILD", "filegroup(name = 'sub')");
    let (graph, _) = get_graph_from_patterns_evaluation(&s, &["//foo/..."], true);
    assert_valid_value(&graph, &get_key_for_label("//foo:foo"), false);
    assert_valid_value(&graph, &get_key_for_label("//foo/sub:sub"), false);
}
