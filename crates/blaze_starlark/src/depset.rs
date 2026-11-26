//! `depset`: the Starlark face of nested sets.
//!
//! Java: com/google/devtools/build/lib/collect/nestedset/Depset.java (value, `depset()`,
//! element and order checks, repr) and the parts of NestedSet.java / NestedSetBuilder.java that
//! decide structure and iteration order (ported in `blaze_collect`).
//!
//! Representation. Java keeps a `NestedSet<Object>` of Starlark values. In starlark-rust values
//! live in garbage-collected, movable heaps and must be reachable through `Trace`, so they cannot
//! sit inside `blaze_collect::NestedSet` (opaque `Arc` nodes). A depset is therefore a Starlark
//! value whose node structure mirrors NestedSet's exactly: children are either one element, or an
//! array of entries, each an element or another depset node that owns an array (Java shares the
//! child's `Object[]`; here the owning depset value stands for the array, and its identity is the
//! array's identity). Construction ([`Depset::from_direct_and_transitive`]) and flattening
//! ([`DepsetGen::to_list`]) port NestedSetBuilder.build, the NestedSet constructor and
//! NestedSet.expand/walk step for step, so `to_list` orders match Java. Freezing a module freezes
//! its depsets with it; depsets from loaded modules are shared by reference.
//!
//! Rust consumers (analysis, actions) convert with [`DepsetGen::to_nested_set`], which rebuilds an
//! equivalent `blaze_collect::NestedSet<E>` (same structure, hence same order and fingerprints).

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::sync::LazyLock;

use allocative::Allocative;
use blaze_collect::NestedSet;
use blaze_collect::NestedSetBuilder;
use blaze_collect::Order;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::environment::GlobalsBuilder;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_complex_value;
use starlark::starlark_module;
use starlark::values::Demand;
use starlark::values::Freeze;
use starlark::values::FreezeResult;
use starlark::values::Freezer;
use starlark::values::FrozenHeap;
use starlark::values::FrozenValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::OwnedFrozenValue;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::UnpackValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::AllocList;
use starlark::values::list::ListRef;
use starlark::values::list_or_tuple::UnpackListOrTuple;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;
use starlark::values::tuple::TupleRef;
use starlark_map::small_set::SmallSet;

use crate::structs::DeeplyImmutable;
use crate::structs::is_immutable;
use crate::thread::ThreadContext;

fn err(msg: String) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// One entry of a node's array (Java: an `Object[]` element — a leaf or a nested `Object[]`).
#[derive(Debug, Clone, Trace, Allocative)]
pub enum DepsetChild<V: starlark::values::ValueLifetimeless> {
    Leaf(V),
    /// A depset whose node owns an array (resolved through `Shared`).
    Array(V),
}

/// A node's children (Java `NestedSet.children`).
#[derive(Debug, Clone, Trace, Allocative)]
pub enum DepsetChildren<V: starlark::values::ValueLifetimeless> {
    Empty,
    Leaf(V),
    Array(Vec<DepsetChild<V>>),
    /// Same array as the given depset (Java: a set that dereferenced a single nested array).
    Shared(V),
}

/// The depset value; [`Depset`] when unfrozen, [`FrozenDepset`] when frozen.
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct DepsetGen<V: starlark::values::ValueLifetimeless> {
    #[allocative(skip)]
    #[trace(unsafe_ignore)]
    order: Order,
    /// Starlark type name of the elements, `None` when empty (Java `ElementType`).
    #[allocative(skip)]
    #[trace(unsafe_ignore)]
    elem_type: Option<&'static str>,
    /// Java `NestedSet.getApproxDepth`.
    depth: i32,
    children: DepsetChildren<V>,
}

starlark_complex_value!(pub Depset);

// SAFETY: `repr(C)`; FrozenValue and Value have the same layout, and the enums only hold `V`.
unsafe impl<'v> Coerce<DepsetGen<Value<'v>>> for DepsetGen<FrozenValue> {}

