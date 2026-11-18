//! Stubs of the package-construction JUnit tests that drive `Package.Builder` and `RuleFactory`
//! from Java, without a BUILD file. They are ignored until they are written on the builder
//! fixture of `tests/package_builder_test.rs`; the reasons say what each needs.
//!
//! InputFileTest, OutputFileTest and RuleTest load BUILD files and moved to the PACKAGE harness of
//! blaze_sky_loading (this crate cannot depend on it):
//! `crates/blaze_sky_loading/src/tests/{input_file_test, output_file_test, rule_test}.rs`.

mod rule_factory_test {
    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleFactoryTest.java RuleFactoryTest.testCreateRule
    #[test]
    #[ignore = "not ported yet: calls RuleFactory.createAndAddRule on a Package.Builder made by PackageFactory.newPackageBuilder, with attribute values from a Java map and a dummy call stack; no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply; to be written on the builder fixture of tests/package_builder_test.rs (new_builder) with the genrule class of Bazel's rule class provider"]
    fn test_create_rule() {
        unimplemented!("RuleFactoryTest.testCreateRule");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleFactoryTest.java RuleFactoryTest.testOutputFileNotEqualDot
    #[test]
    #[ignore = "not ported yet: calls RuleFactory.createAndAddRule on a Package.Builder made by PackageFactory.newPackageBuilder, with attribute values from a Java map and a dummy call stack; no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply; to be written on the builder fixture of tests/package_builder_test.rs (new_builder) with the genrule class of Bazel's rule class provider"]
    fn test_output_file_not_equal_dot() {
        unimplemented!("RuleFactoryTest.testOutputFileNotEqualDot");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleFactoryTest.java RuleFactoryTest.testTestRules
    #[test]
    #[ignore = "not ported yet: constructs a Rule directly (new Rule(pkg, label, ruleClass, location, null)) for every rule class of the provider and checks the attributes of test rules; no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply; to be written on the builder fixture of tests/package_builder_test.rs (new_builder)"]
    fn test_test_rules() {
        unimplemented!("RuleFactoryTest.testTestRules");
    }
}

mod package_test {
    // Java: src/test/java/com/google/devtools/build/lib/packages/PackageTest.java PackageTest.testBuildPartialPopulatesImplicitTestSuiteIgnoresManualTests
    #[test]
    #[ignore = "not ported yet: adds rules of a test-only test rule class (FAUX_TEST_CLASS) to a Package.Builder and calls buildPartial(); no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply; to be written on the builder fixture of tests/package_builder_test.rs (new_builder)"]
    fn test_build_partial_populates_implicit_test_suite_ignores_manual_tests() {
        unimplemented!("PackageTest.testBuildPartialPopulatesImplicitTestSuiteIgnoresManualTests");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/PackageTest.java PackageTest.testBuildPartialPopulatesImplicitTestSuiteValueOnlyForRequestedTags
    #[test]
    #[ignore = "not ported yet: adds rules of a test-only test rule class (FAUX_TEST_CLASS) to a Package.Builder and calls buildPartial(); no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply; to be written on the builder fixture of tests/package_builder_test.rs (new_builder)"]
    fn test_build_partial_populates_implicit_test_suite_value_only_for_requested_tags() {
        unimplemented!(
            "PackageTest.testBuildPartialPopulatesImplicitTestSuiteValueOnlyForRequestedTags"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/PackageTest.java PackageTest.testBuildPartialPopulatesImplicitTestSuitesMatchingTags
    #[test]
    #[ignore = "not ported yet: adds rules of a test-only test rule class (FAUX_TEST_CLASS) to a Package.Builder and calls buildPartial(); no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply; to be written on the builder fixture of tests/package_builder_test.rs (new_builder)"]
    fn test_build_partial_populates_implicit_test_suites_matching_tags() {
        unimplemented!("PackageTest.testBuildPartialPopulatesImplicitTestSuitesMatchingTags");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/PackageTest.java PackageTest.testBuildPartialPopulatesImplicitTestSuiteValueIdempotently
    #[test]
    #[ignore = "not ported yet: adds rules of a test-only test rule class (FAUX_TEST_CLASS) to a Package.Builder and calls buildPartial(); no BUILD file is loaded, so blaze_sky_loading's PACKAGE harness does not apply; to be written on the builder fixture of tests/package_builder_test.rs (new_builder)"]
    fn test_build_partial_populates_implicit_test_suite_value_idempotently() {
        unimplemented!("PackageTest.testBuildPartialPopulatesImplicitTestSuiteValueIdempotently");
    }
}
