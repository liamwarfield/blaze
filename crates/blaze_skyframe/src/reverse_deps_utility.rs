//! Port of `ReverseDepsUtility` and `KeyToConsolidate`.

use crate::SkyKey;
use crate::abstract_in_memory_node_entry::AbstractInMemoryNodeEntry;
use crate::incremental_in_memory_node_entry::IncrementalInMemoryNodeEntry;
use blaze_util::check_state;
use indexmap::IndexSet;
use std::collections::HashSet;
use std::fmt;

/// Java's `AbstractCollection.toString` / `Arrays.toString`: `[a, b, c]`.
pub(crate) fn java_list<T: fmt::Debug>(items: impl IntoIterator<Item = T>) -> String {
    let items: Vec<String> = items.into_iter().map(|i| format!("{i:?}")).collect();
    format!("[{}]", items.join(", "))
}

// Java: com/google/devtools/build/skyframe/KeyToConsolidate.java:35 KeyToConsolidate.Op
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Op {
    /// If the entry is re-evaluating, assert that the reverse dep is already present in the set
    /// of reverse deps and add this reverse dep to the set of reverse deps to signal when this
    /// entry is done. If the entry is already done, do nothing.
    Check,
    /// Add the reverse dep to the set of reverse deps and assert it was not already present. If
    /// the entry is re-evaluating, add this reverse dep to the set of reverse deps to signal when
    /// this entry is done.
    Add,
    /// Remove the reverse dep from the set of reverse deps and assert it was present. If the
    /// entry is re-evaluating, also remove the reverse dep from the set of reverse deps to signal
    /// when this entry is done.
    Remove,
}

impl fmt::Debug for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Op::Check => "CHECK",
            Op::Add => "ADD",
            Op::Remove => "REMOVE",
        })
    }
}

/// Container for a pending operation on the reverse deps set. We store `Op#CHECK` or `Op#ADD`
/// operations as the bare `SkyKey` in order to save the wrapper object in that case.
///
/// When a list of `KeyToConsolidate` operations is processed, each operation is performed in
/// order. Operations on a done or freshly evaluating node entry are straightforward: they apply
/// to the entry's reverse deps. Operations on a re-evaluating node entry have a double meaning:
/// they will eventually be applied to the node entry's existing reverse deps, just as for a done
/// node entry, but they are also used to track the entries that declared/redeclared a reverse
/// dep on this entry during this evaluation (and will thus need to be signaled when this entry
/// finishes evaluating).
///
/// PORT: the "bare" encoding is kept (the variant `Bare`), because its meaning is decided when
/// the operation is decoded (`op(opToStoreBare)`), not when it is stored; interning and the
/// subclass memory layout are skipped (memory format).
// Java: com/google/devtools/build/skyframe/KeyToConsolidate.java:34 KeyToConsolidate
#[derive(Clone)]
pub(crate) enum KeyToConsolidate {
    /// The operation `opToStoreBare`, stored as the bare key.
    Bare(SkyKey),
    // Java: com/google/devtools/build/skyframe/KeyToConsolidate.java:143 KeyToConsolidate.KeyToAdd
    KeyToAdd(SkyKey),
    // Java: com/google/devtools/build/skyframe/KeyToConsolidate.java:154 KeyToConsolidate.KeyToCheck
    KeyToCheck(SkyKey),
    // Java: com/google/devtools/build/skyframe/KeyToConsolidate.java:165 KeyToConsolidate.KeyToRemove
    KeyToRemove(SkyKey),
}

impl KeyToConsolidate {
    /// Gets which operation was delayed for the given object, created using `create`. The same
    /// `op_to_store_bare` passed in to `create` should be passed in here.
    // Java: com/google/devtools/build/skyframe/KeyToConsolidate.java:75 KeyToConsolidate.op
    pub(crate) fn op(&self, op_to_store_bare: Op) -> Op {
        match self {
            KeyToConsolidate::Bare(_) => op_to_store_bare,
            KeyToConsolidate::KeyToAdd(_) => Op::Add,
            KeyToConsolidate::KeyToCheck(_) => Op::Check,
            KeyToConsolidate::KeyToRemove(_) => Op::Remove,
        }
    }

