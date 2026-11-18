//! Port of `NestedSetBuilder.java`.

use std::hash::Hash;

use blaze_util::check_argument;
use indexmap::IndexSet;

use super::nested_set::Children;
use super::{NestedSet, Order};

/// Java: the sealed subclasses `DefaultNestedSetBuilder` and `LinkOrderNestedSetBuilder`, which
/// differ only in how they hold transitive sets.
enum TransitiveSets<E> {
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:288 DefaultNestedSetBuilder
    /// Deduplicates transitive sets (by identity) as they are added.
    Default(Option<IndexSet<NestedSet<E>>>),
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:353 LinkOrderNestedSetBuilder
    /// A specialized builder for [`Order::LINK_ORDER`] that reverses the order of transitive
    /// inputs *before* deduplication.
    ///
    /// The default builder deduplicates transitive sets as they are added. This one, however,
    /// delays deduplication until `build()` is called. Furthermore, for LINK_ORDER, the input
    /// order of transitive sets is reversed to implement a right-to-left traversal. The order of
    /// these operations (reversal and deduplication) affects the final result.
    ///
    /// Consider the transitive inputs `<A, B, A>`, where `A` and `B` are NestedSets with no
    /// common elements.
    ///
    /// - Deduplicate-then-reverse: If we deduplicate first and then reverse, the intermediate
    ///   result is `<B, A>`. When this is passed to `NestedSet::to_list`, the final output is
    ///   `<toList(A), toList(B)>` (because `to_list` also reverses LINK_ORDER sets).
    /// - Reverse-then-deduplicate: If we reverse first and then deduplicate, the intermediate
    ///   result is `<A, B>`. When passed to `NestedSet::to_list`, the final output is
    ///   `<toList(B), toList(A)>`.
    ///
    /// This builder implements the reverse-then-deduplicate strategy. This choice is important
    /// for consistency when deeply equivalent, but distinct, NestedSet instances are used as
    /// transitive inputs.
    ///
    /// Consider transitive inputs `<A, B, A*>`, where `A*` is deeply equivalent to `A` but is a
    /// different instance. With both reverse-then-deduplicate and deduplicate-then-reverse, the
    /// intermediate result is `<A*, B, A>`. `to_list`, which deduplicates `A` element-wise, then
    /// produces `<toList(B), toList(A*)>`. By deep-equality of `A` and `A*`, this result is
    /// equivalent to `<toList(B), toList(A)>` which is only consistent with
    /// reverse-then-deduplicate.
    ///
    /// In summary, this builder ensures that `to_list`, when used with a NestedSet built by this
    /// builder, behaves consistently, regardless of whether transitive inputs are the same
    /// instance or are deeply equivalent (but distinct) instances.
    Link(Option<Vec<NestedSet<E>>>),
}

/// A builder for nested sets.
///
/// The builder supports the standard builder interface (that is, `add`, `add_all` and
/// `add_transitive` followed by `build`), in addition to shortcut method `wrap`. Any duplicate
/// elements will be inserted as-is, and pruned later on during the traversal of the actual
/// NestedSet.
// Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:41 NestedSetBuilder
pub struct NestedSetBuilder<E> {
    order: Order,
    items: Option<IndexSet<E>>,
    transitive_sets: TransitiveSets<E>,
}

impl<E> NestedSetBuilder<E> {
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:45 NestedSetBuilder.newBuilder
    pub fn new_builder(order: Order) -> Self {
        let transitive_sets = match order {
            Order::LINK_ORDER => TransitiveSets::Link(None),
            _ => TransitiveSets::Default(None),
        };
        NestedSetBuilder {
            order,
            items: None,
            transitive_sets,
        }
    }

    /// Returns the order used by this builder.
    ///
    /// This is useful for testing for incompatibilities (via [`Order::is_compatible`]) without
    /// catching a panic from `add_transitive`.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:62 NestedSetBuilder.getOrder
    pub fn get_order(&self) -> Order {
        self.order
    }

    /// Returns whether the set to be built is empty.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:67 NestedSetBuilder.isEmpty
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:310 DefaultNestedSetBuilder.isEmpty
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:387 LinkOrderNestedSetBuilder.isEmpty
    pub fn is_empty(&self) -> bool {
        self.items.is_none()
            && match &self.transitive_sets {
                TransitiveSets::Default(t) => t.is_none(),
                TransitiveSets::Link(t) => t.is_none(),
            }
    }