impl FrozenDepset {
    /// A frozen depset node from its parts (for converting a `blaze_collect::NestedSet` into a
    /// depset with the same structure). `children` must mirror NestedSet's node shape.
    // W4-analysis: needed by blaze_analysis::rule::depsets (NestedSet -> depset).
    pub fn new_frozen(
        order: Order,
        elem_type: Option<&'static str>,
        depth: i32,
        children: DepsetChildren<FrozenValue>,
    ) -> FrozenDepset {
        DepsetGen {
            order,
            elem_type,
            depth,
            children,
        }
    }
}

impl<'v> Freeze for Depset<'v> {
    type Frozen = FrozenDepset;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenDepset> {
        let f = |v: Value<'v>| freezer.freeze(v);
        let child = |c: DepsetChild<Value<'v>>| -> FreezeResult<DepsetChild<FrozenValue>> {
            Ok(match c {
                DepsetChild::Leaf(v) => DepsetChild::Leaf(f(v)?),
                DepsetChild::Array(v) => DepsetChild::Array(f(v)?),
            })
        };
        let children = match self.children {
            DepsetChildren::Empty => DepsetChildren::Empty,
            DepsetChildren::Leaf(v) => DepsetChildren::Leaf(f(v)?),
            DepsetChildren::Shared(v) => DepsetChildren::Shared(f(v)?),
            DepsetChildren::Array(a) => {
                DepsetChildren::Array(a.into_iter().map(child).collect::<FreezeResult<_>>()?)
            }
        };
        Ok(DepsetGen {
            order: self.order,
            elem_type: self.elem_type,
            depth: self.depth,
            children,
        })
    }
}

