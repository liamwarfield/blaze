//! Port of `src/test/java/com/google/devtools/build/lib/runtime/commands/InstrumentationFilterTest.java`
//! (the `--instrumentation_filter` heuristic of the coverage command).

use blaze_events::{EventCollector, EventKind};
use blaze_packages::target::Target;

use crate::instrumentation_filter_support::InstrumentationFilterSupport;
use crate::testing::BuildViewTestCase;

fn get_targets(t: &mut BuildViewTestCase, labels: &[&str]) -> Vec<Target> {
    labels.iter().map(|l| t.get_target(l).unwrap()).collect()
}

fn assert_events_report_instrumentation_filter(events: &EventCollector, expected_filter: &str) {
    let messages: Vec<String> = events
        .iter()
        .map(|e| e.get_message().into_owned())
        .collect();
    assert_eq!(
        messages,
        vec![
            format!("Using default value for --instrumentation_filter: \"{expected_filter}\"."),
            "Override the above default with --instrumentation_filter".to_string(),
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/InstrumentationFilterTest.java:69 InstrumentationFilterTest.testSingleTest
#[test]
fn test_single_test() {
    let mut t = BuildViewTestCase::new();
    let events = EventCollector::with_kinds(&[EventKind::Info]);
    t.scratch.file(
        "my/package1/BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name='t1',srcs=['t1.sh'])"#,
        ],
    );
    let targets = get_targets(&mut t, &["//my/package1:t1"]);
    let expected_filter = "^//my/package1[/:]";
    assert_eq!(
        InstrumentationFilterSupport::compute_instrumentation_filter(&events, &targets),
        expected_filter
    );
    assert_events_report_instrumentation_filter(&events, expected_filter);
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/InstrumentationFilterTest.java:82 InstrumentationFilterTest.testAllTestsInPackage
#[test]
fn test_all_tests_in_package() {
    let mut t = BuildViewTestCase::new();
    let events = EventCollector::with_kinds(&[EventKind::Info]);
    t.scratch.file(
        "foo/test/BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name = "t1", srcs = ["t1.sh"])"#,
            r#"foo_test(name = "t2", srcs = ["t1.sh"])"#,
        ],
    );
    let targets = get_targets(&mut t, &["//foo/test:t1", "//foo/test:t2"]);
    let expected_filter = "^//foo/test[/:]";
    assert_eq!(
        InstrumentationFilterSupport::compute_instrumentation_filter(&events, &targets),
        expected_filter
    );
    assert_events_report_instrumentation_filter(&events, expected_filter);
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/InstrumentationFilterTest.java:105 InstrumentationFilterTest.testMultiplePackages
#[test]
fn test_multiple_packages() {
    let mut t = BuildViewTestCase::new();
    let events = EventCollector::with_kinds(&[EventKind::Info]);
    t.scratch.file(
        "my/package1/BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name = "t1", srcs = ["t1.sh"])"#,
            r#"test_suite(name = "ts", tests = ["//other/package1:t1", "//other/package2:ts"])"#,
        ],
    );
    t.scratch.file(
        "other/package1/BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name='t1',srcs=['t1.sh'])"#,
        ],
    );
    t.scratch.file(
        "other/package2/BUILD",
        &[r#"test_suite(name='ts', tests=['//other/package3:t3'])"#],
    );
    t.scratch.file(
        "other/package3/BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name='t3',srcs=['t3.sh'])"#,
        ],
    );
    let targets = get_targets(&mut t, &["//my/package1:t1", "//other/package1:t1"]);
    let expected_filter = "^//my/package1[/:],^//other/package1[/:]";
    assert_eq!(
        InstrumentationFilterSupport::compute_instrumentation_filter(&events, &targets),
        expected_filter
    );
    assert_events_report_instrumentation_filter(&events, expected_filter);
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/InstrumentationFilterTest.java:140 InstrumentationFilterTest.testTestSuiteExpansion
#[test]
fn test_test_suite_expansion() {
    let mut t = BuildViewTestCase::new();
    let events = EventCollector::with_kinds(&[EventKind::Info]);
    t.scratch.file(
        "my/package1/BUILD",
        &[r#"test_suite(name='ts', tests=['//other/package1:t1', '//other/package2:ts'])"#],
    );
    t.scratch.file(
        "other/package1/BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name='t1',srcs=['t1.sh'])"#,
        ],
    );
    t.scratch.file(
        "other/package2/BUILD",
        &[r#"test_suite(name='ts', tests=['//other/package3:t3'])"#],
    );
    t.scratch.file(
        "other/package3/BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name='t3',srcs=['t3.sh'])"#,
        ],
    );
    let targets = get_targets(&mut t, &["//my/package1:ts"]);
    let expected_filter = "^//my/package1[/:],^//other/package1[/:],^//other/package2[/:]";
    assert_eq!(
        InstrumentationFilterSupport::compute_instrumentation_filter(&events, &targets),
        expected_filter
    );
    assert_events_report_instrumentation_filter(&events, expected_filter);
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/InstrumentationFilterTest.java:161 InstrumentationFilterTest.testParentAndChildPackageCombined
#[test]
fn test_parent_and_child_package_combined() {
    let mut t = BuildViewTestCase::new();
    let events = EventCollector::with_kinds(&[EventKind::Info]);
    t.scratch.file(
        "parent/BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name='t1', srcs=['t1.sh'])"#,
        ],
    );
    t.scratch.file(
        "parent/child/BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name='t2', srcs=['t2.sh'])"#,
        ],
    );
    let targets = get_targets(&mut t, &["//parent:t1", "//parent/child:t2"]);
    let expected_filter = "^//parent[/:]";
    assert_eq!(
        InstrumentationFilterSupport::compute_instrumentation_filter(&events, &targets),
        expected_filter
    );
    assert_events_report_instrumentation_filter(&events, expected_filter);
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/InstrumentationFilterTest.java:178 InstrumentationFilterTest.testJavascriptTests
#[test]
fn test_javascript_tests() {
    let mut t = BuildViewTestCase::new();
    let events = EventCollector::with_kinds(&[EventKind::Info]);
    t.scratch.file(
        "javascript/other/tests/BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name='t1',srcs=['t1.sh'])"#,
        ],
    );
    let targets = get_targets(&mut t, &["//javascript/other/tests:t1"]);
    let expected_filter = "^//javascript/other[/:]";
    assert_eq!(
        InstrumentationFilterSupport::compute_instrumentation_filter(&events, &targets),
        expected_filter
    );
    assert_events_report_instrumentation_filter(&events, expected_filter);
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/InstrumentationFilterTest.java:191 InstrumentationFilterTest.testJavaTests
#[test]
fn test_java_tests() {
    let mut t = BuildViewTestCase::new();
    let events = EventCollector::with_kinds(&[EventKind::Info]);
    t.scratch.file(
        "javatests/other/BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name='t1',srcs=['t1.sh'])"#,
        ],
    );
    let targets = get_targets(&mut t, &["//javatests/other:t1"]);
    let expected_filter = "^//java/other[/:]";
    assert_eq!(
        InstrumentationFilterSupport::compute_instrumentation_filter(&events, &targets),
        expected_filter
    );
    assert_events_report_instrumentation_filter(&events, expected_filter);
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/InstrumentationFilterTest.java:204 InstrumentationFilterTest.testInternal
#[test]
fn test_internal() {
    let mut t = BuildViewTestCase::new();
    let events = EventCollector::with_kinds(&[EventKind::Info]);
    t.scratch.file(
        "another/internal/BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name='t1',srcs=['t1.sh'])"#,
        ],
    );
    let targets = get_targets(&mut t, &["//another/internal:t1"]);
    let expected_filter = "^//another[/:]";
    assert_eq!(
        InstrumentationFilterSupport::compute_instrumentation_filter(&events, &targets),
        expected_filter
    );
    assert_events_report_instrumentation_filter(&events, expected_filter);
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/InstrumentationFilterTest.java:217 InstrumentationFilterTest.testPublic
#[test]
fn test_public() {
    let mut t = BuildViewTestCase::new();
    let events = EventCollector::with_kinds(&[EventKind::Info]);
    t.scratch.file(
        "another/public/BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name='t1',srcs=['t1.sh'])"#,
        ],
    );
    let targets = get_targets(&mut t, &["//another/public:t1"]);
    let expected_filter = "^//another[/:]";
    assert_eq!(
        InstrumentationFilterSupport::compute_instrumentation_filter(&events, &targets),
        expected_filter
    );
    assert_events_report_instrumentation_filter(&events, expected_filter);
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/InstrumentationFilterTest.java:230 InstrumentationFilterTest.testIncludesTopLevel
#[test]
fn test_includes_top_level() {
    let mut t = BuildViewTestCase::new();
    let events = EventCollector::with_kinds(&[EventKind::Info]);
    t.scratch.file(
        "BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name='t1',srcs=['t1.sh'])"#,
        ],
    );
    t.scratch.file(
        "foo/BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name='t1',srcs=['t1.sh'])"#,
        ],
    );
    let targets = get_targets(&mut t, &["//:t1", "//foo:t1"]);
    InstrumentationFilterSupport::compute_instrumentation_filter(&events, &targets);
    assert_events_report_instrumentation_filter(&events, "^//");
    let targets = get_targets(&mut t, &[]);
    let expected_filter = "^//";
    assert_eq!(
        InstrumentationFilterSupport::compute_instrumentation_filter(&events, &targets),
        expected_filter
    );
    assert_events_report_instrumentation_filter(&events, expected_filter);
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/InstrumentationFilterTest.java:250 InstrumentationFilterTest.testTopLevelOnly
#[test]
fn test_top_level_only() {
    let mut t = BuildViewTestCase::new();
    let events = EventCollector::with_kinds(&[EventKind::Info]);
    t.scratch.file(
        "BUILD",
        &[
            r#"load('//test_defs:foo_test.bzl', 'foo_test')"#,
            r#"foo_test(name='t1',srcs=['t1.sh'])"#,
        ],
    );
    let targets = get_targets(&mut t, &["//:t1"]);
    let expected_filter = "^//";
    assert_eq!(
        InstrumentationFilterSupport::compute_instrumentation_filter(&events, &targets),
        expected_filter
    );
    assert_events_report_instrumentation_filter(&events, expected_filter);
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/InstrumentationFilterTest.java:263 InstrumentationFilterTest.testNoTests
#[test]
fn test_no_tests() {
    let mut t = BuildViewTestCase::new();
    let events = EventCollector::with_kinds(&[EventKind::Info]);
    let targets = get_targets(&mut t, &[]);
    let expected_filter = "^//";
    assert_eq!(
        InstrumentationFilterSupport::compute_instrumentation_filter(&events, &targets),
        expected_filter
    );
    // If there are no targets, this doesn't get output at all.
    assert_eq!(events.count(), 0);
}
