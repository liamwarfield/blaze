//! Port of `src/test/java/com/google/devtools/build/lib/buildtool/InstrumentationFilterSupportTest.java`.

use blaze_events::{EventCollector, EventKind};

use crate::instrumentation_filter_support::InstrumentationFilterSupport;
use crate::testing::BuildViewTestCase;

// Java: src/test/java/com/google/devtools/build/lib/buildtool/InstrumentationFilterSupportTest.java:35 InstrumentationFilterSupportTest.testComputeInstrumentationFilter
#[test]
fn test_compute_instrumentation_filter() {
    let mut t = BuildViewTestCase::new();
    let events = EventCollector::with_kinds(&[EventKind::Info]);
    t.scratch
        .file("foo/BUILD", &["filegroup(name='t', srcs=['t.sh'])"]);
    t.scratch
        .file("foobar/BUILD", &["filegroup(name='t', srcs=['t.sh'])"]);
    let targets = vec![
        t.get_target("//foo:t").unwrap(),
        t.get_target("//foobar:t").unwrap(),
    ];
    let expected_filter = "^//foo[/:],^//foobar[/:]";
    assert_eq!(
        InstrumentationFilterSupport::compute_instrumentation_filter(&events, &targets),
        expected_filter
    );
}