/// A child, as a borrowed view over either representation.
#[derive(Clone, Copy)]
enum ChildRef<'v> {
    Leaf(Value<'v>),
    Array(Value<'v>),
}

impl<'v, V: ValueLike<'v>> DepsetGen<V> {
    pub fn order(&self) -> Order {
        self.order
    }

    pub fn is_empty(&self) -> bool {
        matches!(self.children, DepsetChildren::Empty)
    }

    /// Java `Depset.getElementType().toString()`: "empty" or the element type name.
    pub fn element_type(&self) -> &'static str {
        self.elem_type.unwrap_or("empty")
    }

    /// Java `NestedSet.getApproxDepth`.
    pub fn approx_depth(&self) -> i32 {
        self.depth
    }

    /// The array this node's children resolve to, with the identity of its owner, if any.
    fn array(&self) -> Option<(usize, Vec<ChildRef<'v>>)> {
        match &self.children {
            DepsetChildren::Array(a) => Some((
                self as *const Self as usize,
                a.iter()
                    .map(|c| match c {
                        DepsetChild::Leaf(v) => ChildRef::Leaf(v.to_value()),
                        DepsetChild::Array(v) => ChildRef::Array(v.to_value()),
                    })
                    .collect(),
            )),
            DepsetChildren::Shared(owner) => {
                Depset::from_value(owner.to_value()).and_then(|d| d.array())
            }
            _ => None,
        }
    }

    fn single_leaf(&self) -> Option<Value<'v>> {
        match &self.children {
            DepsetChildren::Leaf(v) => Some(v.to_value()),
            _ => None,
        }
    }

    /// Java `Depset.toList` / `NestedSet.toList`: unique elements in traversal order.
    pub fn to_list(&self) -> starlark::Result<Vec<Value<'v>>> {
        match &self.children {
            DepsetChildren::Empty => Ok(Vec::new()),
            DepsetChildren::Leaf(v) => Ok(vec![v.to_value()]),
            _ => {
                let (id, array) = self.array().expect("array");
                // Java: NestedSet.expand / walk (iterative, same visiting order).
                let mut members: SmallSet<Value<'v>> = SmallSet::new();
                let mut sets: HashSet<usize> = HashSet::new();
                sets.insert(id);
                let mut stack: Vec<(Vec<ChildRef<'v>>, usize)> = vec![(array, 0)];
                while let Some(top) = stack.last_mut() {
                    if top.1 == top.0.len() {
                        stack.pop();
                        continue;
                    }
                    let child = top.0[top.1];
                    top.1 += 1;
                    match child {
                        ChildRef::Array(d) => {
                            let d = Depset::from_value(d).expect("depset child");
                            if let Some((id, a)) = d.array()
                                && sets.insert(id)
                            {
                                stack.push((a, 0));
                            }
                        }
                        ChildRef::Leaf(e) => {
                            members.insert_hashed(e.get_hashed()?);
                        }
                    }
                }
                let mut list: Vec<Value<'v>> = members.into_iter().collect();
                if self.order == Order::LINK_ORDER {
                    list.reverse();
                }
                Ok(list)
            }
        }
    }

    /// Rebuilds this depset as a `blaze_collect::NestedSet<E>` with the same structure (so the
    /// same `to_list` order), converting each element with `f`. Shared sub-depsets are converted
    /// once per call.
    pub fn to_nested_set<E: Clone + Eq + std::hash::Hash>(
        &self,
        f: &mut dyn FnMut(Value<'v>) -> starlark::Result<E>,
    ) -> starlark::Result<NestedSet<E>> {
        let mut memo: HashMap<usize, NestedSet<E>> = HashMap::new();
        self.to_nested_set_memo(f, &mut memo)
    }

    fn to_nested_set_memo<E: Clone + Eq + std::hash::Hash>(
        &self,
        f: &mut dyn FnMut(Value<'v>) -> starlark::Result<E>,
        memo: &mut HashMap<usize, NestedSet<E>>,
    ) -> starlark::Result<NestedSet<E>> {
        let key = self as *const Self as usize;
        if let Some(s) = memo.get(&key) {
            return Ok(s.clone());
        }
        let mut b = NestedSetBuilder::new_builder(self.order);
        match &self.children {
            DepsetChildren::Empty => {}
            DepsetChildren::Leaf(v) => {
                b.add(f(v.to_value())?);
            }
            _ => {
                // One node of the Java DAG. Every entry is re-added as a transitive member, in
                // array order (LINK_ORDER's builder reverses its transitive list, so add those
                // reversed): singleton sets are hoisted back to leaves at the same positions and
                // arrays stay arrays, so the rebuilt children array is identical.
                let (_, array) = self.array().unwrap();
                let mut parts = Vec::with_capacity(array.len());
                for c in array {
                    parts.push(match c {
                        ChildRef::Leaf(e) => NestedSetBuilder::create(self.order, [f(e)?]),
                        ChildRef::Array(d) => {
                            Depset::from_value(d).unwrap().to_nested_set_memo(f, memo)?
                        }
                    });
                }
                if self.order == Order::LINK_ORDER {
                    parts.reverse();
                }
                for p in &parts {
                    b.add_transitive(p);
                }
            }
        }
        let s = b.build();
        memo.insert(key, s.clone());
        Ok(s)
    }

    /// [`DepsetGen::to_nested_set`] with a memo the caller keeps across calls, so that one
    /// depset node always converts to the same `NestedSet` node (Java: a `Depset` wraps one
    /// `NestedSet` object and `getSet` returns it; sub-depsets share their `Object[]`). A node
    /// whose children are another depset's array ([`DepsetChildren::Shared`]) is rebuilt around
    /// that depset's set, as the Java constructor dereferences a lone nested array.
    /// `self_is_frozen`: whether the value holding `self` is frozen (its address is then stable
    /// for the life of its frozen heap).
    // W4-analysis: needed by blaze_analysis::rule::depsets (Depset.getSet identity reaches
    // aquery through ActionGraphDump's KnownNestedSets).
    // Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:207 Depset.getSet
    pub fn to_nested_set_with_memo<E: Clone + Eq + std::hash::Hash>(
        &self,
        self_is_frozen: bool,
        f: &mut dyn FnMut(Value<'v>) -> starlark::Result<E>,
        memo: &mut dyn DepsetNodeMemo<E>,
    ) -> starlark::Result<NestedSet<E>> {
        let key = self as *const Self as usize;
        if let Some(s) = memo.get(key, self_is_frozen) {
            return Ok(s);
        }
        let mut b = NestedSetBuilder::new_builder(self.order);
        match &self.children {
            DepsetChildren::Empty => {}
            DepsetChildren::Leaf(v) => {
                b.add(f(v.to_value())?);
            }
            DepsetChildren::Shared(owner) => {
                let owner = owner.to_value();
                let owner_set = Depset::from_value(owner).unwrap().to_nested_set_with_memo(
                    owner.unpack_frozen().is_some(),
                    f,
                    memo,
                )?;
                b.add_transitive(&owner_set);
            }
            DepsetChildren::Array(_) => {
                // As in to_nested_set_memo.
                let (_, array) = self.array().unwrap();
                let mut parts = Vec::with_capacity(array.len());
                for c in array {
                    parts.push(match c {
                        ChildRef::Leaf(e) => NestedSetBuilder::create(self.order, [f(e)?]),
                        ChildRef::Array(d) => Depset::from_value(d)
                            .unwrap()
                            .to_nested_set_with_memo(d.unpack_frozen().is_some(), f, memo)?,
                    });
                }
                if self.order == Order::LINK_ORDER {
                    parts.reverse();
                }
                for p in &parts {
                    b.add_transitive(p);
                }
            }
        }
        Ok(memo.put(key, self_is_frozen, b.build()))
    }
}

/// The memo of [`DepsetGen::to_nested_set_with_memo`]: depset node address to its `NestedSet`.
// W4-analysis: see to_nested_set_with_memo.
pub trait DepsetNodeMemo<E> {
    /// The set already made for the depset node at address `node`, if any. `frozen`: the node's
    /// value is frozen (an unfrozen address is only meaningful while its heap lives).
    fn get(&mut self, node: usize, frozen: bool) -> Option<NestedSet<E>>;
    /// Records `set` for `node` and returns the set to use for it (the recorded one).
    fn put(&mut self, node: usize, frozen: bool, set: NestedSet<E>) -> NestedSet<E>;
}

impl<'v, V: ValueLike<'v>> fmt::Display for DepsetGen<V> {
    // Java: Depset.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "depset([")?;
        if let Ok(list) = self.to_list() {
            for (i, v) in list.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}", v.to_repr())?;
            }
        }
        write!(f, "]")?;
        if self.order != Order::STABLE_ORDER {
            write!(f, ", order = \"{}\"", self.order.get_starlark_name())?;
        }
        write!(f, ")")
    }
}

