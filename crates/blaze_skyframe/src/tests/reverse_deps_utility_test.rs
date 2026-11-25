//! Port of `ReverseDepsUtilityTest`
//! (src/test/java/com/google/devtools/build/skyframe/ReverseDepsUtilityTest.java).
//!
//! Java runs each case for `numElements` 0 to 19 (`@Parameters`); each Rust test loops over the
//! same values.

use super::in_memory_node_entry_test::assert_throws_illegal_state;
use super::*;
use crate::incremental_in_memory_node_entry::IncrementalInMemoryNodeEntry;
use crate::node_entry::DependencyState;
use crate::reverse_deps_utility::ReverseDepsUtility;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Java: `ReverseDepsUtilityTest.Key`, an interned `AbstractSkyKey<Integer>` of
/// `SkyFunctionName.FOR_TESTING`.
#[derive(Clone, PartialEq, Eq, Hash)]
struct IntKey(i32);

impl fmt::Debug for IntKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FOR_TESTING:{}", self.0)
    }
}

impl Key for IntKey {
    type Value = ();
    const FUNCTION: SkyFunctionName = SkyFunctionName("FOR_TESTING");
    // Java: SkyFunctionName.FOR_TESTING (createHermetic), src/main/java com/google/devtools/build/skyframe/SkyFunctionName.java:32
    const HERMETICITY: crate::FunctionHermeticity = crate::FunctionHermeticity::Hermetic;
}

// Java: ReverseDepsUtilityTest.Key.create
fn int_key(arg: i32) -> SkyKey {
    SkyKey::new(IntKey(arg))
}

// Java: ReverseDepsUtilityTest.KEY
fn new_example() -> IncrementalInMemoryNodeEntry {
    IncrementalInMemoryNodeEntry::new(SkyKey::new(IntKey(-1)))
}

// Java: ReverseDepsUtilityTest.parameters
const NUM_ELEMENTS: std::ops::Range<i32> = 0..20;

// Java: src/test/java/com/google/devtools/build/skyframe/ReverseDepsUtilityTest.java:51 ReverseDepsUtilityTest.testAddAndRemove
#[test]
fn test_add_and_remove() {
    for num_elements in NUM_ELEMENTS {
        for num_removals in 0..=num_elements {
            let mut example = new_example();
            for j in 0..num_elements {
                ReverseDepsUtility::add_reverse_dep(&mut example, int_key(j));
            }
            // Not a big test but at least check that it does not blow up.
            assert!(!ReverseDepsUtility::to_string(&example).is_empty());
            assert_eq!(
                ReverseDepsUtility::consolidate_and_get_reverse_deps(
                    &mut example,
                    /* check_consistency= */ true
                )
                .len(),
                num_elements as usize
            );
            for i in 0..num_removals {
                ReverseDepsUtility::remove_reverse_dep(&mut example, int_key(i));
            }
            assert_eq!(
                ReverseDepsUtility::consolidate_and_get_reverse_deps(
                    &mut example,
                    /* check_consistency= */ true
                )
                .len(),
                (num_elements - num_removals) as usize
            );
            assert!(
                example
                    .get_reverse_deps_data_to_consolidate_for_reverse_deps_util()
                    .is_none()
            );
        }
    }
}

