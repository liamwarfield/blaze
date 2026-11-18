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
//!   for the life of the process, so converting a set twice yields the same depset value (Java:
//!   the same Depset over the same NestedSet), and shared sub-DAGs stay shared.
//! - Starlark -> Rust ([`depset_to_nested_set_shared`]): `DepsetGen::to_nested_set` (same
//!   structure), memoized by the identity of the depset value, because Java's `Depset.getSet`
//!   returns the one wrapped `NestedSet` object every time and that identity reaches output
//!   (`ActionGraphDump`'s `KnownNestedSets` numbers sets by node identity). A depset made by
//!   [`nested_set_to_depset`] converts back to the very set it was made from.
//!
//! PORT: memo scope. Frozen depsets (providers of other targets, .bzl globals, converted native
//! sets) are memoized for the life of the process; their heaps live as long (single build per
//! process). Depsets of the rule implementation being evaluated are not frozen yet and their
//! addresses die with the module's heap, so they are memoized only inside a
//! [`DepsetConversionScope`] (one per rule evaluation). Not covered (requested from
//! blaze_starlark: a public `to_nested_set` taking the memo): a sub-depset that is first
//! converted as part of a larger depset and later on its own, and a depset converted both before
//! and after its module is frozen; those get equal but distinct nodes.
//!
//! PORT: keep-alive. The memo holds every converted NestedSet and its frozen depset for the life
//! of the process (single build per process), so values handed out by
//! `OwnedFrozenValue::unchecked_frozen_value` stay valid even where the caller cannot register
//! the frozen heap with its own heap (`StarlarkValue::get_attr` has no frozen heap).

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, LazyLock};

use blaze_actions::Artifact;
use blaze_actions::analysis::starlark::starlark_file::{StarlarkFile, artifact_from_value};
use blaze_collect::nestedset::{NestedSetChild, NestedSetChildren};
use blaze_collect::{NestedSet, Order};
use blaze_starlark::depset::{Depset, DepsetChild, DepsetChildren, FrozenDepset};
use parking_lot::Mutex;
use starlark::values::{FrozenHeap, FrozenValue, OwnedFrozenValue, Value};

/// One memoized conversion: the source node (kept alive so its identity is not reused) and the
/// frozen depset.
struct MemoEntry {
    _keep_alive: Box<dyn std::any::Any + Send + Sync>,
    depset: OwnedFrozenValue,
}

/// Memo of converted nodes, keyed by (element type, node identity).
static MEMO: LazyLock<Mutex<HashMap<(TypeId, usize), MemoEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Frozen depset identity -> the `NestedSet<E>` it stands for (boxed as `Any`).
static FROZEN_TO_SET: LazyLock<Mutex<HashMap<(TypeId, usize), Box<dyn Any + Send + Sync>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

thread_local! {
    /// Unfrozen depset identity -> `NestedSet<E>`, one map per open [`DepsetConversionScope`].
    static UNFROZEN_TO_SET: RefCell<Vec<HashMap<(TypeId, usize), Box<dyn Any>>>> =
        const { RefCell::new(Vec::new()) };
}

/// Frozen heaps whose depset addresses may be keys of [`FROZEN_TO_SET`].
static KEPT_HEAPS: LazyLock<Mutex<Vec<starlark::values::FrozenHeapRef>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// Keeps `heap` for the life of the process, so the addresses of its depsets stay unique.
pub fn keep_frozen_heap_alive(heap: starlark::values::FrozenHeapRef) {
    KEPT_HEAPS.lock().push(heap);
}

/// The lifetime of one Starlark module's unfrozen depsets (one rule implementation evaluation):
/// while it is open, converting the same unfrozen depset value twice yields the same
/// `NestedSet` node, as Java's `Depset.getSet` does.
pub struct DepsetConversionScope(());

impl DepsetConversionScope {
    pub fn enter() -> DepsetConversionScope {
        UNFROZEN_TO_SET.with(|s| s.borrow_mut().push(HashMap::new()));
        DepsetConversionScope(())
    }
}

impl Drop for DepsetConversionScope {
    fn drop(&mut self) {
        UNFROZEN_TO_SET.with(|s| {
            s.borrow_mut().pop();
        });
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
    let key = (TypeId::of::<E>(), d as *const Depset<'v> as usize);
    if v.unpack_frozen().is_some() {
        if let Some(set) = FROZEN_TO_SET.lock().get(&key) {
            return Ok(set
                .downcast_ref::<NestedSet<E>>()
                .expect("keyed by element type")
                .clone());
        }
        let set = d.to_nested_set(f)?;
        // A concurrent conversion of the same depset may have won: keep the first, as there is
        // one set per depset.
        let mut memo = FROZEN_TO_SET.lock();
        let entry = memo.entry(key).or_insert_with(|| Box::new(set));
        return Ok(entry
            .downcast_ref::<NestedSet<E>>()
            .expect("keyed by element type")
            .clone());
    }
    let cached = UNFROZEN_TO_SET.with(|s| {
        s.borrow().last().and_then(|m| {
            m.get(&key).map(|set| {
                set.downcast_ref::<NestedSet<E>>()
                    .expect("keyed by element type")
                    .clone()
            })
        })
    });
    if let Some(set) = cached {
        return Ok(set);
    }
    let set = d.to_nested_set(f)?;
    UNFROZEN_TO_SET.with(|s| {
        if let Some(m) = s.borrow_mut().last_mut() {
            m.insert(key, Box::new(set.clone()));
        }
    });
    Ok(set)
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
    let mut memo = MEMO.lock();
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
        FROZEN_TO_SET.lock().insert(
            (TypeId::of::<E>(), frozen_depset as *const Depset as usize),
            Box::new(set.clone()),
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
    // SAFETY: the memo keeps the frozen heap alive for the life of the process.
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
