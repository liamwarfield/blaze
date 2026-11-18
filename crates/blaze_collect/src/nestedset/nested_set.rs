//! Port of `NestedSet.java`.

use std::collections::HashSet;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Weak};

use blaze_util::{check_argument, check_state};
use indexmap::IndexSet;
use parking_lot::Mutex;

use super::{NestedSetBuilder, Order};

/// The physical representation of a NestedSet graph node (Java: the `Object children` field).
///
/// Java stores either `EMPTY_CHILDREN` (the shared empty `Object[]`), a single element, or an
/// `Object[]` whose entries are elements or nested `Object[]`s. Array identity (`==`) is
/// significant; here it is the identity of the `Arc` allocation.
pub(crate) enum Children<E> {
    /// Java: `NestedSet.EMPTY_CHILDREN`. Only ever the top-level children of an empty set.
    Empty,
    /// A single element (a leaf of the graph).
    Leaf(E),
    /// Java: `Object[]` (a non-leaf node).
    Array(Arc<[Children<E>]>),
}

impl<E: Clone> Clone for Children<E> {
    fn clone(&self) -> Self {
        match self {
            Children::Empty => Children::Empty,
            Children::Leaf(e) => Children::Leaf(e.clone()),
            Children::Array(a) => Children::Array(Arc::clone(a)),
        }
    }
}

pub(crate) fn array_identity<E>(a: &Arc<[Children<E>]>) -> usize {
    Arc::as_ptr(a) as *const () as usize
}

/// Java `Object.equals` on a children value: elements by `equals`, arrays by identity, and the
/// unique `EMPTY_CHILDREN` equal only to itself.
impl<E: PartialEq> PartialEq for Children<E> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Children::Empty, Children::Empty) => true,
            (Children::Leaf(a), Children::Leaf(b)) => a == b,
            (Children::Array(a), Children::Array(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl<E: Eq> Eq for Children<E> {}

impl<E: Hash> Hash for Children<E> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Children::Empty => state.write_u8(0),
            Children::Leaf(e) => {
                state.write_u8(1);
                e.hash(state);
            }
            Children::Array(a) => {
                state.write_u8(2);
                state.write_usize(array_identity(a));
            }
        }
    }
}

struct NestedSetInner<E> {
    /// The set's order and approximate depth, packed to save space.
    ///
    /// The low 2 bits contain the Order.ordinal value.
    ///
    /// The high 30 bits, of which only about 12 are really necessary, contain the depth of the
    /// set; see getApproxDepth. Because the union constructor discards the depths of all but the
    /// deepest nonleaf child, the sets returned by getNonLeaves have inaccurate depths that may
    /// overapproximate the true depth.
    depth_and_order: i32,

    /// Contains the "direct" elements and "transitive" nested sets.
    ///
    /// Direct elements are never arrays. Transitive elements may be arrays, but singletons are
    /// replaced by their sole element (thus transitive arrays always contain at least two logical
    /// elements).
    ///
    /// The relative order of direct and transitive is determined by the Order.
    ///
    /// All empty sets have the value `EMPTY_CHILDREN` ([`Children::Empty`]).
    children: Children<E>,

    /// Cached representation of `toList`.
    ///
    /// For instances with no transitive members, this is always `None` - caching is not
    /// worthwhile, since no traversal is needed. For instances with transitive members, this is
    /// initialized to an empty weak reference (`EMPTY_WEAK_REF`) and replaced with a populated
    /// one when a traversal is performed.
    // PORT: Java caches the un-reversed expansion in a WeakReference and reverses LINK_ORDER
    // lists on each call; this port caches the final (already reversed) list, which is not
    // observable. A Rust `Weak` drops the list as soon as the last caller drops it (Java keeps it
    // until the next GC), so cache hits are rarer; contents are identical.
    cached: Option<Mutex<Weak<Vec<E>>>>,
}