// Same as testAdditionAndRemoval but we add all the reverse deps in one call.
// Java: src/test/java/com/google/devtools/build/skyframe/ReverseDepsUtilityTest.java:76 ReverseDepsUtilityTest.testAddAllAndRemove
#[test]
fn test_add_all_and_remove() {
    for num_elements in NUM_ELEMENTS {
        for num_removals in 0..=num_elements {
            let mut example = new_example();
            for j in 0..num_elements {
                ReverseDepsUtility::add_reverse_dep(&mut example, int_key(j));
            }
            assert_eq!(
                ReverseDepsUtility::consolidate_and_get_reverse_deps(
                    &mut example,
                    /* check_consistency= */ true
                )
                .len(),
                num_elements as usize
            );
            for i in 0..num_removals {
                ReverseDepsUtility::remove_reverse_dep(&mut example, int_key(i));
            }
            assert_eq!(
                ReverseDepsUtility::consolidate_and_get_reverse_deps(
                    &mut example,
                    /* check_consistency= */ true
                )
                .len(),
                (num_elements - num_removals) as usize
            );
            assert!(
                example
                    .get_reverse_deps_data_to_consolidate_for_reverse_deps_util()
                    .is_none()
            );
        }
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/ReverseDepsUtilityTest.java:98 ReverseDepsUtilityTest.testDuplicateCheckOnGetReverseDeps
#[test]
fn test_duplicate_check_on_get_reverse_deps() {
    for num_elements in NUM_ELEMENTS {
        let mut example = new_example();
        for i in 0..num_elements {
            ReverseDepsUtility::add_reverse_dep(&mut example, int_key(i));
        }
        // Should only fail when we call getReverseDeps().
        ReverseDepsUtility::add_reverse_dep(&mut example, int_key(0));
        if num_elements == 0 {
            // Will not throw.
            assert_eq!(
                ReverseDepsUtility::consolidate_and_get_reverse_deps(
                    &mut example,
                    /* check_consistency= */ true
                )
                .len(),
                1
            );
        } else {
            // Java: assertThrows(RuntimeException.class, ...).
            assert!(
                catch_unwind(AssertUnwindSafe(|| {
                    ReverseDepsUtility::consolidate_and_get_reverse_deps(
                        &mut example,
                        /* check_consistency= */ true,
                    );
                }))
                .is_err()
            );
        }
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/ReverseDepsUtilityTest.java:121 ReverseDepsUtilityTest.duplicateAddNoThrowWithoutCheck
#[test]
fn duplicate_add_no_throw_without_check() {
    for num_elements in NUM_ELEMENTS {
        let mut example = new_example();
        for i in 0..num_elements {
            ReverseDepsUtility::add_reverse_dep(&mut example, int_key(i));
        }
        ReverseDepsUtility::add_reverse_dep(&mut example, int_key(0));
        assert_eq!(
            ReverseDepsUtility::consolidate_and_get_reverse_deps(
                &mut example,
                /* check_consistency= */ false
            )
            .len(),
            (num_elements + 1) as usize
        );
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/ReverseDepsUtilityTest.java:133 ReverseDepsUtilityTest.doubleAddThenRemove
#[test]
fn double_add_then_remove() {
    for _num_elements in NUM_ELEMENTS {
        let mut example = new_example();
        let key = int_key(0);
        ReverseDepsUtility::add_reverse_dep(&mut example, key.clone());
        // Should only fail when we call getReverseDeps().
        ReverseDepsUtility::add_reverse_dep(&mut example, key.clone());
        ReverseDepsUtility::remove_reverse_dep(&mut example, key);
        assert_throws_illegal_state(|| {
            ReverseDepsUtility::consolidate_and_get_reverse_deps(
                &mut example,
                /* check_consistency= */ true,
            );
        });
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/ReverseDepsUtilityTest.java:147 ReverseDepsUtilityTest.doubleAddThenRemoveCheckedOnSize
#[test]
fn double_add_then_remove_checked_on_size() {
    for _num_elements in NUM_ELEMENTS {
        let mut example = new_example();
        let fixed_key = int_key(0);
        ReverseDepsUtility::add_reverse_dep(&mut example, fixed_key.clone());
        let key = int_key(1);
        ReverseDepsUtility::add_reverse_dep(&mut example, key.clone());
        // Should only fail when we reach the limit.
        ReverseDepsUtility::add_reverse_dep(&mut example, key.clone());
        example.add_reverse_dep_and_check_if_done(None);
        assert_eq!(
            example.check_if_done_for_dirty_reverse_dep(fixed_key),
            DependencyState::AlreadyEvaluating
        );
        assert_eq!(
            example.check_if_done_for_dirty_reverse_dep(key.clone()),
            DependencyState::AlreadyEvaluating
        );
        let e = assert_throws_illegal_state(|| {
            ReverseDepsUtility::remove_reverse_dep(&mut example, key);
        });
        assert!(e.contains("1 duplicate"), "{e}");
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/ReverseDepsUtilityTest.java:167 ReverseDepsUtilityTest.addRemoveAdd
#[test]
fn add_remove_add() {
    for _num_elements in NUM_ELEMENTS {
        let mut example = new_example();
        let fixed_key = int_key(0);
        ReverseDepsUtility::add_reverse_dep(&mut example, fixed_key.clone());
        let key = int_key(1);
        ReverseDepsUtility::add_reverse_dep(&mut example, key.clone());
        ReverseDepsUtility::remove_reverse_dep(&mut example, key.clone());
        ReverseDepsUtility::add_reverse_dep(&mut example, key.clone());
        let reverse_deps = ReverseDepsUtility::consolidate_and_get_reverse_deps(
            &mut example,
            /* check_consistency= */ true,
        );
        super::in_memory_node_entry_test::assert_contains_exactly(&reverse_deps, &[fixed_key, key]);
    }
}
