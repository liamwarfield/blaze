//! Rust `NestedSet` <-> Starlark `depset` conversion (crates/blaze_analysis/DESIGN.md
//! "Depsets").
//!
//! Java wraps the same `NestedSet` object in a `Depset` (`Depset.of(Artifact.class, set)`), so a
//! depset seen from Starlark has the native set's structure, order and identity. Blaze's depset
//! is a Starlark value mirroring the NestedSet node structure (`blaze_starlark::depset`), so the
//! conversion rebuilds the same DAG node for node:
//! - Rust -> Starlark ([`nested_set_to_depset`]): each NestedSet node becomes one frozen depset
//!   node whose children array has the same entries in the same order (leaves converted with the
//!   element converter, sub-arrays converted recursively). Nodes are memoized by node identity
//!   for the life of the build arena, so converting a set twice yields the same depset value
//!   (Java: the same Depset over the same NestedSet), and shared sub-DAGs stay shared.
//! - Starlark -> Rust ([`depset_to_nested_set_shared`]): `DepsetGen::to_nested_set` (same
//!   structure), memoized by the identity of the depset value, because Java's `Depset.getSet`
//!   returns the one wrapped `NestedSet` object every time and that identity reaches output
//!   (`ActionGraphDump`'s `KnownNestedSets` numbers sets by node identity). A depset made by
//!   [`nested_set_to_depset`] converts back to the very set it was made from.
//!
//! PORT: memo scope. Frozen depsets (providers of other targets, .bzl globals, converted native
//! sets) are memoized for the life of the build arena (`blaze_util::build_arena`, decision D7:
//! released by the server once a command has ended and no evaluator is left; elsewhere it lives
//! for the process); their heaps live as long ([`KeptHeaps`]). Depsets of the rule implementation being evaluated are not frozen yet and their
//! addresses die with the module's heap, so they are memoized inside a
//! [`DepsetConversionScope`] (one per rule evaluation).
//!
//! PORT: freezing. Java's `Depset` is the same object before and after its `Mutability` is
//! frozen, so the set an action got from it while the implementation ran is the set a dependent
//! target gets from the provider later. starlark-rust freezes by copying: the frozen depset has
//! another address. The scope therefore records, for every unfrozen depset it converts, the
//! order in which the conversion visited the depset's nodes; once the module is frozen
//! ([`DepsetConversionScope::adopt_frozen`]) the same walk over the frozen copy pairs every
//! frozen node with the unfrozen node it was copied from, and the frozen node is memoized with
//! that node's set.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::analysis::starlark::starlark_file::{StarlarkFile, artifact_from_value};
use blaze_collect::nestedset::{NestedSetChild, NestedSetChildren};
use blaze_collect::{NestedSet, Order};
use blaze_starlark::depset::{Depset, DepsetChild, DepsetChildren, DepsetNodeMemo, FrozenDepset};
use parking_lot::Mutex;
use starlark::values::{FrozenHeap, FrozenValue, OwnedFrozenValue, Value};

/// One memoized conversion: the source node (kept alive so its identity is not reused) and the
/// frozen depset.
struct MemoEntry {
    _keep_alive: Box<dyn std::any::Any + Send + Sync>,
    depset: OwnedFrozenValue,
}

/// A `NestedSet<E>` of any element type.
type AnySet = Arc<dyn Any + Send + Sync>;

/// The `NestedSet<E>` in `set`.
fn downcast_set<E>(set: &AnySet) -> NestedSet<E>
where
    E: Clone + Eq + Hash + Send + Sync + 'static,
{
    (**set)
        .downcast_ref::<NestedSet<E>>()
        .expect("keyed by element type")
        .clone()
}

/// The memos of this module, in the build arena (PORT: memo scope, see the module comment).
/// Released before [`KeptHeaps`], whose heaps the addresses of `frozen_to_set` point into.
#[derive(Default)]
struct DepsetMemos {
    /// Memo of converted nodes, keyed by (element type, node identity).
    memo: Mutex<HashMap<(TypeId, usize), MemoEntry>>,
    /// Frozen depset identity -> the `NestedSet<E>` it stands for.
    frozen_to_set: Mutex<HashMap<(TypeId, usize), AnySet>>,
}

impl blaze_util::build_arena::ArenaSlot for DepsetMemos {
    const RELEASE_ORDER: blaze_util::build_arena::ReleaseOrder =
        blaze_util::build_arena::ReleaseOrder::Memo;
}