/// A NestedSet is an immutable ordered set of element values of type `E`.
///
/// Conceptually, NestedSet values form a directed acyclic graph (DAG). Each leaf node represents
/// a set containing a single element; there is also a distinguished leaf node representing the
/// empty set. Each non-leaf node represents the union of the sets represented by its successors.
///
/// A NestedSet value represents a node in this graph. The elements of a NestedSet may be
/// enumerated by traversing the complete DAG, eliminating duplicates using an ephemeral hash
/// table. The `to_list` and `to_set` methods provide the result of this traversal as a list or a
/// set, respectively. These operations, which are relatively expensive, are known as "flattening".
/// Computing the size of the set requires flattening.
///
/// By contrast, construction of a new set as a union of existing sets is relatively cheap. The
/// constructor accepts a list of "direct" elements and list of "transitive" nodes. The resulting
/// NestedSet refers to a new graph node representing their union. The relative order of direct
/// and transitive successors is governed by the Order parameter. Duplicates among the "direct"
/// elements are eliminated at construction, again with an ephemeral hash table. If after
/// duplicate elimination the new node would have exactly one successor, whether "direct" or
/// "transitive", the resulting NestedSet reuses the existing node for the sole successor.
///
/// Cloning a `NestedSet` is cheap and yields the *same instance* (Java reference). Like Java,
/// `==` ([`PartialEq`]) and [`Hash`] on `NestedSet` are reference identity; use
/// [`NestedSet::shallow_equals`] or compare [`NestedSet::to_list`] for value comparisons.
///
/// The element type only needs `Clone + Eq + Hash`; a Starlark `depset` supplies its own
/// equality/hash through a wrapper element type.
// Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:82 NestedSet
pub struct NestedSet<E> {
    inner: Arc<NestedSetInner<E>>,
}

impl<E> Clone for NestedSet<E> {
    fn clone(&self) -> Self {
        NestedSet {
            inner: Arc::clone(&self.inner),
        }
    }
}

// The only interior mutability is the toList cache, which is always either empty or a complete
// list, so observing a NestedSet after a panic is safe.
impl<E: std::panic::RefUnwindSafe> std::panic::RefUnwindSafe for NestedSet<E> {}
impl<E: std::panic::RefUnwindSafe> std::panic::UnwindSafe for NestedSet<E> {}

/// Java: `NestedSet` does not override `equals`, so equality is reference identity.
impl<E> PartialEq for NestedSet<E> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl<E> Eq for NestedSet<E> {}

/// Java: identity hash code.
impl<E> Hash for NestedSet<E> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(Arc::as_ptr(&self.inner) as *const () as usize);
    }
}

// Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:562 MAX_ELEMENTS_TO_STRING
pub const MAX_ELEMENTS_TO_STRING: usize = 1_000_000;

/// Remembers children extracted from one-element subsets (see the union constructor).
enum AlreadyInserted<'a, E> {
    /// Java: `ImmutableSet.of()`.
    Nothing,
    /// Java: `alreadyInserted = direct`.
    Direct,
    /// Java: `alreadyInserted = hoisted`.
    Hoisted(HashSet<&'a E>),
}

