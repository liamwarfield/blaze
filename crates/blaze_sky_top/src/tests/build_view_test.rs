//! Port of `src/test/java/com/google/devtools/build/lib/analysis/BuildViewTest.java` (an
//! `AnalysisTestCase` through `BuildViewTestBase`).

use indexmap::IndexSet;

use crate::analysis_failure_event::{AnalysisFailureEvent, LoadingFailureEvent};
use crate::build_view::filter_tests_by_targets;
use crate::testing::build_view_test_case::parse_label;
use crate::testing::{AnalysisTestCase, BuildViewTestCase, Flag, has_error};
use blaze_analysis::rule::rule_error_consumer::AnalysisRootCauseEvent;

fn new_test() -> AnalysisTestCase {
    BuildViewTestCase::analysis_test_case()
}

// Java: BuildViewTest.directoryArtifactInRoot
#[test]
fn directory_artifact_in_root() {
    let mut t = new_test();
    t.scratch.file(
        "BUILD",
        &["genrule(name = 'slurps_dir', srcs = ['.'], outs = ['out'], cmd = 'touch $@')"],
    );
    // Expect no errors.
    t.update(&["//:slurps_dir"]).unwrap();
}

// Java: BuildViewTest.testRuleConfiguredTarget
#[test]
fn test_rule_configured_target() {
    let mut t = new_test();
    t.scratch.file(
        "pkg/BUILD",
        &["genrule(name='foo',\n        cmd = '',\n        srcs=['a.src'],\n        outs=['a.out'])\n"],
    );
    t.update(&["//pkg:foo"]).unwrap();
    let rule_target = t.get_target("//pkg:foo").unwrap();
    assert_eq!(rule_target.as_rule().unwrap().get_rule_class(), "genrule");

    let rule_ctat = t.get_configured_target_and_data("//pkg:foo").unwrap();

    // PORT: isSameInstanceAs: the same rule of the same loaded package.
    assert!(std::sync::Arc::ptr_eq(
        rule_ctat.get_target().as_rule().unwrap(),
        rule_target.as_rule().unwrap()
    ));
}

