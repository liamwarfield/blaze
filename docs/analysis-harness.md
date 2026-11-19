# Analysis test harness

The Rust equivalent of Bazel's analysis test base classes lives in
`crates/blaze_sky_top/src/testing/` (feature `testing`; other crates use it as a dev-dependency
from their `tests/` directory):

| Java | Rust |
| --- | --- |
| `testutil/FoundationTestCase`, `analysis/util/BuildViewTestCase`, `AnalysisTestCase` | `build_view_test_case.rs` (`BuildViewTestCase::new()`, `BuildViewTestCase::analysis_test_case()`) |
| `analysis/util/ConfigurationTestCase` | `configuration_test_case.rs` |
| `analysis/mock/BazelAnalysisMock`, `MockPlatformSupport`, `MockGenruleSupport` | `analysis_mock.rs` |
| `packages/util/MockToolsConfig` | `mock_tools_config.rs` |
| `testutil/Scratch` | `scratch.rs` (a real temporary directory, not an `InMemoryFileSystem`) |
| `bazel/bzlmod/FakeRegistry` | `fake_registry.rs` (a local index registry on disk) |

Differences from Java are listed at the top of each file and marked `PORT:` where they apply.
Java expectations that contain the in-memory file system's absolute paths (`/workspace`,
`/usr/local/google/_blaze_jrluser/FAKEMD5`) are mapped to the test's directories with
`BuildViewTestCase::ws`.

## Tests transcribed by script

Some test classes were transcribed from the Java statement by statement with a converter (Java
string literals, `scratch.file`, `registry.addModule`, BZL_LOAD / SINGLE_EXTENSION evaluation and
the Truth assertions on them). Expectations are the Java test's, character for character; the
converter folds Java string concatenations and text blocks (dedented as `javac` does) into one
literal. Reviewers: sample these against the Java.

| Java test class | methods transcribed by script | of which run | by hand |
| --- | --- | --- | --- |
| `bazel/bzlmod/ModuleExtensionResolutionTest` | 79 | 67 | `useRepo_placeholders` (parameterized) |

`BuildConfigurationValueTest` (`crates/blaze_build_config/tests/build_configuration_value_test.rs`)
and the toolchain lookup tests were ported by hand.

### ModuleExtensionResolutionTest, per method