impl<E> NestedSet<E> {
    /// Returns a new builder.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:91 NestedSet.builder
    pub fn builder(order: Order) -> NestedSetBuilder<E> {
        NestedSetBuilder::new_builder(order)
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:99 NestedSet.withFuture
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:106 NestedSet.forDeserialization
    // PORT: skipped, serialization (deserialization futures are never created, so every
    // `children instanceof ListenableFuture` branch below is dropped).

    /// Constructs an empty NestedSet. Should only be called by Order.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:163 NestedSet.<init>
    pub(crate) fn new_empty(order: Order) -> Self {
        Self::with_children(order, 0, Children::Empty)
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:168 NestedSet.<init>
    pub(crate) fn with_children(order: Order, depth: i32, children: Children<E>) -> Self {
        let depth_and_order = (depth << 2) | order.ordinal();
        // expandWithCaching() assumes that cached == null means there are no transitive members.
        // We could use depth, but that's an approximation in some cases, so avoid relying on it.
        let cached = match &children {
            Children::Array(array) if Self::has_transitive_member(array) => {
                Some(Mutex::new(Weak::new()))
            }
            _ => None,
        };
        NestedSet {
            inner: Arc::new(NestedSetInner {
                depth_and_order,
                children,
                cached,
            }),
        }
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:272 NestedSet.hasTransitiveMember
    fn has_transitive_member(children: &[Children<E>]) -> bool {
        children.iter().any(|c| matches!(c, Children::Array(_)))
    }

    /// The identity of this set's node (Java: the identity of its `children` object): the
    /// address of its children array, or of the set itself for an empty or singleton set.
    // W4-analysis: needed to convert NestedSets to Starlark depsets with the same structure.
    pub fn node_id(&self) -> usize {
        match &self.inner.children {
            Children::Array(array) => Arc::as_ptr(array) as *const () as usize,
            _ => Arc::as_ptr(&self.inner) as *const () as usize,
        }
    }

    /// A public view of this set's node (Java `NestedSet.getChildren` walked one level): its
    /// direct leaves and sub-arrays in array order. A sub-array is returned as a set sharing the
    /// array, so its [`NestedSet::node_id`] is the array's identity.
    // W4-analysis: needed to convert NestedSets to Starlark depsets with the same structure.
    pub fn children(&self) -> NestedSetChildren<E>
    where
        E: Clone,
    {
        let order = self.get_order();
        let depth = self.get_approx_depth();
        match &self.inner.children {
            Children::Empty => NestedSetChildren::Empty,
            Children::Leaf(e) => NestedSetChildren::Leaf(e.clone()),
            Children::Array(array) => NestedSetChildren::Array(
                array
                    .iter()
                    .map(|c| match c {
                        Children::Leaf(e) => NestedSetChild::Leaf(e.clone()),
                        Children::Array(a) => NestedSetChild::Set(Self::with_children(
                            order,
                            (depth - 1).max(1),
                            Children::Array(a.clone()),
                        )),
                        Children::Empty => unreachable!("empty child in an array"),
                    })
                    .collect(),
            ),
        }
    }

    /// Clears the cached list representation to free up memory when no other traversal of the
    /// NestedSet is expected.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:288 NestedSet.clearCachedListRepresentation
    pub fn clear_cached_list_representation(&self) {
        if let Some(cached) = &self.inner.cached {
            *cached.lock() = Weak::new();
        }
    }

    /// Returns the ordering of this nested set.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:295 NestedSet.getOrder
    pub fn get_order(&self) -> Order {
        Order::get_order(self.inner.depth_and_order & 3)
    }

    /// Returns the internal item or array. For use only by NestedSetVisitor and the fingerprint
    /// cache.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:308 NestedSet.getChildren
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:313 NestedSet.getChildrenInterruptibly
    // PORT: skipped, serialization (no futures, so this never blocks or is interrupted).
    pub(crate) fn get_children(&self) -> &Children<E> {
        &self.inner.children
    }

    /// Returns true if the set is empty. Runs in O(1) time (i.e. does not flatten the set).
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:363 NestedSet.isEmpty
    pub fn is_empty(&self) -> bool {
        matches!(self.inner.children, Children::Empty)
    }

    /// Returns true if the set has exactly one element.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:370 NestedSet.isSingleton
    pub fn is_singleton(&self) -> bool {
        Self::is_singleton_children(&self.inner.children)
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:374 NestedSet.isSingleton
    fn is_singleton_children(children: &Children<E>) -> bool {
        matches!(children, Children::Leaf(_))
    }

    /// Returns the approximate depth of the nested set graph. The empty set has depth zero. A
    /// leaf node with a single element has depth 1. A non-leaf node has a depth one greater than
    /// its deepest successor.
    ///
    /// This function may return an overapproximation of the true depth if the NestedSet was
    /// derived from the result of calling `get_non_leaves` or `split_if_exceeds_maximum_size`.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:388 NestedSet.getApproxDepth
    pub fn get_approx_depth(&self) -> i32 {
        ((self.inner.depth_and_order as u32) >> 2) as i32
    }

    /// Returns true if this set depends on data from storage.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:393 NestedSet.isFromStorage
    // PORT: skipped, serialization (never from storage).
    pub fn is_from_storage(&self) -> bool {
        false
    }

    /// Returns true if the contents of this set are currently available in memory.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:403 NestedSet.isReady
    // PORT: skipped, serialization (always in memory).
    pub fn is_ready(&self) -> bool {
        true
    }

    /// Returns the single element; only call this if `is_singleton` returns true.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:420 NestedSet.getSingleton
    pub fn get_singleton(&self) -> &E {
        check_state!(self.is_singleton());
        match &self.inner.children {
            Children::Leaf(e) => e,
            _ => unreachable!(),
        }
    }

    /// Returns a Node, an opaque reference to the logical node of the DAG that this NestedSet
    /// represents.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:731 NestedSet.toNode
    pub fn to_node(&self) -> NestedSetNode<E>
    where
        E: Clone,
    {
        NestedSetNode(self.inner.children.clone())
    }
}

impl<E: Clone + Eq + Hash> NestedSet<E> {
    /// The union constructor (Java: `NestedSet(Order, Set<E>, Collection<NestedSet<E>>,
    /// InterruptStrategy)`). `direct` iterates in insertion order.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:181 NestedSet.<init>
    // PORT: skipped, serialization (InterruptStrategy only matters for deserialization futures).
    pub(crate) fn from_direct_and_transitive(
        order: Order,
        direct: &IndexSet<E>,
        transitive: &[NestedSet<E>],
    ) -> Self {
        // The iteration order of these collections is the order in which we add the items.
        let direct_order: Vec<&E> = match order {
            Order::LINK_ORDER => direct.iter().rev().collect(),
            Order::STABLE_ORDER | Order::COMPILE_ORDER | Order::NAIVE_LINK_ORDER => {
                direct.iter().collect()
            }
        };
        // True if we visit the direct members before the transitive members.
        let preorder = match order {
            Order::LINK_ORDER | Order::STABLE_ORDER | Order::COMPILE_ORDER => false,
            Order::NAIVE_LINK_ORDER => true,
        };

        // Remember children we extracted from one-element subsets. Otherwise we can end up with
        // two of the same child, which is a problem for the fast path in toList().
        let mut already_inserted: AlreadyInserted<'_, E> = AlreadyInserted::Nothing;
        // The candidate array of children.
        let mut children: Vec<Children<E>> = Vec::with_capacity(direct.len() + transitive.len());
        let mut approx_depth: i32 = 0;
        let mut shallow = true; // whether true depth < 3

        for pass in 0..=1 {
            if (pass == 0) == preorder && !direct.is_empty() {
                // PORT: Java rejects Object[] and ByteString members at runtime; the Rust type
                // system keeps elements and arrays apart, so there is nothing to check.
                for member in &direct_order {
                    let contained = match &already_inserted {
                        AlreadyInserted::Nothing => false,
                        AlreadyInserted::Direct => direct.contains(*member),
                        AlreadyInserted::Hoisted(h) => h.contains(*member),
                    };
                    if !contained {
                        children.push(Children::Leaf((*member).clone()));
                        approx_depth = approx_depth.max(2);
                    }
                }
                already_inserted = AlreadyInserted::Direct;
            } else if (pass == 1) == preorder && !transitive.is_empty() {
                let mut hoisted: Option<HashSet<&E>> = None;
                for subset in transitive {
                    approx_depth = approx_depth.max(1 + subset.get_approx_depth());
                    match subset.get_children() {
                        Children::Array(a) => {
                            if a.len() < 2 {
                                panic!("AssertionError: {}", a.len());
                            }
                            children.push(Children::Array(Arc::clone(a)));
                            shallow = false;
                        }
                        Children::Empty => {
                            // Java: EMPTY_CHILDREN is an Object[] of length 0.
                            panic!("AssertionError: 0");
                        }
                        Children::Leaf(c) => {
                            let contained = match &already_inserted {
                                AlreadyInserted::Nothing => false,
                                AlreadyInserted::Direct => direct.contains(c),
                                AlreadyInserted::Hoisted(h) => h.contains(c),
                            };
                            if !contained && hoisted.get_or_insert_with(HashSet::new).insert(c) {
                                children.push(Children::Leaf(c.clone()));
                            }
                        }
                    }
                }
                already_inserted = match hoisted {
                    None => AlreadyInserted::Nothing,
                    Some(h) => AlreadyInserted::Hoisted(h),
                };
            }
        }

        // n == |successors|
        let n = children.len();
        let final_children = if n == 0 {
            approx_depth = 0;
            Children::Empty
        } else if n == 1 {
            // If we ended up wrapping exactly one item or one other set, dereference it.
            approx_depth -= 1;
            children.pop().unwrap()
        } else {
            Children::Array(Arc::from(children))
        };
        let depth_and_order = (approx_depth << 2) | order.ordinal();
        let cached = if shallow {
            None
        } else {
            Some(Mutex::new(Weak::new()))
        };
        NestedSet {
            inner: Arc::new(NestedSetInner {
                depth_and_order,
                children: final_children,
                cached,
            }),
        }
    }

    /// Same as [`NestedSet::to_list`].
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:430 NestedSet.toListInterruptibly
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:455 NestedSet.toListWithTimeout
    // PORT: skipped, serialization (InterruptedException, TimeoutException and
    // MissingFingerprintValueException only arise from deserialization futures; toListWithTimeout
    // is therefore not ported separately).
    pub fn to_list_interruptibly(&self) -> Arc<Vec<E>> {
        self.to_list()
    }

    /// Returns an immutable list of all unique elements of this set (including subsets) in an
    /// implementation-specified order.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:481 NestedSet.toList
    pub fn to_list(&self) -> Arc<Vec<E>> {
        self.actual_children_to_list(&self.inner.children)
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:489 NestedSet.actualChildrenToList
    fn actual_children_to_list(&self, actual_children: &Children<E>) -> Arc<Vec<E>> {
        match actual_children {
            Children::Empty => Arc::new(Vec::new()),
            Children::Leaf(e) => Arc::new(vec![e.clone()]),
            // PORT: the LINK_ORDER reversal (`list.reverse()`) happens inside expand_with_caching
            // so that the cached list is the final one.
            Children::Array(array) => self.expand_with_caching(array),
        }
    }

    /// Returns an immutable set of all unique elements of this set (including subsets) in an
    /// implementation-specified order.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:504 NestedSet.toSet
    pub fn to_set(&self) -> IndexSet<E> {
        self.to_list().iter().cloned().collect()
    }

    /// Important: This does a full traversal of the nested set if it's not been previously
    /// traversed.
    ///
    /// Returns the size of the nested set.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:513 NestedSet.memoizedFlattenAndGetSize
    pub fn memoized_flatten_and_get_size(&self) -> i32 {
        if self.inner.cached.is_none() {
            return match &self.inner.children {
                Children::Array(array) => array.len() as i32,
                // Java: EMPTY_CHILDREN is an Object[] of length 0.
                Children::Empty => 0,
                Children::Leaf(_) => 1,
            };
        }
        self.to_list().len() as i32
    }

    /// Returns true if this set is equal to `other` based on the top-level elements and object
    /// identity (==) of direct subsets. As such, this function can fail to equate `self` with
    /// another `NestedSet` that holds the same elements. It will never fail to detect that two
    /// `NestedSet`s are different, however.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:532 NestedSet.shallowEquals
    pub fn shallow_equals(&self, other: Option<&NestedSet<E>>) -> bool {
        let Some(other) = other else {
            return false;
        };
        if self == other {
            return true;
        }
        self.get_order() == other.get_order()
            && (self.inner.children == other.inner.children
                || (!self.is_singleton()
                    && !other.is_singleton()
                    && Self::arrays_equal(&self.inner.children, &other.inner.children)))
    }

    /// Java `Arrays.equals((Object[]) children, (Object[]) other.children)`, where both values
    /// are `Object[]` (EMPTY_CHILDREN included).
    fn arrays_equal(a: &Children<E>, b: &Children<E>) -> bool {
        fn as_slice<E>(c: &Children<E>) -> Option<&[Children<E>]> {
            match c {
                Children::Empty => Some(&[]),
                Children::Array(arr) => Some(arr),
                Children::Leaf(_) => None,
            }
        }
        match (as_slice(a), as_slice(b)) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        }
    }

    /// Returns a hash code that produces a notion of identity that is consistent with
    /// `shallow_equals`. In other words, if two `NestedSet`s are equal according to
    /// `shallow_equals`, then they return the same `shallow_hash_code`.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:556 NestedSet.shallowHashCode
    // PORT: Java's value mixes identity hash codes and is not reproducible; any hash consistent
    // with shallow_equals is equivalent.
    pub fn shallow_hash_code(&self) -> i32 {
        let mut hasher = DefaultHasher::new();
        self.get_order().hash(&mut hasher);
        match &self.inner.children {
            Children::Leaf(e) => e.hash(&mut hasher),
            Children::Empty => {}
            Children::Array(array) => {
                for c in array.iter() {
                    c.hash(&mut hasher);
                }
            }
        }
        hasher.finish() as i32
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:582 NestedSet.expandWithCaching
    fn expand_with_caching(&self, children: &Arc<[Children<E>]>) -> Arc<Vec<E>> {
        let Some(cached) = &self.inner.cached else {
            // Java: ImmutableList.copyOf(new ArraySharingCollection<>(children)). Without
            // transitive members every child is an element.
            let mut list: Vec<E> = children
                .iter()
                .map(|c| match c {
                    Children::Leaf(e) => e.clone(),
                    _ => unreachable!("array without transitive members holds only elements"),
                })
                .collect();
            if self.get_order() == Order::LINK_ORDER {
                list.reverse();
            }
            return Arc::new(list);
        };

        // Java: synchronized (this), with the field read again under the lock.
        let mut guard = cached.lock();
        if let Some(result) = guard.upgrade() {
            return result;
        }
        let mut list = Self::expand(children);
        if self.get_order() == Order::LINK_ORDER {
            list.reverse();
        }
        let result = Arc::new(list);
        *guard = Arc::downgrade(&result);
        result
    }

    /// Implementation of `to_list` for sets with > 1 element.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:606 NestedSet.expand
    fn expand(children: &Arc<[Children<E>]>) -> Vec<E> {
        let mut members: IndexSet<E> = IndexSet::with_capacity(128);
        let mut sets: HashSet<usize> = HashSet::with_capacity(128);
        sets.insert(array_identity(children));
        Self::walk(&mut sets, &mut members, children);
        members.into_iter().collect()
    }

    /// Performs a depth-first traversal of `children`, tracking visited arrays in `sets` and
    /// visited leaves in `members`.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:618 NestedSet.walk
    // PORT: Java recurses; this uses an explicit stack (same visiting order) so that deep sets
    // cannot overflow the native stack.
    fn walk(sets: &mut HashSet<usize>, members: &mut IndexSet<E>, children: &Arc<[Children<E>]>) {
        let mut stack: Vec<(&[Children<E>], usize)> = vec![(&children[..], 0)];
        while let Some(top) = stack.last_mut() {
            let array: &[Children<E>] = top.0;
            if top.1 == array.len() {
                stack.pop();
                continue;
            }
            let child = &array[top.1];
            top.1 += 1;
            match child {
                Children::Array(a) => {
                    if sets.insert(array_identity(a)) {
                        stack.push((&a[..], 0));
                    }
                }
                Children::Leaf(e) => {
                    if !members.contains(e) {
                        members.insert(e.clone());
                    }
                }
                Children::Empty => unreachable!("EMPTY_CHILDREN is never nested"),
            }
        }
    }

    /// Returns a new NestedSet containing the same elements, but represented using a graph node
    /// whose out-degree does not exceed `max_degree`, which must be at least 2. The operation is
    /// shallow, not deeply recursive. The resulting set's iteration order is undefined.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:666 NestedSet.splitIfExceedsMaximumSize
    pub fn split_if_exceeds_maximum_size(&self, max_degree: i32) -> NestedSet<E> {
        check_argument!(max_degree >= 2, "maxDegree must be at least 2");
        let succs = match &self.inner.children {
            Children::Array(succs) => succs,
            // Java: EMPTY_CHILDREN is an Object[] of length 0 <= maxDegree.
            Children::Empty | Children::Leaf(_) => return self.clone(),
        };
        let nsuccs = succs.len();
        let max_degree = max_degree as usize;
        if nsuccs <= max_degree {
            return self.clone();
        }
        let pieces: Vec<Children<E>> = succs
            .chunks(max_degree)
            .map(|piece| Children::Array(Arc::from(piece.to_vec())))
            .collect();
        let depth = self.get_approx_depth() + 1; // may be an overapproximation

        // TODO(adonovan): (preexisting): if the last piece is a singleton, it must be inlined.

        // Each piece is now smaller than maxDegree, but there may be many pieces.
        // Recursively split pieces. (The recursion affects only the root; it
        // does not traverse into successors.) In practice, maxDegree is large
        // enough that the recursion rarely does any work.
        NestedSet::with_children(self.get_order(), depth, Children::Array(Arc::from(pieces)))
            .split_if_exceeds_maximum_size(max_degree as i32)
    }

    /// Returns the list of this node's successors that are themselves non-leaf nodes.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:693 NestedSet.getNonLeaves
    pub fn get_non_leaves(&self) -> Vec<NestedSet<E>> {
        let Children::Array(array) = &self.inner.children else {
            return Vec::new();
        };
        let mut res = Vec::new();
        for c in array.iter() {
            if let Children::Array(_) = c {
                let depth = self.get_approx_depth() - 1; // possible overapproximation
                res.push(NestedSet::with_children(self.get_order(), depth, c.clone()));
            }
        }
        res
    }

    /// Returns the list of elements (leaf nodes) of this set that are reached by following at
    /// most one graph edge.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:713 NestedSet.getLeaves
    pub fn get_leaves(&self) -> Vec<E> {
        match &self.inner.children {
            Children::Leaf(e) => vec![e.clone()],
            Children::Empty => Vec::new(),
            Children::Array(array) => array
                .iter()
                .filter_map(|c| match c {
                    Children::Leaf(e) => Some(e.clone()),
                    _ => None,
                })
                .collect(),
        }
    }
}

// Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:565 NestedSet.toString
impl<E: Clone + Eq + Hash + fmt::Display> fmt::Display for NestedSet<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Children::Leaf(e) = &self.inner.children {
            return write!(f, "[{e}]");
        }
        let elems = self.to_list();
        let write_list = |f: &mut fmt::Formatter<'_>, items: &[E]| -> fmt::Result {
            f.write_str("[")?;
            for (i, e) in items.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{e}")?;
            }
            f.write_str("]")
        };
        if elems.len() <= MAX_ELEMENTS_TO_STRING {
            return write_list(f, &elems);
        }
        write_list(f, &elems[..MAX_ELEMENTS_TO_STRING])?;
        write!(f, " (truncated, full size {})", elems.len())
    }
}

impl<E: Clone + Eq + Hash + fmt::Debug> fmt::Debug for NestedSet<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NestedSet({}, ", self.get_order())?;
        f.debug_list().entries(self.to_list().iter()).finish()?;
        f.write_str(")")
    }
}

