//! Ports of the Java tests of the classes in this crate.

mod bzl_compile_function_test;
mod bzl_load_function_test;
mod containing_package_lookup_function_test;
mod environment_group_test;
mod exports_files_test;
mod file_function_test;
mod ignored_subdirectories_test;
mod input_file_test;
mod native_existing_rules_test;
mod native_glob_test;
mod native_subpackages_test;
mod oracle_repos_test;
mod output_file_test;
mod package_factory_test;
mod package_function_test;
mod package_group_test;
mod package_loading_test_case;
mod package_lookup_function_test;
mod repo_file_function_test;
mod review_2026_11_18_test;
mod review_s2_builtins_test;
mod resolve_names_test;
mod review_s2_natives_test;
mod rule_class_loading_test;
mod rule_test;
mod select_test;
mod starlark_defined_aspects_loading_test;
mod starlark_rule_class_functions_test;
mod target_pattern_function_test;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Guava `EqualsTester`: values in a group are equal and hash alike; values of different groups
/// are not equal.
pub(crate) fn equals_tester<T: PartialEq + Hash + std::fmt::Debug>(groups: &[Vec<T>]) {
    fn hash_of<T: Hash>(t: &T) -> u64 {
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        h.finish()
    }
    for (i, group) in groups.iter().enumerate() {
        for a in group {
            for b in group {
                assert_eq!(a, b, "group {}", i);
                assert_eq!(hash_of(a), hash_of(b), "hash in group {}", i);
            }
            for (j, other) in groups.iter().enumerate() {
                if i == j {
                    continue;
                }
                for b in other {
                    assert_ne!(a, b, "groups {} and {}", i, j);
                }
            }
        }
    }
}