| Java method | Rust test | state |
| --- | --- | --- |
| `badRepoNameInExtensionImplFunction` | `bad_repo_name_in_extension_impl_function` | runs, passes |
| `complexTypedAttribute` | `complex_typed_attribute` | runs, passes |
| `complexTypedAttribute_default` | `complex_typed_attribute_default` | runs, passes |
| `extensionLoadsRepoFromAnotherExtension` | `extension_loads_repo_from_another_extension` | runs, passes |
| `extensionMetadata` | `extension_metadata` | runs, passes |
| `extensionMetadata_all` | `extension_metadata_all` | runs, passes |
| `extensionMetadata_allAndNone` | `extension_metadata_all_and_none` | runs, passes |
| `extensionMetadata_allDev` | `extension_metadata_all_dev` | runs, passes |
| `extensionMetadata_allUsedTwice` | `extension_metadata_all_used_twice` | runs, passes |
| `extensionMetadata_devUsageWithAllDirectNonDevDeps` | `extension_metadata_dev_usage_with_all_direct_non_dev_deps` | runs, passes |
| `extensionMetadata_devUsageWithDirectNonDevDeps` | `extension_metadata_dev_usage_with_direct_non_dev_deps` | runs, passes |
| `extensionMetadata_duplicateDevRepo` | `extension_metadata_duplicate_dev_repo` | runs, passes |
| `extensionMetadata_duplicateRepo` | `extension_metadata_duplicate_repo` | runs, passes |
| `extensionMetadata_duplicateRepoAcrossTypes` | `extension_metadata_duplicate_repo_across_types` | runs, passes |
| `extensionMetadata_exactlyOneArgIsNone` | `extension_metadata_exactly_one_arg_is_none` | runs, passes |
| `extensionMetadata_exactlyOneArgIsNoneDev` | `extension_metadata_exactly_one_arg_is_none_dev` | runs, passes |
| `extensionMetadata_includes` | `extension_metadata_includes` | runs, passes |
| `extensionMetadata_invalidDevRepoName` | `extension_metadata_invalid_dev_repo_name` | runs, passes |
| `extensionMetadata_invalidRepoName` | `extension_metadata_invalid_repo_name` | runs, passes |
| `extensionMetadata_isolated` | `extension_metadata_isolated` | runs, passes |
| `extensionMetadata_isolatedDev` | `extension_metadata_isolated_dev` | runs, passes |
| `extensionMetadata_noRootUsage` | `extension_metadata_no_root_usage` | runs, passes |
| `extensionMetadata_nonDevUsageWithAllDirectDevDeps` | `extension_metadata_non_dev_usage_with_all_direct_dev_deps` | runs, passes |
| `extensionMetadata_nonDevUsageWithDirectDevDeps` | `extension_metadata_non_dev_usage_with_direct_dev_deps` | runs, passes |
| `extensionMetadata_unsupportedString` | `extension_metadata_unsupported_string` | runs, passes |
| `extensionMetadata_unsupportedStringDev` | `extension_metadata_unsupported_string_dev` | runs, passes |
| `extensionRepoCtxReadsFromAnotherExtensionRepo` | `extension_repo_ctx_reads_from_another_extension_repo` | runs, passes |
| `extensionRepoMapping` | `extension_repo_mapping` | runs, passes |
| `facts_nestedTooDeeply` | `facts_nested_too_deeply` | runs, passes |
| `facts_nonStringKeys` | `facts_non_string_keys` | runs, passes |
| `facts_unsupportedType` | `facts_unsupported_type` | runs, passes |
| `generatedReposHaveCorrectMappings` | `generated_repos_have_correct_mappings` | runs, passes |
| `generatedReposHaveCorrectMappings_internalRepoWins` | `generated_repos_have_correct_mappings_internal_repo_wins` | runs, passes |
| `generatedReposHaveCorrectMappings_moduleOwnRepoName` | `generated_repos_have_correct_mappings_module_own_repo_name` | runs, passes |
| `generatedReposHaveCorrectMappings_strictDepsViolation` | `generated_repos_have_correct_mappings_strict_deps_violation` | runs, passes |
| `importNonExistentRepo` | `import_non_existent_repo` | runs, passes |
| `innate` | `innate` | runs, passes |
| `innate_invalidAttributeValue` | `innate_invalid_attribute_value` | ignored (production gap, see reason) |
| `innate_noSuchRepoRule` | `innate_no_such_repo_rule` | runs, passes |
| `innate_noSuchValue` | `innate_no_such_value` | runs, passes |
| `innate_noSuchValueIfPrivate` | `innate_no_such_value_if_private` | runs, passes |
| `innate_repoRuleDependencies` | `innate_repo_rule_dependencies` | runs, passes |
| `invalidAttributeValue` | `invalid_attribute_value` | ignored (production gap, see reason) |
| `isDevDependency_usages` | `is_dev_dependency_usages` | runs, passes |
| `labels_constructedInModuleExtensionAsString_passedOnToRepoRule` | `labels_constructed_in_module_extension_as_string_passed_on_to_repo_rule` | runs, passes |
| `labels_constructedInModuleExtension_readInModuleExtension` | `labels_constructed_in_module_extension_read_in_module_extension` | runs, passes |
| `labels_fromExtensionGeneratedRepo` | `labels_from_extension_generated_repo` | runs, passes |
| `labels_readInModuleExtension` | `labels_read_in_module_extension` | runs, passes |
| `multipleExtensions_sameName` | `multiple_extensions_same_name` | runs, passes |
| `multipleModules` | `multiple_modules` | runs, passes |
| `multipleModules_devDependency` | `multiple_modules_dev_dependency` | runs, passes |
| `multipleModules_ignoreDevDependency` | `multiple_modules_ignore_dev_dependency` | runs, passes |
| `multipleModules_isolatedUsages` | `multiple_modules_isolated_usages` | runs, passes |
| `nativeExistingRuleIsEmpty` | `native_existing_rule_is_empty` | runs, passes |
| `nonVisibleLabelInLabelAttr` | `non_visible_label_in_label_attr` | ignored (production gap, see reason) |
| `nonVisibleLabelInLabelAttrForwardedFromTag` | `non_visible_label_in_label_attr_forwarded_from_tag` | ignored (production gap, see reason) |
| `nonVisibleLabelInLabelAttrNonRootModule` | `non_visible_label_in_label_attr_non_root_module` | ignored (production gap, see reason) |
| `nonVisibleLabelInLabelKeyedStringDictAttr` | `non_visible_label_in_label_keyed_string_dict_attr` | ignored (production gap, see reason) |
| `nonVisibleLabelInLabelListAttr` | `non_visible_label_in_label_list_attr` | ignored (production gap, see reason) |
| `nonVisibleLabelInLabelListDictAttr` | `non_visible_label_in_label_list_dict_attr` | ignored (production gap, see reason) |
| `overrideRepo_inject` | `override_repo_inject` | runs, passes |
| `overrideRepo_override` | `override_repo_override` | runs, passes |
| `overrideRepo_override_onNonExistentRepoFails` | `override_repo_override_on_non_existent_repo_fails` | runs, passes |
| `printAndFailOnTag` | `print_and_fail_on_tag` | runs, passes |
| `requireRepoExtensionMetadata_allAllowsExplicitModuleExtensionMetadata` | `require_repo_extension_metadata_all_allows_explicit_module_extension_metadata` | runs, passes |
| `requireRepoExtensionMetadata_allAllowsExplicitRepoMetadata` | `require_repo_extension_metadata_all_allows_explicit_repo_metadata` | runs, passes |
| `requireRepoExtensionMetadata_allAllowsLegacyRepoMetadataDict` | `require_repo_extension_metadata_all_allows_legacy_repo_metadata_dict` | runs, passes |
| `requireRepoExtensionMetadata_allErrorsForModuleExtensionDefault` | `require_repo_extension_metadata_all_errors_for_module_extension_default` | ignored (production gap, see reason) |
| `requireRepoExtensionMetadata_allErrorsForRepoRuleDefault` | `require_repo_extension_metadata_all_errors_for_repo_rule_default` | ignored (production gap, see reason) |
| `requireRepoExtensionMetadata_defaultAllowsModuleExtensionDefault` | `require_repo_extension_metadata_default_allows_module_extension_default` | runs, passes |
| `requireRepoExtensionMetadata_rootAllowsDepDefinedModuleExtensionUsedByRootModule` | `require_repo_extension_metadata_root_allows_dep_defined_module_extension_used_by_root_module` | runs, passes |
| `requireRepoExtensionMetadata_rootAllowsDepDefinedRepoRuleUsedByRootModule` | `require_repo_extension_metadata_root_allows_dep_defined_repo_rule_used_by_root_module` | runs, passes |
| `requireRepoExtensionMetadata_rootErrorsForRootModuleExtensionDefault` | `require_repo_extension_metadata_root_errors_for_root_module_extension_default` | ignored (production gap, see reason) |
| `requireRepoExtensionMetadata_rootErrorsForRootRepoRuleDefault` | `require_repo_extension_metadata_root_errors_for_root_repo_rule_default` | ignored (production gap, see reason) |
| `simpleExtension` | `simple_extension` | runs, passes |
| `simpleExtension_nonCanonicalLabel` | `simple_extension_non_canonical_label` | runs, passes |
| `simpleExtension_nonCanonicalLabel_repoName` | `simple_extension_non_canonical_label_repo_name` | runs, passes |
| `tagSortOrder` | `tag_sort_order` | runs, passes |
| `wrongModuleExtensionLabel` | `wrong_module_extension_label` | runs, passes |