/// A Node is an opaque reference to a logical node of the NestedSet DAG.
///
/// The only operation it supports is equality. Branch nodes are equal if and only if they refer
/// to the same logical graph node. Leaf nodes are equal if they refer to equal elements. Two
/// distinct NestedSets may have equal elements.
///
/// Node is provided so that clients can implement their own traversals and detect when they have
/// encountered a subgraph already visited.
// Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:745 NestedSet.Node
pub struct NestedSetNode<E>(pub(crate) Children<E>);

impl<E: Clone> Clone for NestedSetNode<E> {
    fn clone(&self) -> Self {
        NestedSetNode(self.0.clone())
    }
}

// Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:758 Node.equals
impl<E: PartialEq> PartialEq for NestedSetNode<E> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<E: Eq> Eq for NestedSetNode<E> {}

// Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:753 Node.hashCode
impl<E: Hash> Hash for NestedSetNode<E> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

// Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:763 Node.toString
impl<E: Hash> fmt::Debug for NestedSetNode<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut hasher = DefaultHasher::new();
        self.0.hash(&mut hasher);
        // intentionally opaque
        write!(f, "NestedSet.Node@{}", hasher.finish() as i32)
    }
}

impl<E> NestedSetNode<E> {
    /// Returns the element if this node is a leaf (Java: `!(node instanceof Object[])`).
    pub fn as_leaf(&self) -> Option<&E> {
        match &self.0 {
            Children::Leaf(e) => Some(e),
            _ => None,
        }
    }
}

/// One level of a [`NestedSet`]'s node, see [`NestedSet::children`].
// W4-analysis: needed to convert NestedSets to Starlark depsets with the same structure.
#[derive(Clone)]
pub enum NestedSetChildren<E> {
    Empty,
    Leaf(E),
    Array(Vec<NestedSetChild<E>>),
}

/// An entry of a node's array, see [`NestedSet::children`].
// W4-analysis: needed to convert NestedSets to Starlark depsets with the same structure.
#[derive(Clone)]
pub enum NestedSetChild<E> {
    Leaf(E),
    Set(NestedSet<E>),
}