#[starlark_value(type = "depset")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for DepsetGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("depset", depset_methods);
        Some(RES.methods())
    }

    // Java: Depset.truth
    fn to_bool(&self) -> bool {
        !self.is_empty()
    }

    // Java: Depset.equals is NestedSet identity (same children array).
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        let Some(o) = Depset::from_value(other) else {
            return Ok(false);
        };
        Ok(match (self.array(), o.array()) {
            (Some((a, _)), Some((b, _))) => a == b,
            _ => std::ptr::eq(
                self as *const Self as *const u8,
                o as *const Depset as *const u8,
            ),
        })
    }

    fn write_hash(&self, hasher: &mut starlark_map::StarlarkHasher) -> starlark::Result<()> {
        use std::hash::Hash;
        let id = self
            .array()
            .map(|a| a.0)
            .unwrap_or(self as *const Self as usize);
        id.hash(hasher);
        Ok(())
    }

    fn compare(&self, other: Value<'v>) -> starlark::Result<std::cmp::Ordering> {
        Err(err(format!(
            "unsupported comparison: depset <=> {}",
            other.get_type()
        )))
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

#[starlark_module]
fn depset_methods(builder: &mut MethodsBuilder) {
    /// Java: Depset.toListForStarlark
    fn to_list<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let d = Depset::from_value(this).expect("depset");
        Ok(heap.alloc(AllocList(d.to_list()?)))
    }
}

