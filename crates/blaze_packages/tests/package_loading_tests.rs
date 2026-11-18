//! Ports of the package-construction JUnit tests that need BUILD-file loading
//! (`PackageLoadingTestCase` / `BuildViewTestCase` scratch workspaces). They are ignored until the
//! PACKAGE function and native rules can load a scratch BUILD file; the bodies note what the Java
//! test checks.

mod input_file_test {
    // Java: src/test/java/com/google/devtools/build/lib/packages/InputFileTest.java InputFileTest.testGetAssociatedRule
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_get_associated_rule() {
        unimplemented!("InputFileTest.testGetAssociatedRule");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/InputFileTest.java InputFileTest.testInputFileInPackageDirectory
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_input_file_in_package_directory() {
        unimplemented!("InputFileTest.testInputFileInPackageDirectory");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/InputFileTest.java InputFileTest.testInputFileInSubdirectory
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_input_file_in_subdirectory() {
        unimplemented!("InputFileTest.testInputFileInSubdirectory");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/InputFileTest.java InputFileTest.testEquivalenceRelation
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_equivalence_relation() {
        unimplemented!("InputFileTest.testEquivalenceRelation");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/InputFileTest.java InputFileTest.testReduceForSerialization
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_reduce_for_serialization() {
        unimplemented!("InputFileTest.testReduceForSerialization");
    }
}

mod output_file_test {
    // Java: src/test/java/com/google/devtools/build/lib/packages/OutputFileTest.java OutputFileTest.testGetAssociatedRule
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_get_associated_rule() {
        unimplemented!("OutputFileTest.testGetAssociatedRule");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/OutputFileTest.java OutputFileTest.testOutputFileInPackageDir
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_output_file_in_package_dir() {
        unimplemented!("OutputFileTest.testOutputFileInPackageDir");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/OutputFileTest.java OutputFileTest.testOutputFileInSubdirectory
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_output_file_in_subdirectory() {
        unimplemented!("OutputFileTest.testOutputFileInSubdirectory");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/OutputFileTest.java OutputFileTest.testEquivalenceRelation
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_equivalence_relation() {
        unimplemented!("OutputFileTest.testEquivalenceRelation");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/OutputFileTest.java OutputFileTest.testDuplicateOutputFilesInDifferentRules
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_duplicate_output_files_in_different_rules() {
        unimplemented!("OutputFileTest.testDuplicateOutputFilesInDifferentRules");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/OutputFileTest.java OutputFileTest.testOutputFileNameConflictsWithExistingRule
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_output_file_name_conflicts_with_existing_rule() {
        unimplemented!("OutputFileTest.testOutputFileNameConflictsWithExistingRule");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/OutputFileTest.java OutputFileTest.testDuplicateOutputFilesInSameRule
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_duplicate_output_files_in_same_rule() {
        unimplemented!("OutputFileTest.testDuplicateOutputFilesInSameRule");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/OutputFileTest.java OutputFileTest.testOutputFileWithIllegalName
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_output_file_with_illegal_name() {
        unimplemented!("OutputFileTest.testOutputFileWithIllegalName");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/OutputFileTest.java OutputFileTest.testOutputFileWithCrossPackageLabel
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_output_file_with_cross_package_label() {
        unimplemented!("OutputFileTest.testOutputFileWithCrossPackageLabel");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/OutputFileTest.java OutputFileTest.testOutputFileNamedBUILD
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_output_file_named_build() {
        unimplemented!("OutputFileTest.testOutputFileNamedBUILD");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/OutputFileTest.java OutputFileTest.testReduceForSerialization
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_reduce_for_serialization() {
        unimplemented!("OutputFileTest.testReduceForSerialization");
    }
}

mod rule_factory_test {
    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleFactoryTest.java RuleFactoryTest.testCreateRule
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_create_rule() {
        unimplemented!("RuleFactoryTest.testCreateRule");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleFactoryTest.java RuleFactoryTest.testOutputFileNotEqualDot
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_output_file_not_equal_dot() {
        unimplemented!("RuleFactoryTest.testOutputFileNotEqualDot");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleFactoryTest.java RuleFactoryTest.testTestRules
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_test_rules() {
        unimplemented!("RuleFactoryTest.testTestRules");
    }
}

mod rule_test {
    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleTest.java RuleTest.testOutputNameError
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_output_name_error() {
        unimplemented!("RuleTest.testOutputNameError");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleTest.java RuleTest.testIsLocalTestRuleForLocalEquals1
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_is_local_test_rule_for_local_equals1() {
        unimplemented!("RuleTest.testIsLocalTestRuleForLocalEquals1");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleTest.java RuleTest.testDeprecation
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_deprecation() {
        unimplemented!("RuleTest.testDeprecation");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleTest.java RuleTest.testVisibilityValid
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_visibility_valid() {
        unimplemented!("RuleTest.testVisibilityValid");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleTest.java RuleTest.testVisibilityTypo_failsCleanly
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_visibility_typo_fails_cleanly() {
        unimplemented!("RuleTest.testVisibilityTypo_failsCleanly");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleTest.java RuleTest.testVisibilityTypo_whenVisibilityPackageExists_failsCleanly
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_visibility_typo_when_visibility_package_exists_fails_cleanly() {
        unimplemented!("RuleTest.testVisibilityTypo_whenVisibilityPackageExists_failsCleanly");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleTest.java RuleTest.testVisibilityPkgSubpackages_whenVisibilityPackageExists_succeeds
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_visibility_pkg_subpackages_when_visibility_package_exists_succeeds() {
        unimplemented!(
            "RuleTest.testVisibilityPkgSubpackages_whenVisibilityPackageExists_succeeds"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleTest.java RuleTest.testVisibilityMisspelling
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_visibility_misspelling() {
        unimplemented!("RuleTest.testVisibilityMisspelling");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleTest.java RuleTest.testPublicAndPrivateVisibility
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_public_and_private_visibility() {
        unimplemented!("RuleTest.testPublicAndPrivateVisibility");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/RuleTest.java RuleTest.testReduceForSerialization
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_reduce_for_serialization() {
        unimplemented!("RuleTest.testReduceForSerialization");
    }
}

mod package_test {
    // Java: src/test/java/com/google/devtools/build/lib/packages/PackageTest.java PackageTest.testBuildPartialPopulatesImplicitTestSuiteIgnoresManualTests
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_build_partial_populates_implicit_test_suite_ignores_manual_tests() {
        unimplemented!("PackageTest.testBuildPartialPopulatesImplicitTestSuiteIgnoresManualTests");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/PackageTest.java PackageTest.testBuildPartialPopulatesImplicitTestSuiteValueOnlyForRequestedTags
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_build_partial_populates_implicit_test_suite_value_only_for_requested_tags() {
        unimplemented!(
            "PackageTest.testBuildPartialPopulatesImplicitTestSuiteValueOnlyForRequestedTags"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/PackageTest.java PackageTest.testBuildPartialPopulatesImplicitTestSuitesMatchingTags
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_build_partial_populates_implicit_test_suites_matching_tags() {
        unimplemented!("PackageTest.testBuildPartialPopulatesImplicitTestSuitesMatchingTags");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/PackageTest.java PackageTest.testBuildPartialPopulatesImplicitTestSuiteValueIdempotently
    #[test]
    #[ignore = "needs BUILD loading (PackageLoadingTestCase scratch workspace)"]
    fn test_build_partial_populates_implicit_test_suite_value_idempotently() {
        unimplemented!("PackageTest.testBuildPartialPopulatesImplicitTestSuiteValueIdempotently");
    }
}