    /// Creates a builder for stable order nested sets.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:244 NestedSetBuilder.stableOrder
    pub fn stable_order() -> Self {
        Self::new_builder(Order::STABLE_ORDER)
    }

    /// Creates a builder for compile order nested sets.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:251 NestedSetBuilder.compileOrder
    pub fn compile_order() -> Self {
        Self::new_builder(Order::COMPILE_ORDER)
    }

    /// Creates a builder for link order nested sets.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:258 NestedSetBuilder.linkOrder
    pub fn link_order() -> Self {
        Self::new_builder(Order::LINK_ORDER)
    }

    /// Creates a builder for naive link order nested sets.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:265 NestedSetBuilder.naiveLinkOrder
    pub fn naive_link_order() -> Self {
        Self::new_builder(Order::NAIVE_LINK_ORDER)
    }

    /// Creates an empty nested set.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:237 NestedSetBuilder.emptySet
    pub fn empty_set(order: Order) -> NestedSet<E> {
        order.empty_set()
    }
}

impl<E: Clone + Eq + Hash> NestedSetBuilder<E> {
    /// Adds a direct member to the set to be built.
    ///
    /// The relative left-to-right order of direct members is preserved from the sequence of
    /// calls to `add` and `add_all`. Since the traversal [`Order`] controls whether direct members
    /// appear before or after transitive ones, the interleaving of `add`/`add_all` with
    /// `add_transitive` does not matter.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:83 NestedSetBuilder.add
    pub fn add(&mut self, element: E) -> &mut Self {
        self.items.get_or_insert_with(IndexSet::new).insert(element);
        self
    }

    /// Adds a sequence of direct members to the set to be built. Equivalent to invoking `add`
    /// for each item in `elements`, in order.
    ///
    /// The relative left-to-right order of direct members is preserved from the sequence of
    /// calls to `add` and `add_all`. Since the traversal [`Order`] controls whether direct members
    /// appear before or after transitive ones, the interleaving of `add`/`add_all` with
    /// `add_transitive` does not matter.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:105 NestedSetBuilder.addAll
    pub fn add_all<I: IntoIterator<Item = E>>(&mut self, elements: I) -> &mut Self {
        let mut elements = elements.into_iter().peekable();
        if self.items.is_none() {
            if elements.peek().is_none() {
                return self; // avoid allocating an empty set
            }
            self.items = Some(IndexSet::with_capacity(elements.size_hint().0));
        }
        self.items.as_mut().unwrap().extend(elements);
        self
    }