// -------------------------------------------------------------------------------------------
// Construction

/// Empty depsets, one per order, shared like Java's `Order.emptyDepset()`.
static EMPTY_DEPSETS: LazyLock<Vec<OwnedFrozenValue>> = LazyLock::new(|| {
    let heap = FrozenHeap::new();
    let values: Vec<FrozenValue> = Order::VALUES
        .iter()
        .map(|o| {
            heap.alloc(DepsetGen::<FrozenValue> {
                order: *o,
                elem_type: None,
                depth: 0,
                children: DepsetChildren::Empty,
            })
        })
        .collect();
    let heap = heap.into_ref();
    values
        .into_iter()
        // SAFETY: the values live on `heap`, which each OwnedFrozenValue keeps alive.
        .map(|v| unsafe { OwnedFrozenValue::new(heap.clone(), v) })
        .collect()
});

/// Java `Order.emptyDepset()` for an evaluation's heap.
pub fn empty_depset<'v>(order: Order, eval: &Evaluator<'v, '_, '_>) -> Value<'v> {
    let owned = &EMPTY_DEPSETS[order.ordinal() as usize];
    // SAFETY: adds the static heap as a reference of the evaluation's frozen heap.
    Value::new_frozen(unsafe { owned.owned_frozen_value(eval.frozen_heap()) })
}

// Java: Depset.checkElement
fn check_element(x: Value, strict: bool) -> starlark::Result<()> {
    if strict && !is_immutable(x) {
        return Err(err("depset elements must not be mutable values".to_owned()));
    }
    let t = x.get_type();
    if t == "list" || t == "dict" {
        return Err(err(format!("depsets cannot contain items of type '{t}'")));
    }
    Ok(())
}

// Java: Depset.checkType
fn check_type(
    existing: Option<&'static str>,
    new: &'static str,
) -> starlark::Result<Option<&'static str>> {
    match existing {
        None => Ok(Some(new)),
        Some(e) if e == new => Ok(Some(new)),
        Some(e) => Err(err(format!(
            "cannot add an item of type '{new}' to a depset of '{e}'"
        ))),
    }
}