fn depset_memos() -> Arc<DepsetMemos> {
    blaze_util::build_arena::slot::<DepsetMemos>()
}

/// One node visited by the conversion of an unfrozen depset, in the order of the memo lookups
/// of `DepsetGen::to_nested_set_with_memo`.
#[derive(Clone, Copy)]
enum Visit {
    /// A frozen node. Freezing keeps it, so the walk over the frozen copy finds the same node
    /// here; nothing below it is recorded.
    Frozen,
    /// The unfrozen node at address `node`. `hit`: its set existed already, so the conversion
    /// did not look below it.
    Unfrozen { node: usize, hit: bool },
}

/// The visits of the conversion in progress.
#[derive(Default)]
struct Recording {
    visits: Vec<Visit>,
    /// Number of frozen nodes being converted (a frozen node only has frozen nodes below it).
    frozen_depth: usize,
}

/// An unfrozen depset whose conversion made new sets.
struct ConvertedRoot {
    /// The depset value. PORT: its lifetime (the evaluated module's heap) is erased to store it
    /// in the scope; [`DepsetConversionScope::converted_depsets`] gives it back.
    value: Value<'static>,
    /// The element type it was converted to.
    element: TypeId,
    visits: Vec<Visit>,
}

/// The state of one open [`DepsetConversionScope`].
#[derive(Default)]
struct ScopeState {
    /// Unfrozen depset identity -> `NestedSet<E>`.
    sets: HashMap<(TypeId, usize), AnySet>,
    roots: Vec<ConvertedRoot>,
    recording: Option<Recording>,
}

thread_local! {
    /// The open [`DepsetConversionScope`]s of this thread, innermost last.
    static SCOPES: RefCell<Vec<ScopeState>> = const { RefCell::new(Vec::new()) };
}

/// Frozen heaps whose depset addresses may be keys of [`DepsetMemos::frozen_to_set`], in the
/// build arena (released after the memos).
#[derive(Default)]
struct KeptHeaps(Mutex<Vec<starlark::values::FrozenHeapRef>>);

impl blaze_util::build_arena::ArenaSlot for KeptHeaps {
    const RELEASE_ORDER: blaze_util::build_arena::ReleaseOrder =
        blaze_util::build_arena::ReleaseOrder::Heaps;

    fn held_count(&self) -> usize {
        self.0.lock().len()
    }
}

/// Keeps `heap` for the life of the build arena, so the addresses of its depsets stay unique
/// while they are keys of the memo.
pub fn keep_frozen_heap_alive(heap: starlark::values::FrozenHeapRef) {
    blaze_util::build_arena::slot::<KeptHeaps>()
        .0
        .lock()
        .push(heap);
}

/// The lifetime of one Starlark module's unfrozen depsets (one rule implementation evaluation):
/// while it is open, converting the same unfrozen depset value twice yields the same
/// `NestedSet` node, as Java's `Depset.getSet` does. When the module is frozen,
/// [`DepsetConversionScope::converted_depsets`] and [`DepsetConversionScope::adopt_frozen`]
/// carry the sets over to the frozen depsets.
pub struct DepsetConversionScope(());

impl DepsetConversionScope {
    pub fn enter() -> DepsetConversionScope {
        SCOPES.with(|s| s.borrow_mut().push(ScopeState::default()));
        DepsetConversionScope(())
    }