    /// Adds a nested set as a transitive member to the set to be built.
    ///
    /// The relative left-to-right order of transitive members is preserved from the sequence of
    /// calls to `add_transitive`. Since the traversal [`Order`] controls whether direct members
    /// appear before or after transitive ones, the interleaving of `add`/`add_all` with
    /// `add_transitive` does not matter.
    ///
    /// The [`Order`] of the added set must be compatible with the order of this builder (see
    /// [`Order::is_compatible`]). This is true even if the added set is empty. Strictly speaking,
    /// it is not technically necessary that two nested sets have compatible orders for them to be
    /// combined as part of one larger set. But checking for it helps readability and protects
    /// against bugs. Since [`Order::STABLE_ORDER`] is compatible with everything, it effectively
    /// disables the check. This can be used as an escape hatch to mix and match the set
    /// arbitrarily, including sharing the set as part of multiple other larger sets that have
    /// disagreeing orders.
    ///
    /// The relative order of the elements of an added set are preserved, unless it has
    /// duplicates or overlaps with other added sets, or its order is different from that of the
    /// builder.
    ///
    /// Panics (Java: `IllegalArgumentException`) if the order of `subset` is not compatible with
    /// the order of this builder.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:143 NestedSetBuilder.addTransitive
    pub fn add_transitive(&mut self, subset: &NestedSet<E>) -> &mut Self {
        check_argument!(
            self.get_order().is_compatible(subset.get_order()),
            "Order mismatch: {} != {}",
            subset.get_order().get_starlark_name(),
            self.get_order().get_starlark_name()
        );
        if !subset.is_empty() {
            self.add_transitive_impl(subset);
        }
        self
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:297 DefaultNestedSetBuilder.addTransitiveImpl
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:362 LinkOrderNestedSetBuilder.addTransitiveImpl
    fn add_transitive_impl(&mut self, subset: &NestedSet<E>) {
        match &mut self.transitive_sets {
            TransitiveSets::Default(t) => {
                t.get_or_insert_with(IndexSet::new).insert(subset.clone());
            }
            TransitiveSets::Link(t) => {
                t.get_or_insert_with(Vec::new).push(subset.clone());
            }
        }
    }

    /// Builds the actual nested set.
    ///
    /// This method may be called multiple times with interleaved `add`, `add_all` and
    /// `add_transitive` calls.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:165 NestedSetBuilder.build
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:181 NestedSetBuilder.buildInternal
    pub fn build(&self) -> NestedSet<E> {
        if self.is_empty() {
            return self.get_order().empty_set();
        }

        let no_items = IndexSet::new();
        let direct: &IndexSet<E> = self.items.as_ref().unwrap_or(&no_items);
        let transitive = self.get_transitive();

        // When there is exactly one transitive set, we can reuse it if its order matches and
        // either there are no direct members, or the only direct member equals the transitive
        // set's singleton.
        if transitive.len() == 1 && direct.len() <= 1 {
            let candidate = &transitive[0];
            if candidate.get_order() == self.get_order()
                && (direct.is_empty()
                    || matches!(candidate.get_children(),
                        Children::Leaf(c) if c == direct.get_index(0).unwrap()))
            {
                return candidate.clone();
            }
        }

        NestedSet::from_direct_and_transitive(self.get_order(), direct, &transitive)
    }

    /// Same as [`NestedSetBuilder::build`].
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:177 NestedSetBuilder.buildInterruptibly
    // PORT: skipped, serialization (InterruptedException only arises from deserialization
    // futures).
    pub fn build_interruptibly(&self) -> NestedSet<E> {
        self.build()
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:305 DefaultNestedSetBuilder.getTransitive
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:371 LinkOrderNestedSetBuilder.getTransitive
    fn get_transitive(&self) -> Vec<NestedSet<E>> {
        match &self.transitive_sets {
            TransitiveSets::Default(None) | TransitiveSets::Link(None) => Vec::new(),
            TransitiveSets::Default(Some(t)) => t.iter().cloned().collect(),
            TransitiveSets::Link(Some(t)) => {
                let size = t.len();
                if size == 1 {
                    return t.clone();
                }
                let mut reversed_and_deduped: IndexSet<NestedSet<E>> =
                    IndexSet::with_capacity(size);
                for set in t.iter().rev() {
                    reversed_and_deduped.insert(set.clone());
                }
                reversed_and_deduped.into_iter().collect()
            }
        }
    }

    /// Creates a nested set from a given list of items.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:215 NestedSetBuilder.wrap
    // PORT: Java memoizes STABLE_ORDER wraps of the same ImmutableList instance (a weak-keyed
    // identity cache) so they return the same NestedSet instance. That only affects instance
    // sharing, not contents, and Rust has no list identity to key on; skipped.
    pub fn wrap<I: IntoIterator<Item = E>>(order: Order, wrapped_items: I) -> NestedSet<E> {
        let mut wrapped_items = wrapped_items.into_iter().peekable();
        if wrapped_items.peek().is_none() {
            return order.empty_set();
        }
        NestedSetBuilder::new_builder(order)
            .add_all(wrapped_items)
            .build()
    }

    /// Creates a nested set with the given list of items as its elements.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:230 NestedSetBuilder.create
    pub fn create<I: IntoIterator<Item = E>>(order: Order, elems: I) -> NestedSet<E> {
        Self::wrap(order, elems)
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:269 NestedSetBuilder.fromNestedSet
    pub fn from_nested_set(set: &NestedSet<E>) -> Self {
        let mut builder = NestedSetBuilder::new_builder(set.get_order());
        builder.add_transitive(set);
        builder
    }

    /// Creates a Builder with the contents of 'sets'.
    ///
    /// If 'sets' is empty, a stable-order empty NestedSet is returned.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetBuilder.java:278 NestedSetBuilder.fromNestedSets
    pub fn from_nested_sets(sets: &[NestedSet<E>]) -> Self {
        let Some(first_set) = sets.first() else {
            return Self::stable_order();
        };
        let mut result = NestedSetBuilder::new_builder(first_set.get_order());
        for set in sets {
            result.add_transitive(set);
        }
        result
    }
}
