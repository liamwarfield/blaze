//! The tests of Java's `ConfiguredTargetQueryTest`.
//!
//! Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryTest.java

use super::*;

#[allow(unused_imports)]
use blaze_query_engine::Setting;

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryTest.java:59 ConfiguredTargetQueryTest.testMultipleTopLevelConfigurations_nullConfigs
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
