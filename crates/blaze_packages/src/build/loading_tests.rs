//! Stubs of the BUILD-loading JUnit tests of `packages/` that are not ported yet.
//!
//! The tests of `PackageFactoryTest`, `ExportsFilesTest`, `NativeGlobTest`, `SelectTest`,
//! `NativeExistingRulesTest` and `NativeSubpackagesTest` that load BUILD files run on the PACKAGE
//! harness of blaze_sky_loading (this crate cannot depend on it): see
//! `crates/blaze_sky_loading/src/tests/{package_factory_test, exports_files_test,
//! native_glob_test, select_test, native_existing_rules_test, native_subpackages_test}.rs`. What
//! stays here is ignored, never deleted, with the reason each one cannot run yet.

// Java: src/test/java/com/google/devtools/build/lib/packages/PackageFactoryTest.java
// The other tests of this class moved to crates/blaze_sky_loading/src/tests/package_factory_test.rs.
mod package_factory_test {
    #[test]
    #[ignore = "asserts through a GlobCache built in the test on the package directory (Java new GlobCache(...).globUnsorted(include, exclude, FILES_AND_DIRS, true)); a unit test of build/glob_cache.rs, not of package loading"]
    fn test_recursive_glob() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_duplicate_macro_names_disallowed() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_macro_and_rule_clash_macro_declared_first() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_macro_and_rule_clash_rule_declared_first() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_macro_and_output_clash_macro_declared_first() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_macro_and_output_clash_output_declared_first() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_macro_may_collide_with_prefix_of_output() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_macro_and_environment_group_clash_macro_declared_first() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_macro_and_environment_group_clash_environment_group_declared_first() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_macro_and_package_group_clash_macro_declared_first() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_macro_and_package_group_clash_package_group_declared_first() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_macro_and_input_clash_macro_declared_first() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_macro_and_input_clash_input_declared_first() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_implicitly_created_input_is_created_even_inside_macro_namespace() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_implicitly_created_input_is_not_created_if_macro_declares_target() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_implicitly_created_input_is_not_created_if_macro_name_matches_exactly() {
    }

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_implicitly_created_input_is_created_by_usage_in_macro_attr() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_implicitly_created_input_is_not_created_by_usage_in_macro_body() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_deferred_evaluation_expands_transitively() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_recursion_prohibited_with_eager_evaluation() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_recursion_prohibited_with_deferred_evaluation() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_symbolic_macro_indirect_recursion_also_prohibited() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_declaration_visibility_unioning_occurs_both_inside_and_outside_macros() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_declaration_visibility_unioning_uses_innermost_macro_location() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_declaration_visibility_unioning_does_not_apply_package_default_visibility() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_implicit_visibility_works_with_package_default_visibility() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_package_default_visibility_plays_well_with_private_visibility() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_package_default_visibility_succeeds_if_no_default_visibility_set() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_declaration_visibility_unioning_works_with_public_private_and_duplicate_visibilities() {
    }

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_declaration_visibility_unioning_applies_to_exports_files() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_declaration_visibility_unioning_has_no_effect_on_package_groups() {}

    #[test]
    #[ignore = "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!); port onto blaze_sky_loading's PackageLoadingTestCase (src/tests/package_factory_test.rs) when they do"]
    fn test_declaration_visibility_unioning_fails_gracefully_on_invalid_visibility() {}

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