// Java: BuildViewTest.testFilterByTargets
#[test]
fn test_filter_by_targets() {
    let mut t = new_test();
    t.scratch.file(
        "tests/BUILD",
        &[r#"load('//test_defs:foo_test.bzl', 'foo_test')
foo_test(name = 'small_test_1',
        srcs = ['small_test_1.sh'],
        data = [':xUnit'],
        size = 'small',
        tags = ['tag1'])

foo_test(name = 'small_test_2',
        srcs = ['small_test_2.sh'],
        size = 'small',
        tags = ['tag2'])


test_suite( name = 'smallTests', tags=['small'])
"#],
    );
    //scratch.file("tests/small_test_1.py");

    t.update(&["//tests:smallTests"]).unwrap();
    let test1 = t
        .get_configured_target_and_data("//tests:small_test_1")
        .unwrap();
    let test2 = t
        .get_configured_target_and_data("//tests:small_test_2")
        .unwrap();
    let suite = t
        .get_configured_target_and_data("//tests:smallTests")
        .unwrap();

    let test1_ct = test1.get_configured_target().clone();
    let test2_ct = test2.get_configured_target().clone();
    let suite_ct = suite.get_configured_target().clone();
    t.assert_no_events(); // start from a clean slate

    let targets = vec![test1_ct, test2_ct, suite_ct];
    let allowed: IndexSet<_> = [test1.get_target_label(), suite.get_target_label()]
        .into_iter()
        .collect();
    let targets = filter_tests_by_targets(&targets, &allowed);
    let labels: Vec<String> = targets
        .iter()
        .map(|ct| ct.get_label().get_canonical_form())
        .collect();
    assert_eq!(labels, vec!["//tests:small_test_1", "//tests:smallTests"]);
}

// Java: BuildViewTest.testSourceArtifact
#[test]
#[ignore = "needs TestRuleClassProvider's test rules (testing_dummy_rule)"]
fn test_source_artifact() {}

// Java: BuildViewTest.testGeneratedArtifact
#[test]
#[ignore = "needs TestRuleClassProvider's test rules (testing_dummy_rule)"]
fn test_generated_artifact() {}

// Java: BuildViewTest.testGetArtifactOwnerInStarlark
#[test]
fn test_get_artifact_owner_in_starlark() {
    let mut t = new_test();
    t.scratch.file(
        "foo/rule.bzl",
        &[r#"def _impl(ctx):
  f = ctx.actions.declare_file('rule_output')
  print('f owner is ' + str(f.owner))
  ctx.actions.write(
    output = f,
    content = 'foo',
  )
gen = rule(implementation = _impl)
"#],
    );
    t.scratch.file(
        "foo/BUILD",
        &["load(':rule.bzl', 'gen')\ngen(name = 'a')\n"],
    );

    t.update(&["//foo:a"]).unwrap();
    let expected = t.ws("DEBUG /workspace/foo/rule.bzl:3:8: f owner is @@//foo:a");
    t.assert_contains_event(&expected);
}

// Java: BuildViewTest.testSyntaxErrorInDepPackage
#[test]
fn test_syntax_error_in_dep_package() {
    let mut t = new_test();
    // Check that a loading error in a dependency is properly reported.
    t.scratch.file(
        "a/BUILD",
        &[
            "genrule(name='x',",
            "        srcs = ['file.txt'],",
            "        outs = ['foo'],",
            "        cmd = 'echo')",
            "@", // syntax error
        ],
    );

    t.scratch.file(
        "b/BUILD",
        &["genrule(name= 'cc',\n        tools = ['//a:x'],\n        outs = ['bar'],\n        cmd = 'echo')\n"],
    );

    t.remove_fail_fast_handler();
    let flags = t.default_flags().with(Flag::KeepGoing);
    let result = t.update_with(&flags, &[], &["//b:cc"]).unwrap();
    let error = has_error(result);

    t.assert_contains_event("invalid character: '@'");
    assert!(error);
}

// Java: BuildViewTest.testReportsVisibilityAnalysisRootCauses
#[test]
fn test_reports_visibility_analysis_root_causes() {
    let mut t = new_test();
    t.scratch.file(
        "private/BUILD",
        &["genrule(\n    name='private',\n    outs=['private.out'],\n    cmd='',\n    visibility=['//visibility:private'])\n"],
    );
    t.scratch.file(
        "foo/BUILD",
        &["genrule(\n    name='foo',\n    tools=[':bar'],\n    outs=['foo.out'],\n    cmd='')\ngenrule(\n    name='bar',\n    tools=['//private'],\n    outs=['bar.out'],\n    cmd='')\n"],
    );

    t.remove_fail_fast_handler();
    let flags = t.default_flags().with(Flag::KeepGoing);
    let result = t.update_with(&flags, &[], &["//foo"]).unwrap();
    assert!(has_error(result));

    let events = t.event_bus.get::<AnalysisFailureEvent>();
    assert_eq!(events.len(), 1, "{events:?}");
    let event = &events[0];
    assert_eq!(
        event
            .get_legacy_failure_reason()
            .unwrap()
            .get_canonical_form(),
        "//foo:bar"
    );
    assert_eq!(
        event.get_failed_target().get_label().get_canonical_form(),
        "//foo:foo"
    );

    let causes = t.event_bus.get::<AnalysisRootCauseEvent>();
    assert_eq!(causes.len(), 1, "{causes:?}");
    assert_eq!(causes[0].label.get_canonical_form(), "//foo:bar");
}

// Java: BuildViewTest.testReportsNonExistentPackageAnalysisRootCausesNoKeepGoing
#[test]
#[ignore = "not ported yet: body (needs BuildViewTestBase.AnalysisFailureRecorder on the event bus; the mock rules_java is in the harness)"]
fn test_reports_non_existent_package_analysis_root_causes_no_keep_going() {}

// Java: BuildViewTest.testReportsNonExistentPackageAnalysisRootCausesKeepGoing
#[test]
#[ignore = "not ported yet: body (needs BuildViewTestBase.AnalysisFailureRecorder on the event bus; the mock rules_java is in the harness)"]
fn test_reports_non_existent_package_analysis_root_causes_keep_going() {}

// Java: BuildViewTest.testReportsNonExistentPackageInPackageGroupKeepGoing
#[test]
#[ignore = "not ported yet: body (needs BuildViewTestBase.AnalysisFailureRecorder on the event bus; the mock rules_java is in the harness)"]
fn test_reports_non_existent_package_in_package_group_keep_going() {}

// Java: BuildViewTest.testTestOnlyFailureReported
#[test]
fn test_test_only_failure_reported() {
    let mut t = new_test();
    t.scratch.file(
        "foo/BUILD",
        &["genrule(\n    name='foo',\n    tools=[':bar'],\n    outs=['foo.out'],\n    cmd='')\ngenrule(\n    name='bar',\n    outs=['bar.out'],\n    testonly=1,\n    cmd='')\n"],
    );

    t.remove_fail_fast_handler();
    let flags = t.default_flags().with(Flag::KeepGoing);
    let result = t.update_with(&flags, &[], &["//foo"]).unwrap();
    assert!(has_error(result));

    let events = t.event_bus.get::<AnalysisFailureEvent>();
    assert_eq!(events.len(), 1, "{events:?}");
    let event = &events[0];
    assert_eq!(
        event
            .get_legacy_failure_reason()
            .unwrap()
            .get_canonical_form(),
        "//foo:foo"
    );
    assert_eq!(
        event.get_failed_target().get_label().get_canonical_form(),
        "//foo:foo"
    );

    let causes = t.event_bus.get::<AnalysisRootCauseEvent>();
    assert_eq!(causes.len(), 1, "{causes:?}");
    assert_eq!(causes[0].label.get_canonical_form(), "//foo:foo");
}

// Java: BuildViewTest.testAnalysisReportsDependencyCycle
#[test]
fn test_analysis_reports_dependency_cycle() {
    let mut t = new_test();
    t.scratch.file(
        "foo/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name='foo',deps=['//bar'])",
        ],
    );
    t.scratch.file(
        "bar/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name='bar',deps=[':bar'])",
        ],
    );

    t.remove_fail_fast_handler();
    let flags = t.default_flags().with(Flag::KeepGoing);
    let result = t.update_with(&flags, &[], &["//foo"]).unwrap();
    assert!(has_error(result));
    let events = t.event_bus.get::<AnalysisFailureEvent>();
    assert_eq!(events.len(), 1, "{events:?}");
    // getConfigurationId() is not the null configuration's.
    assert!(
        events[0]
            .get_failed_target()
            .get_configuration_key()
            .is_some(),
        "{:?}",
        events[0]
    );
}