impl<'v> Depset<'v> {
    /// Java: Depset.fromDirectAndTransitive, followed by NestedSetBuilder.build and the
    /// NestedSet constructor.
    pub fn from_direct_and_transitive(
        order: Order,
        direct: &[Value<'v>],
        transitive: &[Value<'v>],
        strict: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let mut elem_type: Option<&'static str> = None;
        for x in direct {
            check_element(*x, strict)?;
            elem_type = check_type(elem_type, x.get_type())?;
        }
        // NestedSetBuilder.addAll: a LinkedHashSet of direct items.
        let mut items: SmallSet<Value<'v>> = SmallSet::new();
        for x in direct {
            items.insert_hashed(x.get_hashed()?);
        }
        // NestedSetBuilder.addTransitive: deduplicated by identity; LINK_ORDER reverses.
        let mut trans: Vec<&'v Depset<'v>> = Vec::new();
        let mut seen: HashSet<usize> = HashSet::new();
        for x in transitive {
            let d = Depset::from_value(*x).ok_or_else(|| {
                err(format!(
                    "at index 0 of transitive, got element of type {}, want depset",
                    x.get_type()
                ))
            })?;
            if d.is_empty() {
                continue;
            }
            elem_type = check_type(elem_type, d.element_type())?;
            if !order.is_compatible(d.order()) {
                return Err(err(format!(
                    "Order '{}' is incompatible with order '{}'",
                    order.get_starlark_name(),
                    d.order().get_starlark_name()
                )));
            }
            // Java: DefaultNestedSetBuilder keys a LinkedHashSet by NestedSet equality (identity
            // of the children array); LinkOrderNestedSetBuilder keeps a list.
            let id = d
                .array()
                .map(|a| a.0)
                .unwrap_or(d as *const Depset as usize);
            if order == Order::LINK_ORDER || seen.insert(id) {
                trans.push(d);
            }
        }
        if order == Order::LINK_ORDER && trans.len() > 1 {
            // Java: LinkOrderNestedSetBuilder.getTransitive — reversed, then deduplicated.
            let mut out = Vec::new();
            let mut seen = HashSet::new();
            for d in trans.iter().rev() {
                let id = d
                    .array()
                    .map(|a| a.0)
                    .unwrap_or(*d as *const Depset as usize);
                if seen.insert(id) {
                    out.push(*d);
                }
            }
            trans = out;
        }
        let direct: Vec<Value<'v>> = items.into_iter().collect();
        if direct.is_empty() && trans.is_empty() {
            return Ok(empty_depset(order, eval));
        }

        // NestedSetBuilder.build: reuse a single transitive set.
        if trans.len() == 1 && direct.len() <= 1 {
            let c = trans[0];
            if c.order() == order
                && (direct.is_empty()
                    || c.single_leaf()
                        .is_some_and(|l| l.equals(direct[0]).unwrap_or(false)))
            {
                return Ok(transitive
                    .iter()
                    .find(|v| Depset::from_value(**v).is_some_and(|d| std::ptr::eq(d, c)))
                    .copied()
                    .unwrap());
            }
        }

        // NestedSet.<init>(order, direct, transitive)
        let direct_order: Vec<Value<'v>> = if order == Order::LINK_ORDER {
            direct.iter().rev().copied().collect()
        } else {
            direct.clone()
        };
        let preorder = order == Order::NAIVE_LINK_ORDER;
        let contains_direct = |v: Value<'v>| direct.iter().any(|d| d.equals(v).unwrap_or(false));
        enum Already<'v> {
            Nothing,
            Direct,
            Hoisted(Vec<Value<'v>>),
        }
        let mut already = Already::Nothing;
        let mut children: Vec<DepsetChild<Value<'v>>> = Vec::new();
        let mut approx_depth = 0;
        // The values of transitive depsets, to refer to their arrays.
        let value_of = |d: &'v Depset<'v>| -> Value<'v> {
            *transitive
                .iter()
                .find(|v| Depset::from_value(**v).is_some_and(|x| std::ptr::eq(x, d)))
                .unwrap()
        };
        for pass in 0..=1 {
            if (pass == 0) == preorder && !direct.is_empty() {
                for m in &direct_order {
                    let contained = match &already {
                        Already::Nothing => false,
                        Already::Direct => contains_direct(*m),
                        Already::Hoisted(h) => h.iter().any(|x| x.equals(*m).unwrap_or(false)),
                    };
                    if !contained {
                        children.push(DepsetChild::Leaf(*m));
                        approx_depth = approx_depth.max(2);
                    }
                }
                already = Already::Direct;
            } else if (pass == 1) == preorder && !trans.is_empty() {
                let mut hoisted: Option<Vec<Value<'v>>> = None;
                for d in &trans {
                    approx_depth = approx_depth.max(1 + d.approx_depth());
                    match &d.children {
                        DepsetChildren::Array(_) => children.push(DepsetChild::Array(value_of(d))),
                        DepsetChildren::Shared(owner) => children.push(DepsetChild::Array(*owner)),
                        DepsetChildren::Leaf(c) => {
                            let contained = match &already {
                                Already::Nothing => false,
                                Already::Direct => contains_direct(*c),
                                Already::Hoisted(h) => {
                                    h.iter().any(|x| x.equals(*c).unwrap_or(false))
                                }
                            };
                            let h = hoisted.get_or_insert_with(Vec::new);
                            if !contained && !h.iter().any(|x| x.equals(*c).unwrap_or(false)) {
                                h.push(*c);
                                children.push(DepsetChild::Leaf(*c));
                            }
                        }
                        DepsetChildren::Empty => unreachable!("empty transitive sets are skipped"),
                    }
                }
                already = match hoisted {
                    None => Already::Nothing,
                    Some(h) => Already::Hoisted(h),
                };
            }
        }
        let children = match children.len() {
            0 => {
                approx_depth = 0;
                DepsetChildren::Empty
            }
            1 => {
                approx_depth -= 1;
                match children.pop().unwrap() {
                    DepsetChild::Leaf(v) => DepsetChildren::Leaf(v),
                    DepsetChild::Array(owner) => DepsetChildren::Shared(owner),
                }
            }
            _ => DepsetChildren::Array(children),
        };
        Ok(eval.heap().alloc(DepsetGen {
            order,
            elem_type,
            depth: approx_depth,
            children,
        }))
    }
}

