//! Tests of the cquery output formatter callbacks that need infrastructure not ported yet
//! (every cquery output formatter callback is ported: `TransitionsOutputFormatterCallback` and
//! `StarlarkOutputFormatterCallback` have their own modules).

#[cfg(test)]
mod tests {
    // FilesOutputFormatterCallback reads the top-level aspects through
    // ConfiguredTargetQueryEnvironment::get_top_level_aspects; the callback itself is not ported.

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:206 FilesOutputFormatterCallbackTest.withAspect_customOutputGroupOnly
    #[test]
    #[ignore = "needs the BuildViewTestCase harness (blaze_sky_top::testing)"]
    fn with_aspect_custom_output_group_only() {}

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:219 FilesOutputFormatterCallbackTest.withAspect_sharedOutputGroupOnly
    #[test]
    #[ignore = "needs the BuildViewTestCase harness (blaze_sky_top::testing)"]
    fn with_aspect_shared_output_group_only() {}

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:234 FilesOutputFormatterCallbackTest.withAspects_customOutputGroupOnly
    #[test]
    #[ignore = "needs the BuildViewTestCase harness (blaze_sky_top::testing)"]
    fn with_aspects_custom_output_group_only() {}

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:253 FilesOutputFormatterCallbackTest.withAspects_sharedOutputGroupOnly
    #[test]
    #[ignore = "needs the BuildViewTestCase harness (blaze_sky_top::testing)"]
    fn with_aspects_shared_output_group_only() {}

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:270 FilesOutputFormatterCallbackTest.withAspects_noIncludeAspects
    #[test]
    #[ignore = "needs the BuildViewTestCase harness (blaze_sky_top::testing)"]
    fn with_aspects_no_include_aspects() {}
}
