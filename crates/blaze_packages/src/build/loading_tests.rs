//! Stubs of the BUILD-loading JUnit tests of `packages/` that are not ported yet.
//!
//! The tests of `PackageFactoryTest`, `ExportsFilesTest`, `NativeGlobTest`, `SelectTest`,
//! `NativeExistingRulesTest` and `NativeSubpackagesTest` that load BUILD files run on the PACKAGE
//! harness of blaze_sky_loading (this crate cannot depend on it): see
//! `crates/blaze_sky_loading/src/tests/{package_factory_test, exports_files_test,
//! native_glob_test, select_test, native_existing_rules_test, native_subpackages_test}.rs`. What
//! stays here is ignored, never deleted, with the reason each one cannot run yet.

// Java: src/test/java/com/google/devtools/build/lib/packages/PackageFactoryTest.java
// The other tests of this class, the symbolic macro and declaration visibility tests included,
// moved to crates/blaze_sky_loading/src/tests/package_factory_test.rs.
mod package_factory_test {
    #[test]
    #[ignore = "asserts through a GlobCache built in the test on the package directory (Java new GlobCache(...).globUnsorted(include, exclude, FILES_AND_DIRS, true)); a unit test of build/glob_cache.rs, not of package loading"]
    fn test_recursive_glob() {}
































    #[test]
    #[ignore = "unit test of PackageFactory.checkBuildSyntax's glob pattern extraction on a parsed file; the port is blaze_starlark::syntax (dot_bazel.rs) and its test belongs to that crate"]
    fn test_glob_pattern_extractor() {}
}

// Java: src/test/java/com/google/devtools/build/lib/packages/ExportsFilesTest.java
// Moved to crates/blaze_sky_loading/src/tests/exports_files_test.rs.

// Java: src/test/java/com/google/devtools/build/lib/packages/NativeGlobTest.java
// Moved to crates/blaze_sky_loading/src/tests/native_glob_test.rs.

// Java: src/test/java/com/google/devtools/build/lib/packages/SelectTest.java
// testPlusIncompatibleType, testUnionIncompatibleType and testRepr moved to
// crates/blaze_sky_loading/src/tests/select_test.rs.
mod select_test {
    // Java: SelectTest.testSelect
    #[test]
    #[ignore = "asserts on the SelectorList value of a directly evaluated expression (getElements(), SelectorValue.getDictionary()); needs a Starlark evaluation helper with select() in this crate's tests (the PACKAGE harness of blaze_sky_loading observes only events and targets)"]
    fn test_select() {}

    // Java: SelectTest.testPlus
    #[test]
    #[ignore = "asserts on the SelectorList value of a directly evaluated expression (getElements()); needs a Starlark evaluation helper with select() in this crate's tests (the PACKAGE harness of blaze_sky_loading observes only events and targets)"]
    fn test_plus() {}

    // Java: SelectTest.testKeyResolution
    #[test]
    #[ignore = "asserts on SelectorValue.getDictionary() of an expression evaluated with a BazelModuleContext of //other/pkg:def.bzl and a repository mapping (other_repo -> other_repo+), with --incompatible_resolve_select_keys_eagerly on and off; needs a Starlark evaluation helper with select() and a module context in this crate's tests"]
    fn test_key_resolution() {}
}

// Java: src/test/java/com/google/devtools/build/lib/packages/NativeExistingRulesTest.java
// Moved to crates/blaze_sky_loading/src/tests/native_existing_rules_test.rs.

// Java: src/test/java/com/google/devtools/build/lib/packages/NativeSubpackagesTest.java
// Moved to crates/blaze_sky_loading/src/tests/native_subpackages_test.rs.
