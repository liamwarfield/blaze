//! Port of `com.google.devtools.build.lib.collect.nestedset`.
//!
//! Not ported here:
//! - `Depset` (the Starlark `depset` wrapper) lives in a later Starlark-facing crate.
//! - `ArtifactNestedSetKey` depends on actions/Skyframe and belongs with them.
// PORT: skipped, serialization (DeferredNestedSetCodec, NestedArrayCodec, NestedSetCodecWithStore,
// NestedSetCodecTestUtils, NestedSetSerializationCache, NestedSetStore).

mod digest_map;
mod nested_set;
mod nested_set_builder;
mod nested_set_fingerprint_cache;
mod nested_set_visitor;
mod order;

pub use nested_set::{
    MAX_ELEMENTS_TO_STRING, NestedSet, NestedSetChild, NestedSetChildren, NestedSetNode,
};
pub use nested_set_builder::NestedSetBuilder;
pub use nested_set_fingerprint_cache::NestedSetFingerprintCache;
pub use nested_set_visitor::{
    NestedSetVisitor, NestedSetVisitorReceiver, NestedSetVisitorVisitedState,
};
pub use order::{Order, OrderParseError};
