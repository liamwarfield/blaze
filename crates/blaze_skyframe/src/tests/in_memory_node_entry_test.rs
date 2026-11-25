//! Port of `InMemoryNodeEntryTest` (src/test/java/com/google/devtools/build/skyframe/
//! InMemoryNodeEntryTest.java): the test cases shared by both entry kinds. Each is a function
//! taking the [`Kind`] under test; `incremental_in_memory_node_entry_test.rs` and
//! `non_incremental_in_memory_node_entry_test.rs` run them as `<name>_incremental` and
//! `<name>_non_incremental`.
//!
//! Java runs every case twice, with `@TestParameter boolean isPartialReevaluation`. Only
//! `false` is ported (partial reevaluation is not ported, docs/decisions.md 2026-11-19); the
//! `true` runs are ignored stubs in the two runner files.

use super::*;
use crate::in_memory_node_entry::InMemoryNodeEntry;
use crate::node_entry::{DependencyState, LifecycleState, NodeEntry};
use crate::value_with_metadata::{ValueWithMetadata, no_events};
use crate::version::Version;
use indexmap::IndexSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Java: `GraphTester.skyKey(name)`, an `AbstractSkyKey<String>` of `SkyFunctionName.FOR_TESTING`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct TestKey(pub(super) String);

impl fmt::Debug for TestKey {
    // Java: AbstractSkyKey.toString: functionName() + ":" + arg.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FOR_TESTING:{}", self.0)
    }
}

/// PORT: in Java the values decide equality (`IntegerValue.equals`, identity for
/// `new SkyValue() {}`). In Rust the key does ([`Key::values_equal`]): two `IntegerValue`s are
/// equal by value, and any other value (an [`AnonymousValue`]) is a different type, never equal,
/// which is Java's identity equality between two distinct anonymous objects.
impl Key for TestKey {
    type Value = IntegerValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("FOR_TESTING");
    // Java: SkyFunctionName.FOR_TESTING (createHermetic), src/main/java com/google/devtools/build/skyframe/SkyFunctionName.java:32
    const HERMETICITY: crate::FunctionHermeticity = crate::FunctionHermeticity::Hermetic;
    fn values_equal(old: &IntegerValue, new: &IntegerValue) -> bool {
        old == new
    }
}

// Java: InMemoryNodeEntryTest.key
pub(super) fn key(name: &str) -> SkyKey {
    SkyKey::new(TestKey(name.to_owned()))
}

// Java: InMemoryNodeEntryTest.IntegerValue
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct IntegerValue(pub(super) i32);

/// Java: `new SkyValue() {}`.
pub(super) struct AnonymousValue;

pub(super) fn integer_value(value: i32) -> SkyValue {
    Arc::new(IntegerValue(value))
}

pub(super) fn new_sky_value() -> SkyValue {
    Arc::new(AnonymousValue)
}

/// The value's `IntegerValue`, for `assertThat(v).isEqualTo(new IntegerValue(n))`.
pub(super) fn as_integer(value: Option<SkyValue>) -> Option<i32> {
    value.map(|v| v.downcast_ref::<IntegerValue>().expect("an IntegerValue").0)
}

/// The entry kind under test (Java: the concrete subclass of `InMemoryNodeEntryTest`).
pub(super) struct Kind {
    keep_edges: bool,
    pub(super) initial_version: Version,
}

impl Kind {
    // Java: IncrementalInMemoryNodeEntryTest.createEntry / getInitialVersion
    pub(super) fn incremental() -> Kind {
        Kind {
            keep_edges: true,
            initial_version: Version::Int(IntVersion::of(0)),
        }
    }

    // Java: NonIncrementalInMemoryNodeEntryTest.createEntry / getInitialVersion
    pub(super) fn non_incremental() -> Kind {
        Kind {
            keep_edges: false,
            initial_version: Version::constant(),
        }
    }

    // Java: InMemoryNodeEntryTest.createEntry (REGULAR_KEY; isPartialReevaluation is false)
    pub(super) fn create_entry(&self) -> InMemoryNodeEntry {
        InMemoryNodeEntry::new(key("regular"), self.keep_edges)
    }
}

// Java: InMemoryNodeEntryTest.setValue
pub(super) fn set_value(
    entry: &mut InMemoryNodeEntry,
    value: Option<SkyValue>,
    error_info: Option<ErrorInfo>,
    graph_version: Version,
) -> IndexSet<SkyKey> {
    entry.set_value(
        ValueWithMetadata::normal(value, error_info, no_events()),
        graph_version,
        /* max_transitive_source_version= */ None,
    )
}

