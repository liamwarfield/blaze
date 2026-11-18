//! Port of `NestedSetVisitor.java`.

use std::marker::PhantomData;

use super::NestedSet;
use super::nested_set::{Children, NestedSetNode};

/// For each element of the NestedSet the `Receiver` will receive one element during the
/// visitation.
// Java: com/google/devtools/build/lib/collect/nestedset/NestedSetVisitor.java:34 NestedSetVisitor.Receiver
pub trait NestedSetVisitorReceiver<E> {
    fn accept(&mut self, arg: &E);
}

impl<E, F: FnMut(&E)> NestedSetVisitorReceiver<E> for F {
    fn accept(&mut self, arg: &E) {
        self(arg)
    }
}

/// Allows [`NestedSetVisitor`] to keep track of the seen nodes and transitive sets.
// Java: com/google/devtools/build/lib/collect/nestedset/NestedSetVisitor.java:83 NestedSetVisitor.VisitedState
pub trait NestedSetVisitorVisitedState<E> {
    /// Determines whether the given node needs to be visited, recording the visitation attempt
    /// if this VisitedState deduplicates visitations.
    ///
    /// A return of `true` means that:
    ///
    /// - If `node` is a leaf element: `Receiver::accept` will be invoked with `node`.
    /// - If `node` is a non-leaf: visitation will traverse its children.
    ///
    /// Java passes the raw `Object` (an element or an `Object[]`); the port passes the equivalent
    /// [`NestedSetNode`] (elements compare by value, non-leaves by identity).
    fn need_to_visit(&mut self, node: &NestedSetNode<E>) -> bool;
}

impl<E, F: FnMut(&NestedSetNode<E>) -> bool> NestedSetVisitorVisitedState<E> for F {
    fn need_to_visit(&mut self, node: &NestedSetNode<E>) -> bool {
        self(node)
    }
}

/// NestedSetVisitor facilitates a transitive visitation over a NestedSet. The callback may be
/// called from multiple threads, and must be thread-safe.
///
/// The visitation is iterative: The caller may invoke a NestedSet within the top-level NestedSet
/// in any order.
// Java: com/google/devtools/build/lib/collect/nestedset/NestedSetVisitor.java:28 NestedSetVisitor
pub struct NestedSetVisitor<E, R, V> {
    callback: R,
    visited: V,
    _element: PhantomData<fn(&E)>,
}

impl<E: Clone, R: NestedSetVisitorReceiver<E>, V: NestedSetVisitorVisitedState<E>>
    NestedSetVisitor<E, R, V>
{
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetVisitor.java:42 NestedSetVisitor.<init>
    pub fn new(callback: R, visited: V) -> Self {
        NestedSetVisitor {
            callback,
            visited,
            _element: PhantomData,
        }
    }

    /// Transitively visit a nested set.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetVisitor.java:52 NestedSetVisitor.visit
    // PORT: skipped, serialization (InterruptedException only arises from deserialization
    // futures, so this is infallible).
    pub fn visit(&mut self, nested_set: &NestedSet<E>) {
        // We can short-circuit empty nested set visitation here, avoiding load on the shared map
        // VisitedState#seenNodes.
        if !nested_set.is_empty() {
            self.visit_raw(nested_set.get_children());
        }
    }

    /// Visit every entry in a collection.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetVisitor.java:61 NestedSetVisitor.visit
    pub fn visit_collection<'a, I>(&mut self, collection: I)
    where
        I: IntoIterator<Item = &'a E>,
        E: 'a,
    {
        for e in collection {
            if self
                .visited
                .need_to_visit(&NestedSetNode(Children::Leaf(e.clone())))
            {
                self.callback.accept(e);
            }
        }
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetVisitor.java:70 NestedSetVisitor.visitRaw
    fn visit_raw(&mut self, node: &Children<E>) {
        if self.visited.need_to_visit(&NestedSetNode(node.clone())) {
            match node {
                Children::Array(array) => {
                    for child in array.iter() {
                        self.visit_raw(child);
                    }
                }
                Children::Leaf(e) => self.callback.accept(e),
                Children::Empty => unreachable!("empty sets are short-circuited in visit"),
            }
        }
    }
}
