//! Port of `ModuleExtensionResolutionTest` (src/test/java/com/google/devtools/build/lib/bazel/bzlmod).
//!
//! These are end-to-end tests (module files, .bzl evaluation, SINGLE_EXTENSION_EVAL through
//! BZL_LOAD) that need the full Skyframe setup of `BuildViewTestCase`. The pure-logic parts of
//! some of them are tested directly below the ignored tests.

use blaze_bzlmod_model::LockfileModuleExtensionMetadata;
use blaze_bzlmod_model::ModuleExtensionUsage;
use blaze_bzlmod_model::ModuleExtensionUsageProxy;
use blaze_bzlmod_model::UseAllRepos;
use blaze_events::Location;
use blaze_repo_rules::bzlmod::module_extension_metadata::generate_fixup;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;
use indexmap::IndexSet;

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn simple_extension() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn simple_extension_non_canonical_label() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn simple_extension_non_canonical_label_repo_name() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn multiple_extensions_same_name() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn multiple_modules() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn multiple_modules_dev_dependency() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn multiple_modules_ignore_dev_dependency() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn multiple_modules_isolated_usages() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn labels_read_in_module_extension() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn labels_passed_on_to_repo_rule() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn labels_from_extension_generated_repo() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn labels_constructed_in_module_extension_read_in_module_extension() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn labels_constructed_in_module_extension_as_string_passed_on_to_repo_rule() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn complex_typed_attribute() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn complex_typed_attribute_default() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn generated_repos_have_correct_mappings() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn generated_repos_have_correct_mappings_module_own_repo_name() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn generated_repos_have_correct_mappings_internal_repo_wins() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn generated_repos_have_correct_mappings_strict_deps_violation() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn wrong_module_extension_label() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn import_non_existent_repo() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn invalid_attribute_value() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn bad_repo_name_in_extension_impl_function() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn non_visible_label_in_label_attr() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn non_visible_label_in_label_attr_non_root_module() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn non_visible_label_in_label_attr_forwarded_from_tag() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn non_visible_label_in_label_list_attr() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn non_visible_label_in_label_keyed_string_dict_attr() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn non_visible_label_in_label_list_dict_attr() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn native_existing_rule_is_empty() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_loads_repo_from_another_extension() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_repo_ctx_reads_from_another_extension_repo() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn test_report_repo_and_bzl_cycles_circular_ext_repos_ctx_read() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn test_report_repo_and_bzl_cycles_circular_ext_repos_load_in_def_file() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn test_report_repo_and_bzl_cycles_ext_repo_load_self_cycle() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_exactly_one_arg_is_none() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_exactly_one_arg_is_none_dev() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_all_used_twice() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_all_and_none() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_unsupported_string() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_unsupported_string_dev() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_invalid_repo_name() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_invalid_dev_repo_name() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_duplicate_repo() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_duplicate_dev_repo() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_duplicate_repo_across_types() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_dev_usage_with_all_direct_non_dev_deps() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_non_dev_usage_with_all_direct_dev_deps() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_dev_usage_with_direct_non_dev_deps() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_non_dev_usage_with_direct_dev_deps() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_includes() {}

