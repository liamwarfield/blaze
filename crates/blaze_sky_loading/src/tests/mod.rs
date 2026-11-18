//! Ports of the Java tests of the classes in this crate.

mod bzl_compile_function_test;
mod bzl_load_function_test;
mod containing_package_lookup_function_test;
mod file_function_test;
mod ignored_subdirectories_test;
mod oracle_repos_test;
mod package_function_test;
mod package_lookup_function_test;
mod repo_file_function_test;
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