/// `x` as a sequence for `depset()`'s `direct`/`transitive` (Java `Sequence.noneableCast`).
fn sequence<'v>(x: Value<'v>, what: &str) -> starlark::Result<Vec<Value<'v>>> {
    if x.is_none() {
        return Ok(Vec::new());
    }
    if let Some(l) = ListRef::from_value(x) {
        return Ok(l.content().to_vec());
    }
    if let Some(t) = TupleRef::from_value(x) {
        return Ok(t.content().to_vec());
    }
    // A binding error (Java checkParamValue): Bazel locates it at the call's enclosing statement
    // when the call has named arguments (starlark patch 0061).
    Err(starlark::values::bazel_argument_binding_error(format!(
        "in call to depset(), parameter '{what}' got value of type '{}', want 'sequence or NoneType'",
        x.get_type()
    )))
}

/// Registers `depset` (Java `Depset.DepsetLibrary`, in the BUILD and .bzl environments).
#[starlark_module]
pub fn register_depset(builder: &mut GlobalsBuilder) {
    /// Java: Depset.java DepsetLibrary.depset and Depset.depset
    fn depset<'v>(
        #[starlark(default = NoneOr::None)] direct: NoneOr<Value<'v>>,
        #[starlark(default = "default")] order: &str,
        #[starlark(require = named, default = NoneOr::None)] transitive: NoneOr<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let order = Order::parse(order).map_err(|e| err(e.to_string()))?;
        let direct = sequence(direct.into_option().unwrap_or(Value::new_none()), "direct")?;
        let transitive_v = transitive.into_option().unwrap_or(Value::new_none());
        let transitive = if transitive_v.is_none() {
            Vec::new()
        } else {
            let items = UnpackListOrTuple::<Value>::unpack_value_opt(transitive_v).ok_or_else(
                || {
                    // Java checkParamValue: `@Param(allowedTypes = {Sequence, NoneType})`
                    // (StarlarkBuiltinsApi DepsetLibrary.depset; the generated table's text).
                    starlark::values::bazel_argument_binding_error(format!(
                        "in call to depset(), parameter 'transitive' got value of type '{}', want 'sequence or NoneType'",
                        transitive_v.get_type()
                    ))
                },
            )?;
            for (i, t) in items.items.iter().enumerate() {
                if Depset::from_value(*t).is_none() {
                    return Err(err(format!(
                        "at index {i} of transitive, got element of type {}, want depset",
                        t.get_type()
                    )));
                }
            }
            items.items
        };
        let (strict, limit) = match ThreadContext::get(eval) {
            Ok(cx) => (
                cx.semantics.incompatible_always_check_depset_elements,
                cx.semantics.nested_set_depth_limit,
            ),
            Err(_) => (true, 3500),
        };
        let result = Depset::from_direct_and_transitive(order, &direct, &transitive, strict, eval)?;
        let depth = Depset::from_value(result).unwrap().approx_depth();
        if depth > limit {
            return Err(err(format!("depset depth {depth} exceeds limit ({limit})")));
        }
        Ok(result)
    }
}
