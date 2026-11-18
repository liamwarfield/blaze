//! Port of `com.google.devtools.build.lib.collect` (the `nestedset` package and the small helpers
//! it needs).
//!
//! `CompactHashSet` (`lib/collect/compacthashset`) is used by Java only as an insertion-ordered
//! hash set without removals; the port uses `indexmap::IndexSet`, which has the same iteration
//! order.
#![allow(clippy::too_many_arguments, clippy::new_without_default)]

pub mod command_line_item;
pub mod nestedset;

pub use nestedset::{
    NestedSet, NestedSetBuilder, NestedSetFingerprintCache, NestedSetNode, NestedSetVisitor,
    NestedSetVisitorReceiver, NestedSetVisitorVisitedState, Order,
};
