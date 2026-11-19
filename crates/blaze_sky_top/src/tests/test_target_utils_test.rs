//! Port of the `TestFilter` tests of
//! `src/test/java/com/google/devtools/build/lib/packages/TestTargetUtilsTest.java` (a
//! `PackageLoadingTestCase`; here on the analysis test harness, whose `getTarget` loads the
//! package).
//!
//! `testSortTagsBySenseSeparatesTagsNaively` is ported next to `sortTagsBySense` in
//! blaze_packages (`target_utils.rs`).

use std::collections::BTreeSet;

use blaze_optdefs::command::loading_options::LoadingOptions;
use blaze_optdefs::common::test_size::TestSize;
use blaze_optdefs::common::test_timeout::TestTimeout;
use blaze_options::Options;
use blaze_packages::target::Target;

use crate::target_pattern_phase::{TestFilter, test_size_filter, test_timeout_filter};
use crate::testing::BuildViewTestCase;

fn create_targets(t: &mut BuildViewTestCase) -> (Target, Target, Target) {
    t.scratch.file(
        "tests/BUILD",
        &[r#"load('//test_defs:foo_binary.bzl', 'foo_binary')
load('//test_defs:foo_test.bzl', 'foo_test')
foo_test(
    name = "small_test_1",
    size = "small",
    srcs = ["small_test_1.sh"],
    data = [":xUnit"],
    tags = ["tag1"],
)

foo_test(
    name = "small_test_2",
    size = "small",
    srcs = ["small_test_2.sh"],
    data = ["//testing/shbase:googletest.sh"],
    tags = ["tag2"],
)

foo_test(
    name = "large_test_1",
    size = "large",
    srcs = ["large_test_1.sh"],
    data = [
        ":xUnit",
        "//testing/shbase:googletest.sh",
    ],
    tags = ["tag1"],
)

foo_binary(
    name = "notest",
    srcs = ["notest.sh"],
)

filegroup(name = "xUnit")

test_suite(
    name = "smallTests",
    tags = ["small"],
)
"#],
    );
    let test1 = t.get_target("//tests:small_test_1").unwrap();
    let test2 = t.get_target("//tests:small_test_2").unwrap();
    let test1b = t.get_target("//tests:large_test_1").unwrap();
    (test1, test2, test1b)
}

fn sizes(sizes: &[TestSize]) -> BTreeSet<TestSize> {
    sizes.iter().copied().collect()
}

// Java: src/test/java/com/google/devtools/build/lib/packages/TestTargetUtilsTest.java:105 TestTargetUtilsTest.testFilterBySize
#[test]
fn test_filter_by_size() {
    let mut t = BuildViewTestCase::new();
    let (test1, test2, test1b) = create_targets(&mut t);
    let allowed = sizes(&[TestSize::SMALL, TestSize::LARGE]);
    let size_filter = test_size_filter(&allowed);
    assert!(size_filter(&test1));
    assert!(size_filter(&test2));
    assert!(size_filter(&test1b));
    let allowed = sizes(&[TestSize::SMALL]);
    let size_filter = test_size_filter(&allowed);
    assert!(size_filter(&test1));
    assert!(size_filter(&test2));
    assert!(!size_filter(&test1b));
}

// Java: src/test/java/com/google/devtools/build/lib/packages/TestTargetUtilsTest.java:118 TestTargetUtilsTest.testFilterByLang
// PORT: Java mocks a Rule whose RuleClass is named "positive_test", then "negative_test"; here
// two Starlark test rules with those names stand for it.
#[test]
fn test_filter_by_lang() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "pkg/rules.bzl",
        &[
            "def _impl(ctx):",
            "    out = ctx.actions.declare_file(ctx.label.name)",
            "    ctx.actions.write(out, '', is_executable = True)",
            "    return [DefaultInfo(executable = out)]",
            "positive_test = rule(implementation = _impl, test = True)",
            "negative_test = rule(implementation = _impl, test = True)",
        ],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[
            "load(':rules.bzl', 'negative_test', 'positive_test')",
            "positive_test(name = 'a')",
            "negative_test(name = 'b')",
        ],
    );
    let mut options = Options::<LoadingOptions>::get_defaults();
    options.test_lang_filter_list = vec!["positive".to_owned(), "-negative".to_owned()];
    options.test_size_filter_set = Default::default();
    options.test_timeout_filter_set = Default::default();
    options.test_tag_filter_list = Vec::new();
    let filter = TestFilter::for_options(&options);
    assert!(filter.apply(&t.get_target("//pkg:a").unwrap()));
    assert!(!filter.apply(&t.get_target("//pkg:b").unwrap()));
}

// Java: src/test/java/com/google/devtools/build/lib/packages/TestTargetUtilsTest.java:144 TestTargetUtilsTest.testFilterByTimeout
#[test]
fn test_filter_by_timeout() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "timeouts/BUILD",
        &[r#"load('//test_defs:foo_test.bzl', 'foo_test')
foo_test(
    name = "long_timeout",
    size = "small",
    timeout = "long",
    srcs = ["a.sh"],
)

foo_test(
    name = "short_timeout",
    size = "small",
    srcs = ["b.sh"],
)

foo_test(
    name = "moderate_timeout",
    size = "small",
    timeout = "moderate",
    srcs = ["c.sh"],
)
"#],
    );
    let long_test = t.get_target("//timeouts:long_timeout").unwrap();
    let short_test = t.get_target("//timeouts:short_timeout").unwrap();
    let moderate_test = t.get_target("//timeouts:moderate_timeout").unwrap();

    let allowed: BTreeSet<TestTimeout> = [TestTimeout::SHORT, TestTimeout::LONG].into();
    let timeout_filter = test_timeout_filter(&allowed);
    assert!(timeout_filter(&long_test));
    assert!(timeout_filter(&short_test));
    assert!(!timeout_filter(&moderate_test));
}

// Java: src/test/java/com/google/devtools/build/lib/packages/TestTargetUtilsTest.java:181 TestTargetUtilsTest.testSkyframeExpandTestSuites
#[test]
#[ignore = "needs a direct evaluation of TestsForTargetPatternValue keys on the test harness's evaluator"]
fn test_skyframe_expand_test_suites() {
    unimplemented!("TestsForTargetPatternValue.key(suiteLabels) evaluated with keep_going=false")
}