/// The fixup part of `extensionMetadata_all`: the warning and the buildozer commands generated
/// from the extension's metadata (`root_module_direct_deps='all', root_module_direct_dev_deps=[]`)
/// and the root module's usage. PORT: the evaluation part (the INVALID_EXTENSION_IMPORT error and
/// the event count) needs the BuildViewTestCase Skyframe setup.
#[test]
fn extension_metadata_all() {
    let proxy = |name: &str, dev: bool, line: i32, imports: &[&str]| ModuleExtensionUsageProxy {
        location: Location::new("/workspace/MODULE.bazel", line, 20),
        proxy_name: name.to_owned(),
        containing_module_file_path: PathFragment::create("MODULE.bazel"),
        dev_dependency: dev,
        imports: imports
            .iter()
            .map(|i| (i.to_string(), i.to_string()))
            .collect(),
    };
    let root_usage = ModuleExtensionUsage {
        extension_bzl_file: "@ext//:defs.bzl".to_owned(),
        extension_name: "ext".to_owned(),
        isolation_key: None,
        proxies: vec![
            proxy(
                "ext",
                false,
                3,
                &["direct_dep", "indirect_dep", "invalid_dep"],
            ),
            proxy(
                "ext_dev",
                true,
                5,
                &["direct_dev_dep", "indirect_dev_dep", "invalid_dev_dep"],
            ),
        ],
        tags: Vec::new(),
        repo_overrides: IndexMap::new(),
    };
    let metadata = LockfileModuleExtensionMetadata {
        explicit_root_module_direct_deps: None,
        explicit_root_module_direct_dev_deps: None,
        use_all_repos: UseAllRepos::Regular,
        reproducible: false,
    };
    let all_repos: IndexSet<String> = [
        "direct_dep",
        "direct_dev_dep",
        "missing_direct_dep",
        "missing_direct_dev_dep",
        "indirect_dep",
        "indirect_dev_dep",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let fixup = generate_fixup(&metadata, &root_usage, &all_repos)
        .unwrap()
        .expect("a fixup");

    assert_eq!(
        fixup.warning.get_message(),
        "The module extension ext defined in @ext//:defs.bzl reported incorrect imports of \
         repositories via use_repo():\n\
         \n\
         Imported, but not created by the extension (will cause the build to fail):\n    \
         invalid_dep, invalid_dev_dep\n\
         \n\
         Not imported, but reported as direct dependencies by the extension (may cause the build \
         to fail):\n    \
         missing_direct_dep, missing_direct_dev_dep\n\
         \n\
         Imported as a dev dependency, but reported as a regular dependency by the extension \
         (may cause the build to fail when used by other modules):\n    \
         direct_dev_dep, indirect_dev_dep\n\
         \n\
         Fix the use_repo calls by running 'bazel mod tidy'."
    );
    assert_eq!(
        fixup.warning.get_location().map(|l| l.to_string()),
        Some("/workspace/MODULE.bazel:3:20".to_owned())
    );
    let commands: Vec<(String, String)> = fixup
        .module_file_path_to_buildozer_commands
        .iter()
        .flat_map(|(path, commands)| {
            commands
                .iter()
                .map(move |c| (path.get_path_string().to_owned(), c.clone()))
        })
        .collect();
    assert_eq!(
        commands,
        vec![
            (
                "MODULE.bazel".to_owned(),
                "use_repo_add ext direct_dev_dep indirect_dev_dep missing_direct_dep \
                 missing_direct_dev_dep"
                    .to_owned()
            ),
            (
                "MODULE.bazel".to_owned(),
                "use_repo_remove ext invalid_dep".to_owned()
            ),
            (
                "MODULE.bazel".to_owned(),
                "use_repo_remove ext_dev direct_dev_dep indirect_dev_dep invalid_dev_dep"
                    .to_owned()
            ),
        ]
    );
}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_all_dev() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_no_root_usage() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_isolated() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_metadata_isolated_dev() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn facts_supported_types() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn facts_unsupported_type() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn facts_non_string_keys() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn facts_nested_too_deeply() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn require_repo_extension_metadata_default_allows_module_extension_default() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn require_repo_extension_metadata_all_errors_for_module_extension_default() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn require_repo_extension_metadata_root_errors_for_root_module_extension_default() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn require_repo_extension_metadata_root_allows_dep_defined_module_extension_used_by_root_module() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn require_repo_extension_metadata_all_allows_explicit_module_extension_metadata() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn require_repo_extension_metadata_all_errors_for_repo_rule_default() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn require_repo_extension_metadata_root_errors_for_root_repo_rule_default() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn require_repo_extension_metadata_root_allows_dep_defined_repo_rule_used_by_root_module() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn require_repo_extension_metadata_all_allows_explicit_repo_metadata() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn require_repo_extension_metadata_all_allows_legacy_repo_metadata_dict() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn is_dev_dependency_usages() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn print_and_fail_on_tag() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn tag_sort_order() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn innate() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn innate_repo_rule_dependencies() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn innate_no_such_repo_rule() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn innate_no_such_value() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn innate_no_such_value_if_private() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn innate_invalid_attribute_value() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn extension_repo_mapping() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn override_repo_override() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn override_repo_override_on_non_existent_repo_fails() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn override_repo_inject() {}

#[test]
#[ignore = "needs the BuildViewTestCase Skyframe setup (module files, BZL_LOAD, repository fetching)"]
fn use_repo_placeholders() {}