// Java: BuildViewTest.testReportsLoadingRootCauses
#[test]
fn test_reports_loading_root_causes() {
    let mut t = new_test();
    // This test checks that two simultaneous errors are both reported:
    // - missing outs attribute,
    // - package referenced in tools does not exist
    t.scratch.file(
        "pkg/BUILD",
        &["genrule(name='foo',\n        tools=['//nopackage:missing'],\n        cmd='')\n"],
    );

    t.remove_fail_fast_handler();
    let flags = t.default_flags().with(Flag::KeepGoing);
    let result = t.update_with(&flags, &[], &["//pkg:foo"]).unwrap();
    assert!(has_error(result));

    let analysis_events = t.event_bus.get::<AnalysisFailureEvent>();
    assert_eq!(analysis_events.len(), 1, "{analysis_events:?}");
    let analysis_failure_event = &analysis_events[0];
    assert_eq!(
        analysis_failure_event
            .get_failed_target()
            .get_label()
            .get_canonical_form(),
        "//pkg:foo"
    );
    let analysis_failure_causes = analysis_failure_event.get_root_causes();
    let missing_package_cause = if matches!(
        analysis_failure_causes[0],
        crate::analysis_failure_event::AnalysisRootCause::AnalysisFailed { .. }
    ) {
        &analysis_failure_causes[0]
    } else {
        &analysis_failure_causes[1]
    };
    assert_eq!(
        missing_package_cause.get_label(),
        &parse_label("//nopackage:missing")
    );
    t.assert_contains_event("missing value for mandatory attribute 'outs'");
    t.assert_contains_event("no such package 'nopackage'");
    // Skyframe correctly reports the other root cause as the genrule itself (since it is
    // missing attributes).
    let loading_events = t.event_bus.get::<LoadingFailureEvent>();
    assert_eq!(loading_events.len(), 1, "{loading_events:?}");
    assert!(loading_events.contains(&LoadingFailureEvent::new(
        parse_label("//pkg:foo"),
        parse_label("//pkg:foo")
    )));
}

// Java: BuildViewTest.testMultipleRootCauseReporting
#[test]
fn test_multiple_root_cause_reporting() {
    let mut t = new_test();
    t.scratch.file(
        "gp/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'gp', deps = ['//p:p'])",
        ],
    );
    t.scratch.file(
        "p/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'p', deps = ['//c1:not', '//c2:not'])",
        ],
    );
    t.scratch.file("c1/BUILD", &[]);
    t.scratch.file("c2/BUILD", &[]);
    t.remove_fail_fast_handler();
    let flags = t.default_flags().with(Flag::KeepGoing);
    let result = t.update_with(&flags, &[], &["//gp"]).unwrap();
    assert!(has_error(result));
    let events = t.event_bus.get::<LoadingFailureEvent>();
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(events.contains(&LoadingFailureEvent::new(
        parse_label("//gp"),
        parse_label("//c1:not")
    )));
    assert!(events.contains(&LoadingFailureEvent::new(
        parse_label("//gp"),
        parse_label("//c2:not")
    )));
}

