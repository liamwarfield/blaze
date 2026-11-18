//! Immutable sets (Java `StarlarkSet.immutableCopyOf`).
//!
//! Java: net/starlark/java/eval/StarlarkSet.java (copyOf, immutableCopyOf, isImmutable,
//! checkMutable callers). The set state comes from starlark-rust BLAZE PATCH 0025
//! (`starlark::values::set::alloc_immutable_set`, `set_is_immutable`).

use starlark::values::Value;

/// Java `StarlarkSet.isImmutable` for a set, `None` when `v` is not a set. Frozen sets and sets
/// made by `starlark::values::set::alloc_immutable_set` (Java `StarlarkSet.immutableCopyOf`) are
/// immutable without looking at their elements (Java: the set's mutability is frozen); other
/// sets are mutable.
// Java: net/starlark/java/eval/StarlarkSet.java:187 StarlarkSet.isImmutable
pub fn set_is_immutable(v: Value) -> Option<bool> {
    starlark::values::set::set_is_immutable(v)
}
