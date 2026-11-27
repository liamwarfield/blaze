//! The JUnit tests of `skyframe/packages` (AbstractPackageLoaderTest, run through
//! BazelPackageLoaderTest). The standalone package loader is a library for other tools that the
//! bazel binary does not link (see the `PORT: skipped` lines in `package_function.rs`); its only
//! end-to-end test, src/test/shell/bazel/bazel_package_loader_test.sh, runs the Java
//! `BazelPackageLoaderTester` and uses the binary under test only for `bazel info install_base`.
//! One ignored test per Java @Test method (names kept), so that the counts show them.

mod abstract_package_loader_test {
    // Java tests: src/test/java/com/google/devtools/build/lib/skyframe/packages/AbstractPackageLoaderTest.java
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn simple_no_package() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn simple_bad_package() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn simple_good_package() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn simple_multiple_good_package() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn test_good_and_bad_and_missing_packages() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn load_packages_tolerates_duplicates() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn simple_good_package_starlark() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn external_file_supported_by_default() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn external_file_assume_non_existent_and_immutable() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn test_non_package_events_reported() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn test_closes_fork_join_pool() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn loading_context_load_modules_basic_functionality() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn loading_context_load_modules_fails_on_broken_module() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn loading_context_load_modules_fails_on_cycle() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn loading_context_resets_loaded_events() {}
    #[test]
    #[ignore = "the standalone package loader (AbstractPackageLoader) is not ported"]
    fn loading_context_get_repository_mapping_basic_functionality() {}
}

mod bazel_package_loader_test {
    // Java tests: src/test/java/com/google/devtools/build/lib/skyframe/packages/BazelPackageLoaderTest.java
    #[test]
    #[ignore = "the standalone package loader (BazelPackageLoader) is not ported"]
    fn simple_local_repository_package() {}
    #[test]
    #[ignore = "the standalone package loader (BazelPackageLoader) is not ported"]
    fn build_dot_bazel_for_subpackage_check_during_globbing() {}
}