/// Regression test for: "Package group includes are broken"
// Java: BuildViewTest.testTopLevelPackageGroup
#[test]
fn test_top_level_package_group() {
    let mut t = new_test();
    t.scratch.file(
        "tropical/BUILD",
        &["package_group(name='guava', includes=[':mango'])\npackage_group(name='mango')\n"],
    );

    // If the analysis phase results in an error, this will throw an exception
    t.update(&["//tropical:guava"]).unwrap();

    // Check if the included package group also got analyzed
    assert!(
        t.get_configured_target_in("//tropical:mango", None)
            .is_some()
    );
}

// Java: BuildViewTest.testTopLevelInputFile
#[test]
fn test_top_level_input_file() {
    let mut t = new_test();
    t.scratch
        .file("tropical/BUILD", &["exports_files(['file.txt'])"]);
    t.update(&["//tropical:file.txt"]).unwrap();
    assert!(
        t.get_configured_target_in("//tropical:file.txt", None)
            .is_some()
    );
}

// Java: BuildViewTest.testAnalysisErrorMessageWithKeepGoing
#[test]
fn test_analysis_error_message_with_keep_going() {
    let mut t = new_test();
    t.scratch.file(
        "a/foo_one.bzl",
        &[r#"def _impl(ctx):
  if len(ctx.files.srcs) != 1:
     fail("you must specify exactly one file in 'srcs'", attr = "srcs")
foo_one = rule(
  implementation = _impl,
  attrs = {
    "srcs": attr.label_list(allow_files=True),
  },
)
"#],
    );
    t.scratch.file(
        "a/BUILD",
        &[
            "load(':foo_one.bzl', 'foo_one')",
            "foo_one(name='a', srcs=['a1.sh', 'a2.sh'])",
        ],
    );
    t.remove_fail_fast_handler();
    let flags = t.default_flags().with(Flag::KeepGoing);
    let result = t.update_with(&flags, &[], &["//a"]).unwrap();
    assert!(has_error(result));
    t.assert_contains_event("errors encountered while analyzing target '//a:a'");
}

/// Regression test: Exception in ConfiguredTargetGraph.checkForCycles() when multiple top-level
/// targets depend on the same cycle.
// Java: BuildViewTest.testCircularDependencyBelowTwoTargets
#[test]
fn test_circular_dependency_below_two_targets() {
    let mut t = new_test();
    t.scratch.file(
        "foo/BUILD",
        &[r#"load('//test_defs:foo_library.bzl', 'foo_library')
foo_library(name = 'top1', srcs = ['top1.sh'], deps = [':rec1'])
foo_library(name = 'top2', srcs = ['top2.sh'], deps = [':rec1'])
foo_library(name = 'rec1', srcs = ['rec1.sh'], deps = [':rec2'])
foo_library(name = 'rec2', srcs = ['rec2.sh'], deps = [':rec1'])
"#],
    );
    t.remove_fail_fast_handler();
    let flags = t.default_flags().with(Flag::KeepGoing);
    let result = t
        .update_with(&flags, &[], &["//foo:top1", "//foo:top2"])
        .unwrap();
    assert!(has_error(result));
    t.assert_contains_event("in foo_library rule //foo:rec1: cycle in dependency graph:\n");
    t.assert_contains_event("in foo_library rule //foo:top");
}

// Java: BuildViewTest.testErrorBelowCycleKeepGoing
#[test]
fn test_error_below_cycle_keep_going() {
    let mut t = new_test();
    t.scratch.file(
        "foo/BUILD",
        &[r#"load('//test_defs:foo_library.bzl', 'foo_library')
foo_library(name = 'top', deps = ['mid'])
foo_library(name = 'mid', deps = ['bad', 'cycle1'])
foo_library(name = 'bad', srcs = ['//badbuild:isweird'])
foo_library(name = 'cycle1', deps = ['cycle2', 'mid'])
foo_library(name = 'cycle2', deps = ['cycle1'])
"#],
    );
    t.scratch.file("badbuild/BUILD", &[""]);
    t.remove_fail_fast_handler();
    let flags = t.default_flags().with(Flag::KeepGoing);
    let _ = t.update_with(&flags, &[], &["//foo:top"]);
    t.assert_contains_event(
        "no such target '//badbuild:isweird': target 'isweird' not declared in package 'badbuild'",
    );
    t.assert_contains_event("and referenced by '//foo:bad'");
    t.assert_contains_event("in foo_library rule //foo");
    t.assert_contains_event("cycle in dependency graph");
}
