//! Port of `Order.java`.

use std::fmt;

use super::NestedSet;

/// Type of a nested set (defines order).
///
/// STABLE_ORDER: an unspecified traversal order. Use when the order of elements does not matter.
/// In Starlark it is called "default"; its older deprecated name is "stable".
///
/// COMPILE_ORDER: left-to-right postorder. In Starlark it is called "postorder"; its older
/// deprecated name is "compile".
///
/// For example, for the nested set {B, D, {A, C}}, the iteration order is "A C B D"
/// (child-first).
///
/// This type of set would typically be used for artifacts where elements of nested sets go before
/// the direct members of a set, for example in the case of Javascript dependencies.
///
/// LINK_ORDER: a variation of left-to-right preorder that enforces topological sorting. In
/// Starlark it is called "topological"; its older deprecated name is "link".
///
/// For example, for the nested set {A, C, {B, D}}, the iteration order is "A C B D"
/// (parent-first).
///
/// This type of set would typically be used for artifacts where elements of nested sets go after
/// the direct members of a set, for example when providing a list of libraries to the C++ compiler.
///
/// The custom ordering has the property that elements of nested sets always come before elements
/// of descendant nested sets. Left-to-right order is preserved if possible, both for items and for
/// references to nested sets.
///
/// The left-to-right pre-order-like ordering is implemented by running a right-to-left postorder
/// traversal and then reversing the result.
///
/// The reason naive left-to-right preordering is not used here is that it does not handle
/// diamond-like structures properly. For example, take the following structure (nesting
/// downwards):
///
/// ```text
///    A
///   / \
///  B   C
///   \ /
///    D
/// ```
///
/// Naive preordering would produce "A B D C", which does not preserve the "parent before child"
/// property: C is a parent of D, so C should come before D. Either "A B C D" or "A C B D" would be
/// acceptable. This implementation returns the first option of the two so that left-to-right order
/// is preserved.
///
/// In case the nested sets form a tree, the ordering algorithm is equivalent to standard
/// left-to-right preorder.
///
/// Sometimes it may not be possible to preserve left-to-right order:
///
/// ```text
///      A
///    /   \
///   B     C
///  / \   / \
///  \   E   /
///   \     /
///    \   /
///      D
/// ```
///
/// The left branch (B) would indicate "D E" ordering and the right branch (C) dictates "E D". In
/// such cases ordering is decided by the rightmost branch because of the list reversing behind the
/// scenes, so the ordering in the final enumeration will be "E D".
///
/// NAIVE_LINK_ORDER: a left-to-right preordering. In Starlark it is called "preorder"; its older
/// deprecated name is "naive_link".
///
/// For example, for the nested set {B, D, {A, C}}, the iteration order is "B D A C".
///
/// The order is called naive because it does no special treatment of dependency graphs that are
/// not trees. For such graphs the property of parent-before-dependencies in the iteration order
/// will not be upheld. For example, the diamond-shape graph A->{B, C}, B->{D}, C->{D} will be
/// enumerated as "A B D C" rather than "A B C D" or "A C B D".
///
/// The difference from LINK_ORDER is that this order gives priority to left-to-right order over
/// dependencies-after-parent ordering. Note that the latter is usually more important, so please
/// use LINK_ORDER whenever possible.
// Java: com/google/devtools/build/lib/collect/nestedset/Order.java:104 Order
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Order {
    STABLE_ORDER,
    COMPILE_ORDER,
    LINK_ORDER,
    NAIVE_LINK_ORDER,
}

/// Error of [`Order::parse`]; Java throws `IllegalArgumentException("Invalid order: " + name)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderParseError {
    pub message: String,
}

impl fmt::Display for OrderParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for OrderParseError {}

impl Order {
    /// Java: `Order.values()` (ordinal order).
    pub const VALUES: [Order; 4] = [
        Order::STABLE_ORDER,
        Order::COMPILE_ORDER,
        Order::LINK_ORDER,
        Order::NAIVE_LINK_ORDER,
    ];

    /// Java: `Enum.ordinal()`.
    pub fn ordinal(self) -> i32 {
        match self {
            Order::STABLE_ORDER => 0,
            Order::COMPILE_ORDER => 1,
            Order::LINK_ORDER => 2,
            Order::NAIVE_LINK_ORDER => 3,
        }
    }

    /// Java: `Enum.name()`.
    pub fn name(self) -> &'static str {
        match self {
            Order::STABLE_ORDER => "STABLE_ORDER",
            Order::COMPILE_ORDER => "COMPILE_ORDER",
            Order::LINK_ORDER => "LINK_ORDER",
            Order::NAIVE_LINK_ORDER => "NAIVE_LINK_ORDER",
        }
    }

    /// Returns an empty set of the given ordering.
    // Java: com/google/devtools/build/lib/collect/nestedset/Order.java:151 Order.emptySet
    // PORT: Java keeps one shared empty instance per order. Rust cannot keep a generic static per
    // element type, so each call allocates a new empty set; all empty sets are still equal under
    // shallowEquals and isEmpty, which is what callers observe.
    pub fn empty_set<E>(self) -> NestedSet<E> {
        NestedSet::new_empty(self)
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/Order.java:156 Order.emptyDepset
    // PORT: Depset is ported in the Starlark-facing crate; it builds its empty depset from
    // `empty_set`.

    // Java: com/google/devtools/build/lib/collect/nestedset/Order.java:160 Order.getStarlarkName
    pub fn get_starlark_name(self) -> &'static str {
        match self {
            Order::STABLE_ORDER => "default",
            Order::COMPILE_ORDER => "postorder",
            Order::LINK_ORDER => "topological",
            Order::NAIVE_LINK_ORDER => "preorder",
        }
    }

    /// Parses the given string as a nested set order.
    ///
    /// Returns an error (Java: `IllegalArgumentException`) if the name is not valid.
    // Java: com/google/devtools/build/lib/collect/nestedset/Order.java:171 Order.parse
    pub fn parse(name: &str) -> Result<Order, OrderParseError> {
        for current in Order::VALUES {
            if current.get_starlark_name() == name {
                return Ok(current);
            }
        }
        Err(OrderParseError {
            message: format!("Invalid order: {name}"),
        })
    }

    /// Determines whether two orders are considered compatible.
    ///
    /// An order is compatible with itself (reflexivity) and all orders are compatible with
    /// [`Order::STABLE_ORDER`]; the rest of the combinations are incompatible.
    // Java: com/google/devtools/build/lib/collect/nestedset/Order.java:185 Order.isCompatible
    pub fn is_compatible(self, other: Order) -> bool {
        self == other || self == Order::STABLE_ORDER || other == Order::STABLE_ORDER
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/Order.java:203 Order.getOrder
    pub fn get_order(ordinal: i32) -> Order {
        Order::VALUES[ordinal as usize]
    }
}

/// Java's `Enum.toString()` is the constant name.
impl fmt::Display for Order {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