    /// Gets the key whose operation was delayed for the given object.
    // Java: com/google/devtools/build/skyframe/KeyToConsolidate.java:93 KeyToConsolidate.key
    pub(crate) fn key(&self) -> &SkyKey {
        match self {
            KeyToConsolidate::Bare(key)
            | KeyToConsolidate::KeyToAdd(key)
            | KeyToConsolidate::KeyToCheck(key)
            | KeyToConsolidate::KeyToRemove(key) => key,
        }
    }

    /// Creates a new operation, encoding the operation `op` with reverse dep `key`. To save
    /// memory, the caller should specify the most common operation expected as
    /// `opToStoreBare`. That operation will be encoded as the raw `key`, saving the memory of an
    /// object wrapper. Whatever `opToStoreBare` is set to here, the same value must be passed in
    /// to `op` when decoding an operation emitted by this method.
    // Java: com/google/devtools/build/skyframe/KeyToConsolidate.java:108 KeyToConsolidate.create
    pub(crate) fn create(
        key: SkyKey,
        op: Op,
        entry: &IncrementalInMemoryNodeEntry,
    ) -> KeyToConsolidate {
        if op == ReverseDepsUtility::get_op_to_store_bare(entry) {
            return KeyToConsolidate::Bare(key);
        }
        match op {
            Op::Check => KeyToConsolidate::KeyToCheck(key),
            Op::Remove => KeyToConsolidate::KeyToRemove(key),
            Op::Add => KeyToConsolidate::KeyToAdd(key),
        }
    }
}

impl fmt::Debug for KeyToConsolidate {
    // Java: com/google/devtools/build/skyframe/KeyToConsolidate.java:67 KeyToConsolidate.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyToConsolidate::Bare(key) => write!(f, "{key:?}"),
            KeyToConsolidate::KeyToAdd(key) => write!(f, "KeyToAdd{{key={key:?}}}"),
            KeyToConsolidate::KeyToCheck(key) => write!(f, "KeyToCheck{{key={key:?}}}"),
            KeyToConsolidate::KeyToRemove(key) => write!(f, "KeyToRemove{{key={key:?}}}"),
        }
    }
}

/// A utility class that allows us to store reverse dependencies in a memory-efficient way. At
/// the same time it allows us to group the removals and uniqueness checks so that it also
/// performs well.
///
/// The operations `addReverseDep` and `removeReverseDep` here are optimized for a done entry.
/// Done entries rarely have rdeps added and removed, but do have `Op#CHECK` operations performed
/// frequently. As well, done node entries may never have their data forcibly consolidated, since
/// their reverse deps will only be retrieved as a whole if they are marked dirty. Thus, we
/// consolidate periodically.
///
/// `IncrementalInMemoryNodeEntry` manages pending reverse dep operations on a marked-dirty or
/// initially evaluating node itself, using similar logic tuned to those cases, and calls into
/// `consolidateDataAndReturnNewElements` when transitioning to done.
///
/// PORT: Java stores the reverse deps of a done entry as an empty list, a bare `SkyKey` (1
/// rdep), a `SkyKey[]` (2-4) or an `ArrayList` (5+). Here they are always a `Vec` in the same
/// order (memory format). Java's `raw instanceof SkyKey` ("single reverse dep") is exactly
/// "one element" (every writer stores one element bare), so the single-element branches test
/// `len() == 1`. The sets are `IndexSet` (Java `CompactHashSet`: insertion order, and a removal
/// moves the last element into the hole, which is `IndexSet::swap_remove`).
// Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:59 ReverseDepsUtility
pub(crate) struct ReverseDepsUtility;

impl ReverseDepsUtility {
    /// Returns the `Op` to store bare instead of wrapping in `KeyToConsolidate`.
    ///
    /// We can store one type of operation bare in order to save memory. For nodes on their
    /// initial build and nodes not keeping reverse deps, most operations are `Op#ADD`.
    ///
    /// Done nodes have very few delayed ops - `Op#CHECK` is never stored on a done node and
    /// `Op#ADD` is only delayed if there are already pending delayed ops. Returning `Op#CHECK` in
    /// this case just makes it easy to distinguish from nodes on their initial build.
    // Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:71 ReverseDepsUtility.getOpToStoreBare
    pub(crate) fn get_op_to_store_bare(entry: &IncrementalInMemoryNodeEntry) -> Op {
        let Some(dirty_building_state) = entry.dbs() else {
            return Op::Check;
        };
        if dirty_building_state.is_incremental() {
            Op::Check
        } else {
            Op::Add
        }
    }

    // Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:79 ReverseDepsUtility.maybeDelayReverseDepOp
    fn maybe_delay_reverse_dep_op(
        entry: &mut IncrementalInMemoryNodeEntry,
        reverse_dep: SkyKey,
        op: Op,
    ) {
        let current_reverse_dep_size = entry.get_reverse_deps_raw_for_reverse_deps_util().len();
        if entry
            .get_reverse_deps_data_to_consolidate_for_reverse_deps_util()
            .is_none()
        {
            entry.set_reverse_deps_data_to_consolidate_for_reverse_deps_util(Some(
                Vec::with_capacity(current_reverse_dep_size),
            ));
        }
        let key_to_consolidate = KeyToConsolidate::create(reverse_dep, op, entry);
        let consolidations = entry
            .reverse_deps_data_to_consolidate
            .as_mut()
            .expect("set above");
        consolidations.push(key_to_consolidate);
        // TODO(janakr): Should we consolidate more aggressively? This threshold can be
        // customized.
        if consolidations.len() >= current_reverse_dep_size {
            ReverseDepsUtility::consolidate_data(entry);
        }
    }

    // Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:114 ReverseDepsUtility.addReverseDep
    pub(crate) fn add_reverse_dep(
        entry: &mut IncrementalInMemoryNodeEntry,
        new_reverse_dep: SkyKey,
    ) {
        if entry
            .get_reverse_deps_data_to_consolidate_for_reverse_deps_util()
            .is_some()
        {
            ReverseDepsUtility::maybe_delay_reverse_dep_op(entry, new_reverse_dep, Op::Add);
            return;
        }
        // PORT: Java switches between bare key, array and ArrayList by size (memory format).
        entry.reverse_deps.push(new_reverse_dep);
    }

    /// See `addReverseDep` method.
    // Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:141 ReverseDepsUtility.removeReverseDep
    pub(crate) fn remove_reverse_dep(
        entry: &mut IncrementalInMemoryNodeEntry,
        reverse_dep: SkyKey,
    ) {
        ReverseDepsUtility::maybe_delay_reverse_dep_op(entry, reverse_dep, Op::Remove);
    }

    // Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:145 ReverseDepsUtility.removeReverseDepsMatching
    pub(crate) fn remove_reverse_deps_matching(
        entry: &mut IncrementalInMemoryNodeEntry,
        deleted_keys: &HashSet<SkyKey>,
    ) {
        ReverseDepsUtility::consolidate_data(entry);
        let current_reverse_deps: IndexSet<SkyKey> =
            ReverseDepsUtility::consolidate_and_get_reverse_deps(
                entry, /* check_consistency= */ true,
            )
            .into_iter()
            .collect();
        // Java: Sets.difference(currentReverseDeps, deletedKeys), in currentReverseDeps order.
        let difference: IndexSet<SkyKey> = current_reverse_deps
            .into_iter()
            .filter(|k| !deleted_keys.contains(k))
            .collect();
        ReverseDepsUtility::write_reverse_deps_set(entry, difference);
    }

    // Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:153 ReverseDepsUtility.consolidateAndGetReverseDeps
    pub(crate) fn consolidate_and_get_reverse_deps(
        entry: &mut IncrementalInMemoryNodeEntry,
        check_consistency: bool,
    ) -> Vec<SkyKey> {
        ReverseDepsUtility::consolidate_data(entry);

        // TODO(bazel-team): Unfortunately, we need to make a copy here right now to be on the
        // safe side wrt. thread-safety. The parents of a node get modified when any of the
        // parents is deleted, and we can't handle that right now.
        let raw = entry.get_reverse_deps_raw_for_reverse_deps_util();
        if raw.len() == 1 {
            return raw.clone();
        }
        let reverse_deps = raw;
        if !check_consistency {
            return reverse_deps.clone();
        }
        let set: IndexSet<SkyKey> = reverse_deps.iter().cloned().collect();
        if set.len() != reverse_deps.len() {
            let mut seen: HashSet<&SkyKey> = HashSet::new();
            let mut duplicates: HashSet<&SkyKey> = HashSet::new();
            for key in reverse_deps {
                // PORT: as in Java, this collects every first occurrence (`seen.add` returns
                // true), not the duplicates; the set is a Java HashSet, whose order this
                // crash message does not reproduce (java-hash-order).
                if seen.insert(key) {
                    duplicates.insert(key);
                }
            }
            panic!(
                "IllegalStateException: In node {:?}: duplicate reverse deps present: {}",
                entry,
                java_list(&duplicates)
            );
        }
        set.into_iter().collect()
    }

    // Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:184 ReverseDepsUtility.returnNewElements
    pub(crate) fn return_new_elements(entry: &IncrementalInMemoryNodeEntry) -> IndexSet<SkyKey> {
        ReverseDepsUtility::compute_new_elements(entry).0
    }

    /// The computation of `consolidateDataAndReturnNewElements(entry, mutateObject)`: returns
    /// the new reverse deps and the set to write if `mutateObject` (`None`: no data).
    ///
    /// PORT: split from the mutation so that the non-mutating caller can take `&self`.
    // Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:188 ReverseDepsUtility.consolidateDataAndReturnNewElements
    fn compute_new_elements(
        entry: &IncrementalInMemoryNodeEntry,
    ) -> (IndexSet<SkyKey>, Option<IndexSet<SkyKey>>) {
        let Some(data_to_consolidate) =
            entry.get_reverse_deps_data_to_consolidate_for_reverse_deps_util()
        else {
            return (IndexSet::new(), None);
        };

        // On a node's initial build (or if not keeping rdeps), we don't need to build up two
        // different sets for "all reverse deps" and "new reverse deps" - they are all new.
        let op_to_store_bare = ReverseDepsUtility::get_op_to_store_bare(entry);
        let all_rdeps_are_new = op_to_store_bare == Op::Add;
        let mut all_reverse_deps: Option<IndexSet<SkyKey>>;
        let mut new_reverse_deps: IndexSet<SkyKey>;
        let raw = entry.get_reverse_deps_raw_for_reverse_deps_util();
        if raw.len() == 1 {
            check_state!(!all_rdeps_are_new, "{:?}", entry);
            all_reverse_deps = Some(IndexSet::from([raw[0].clone()]));
            new_reverse_deps = IndexSet::new();
        } else {
            let reverse_deps_as_list = raw;
            if all_rdeps_are_new {
                check_state!(reverse_deps_as_list.is_empty(), "{:?}", entry);
                all_reverse_deps = None;
                new_reverse_deps = IndexSet::with_capacity(data_to_consolidate.len());
            } else {
                all_reverse_deps = Some(ReverseDepsUtility::get_reverse_deps_set(
                    entry,
                    reverse_deps_as_list,
                ));
                new_reverse_deps = IndexSet::new();
            }
        }

        for key_to_consolidate in data_to_consolidate {
            let key = key_to_consolidate.key();
            match key_to_consolidate.op(op_to_store_bare) {
                Op::Check => {
                    check_state!(!all_rdeps_are_new, "{:?}", entry);
                    let all = all_reverse_deps.as_ref().expect("not all new");
                    check_state!(
                        all.contains(key),
                        "Reverse dep not present: {:?} {} {} {:?}",
                        key_to_consolidate,
                        java_list(all),
                        java_list(data_to_consolidate),
                        entry
                    );
                    check_state!(
                        new_reverse_deps.insert(key.clone()),
                        "Duplicate new reverse dep: {:?} {} {} {:?}",
                        key_to_consolidate,
                        java_list(all),
                        java_list(data_to_consolidate),
                        entry
                    );
                }
                Op::Remove => {
                    if !all_rdeps_are_new {
                        let all = all_reverse_deps.as_mut().expect("not all new");
                        check_state!(
                            all.swap_remove(key),
                            "Reverse dep to be removed not present: {:?} {} {} {:?}",
                            key_to_consolidate,
                            java_list(&*all),
                            java_list(data_to_consolidate),
                            entry
                        );
                    }
                    check_state!(
                        new_reverse_deps.swap_remove(key) || !all_rdeps_are_new,
                        "Reverse dep to be removed not present: {:?} {} {} {:?}",
                        key_to_consolidate,
                        java_list(&new_reverse_deps),
                        java_list(data_to_consolidate),
                        entry
                    );
                }
                Op::Add => {
                    if !all_rdeps_are_new {
                        let all = all_reverse_deps.as_mut().expect("not all new");
                        check_state!(
                            all.insert(key.clone()),
                            "Duplicate reverse deps: {:?} {} {} {:?}",
                            key_to_consolidate,
                            java_list(raw),
                            java_list(data_to_consolidate),
                            entry
                        );
                    }
                    check_state!(
                        new_reverse_deps.insert(key.clone()),
                        "Duplicate new reverse deps: {:?} {} {} {:?}",
                        key_to_consolidate,
                        java_list(raw),
                        java_list(data_to_consolidate),
                        entry
                    );
                }
            }
        }
        let to_write = if all_rdeps_are_new {
            new_reverse_deps.clone()
        } else {
            all_reverse_deps.expect("not all new")
        };
        (new_reverse_deps, Some(to_write))
    }

    // Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:284 ReverseDepsUtility.consolidateDataAndReturnNewElements
    pub(crate) fn consolidate_data_and_return_new_elements(
        entry: &mut IncrementalInMemoryNodeEntry,
    ) -> IndexSet<SkyKey> {
        let (new_reverse_deps, to_write) = ReverseDepsUtility::compute_new_elements(entry);
        // Java: if (mutateObject) { ... } (only when there was data to consolidate).
        if let Some(to_write) = to_write {
            entry.set_reverse_deps_data_to_consolidate_for_reverse_deps_util(None);
            ReverseDepsUtility::write_reverse_deps_set(entry, to_write);
        }
        new_reverse_deps
    }

    // Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:288 ReverseDepsUtility.consolidateData
    pub(crate) fn consolidate_data(entry: &mut IncrementalInMemoryNodeEntry) {
        let Some(data_to_consolidate) = entry.reverse_deps_data_to_consolidate.take() else {
            return;
        };
        if entry.get_reverse_deps_raw_for_reverse_deps_util().len() == 1 {
            let raw = entry.get_reverse_deps_raw_for_reverse_deps_util()[0].clone();
            check_state!(
                data_to_consolidate.len() == 1,
                "dataToConsolidate not size 1 even though only one rdep: {} {:?} {:?}",
                java_list(&data_to_consolidate),
                raw,
                entry
            );
            let key_to_consolidate = &data_to_consolidate[0];
            let key = key_to_consolidate.key();
            check_state!(
                ReverseDepsUtility::get_op_to_store_bare(entry) == Op::Check,
                "{:?}",
                entry
            );
            match key_to_consolidate.op(Op::Check) {
                Op::Remove => {
                    entry.set_reverse_deps_for_reverse_deps_util(Vec::new());
                    // Fall through to check.
                    check_state!(
                        *key == raw,
                        "{:?} {:?} {:?}",
                        key_to_consolidate,
                        raw,
                        entry
                    );
                }
                Op::Check => {
                    check_state!(
                        *key == raw,
                        "{:?} {:?} {:?}",
                        key_to_consolidate,
                        raw,
                        entry
                    );
                }
                Op::Add => panic!(
                    "IllegalStateException: Shouldn't delay add if only one element: {:?}, {:?}, {:?}",
                    key_to_consolidate, raw, entry
                ),
            }
            return;
        }
        let raw = entry.get_reverse_deps_raw_for_reverse_deps_util();
        let mut reverse_deps_as_set = ReverseDepsUtility::get_reverse_deps_set(entry, raw);

        check_state!(
            ReverseDepsUtility::get_op_to_store_bare(entry) == Op::Check,
            "{:?}",
            entry
        );
        for key_to_consolidate in &data_to_consolidate {
            let key = key_to_consolidate.key();
            match key_to_consolidate.op(Op::Check) {
                Op::Check => {
                    check_state!(
                        reverse_deps_as_set.contains(key),
                        "{:?} {} {} {:?}",
                        key_to_consolidate,
                        java_list(&reverse_deps_as_set),
                        java_list(&data_to_consolidate),
                        entry
                    );
                }
                Op::Remove => {
                    check_state!(
                        reverse_deps_as_set.swap_remove(key),
                        "{:?} {} {} {:?}",
                        key_to_consolidate,
                        java_list(raw),
                        java_list(&data_to_consolidate),
                        entry
                    );
                }
                Op::Add => {
                    check_state!(
                        reverse_deps_as_set.insert(key.clone()),
                        "{:?} {} {} {:?}",
                        key_to_consolidate,
                        java_list(raw),
                        java_list(&data_to_consolidate),
                        entry
                    );
                }
            }
        }
        ReverseDepsUtility::write_reverse_deps_set(entry, reverse_deps_as_set);
    }

    // Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:362 ReverseDepsUtility.writeReverseDepsSet
    fn write_reverse_deps_set(
        entry: &mut IncrementalInMemoryNodeEntry,
        reverse_deps_as_set: IndexSet<SkyKey>,
    ) {
        if !entry.keeps_edges() || reverse_deps_as_set.is_empty() {
            entry.set_reverse_deps_for_reverse_deps_util(Vec::new());
        } else {
            // PORT: Java picks bare key / array / ArrayList by size (memory format); the order
            // is the set's in every case.
            entry.set_reverse_deps_for_reverse_deps_util(reverse_deps_as_set.into_iter().collect());
        }
    }

    // Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:375 ReverseDepsUtility.getReverseDepsSet
    fn get_reverse_deps_set(
        entry: &IncrementalInMemoryNodeEntry,
        reverse_deps_as_list: &[SkyKey],
    ) -> IndexSet<SkyKey> {
        let reverse_deps_as_set: IndexSet<SkyKey> = reverse_deps_as_list.iter().cloned().collect();
        ReverseDepsUtility::check_for_duplicates(&reverse_deps_as_set, reverse_deps_as_list, entry);
        reverse_deps_as_set
    }

    // Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:382 ReverseDepsUtility.checkForDuplicates
    pub(crate) fn check_for_duplicates(
        reverse_deps_as_set: &IndexSet<SkyKey>,
        reverse_deps_as_list: &[SkyKey],
        entry: &dyn fmt::Debug,
    ) {
        if reverse_deps_as_set.len() == reverse_deps_as_list.len() {
            return;
        }
        // We're about to crash. Try to print an informative error message.
        let mut seen: HashSet<&SkyKey> = HashSet::new();
        let mut duplicates: Vec<&SkyKey> = Vec::new();
        for key in reverse_deps_as_list {
            if !seen.insert(key) {
                duplicates.push(key);
            }
        }
        panic!(
            "IllegalStateException: {} duplicate reverse deps: {} for {:?}",
            reverse_deps_as_list.len() - reverse_deps_as_set.len(),
            java_list(duplicates),
            entry
        );
    }

    /// PORT: Java prints the raw storage object (`toString` of a bare key, of a `SkyKey[]`, which
    /// is not reproducible, or of a list); here one element prints bare and more print as a
    /// list. `singleReverseDep` is `isSingleReverseDep(entry)` in Java, a call with the entry
    /// rather than the raw object, so it is always false.
    // Java: com/google/devtools/build/skyframe/ReverseDepsUtility.java:403 ReverseDepsUtility.toString
    pub(crate) fn to_string(entry: &IncrementalInMemoryNodeEntry) -> String {
        let raw = entry.get_reverse_deps_raw_for_reverse_deps_util();
        let reverse_deps = if raw.len() == 1 {
            format!("{:?}", raw[0])
        } else {
            java_list(raw)
        };
        let data_to_consolidate =
            match entry.get_reverse_deps_data_to_consolidate_for_reverse_deps_util() {
                Some(data) => java_list(data),
                None => "null".to_owned(),
            };
        format!(
            "ReverseDeps{{reverseDeps={reverse_deps}, singleReverseDep=false, dataToConsolidate={data_to_consolidate}}}"
        )
    }
}