/// `ErrorInfo.fromException(new ReifiedSkyFunctionException(new GenericFunctionException(new
/// SomeErrorException(message), transience)), isTransitivelyTransient)`.
pub(super) fn error_info(
    message: &str,
    transience: Transience,
    is_transitively_transient: bool,
) -> ErrorInfo {
    ErrorInfo::from_exception(
        &SkyFunctionException::new(SomeErrorException(message.to_owned()), transience),
        is_transitively_transient,
    )
}

/// `assertThat(actual).isEqualTo(expected)` for `ErrorInfo`.
/// PORT: `ErrorInfo.equals` (exception class and message, cycles, flags) has no Rust port; its
/// `toString` covers the same fields.
pub(super) fn assert_error_info_eq(actual: Option<&ErrorInfo>, expected: &ErrorInfo) {
    assert_eq!(
        format!("{:?}", actual.expect("has an error")),
        format!("{expected:?}")
    );
}

/// `assertThrows(...)`: runs `f`, which must panic, and returns the panic message.
pub(super) fn assert_throws(f: impl FnOnce()) -> String {
    let err = catch_unwind(AssertUnwindSafe(f)).expect_err("expected a panic");
    if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        (*s).to_owned()
    } else {
        String::new()
    }
}

/// `assertThrows(IllegalStateException.class, ...)`.
pub(super) fn assert_throws_illegal_state(f: impl FnOnce()) -> String {
    let message = assert_throws(f);
    assert!(
        message.starts_with("IllegalStateException"),
        "expected IllegalStateException, got: {message}"
    );
    message
}

/// Truth `containsExactly` (any order, same multiset).
pub(super) fn assert_contains_exactly<'a>(
    actual: impl IntoIterator<Item = &'a SkyKey>,
    expected: &[SkyKey],
) {
    let mut actual: Vec<String> = actual.into_iter().map(|k| format!("{k:?}")).collect();
    let mut expected: Vec<String> = expected.iter().map(|k| format!("{k:?}")).collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
}

