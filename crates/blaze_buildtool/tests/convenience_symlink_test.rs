//! Port of `src/test/java/com/google/devtools/build/lib/buildtool/ConvenienceSymlinkTest.java`.
//!
//! Every case is a `BuildIntegrationTestCase` (a full in-process build with custom rules and
//! transitions); none can run until the build driver and analysis exist, so each is ignored.

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn sanity_check_files_have_null_configurations() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn sanity_check_output_directory() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn building_nothing_unsets_symlinks() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn building_only_targets_with_null_configurations_unsets_symlinks() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn building_targets_with_different_output_directories_unsets_symlinks_if_none_are_top_level() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn building_targets_with_different_output_directories_sets_symlinks_if_any_are_top_level() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn building_targets_with_same_configuration_sets_symlinks() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn building_same_configuration_targets_with_different_configuration_deps_sets_symlinks() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn different_configuration_same_output_directory_sets_symlinks() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn null_configuration_with_other_matching_output_dir_sets_symlinks() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn setting_symlinks_replaces_symlinks_already_present() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn setting_symlinks_creates_symlinks_if_not_already_present() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn genfiles_link_omitted_with_incompatible_flag() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn genfiles_link_present_without_incompatible_flag() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn setting_symlinks_does_not_replace_normal_files_already_present() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn setting_symlinks_does_not_replace_directories_already_present() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn setting_symlinks_replaces_symlinks_even_if_not_pointing_inside_exec_root() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn setting_symlinks_creates_directories_if_needed() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn setting_symlinks_does_nothing_when_parent_exists_and_is_not_a_directory() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn setting_symlinks_uses_existing_or_populated_parent_directory_as_normal() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn setting_symlinks_ignores_symlinks_with_different_prefix() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn unsetting_symlinks_removes_configuration_symlinks_if_already_present() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn unsetting_symlinks_succeeds_if_not_already_present() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn unsetting_symlinks_does_not_remove_normal_files_already_present() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn unsetting_symlinks_does_not_remove_directories_already_present() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn unsetting_symlinks_removes_symlinks_even_if_not_pointing_inside_exec_root() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn unsetting_symlinks_ignores_symlinks_with_different_prefix() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn symlink_prefix_special_no_create_value_does_not_create_or_delete_symlinks() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn convenience_symlinks_ignore_leave_symlinks_untouched() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn convenience_symlinks_normal_create_symlinks() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (in-process build driver)"]
fn convenience_symlinks_clean_deletes_and_does_not_create_symlinks() {}