    /// The unfrozen depsets whose conversion in this scope made new sets, in conversion order.
    /// The caller keeps them reachable while the module is frozen and passes their frozen
    /// copies to [`DepsetConversionScope::adopt_frozen`].
    ///
    /// # Safety
    ///
    /// `'v` must be the lifetime of the heap the depsets converted in this scope live on, and
    /// that heap must not have been garbage collected since the scope was entered.
    pub unsafe fn converted_depsets<'v>(&self) -> Vec<Value<'v>> {
        SCOPES.with(|s| {
            s.borrow()
                .last()
                .map(|scope| {
                    scope
                        .roots
                        .iter()
                        .map(|root| {
                            // SAFETY: the caller's contract; the value was stored from a
                            // `Value<'v>` of that heap.
                            unsafe { std::mem::transmute::<Value<'static>, Value<'v>>(root.value) }
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    /// Makes every frozen depset stand for the set its unfrozen original was converted to in
    /// this scope. `frozen[i]` is the frozen copy of `converted_depsets()[i]`.
    // Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:207 Depset.getSet
    // (the Depset object and its NestedSet are the same before and after freezing).
    pub fn adopt_frozen(&self, frozen: &[OwnedFrozenValue]) {
        let adopted = SCOPES.with(|s| {
            let s = s.borrow();
            let mut adopted: Vec<((TypeId, usize), AnySet)> = Vec::new();
            let Some(scope) = s.last() else {
                return adopted;
            };
            assert_eq!(
                frozen.len(),
                scope.roots.len(),
                "one frozen depset per converted depset"
            );
            for (root, frozen) in scope.roots.iter().zip(frozen) {
                let depset = Depset::from_value(frozen.value()).expect("a frozen depset");
                let mut replay = Replay {
                    root,
                    sets: &scope.sets,
                    next: 0,
                    adopted: &mut adopted,
                };
                // The walk of the conversion, over the frozen copy: it visits the frozen copy
                // of each node where the conversion visited the node. The sets it builds (of
                // placeholder elements) are dropped.
                depset
                    .to_nested_set_with_memo::<()>(true, &mut |_| Ok(()), &mut replay)
                    .expect("the placeholder conversion does not fail");
                assert_eq!(
                    replay.next,
                    root.visits.len(),
                    "a frozen depset has the structure of its unfrozen original"
                );
            }
            adopted
        });
        let memos = depset_memos();
        let mut memo = memos.frozen_to_set.lock();
        for (key, set) in adopted {
            memo.entry(key).or_insert(set);
        }
    }
}

impl Drop for DepsetConversionScope {
    fn drop(&mut self) {
        SCOPES.with(|s| {
            s.borrow_mut().pop();
        });
    }
}

/// The memo of [`DepsetConversionScope::adopt_frozen`]'s walk over the frozen copy of `root`:
/// answers every lookup as the conversion of `root` was answered, and pairs the frozen node of
/// the lookup with the unfrozen node of the conversion's lookup at the same position.
struct Replay<'a> {
    root: &'a ConvertedRoot,
    sets: &'a HashMap<(TypeId, usize), AnySet>,
    /// Index of the next visit.
    next: usize,
    /// (element type, frozen node) -> the set of the unfrozen node.
    adopted: &'a mut Vec<((TypeId, usize), AnySet)>,
}

impl DepsetNodeMemo<()> for Replay<'_> {
    fn get(&mut self, node: usize, _frozen: bool) -> Option<NestedSet<()>> {
        let visit = self.root.visits.get(self.next).copied();
        self.next += 1;
        match visit {
            Some(Visit::Unfrozen {
                node: unfrozen,
                hit,
            }) => {
                if let Some(set) = self.sets.get(&(self.root.element, unfrozen)) {
                    self.adopted.push(((self.root.element, node), set.clone()));
                }
                if hit {
                    // Not walked further, as in the conversion. (An empty set of the default
                    // order can be added to a builder of any order.)
                    Some(Order::STABLE_ORDER.empty_set())
                } else {
                    None
                }
            }
            // A frozen node was not copied. Past the end the walks differ, which adopt_frozen
            // reports; stop walking.
            Some(Visit::Frozen) | None => Some(Order::STABLE_ORDER.empty_set()),
        }
    }

    fn put(&mut self, _node: usize, _frozen: bool, set: NestedSet<()>) -> NestedSet<()> {
        set
    }
}

/// The recording of the innermost open scope, if a conversion is being recorded.
fn with_recording(f: impl FnOnce(&mut Recording)) {
    SCOPES.with(|s| {
        if let Some(recording) = s
            .borrow_mut()
            .last_mut()
            .and_then(|scope| scope.recording.as_mut())
        {
            f(recording);
        }
    });
}

/// Replaces the recording of the innermost open scope and returns the previous one; `None`
/// when no scope is open.
fn replace_recording(recording: Option<Recording>) -> Option<Option<Recording>> {
    SCOPES.with(|s| {
        s.borrow_mut()
            .last_mut()
            .map(|scope| std::mem::replace(&mut scope.recording, recording))
    })
}

/// The process-wide and per-evaluation memos as `to_nested_set_with_memo`'s memo.
struct SharedMemo<E>(std::marker::PhantomData<E>);

impl<E> DepsetNodeMemo<E> for SharedMemo<E>
where
    E: Clone + Eq + Hash + Send + Sync + 'static,
{
    fn get(&mut self, node: usize, frozen: bool) -> Option<NestedSet<E>> {
        let key = (TypeId::of::<E>(), node);
        if frozen {
            let found = depset_memos()
                .frozen_to_set
                .lock()
                .get(&key)
                .map(downcast_set::<E>);
            let hit = found.is_some();
            with_recording(|recording| {
                if recording.frozen_depth == 0 {
                    recording.visits.push(Visit::Frozen);
                }
                if !hit {
                    // Converted now: `put` follows once the nodes below it are done.
                    recording.frozen_depth += 1;
                }
            });
            return found;
        }
        SCOPES.with(|s| {
            let mut s = s.borrow_mut();
            let scope = s.last_mut()?;
            let found = scope.sets.get(&key).map(downcast_set::<E>);
            if let Some(recording) = scope.recording.as_mut() {
                recording.visits.push(Visit::Unfrozen {
                    node,
                    hit: found.is_some(),
                });
            }
            found
        })
    }

    fn put(&mut self, node: usize, frozen: bool, set: NestedSet<E>) -> NestedSet<E> {
        let key = (TypeId::of::<E>(), node);
        if frozen {
            // A concurrent conversion of the same depset may have won: keep the first, as there
            // is one set per depset.
            let kept = {
                let memos = depset_memos();
                let mut memo = memos.frozen_to_set.lock();
                let entry = memo.entry(key).or_insert_with(|| Arc::new(set) as AnySet);
                downcast_set::<E>(entry)
            };
            with_recording(|recording| {
                recording.frozen_depth = recording.frozen_depth.saturating_sub(1);
            });
            return kept;
        }
        SCOPES.with(|s| {
            if let Some(scope) = s.borrow_mut().last_mut() {
                scope.sets.insert(key, Arc::new(set.clone()) as AnySet);
            }
        });
        set
    }
}

/// `Depset.getSet`: the `NestedSet` of depset `v` (which must be the value `d` was unpacked
/// from), converting elements with `f`. See the module comment for the identity it keeps.
// Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:207 Depset.getSet
pub fn depset_to_nested_set_shared<'v, E>(
    v: Value<'v>,
    d: &Depset<'v>,
    f: &mut dyn FnMut(Value<'v>) -> starlark::Result<E>,
) -> starlark::Result<NestedSet<E>>
where
    E: Clone + Eq + Hash + Send + Sync + 'static,
{
    let mut memo = SharedMemo::<E>(std::marker::PhantomData);
    if v.unpack_frozen().is_some() {
        return d.to_nested_set_with_memo(true, f, &mut memo);
    }
    // An unfrozen depset: record the walk, for DepsetConversionScope::adopt_frozen.
    let Some(outer) = replace_recording(Some(Recording::default())) else {
        // No scope is open: nothing is memoized for unfrozen depsets.
        return d.to_nested_set_with_memo(false, f, &mut memo);
    };
    let result = d.to_nested_set_with_memo(
        false,
        &mut |x: Value<'v>| -> starlark::Result<E> {
            // The element conversion is not part of the walk.
            let recording = replace_recording(None).flatten();
            let element = f(x);
            replace_recording(recording);
            element
        },
        &mut memo,
    );
    let recording = replace_recording(outer).flatten();
    if let (Ok(_), Some(recording)) = (&result, recording) {
        // A depset converted before has its set already: its walk visits nothing new.
        if matches!(
            recording.visits.first(),
            Some(Visit::Unfrozen { hit: false, .. })
        ) {
            // SAFETY: only the lifetime changes; the value is given back with the lifetime of
            // its heap by `converted_depsets` (see its contract) while the scope is open.
            let value = unsafe { std::mem::transmute::<Value<'v>, Value<'static>>(v) };
            SCOPES.with(|s| {
                if let Some(scope) = s.borrow_mut().last_mut() {
                    scope.roots.push(ConvertedRoot {
                        value,
                        element: TypeId::of::<E>(),
                        visits: recording.visits,
                    });
                }
            });
        }
    }
    result
}

/// Converts `set` to a frozen Starlark depset with the same node structure, converting elements
/// with `convert` (which allocates on the given frozen heap). `elem_type` is the Starlark type
/// name of the elements (Java `Depset.ElementType`, e.g. `File`, `string`).
// Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:180 Depset.of
pub fn nested_set_to_depset<E>(
    set: &NestedSet<E>,
    elem_type: &'static str,
    convert: &dyn Fn(&E, &FrozenHeap) -> FrozenValue,
) -> OwnedFrozenValue
where
    E: Clone + Eq + Hash + Send + Sync + 'static,
{
    let memos = depset_memos();
    let mut memo = memos.memo.lock();
    convert_node(set, elem_type, convert, &mut memo)
}

fn convert_node<E>(
    set: &NestedSet<E>,
    elem_type: &'static str,
    convert: &dyn Fn(&E, &FrozenHeap) -> FrozenValue,
    memo: &mut HashMap<(TypeId, usize), MemoEntry>,
) -> OwnedFrozenValue
where
    E: Clone + Eq + Hash + Send + Sync + 'static,
{
    let key = (TypeId::of::<E>(), set.node_id());
    if let Some(entry) = memo.get(&key) {
        return entry.depset.clone();
    }
    let order: Order = set.get_order();
    let heap = FrozenHeap::new();
    let (elem, children) = match set.children() {
        NestedSetChildren::Empty => (None, DepsetChildren::Empty),
        NestedSetChildren::Leaf(e) => (Some(elem_type), DepsetChildren::Leaf(convert(&e, &heap))),
        NestedSetChildren::Array(entries) => {
            let mut out = Vec::with_capacity(entries.len());
            for entry in entries {
                match entry {
                    NestedSetChild::Leaf(e) => out.push(DepsetChild::Leaf(convert(&e, &heap))),
                    NestedSetChild::Set(sub) => {
                        let sub_depset = convert_node(&sub, elem_type, convert, memo);
                        heap.add_reference(sub_depset.owner());
                        // SAFETY: `sub_depset`'s heap is referenced by `heap` (and kept alive by
                        // the memo).
                        out.push(DepsetChild::Array(unsafe {
                            sub_depset.unchecked_frozen_value()
                        }));
                    }
                }
            }
            (Some(elem_type), DepsetChildren::Array(out))
        }
    };
    let depset = FrozenDepset::new_frozen(order, elem, set.get_approx_depth(), children);
    let value: FrozenValue = heap.alloc(depset);
    let heap_ref = heap.into_ref();
    // SAFETY: `value` was allocated on `heap_ref`.
    let owned = unsafe { OwnedFrozenValue::new(heap_ref, value) };
    // Java: this depset wraps `set`; `getSet` gives `set` back.
    if let Some(frozen_depset) = Depset::from_value(value.to_value()) {
        depset_memos().frozen_to_set.lock().insert(
            (TypeId::of::<E>(), frozen_depset as *const Depset as usize),
            Arc::new(set.clone()) as AnySet,
        );
    }
    memo.insert(
        key,
        MemoEntry {
            _keep_alive: Box::new(set.clone()),
            depset: owned.clone(),
        },
    );
    owned
}

/// `Depset.of(Artifact.class, set)`: a depset of `File`s.
// Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:180 Depset.of
pub fn artifacts_to_depset(set: &NestedSet<Artifact>) -> OwnedFrozenValue {
    nested_set_to_depset(set, "File", &|a: &Artifact, heap: &FrozenHeap| {
        heap.alloc(StarlarkFile::new(a.clone()))
    })
}

/// `Depset.of(String.class, set)`: a depset of strings.
pub fn strings_to_depset(set: &NestedSet<String>) -> OwnedFrozenValue {
    nested_set_to_depset(set, "string", &|s: &String, heap: &FrozenHeap| {
        heap.alloc(s.as_str())
    })
}

/// A frozen depset as a value usable on any heap.
///
/// PORT: keep-alive (see the module comment): the memo keeps the depset's heap alive.
pub fn depset_value<'v>(depset: &OwnedFrozenValue) -> Value<'v> {
    // SAFETY: the depset comes from the memo ([`nested_set_to_depset`]), whose entry keeps its
    // heap for the life of the build arena. The value (and every frozen value that comes to
    // point at it) is held by an evaluator: a running evaluation or a graph value. The arena
    // outlives them: it is released only when no evaluator holds a lease, i.e. after every
    // evaluation ended and every graph was dropped.
    unsafe { depset.unchecked_frozen_value() }.to_value()
}

/// `Depset.cast(x, Artifact.class, what)`: the depset's NestedSet of artifacts, with Java's
/// error text.
// Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:262 Depset.cast
pub fn depset_to_artifacts(v: Value, what: &str) -> starlark::Result<NestedSet<Artifact>> {
    depset_cast(v, what, "File", &mut |x: Value| artifact_from_value(x))
}

/// `Depset.cast(x, String.class, what)`.
// Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:262 Depset.cast
pub fn depset_to_strings(v: Value, what: &str) -> starlark::Result<NestedSet<String>> {
    depset_cast(v, what, "string", &mut |x: Value| {
        x.unpack_str().map(str::to_owned)
    })
}

// Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:262 Depset.cast
// Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:207 Depset.getSet
fn depset_cast<'v, E: Clone + Eq + Hash + Send + Sync + 'static>(
    v: Value<'v>,
    what: &str,
    class_type: &str,
    convert: &mut dyn FnMut(Value<'v>) -> Option<E>,
) -> starlark::Result<NestedSet<E>> {
    let err = |m: String| starlark::Error::new_other(anyhow::anyhow!("{m}"));
    let Some(d) = Depset::from_value(v) else {
        return Err(err(format!(
            "for {what}, got {}, want a depset of {class_type}",
            v.get_type()
        )));
    };
    if !d.is_empty() && d.element_type() != class_type {
        return Err(err(format!(
            "for '{what}', got a depset of '{}', expected a depset of '{class_type}'",
            d.element_type()
        )));
    }
    depset_to_nested_set_shared(v, d, &mut |x: Value<'v>| {
        convert(x).ok_or_else(|| {
            err(format!(
                "for '{what}', got a depset of '{}', expected a depset of '{class_type}'",
                x.get_type()
            ))
        })
    })
}

/// Keeps `Arc` in scope for callers that store converted depsets with providers.
pub type SharedDepset = Arc<OwnedFrozenValue>;

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_collect::NestedSetBuilder;

    #[test]
    fn memoized_by_node_identity() {
        let set = NestedSetBuilder::create(Order::STABLE_ORDER, ["a".to_owned(), "b".to_owned()]);
        let d1 = strings_to_depset(&set);
        let d2 = strings_to_depset(&set.clone());
        assert!(d1.value().ptr_eq(d2.value()));
    }

    #[test]
    fn converted_native_set_converts_back_to_itself() {
        // Java: Depset.of(String.class, set).getSet(String.class) == set.
        let set = NestedSetBuilder::create(Order::STABLE_ORDER, ["p".to_owned(), "q".to_owned()]);
        let d = strings_to_depset(&set);
        let back = depset_to_strings(d.value(), "x").unwrap();
        assert_eq!(back.node_id(), set.node_id());
        let again = depset_to_strings(d.value(), "x").unwrap();
        assert_eq!(again.node_id(), set.node_id());
    }

    /// A frozen depset `["z", ["x", "y"]]` that no conversion has seen (like a provider field of
    /// a dependency that made no action from it).
    fn unconverted_frozen_depset() -> OwnedFrozenValue {
        let heap = FrozenHeap::new();
        let lower: FrozenValue = heap.alloc(FrozenDepset::new_frozen(
            Order::STABLE_ORDER,
            Some("string"),
            2,
            DepsetChildren::Array(vec![
                DepsetChild::Leaf(heap.alloc("x")),
                DepsetChild::Leaf(heap.alloc("y")),
            ]),
        ));
        let upper: FrozenValue = heap.alloc(FrozenDepset::new_frozen(
            Order::STABLE_ORDER,
            Some("string"),
            3,
            DepsetChildren::Array(vec![
                DepsetChild::Leaf(heap.alloc("z")),
                DepsetChild::Array(lower),
            ]),
        ));
        let heap_ref = heap.into_ref();
        keep_frozen_heap_alive(heap_ref.clone());
        // SAFETY: `upper` was allocated on `heap_ref`.
        unsafe { OwnedFrozenValue::new(heap_ref, upper) }
    }

    #[test]
    fn depset_converted_before_freezing_keeps_its_set_when_frozen() {
        // Java: a Depset and the NestedSet it wraps are the same objects before and after the
        // rule implementation's Mutability is frozen: Depset.getSet() of a provider's depset is
        // the set the target's own actions got from it (the oracle's aquery gives both one
        // depSetOfFiles id, e.g. brotli //:brotlicommon's headers).
        let native =
            NestedSetBuilder::create(Order::STABLE_ORDER, ["n1".to_owned(), "n2".to_owned()]);
        let native_depset = strings_to_depset(&native);
        let dep_depset = unconverted_frozen_depset();
        starlark::environment::Module::with_temp_heap(|module| {
            let scope = DepsetConversionScope::enter();
            let heap = module.heap();
            let (inner, outer) = {
                let mut eval = starlark::eval::Evaluator::new(&module);
                let inner = Depset::from_direct_and_transitive(
                    Order::STABLE_ORDER,
                    &[heap.alloc("a"), heap.alloc("b")],
                    &[],
                    true,
                    &mut eval,
                )
                .unwrap();
                let outer = Depset::from_direct_and_transitive(
                    Order::STABLE_ORDER,
                    &[heap.alloc("c")],
                    &[
                        depset_value(&dep_depset),
                        inner,
                        depset_value(&native_depset),
                    ],
                    true,
                    &mut eval,
                )
                .unwrap();
                (inner, outer)
            };
            module.set("inner", inner);
            module.set("outer", outer);

            let before = depset_to_strings(outer, "x").unwrap();
            let before_inner = depset_to_strings(inner, "x").unwrap();
            let before_dep = depset_to_strings(dep_depset.value(), "x").unwrap();
            // The members keep their sets (arrays).
            let NestedSetChildren::Array(children) = before.children() else {
                panic!("outer has four children");
            };
            let member_ids: Vec<usize> = children
                .iter()
                .filter_map(|child| match child {
                    NestedSetChild::Set(set) => Some(set.node_id()),
                    NestedSetChild::Leaf(_) => None,
                })
                .collect();
            assert_eq!(
                member_ids,
                [
                    before_dep.node_id(),
                    before_inner.node_id(),
                    native.node_id()
                ]
            );

            // Only `outer` made new sets; `inner` was converted as its member.
            // SAFETY: the depsets live on `module`'s heap, which was not garbage collected.
            let converted: Vec<Value> = unsafe { scope.converted_depsets() };
            assert_eq!(converted.len(), 1);
            assert!(converted[0].ptr_eq(outer));

            let frozen = module.freeze().unwrap();
            keep_frozen_heap_alive(frozen.frozen_heap().clone());
            scope.adopt_frozen(&[frozen.get("outer").unwrap()]);
            drop(scope);

            let frozen_outer = frozen.get("outer").unwrap();
            let frozen_inner = frozen.get("inner").unwrap();
            assert!(!frozen_outer.value().ptr_eq(outer));
            let after = depset_to_strings(frozen_outer.value(), "x").unwrap();
            assert_eq!(after.node_id(), before.node_id());
            let after_inner = depset_to_strings(frozen_inner.value(), "x").unwrap();
            assert_eq!(after_inner.node_id(), before_inner.node_id());
        });
    }

    #[test]
    fn unfrozen_depset_converts_to_one_set_inside_a_scope() {
        // Java: Depset.getSet returns the wrapped NestedSet every time.
        starlark::environment::Module::with_temp_heap(|module| {
            let _scope = DepsetConversionScope::enter();
            let heap = module.heap();
            let mut eval = starlark::eval::Evaluator::new(&module);
            let depset = Depset::from_direct_and_transitive(
                Order::STABLE_ORDER,
                &[heap.alloc("a"), heap.alloc("b")],
                &[],
                true,
                &mut eval,
            )
            .unwrap();
            let first = depset_to_strings(depset, "x").unwrap();
            let second = depset_to_strings(depset, "x").unwrap();
            assert_eq!(first.node_id(), second.node_id());
        });
    }

    #[test]
    fn to_list_order_preserved() {
        let inner = NestedSetBuilder::create(Order::STABLE_ORDER, ["x".to_owned(), "y".to_owned()]);
        let mut b = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
        b.add("z".to_owned());
        b.add_transitive(&inner);
        let set = b.build();
        let d = strings_to_depset(&set);
        let depset = Depset::from_value(d.value()).unwrap();
        let list: Vec<String> = depset
            .to_list()
            .unwrap()
            .into_iter()
            .map(|v| v.unpack_str().unwrap().to_owned())
            .collect();
        let expected: Vec<String> = set.to_list().iter().cloned().collect();
        assert_eq!(list, expected);
    }
}