/// `assertThatNodeEntry(entry).hasTemporaryDirectDepsThat()`: the deps, flattened.
pub(super) fn temporary_direct_deps(entry: &mut InMemoryNodeEntry) -> Vec<SkyKey> {
    entry
        .get_temporary_direct_deps()
        .get_all_elements_as_iterable()
        .cloned()
        .collect()
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:83 InMemoryNodeEntryTest.entryAtStartOfEvaluation
pub(super) fn entry_at_start_of_evaluation(kind: &Kind) {
    let mut entry = kind.create_entry();
    assert!(entry.is_dirty());
    assert!(!entry.is_done());
    assert_eq!(
        entry.get_lifecycle_state(),
        LifecycleState::NotYetEvaluating
    );
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    assert!(entry.is_dirty());
    assert!(!entry.is_done());
    assert_eq!(entry.get_lifecycle_state(), LifecycleState::NeedsRebuilding);
    assert!(entry.is_ready_to_evaluate());
    assert!(!entry.has_unsignaled_deps());
    assert!(entry.is_changed());
    assert!(entry.get_temporary_direct_deps().is_empty());
    // `getTemporaryDirectDeps() instanceof GroupedDeps.WithHashSet` is isPartialReevaluation,
    // false here: there is no WithHashSet.
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:101 InMemoryNodeEntryTest.signalEntry
pub(super) fn signal_entry(kind: &Kind) {
    let initial_version = kind.initial_version;
    let mut entry = kind.create_entry();
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    let dep1 = key("dep1");
    entry.add_singleton_temporary_direct_dep(dep1.clone());
    assert!(!entry.is_ready_to_evaluate());
    assert!(entry.has_unsignaled_deps());
    assert!(entry.signal_dep(initial_version, Some(&dep1)));
    assert!(entry.is_ready_to_evaluate());
    assert!(!entry.has_unsignaled_deps());
    assert_contains_exactly(
        &temporary_direct_deps(&mut entry),
        std::slice::from_ref(&dep1),
    );
    let dep2 = key("dep2");
    let dep3 = key("dep3");
    entry.add_singleton_temporary_direct_dep(dep2.clone());
    entry.add_singleton_temporary_direct_dep(dep3.clone());
    assert!(!entry.is_ready_to_evaluate());
    assert!(entry.has_unsignaled_deps());
    assert!(!entry.signal_dep(initial_version, Some(&dep2)));
    assert!(!entry.is_ready_to_evaluate());
    assert!(entry.has_unsignaled_deps());
    assert!(entry.signal_dep(initial_version, Some(&dep3)));
    assert!(entry.is_ready_to_evaluate());
    assert!(!entry.has_unsignaled_deps());
    assert!(
        set_value(
            &mut entry,
            Some(new_sky_value()),
            /* error_info= */ None,
            initial_version
        )
        .is_empty()
    );
    assert!(entry.is_done());
    assert_eq!(entry.get_lifecycle_state(), LifecycleState::Done);
    assert_eq!(entry.get_version(), initial_version);
    if !entry.keeps_edges() {
        return;
    }
    assert_contains_exactly(&entry.get_direct_deps(), &[dep1, dep2, dep3]);
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:156 InMemoryNodeEntryTest.reverseDeps
pub(super) fn reverse_deps(kind: &Kind) {
    let mut entry = kind.create_entry();
    let mother = key("mother");
    let father = key("father");
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(Some(mother.clone())),
        DependencyState::NeedsScheduling
    );
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(None),
        DependencyState::AlreadyEvaluating
    );
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(Some(father.clone())),
        DependencyState::AlreadyEvaluating
    );
    entry.mark_rebuilding();
    assert_contains_exactly(
        &set_value(
            &mut entry,
            Some(new_sky_value()),
            /* error_info= */ None,
            kind.initial_version,
        ),
        &[mother.clone(), father.clone()],
    );
    if !entry.keeps_edges() {
        return;
    }
    assert_contains_exactly(
        &entry.get_reverse_deps_for_done_entry(),
        &[mother.clone(), father],
    );
    assert!(entry.is_done());
    entry.remove_reverse_dep(mother.clone());
    assert!(!entry.get_reverse_deps_for_done_entry().contains(&mother));
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:179 InMemoryNodeEntryTest.errorValue
pub(super) fn error_value(kind: &Kind) {
    let mut entry = kind.create_entry();
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    let error_info = error_info("oops", Transience::Persistent, false);
    assert!(
        set_value(
            &mut entry,
            /* value= */ None,
            Some(error_info.clone()),
            kind.initial_version
        )
        .is_empty()
    );
    assert!(entry.is_done());
    assert_eq!(entry.get_lifecycle_state(), LifecycleState::Done);
    assert!(entry.get_value().is_none());
    assert!(entry.to_value().is_none());
    assert_error_info_eq(entry.get_error_info(), &error_info);
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:196 InMemoryNodeEntryTest.errorAndValue
pub(super) fn error_and_value(kind: &Kind) {
    let mut entry = kind.create_entry();
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    let error_info = error_info("oops", Transience::Persistent, false);
    set_value(
        &mut entry,
        Some(new_sky_value()),
        Some(error_info.clone()),
        kind.initial_version,
    );
    assert!(entry.is_done());
    assert_eq!(entry.get_lifecycle_state(), LifecycleState::Done);
    assert_error_info_eq(entry.get_error_info(), &error_info);
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:211 InMemoryNodeEntryTest.crashOnNullErrorAndValue
pub(super) fn crash_on_null_error_and_value(kind: &Kind) {
    let mut entry = kind.create_entry();
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    assert_throws_illegal_state(|| {
        set_value(
            &mut entry,
            /* value= */ None,
            /* error_info= */ None,
            kind.initial_version,
        );
    });
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:221 InMemoryNodeEntryTest.crashOnTooManySignals
pub(super) fn crash_on_too_many_signals(kind: &Kind) {
    let mut entry = kind.create_entry();
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    assert_throws_illegal_state(|| {
        entry.signal_dep(kind.initial_version, None);
    });
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:229 InMemoryNodeEntryTest.crashOnSetValueWhenDone
pub(super) fn crash_on_set_value_when_done(kind: &Kind) {
    let mut entry = kind.create_entry();
    entry.add_reverse_dep_and_check_if_done(None); // Start evaluation.
    entry.mark_rebuilding();
    set_value(
        &mut entry,
        Some(new_sky_value()),
        /* error_info= */ None,
        kind.initial_version,
    );
    assert!(entry.is_done());
    assert_throws_illegal_state(|| {
        set_value(
            &mut entry,
            Some(new_sky_value()),
            /* error_info= */ None,
            kind.initial_version,
        );
    });
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:241 InMemoryNodeEntryTest.crashOnAddReverseDepTwice
pub(super) fn crash_on_add_reverse_dep_twice(kind: &Kind) {
    let mut entry = kind.create_entry();
    let parent = key("parent");
    assert_eq!(
        entry.add_reverse_dep_and_check_if_done(Some(parent.clone())),
        DependencyState::NeedsScheduling
    );
    entry.add_reverse_dep_and_check_if_done(Some(parent));
    entry.mark_rebuilding();
    let e = assert_throws_illegal_state(|| {
        set_value(
            &mut entry,
            Some(new_sky_value()),
            /* error_info= */ None,
            kind.initial_version,
        );
    });
    // Java: containsMatch("[Dd]uplicate( new)? reverse deps").
    assert!(
        [
            "Duplicate reverse deps",
            "duplicate reverse deps",
            "Duplicate new reverse deps",
            "duplicate new reverse deps"
        ]
        .iter()
        .any(|p| e.contains(p)),
        "{e}"
    );
}

fn keys(names: &[&str]) -> Vec<SkyKey> {
    names.iter().map(|n| key(n)).collect()
}

fn key_set(names: &[&str]) -> IndexSet<SkyKey> {
    keys(names).into_iter().collect()
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:280 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups
pub(super) fn add_temporary_direct_deps_in_groups(kind: &Kind) {
    let mut entry = kind.create_entry();
    entry.add_reverse_dep_and_check_if_done(None);
    entry.add_temporary_direct_deps_in_groups(
        &key_set(&["1A", "2A", "2B", "3A", "3B", "3C", "4A", "4B", "4C", "4D"]),
        &[1, 2, 3, 4],
    );
    assert_eq!(
        entry.get_temporary_direct_deps().groups(),
        &[
            keys(&["1A"]),
            keys(&["2A", "2B"]),
            keys(&["3A", "3B", "3C"]),
            keys(&["4A", "4B", "4C", "4D"]),
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:298 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_toleratesEmpty
pub(super) fn add_temporary_direct_deps_in_groups_tolerates_empty(kind: &Kind) {
    let mut entry = kind.create_entry();
    entry.add_reverse_dep_and_check_if_done(None);
    entry.add_temporary_direct_deps_in_groups(&IndexSet::new(), &[]);
    assert!(entry.get_temporary_direct_deps().is_empty());
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:306 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_toleratesGroupSizeOfZero
pub(super) fn add_temporary_direct_deps_in_groups_tolerates_group_size_of_zero(kind: &Kind) {
    let mut entry = kind.create_entry();
    entry.add_reverse_dep_and_check_if_done(None);
    entry.add_temporary_direct_deps_in_groups(&key_set(&["dep"]), &[0, 1, 0]);
    assert_eq!(
        entry.get_temporary_direct_deps().groups(),
        &[keys(&["dep"])]
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:314 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_notEnoughGroups_throws
pub(super) fn add_temporary_direct_deps_in_groups_not_enough_groups_throws(kind: &Kind) {
    let mut entry = kind.create_entry();
    entry.add_reverse_dep_and_check_if_done(None);
    assert_throws(|| {
        entry.add_temporary_direct_deps_in_groups(&key_set(&["dep"]), &[]);
    });
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:324 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_tooManyGroups_throws
pub(super) fn add_temporary_direct_deps_in_groups_too_many_groups_throws(kind: &Kind) {
    let mut entry = kind.create_entry();
    assert_throws(|| {
        entry.add_temporary_direct_deps_in_groups(&IndexSet::new(), &[1]);
    });
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:332 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_depsLeftOver_throws
pub(super) fn add_temporary_direct_deps_in_groups_deps_left_over_throws(kind: &Kind) {
    let mut entry = kind.create_entry();
    entry.add_reverse_dep_and_check_if_done(None);
    assert_throws(|| {
        entry.add_temporary_direct_deps_in_groups(&key_set(&["1", "2", "3"]), &[1, 1]);
    });
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryNodeEntryTest.java:343 InMemoryNodeEntryTest.addTemporaryDirectDepsInGroups_depsExhausted_throws
pub(super) fn add_temporary_direct_deps_in_groups_deps_exhausted_throws(kind: &Kind) {
    let mut entry = kind.create_entry();
    entry.add_reverse_dep_and_check_if_done(None);
    assert_throws(|| {
        entry.add_temporary_direct_deps_in_groups(&key_set(&["1", "2", "3"]), &[1, 1, 2]);
    });
}

/// The entry is `Send`, so the graph can keep it in a `Mutex` shared by evaluator threads.
#[test]
fn in_memory_node_entry_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<InMemoryNodeEntry>();
}
