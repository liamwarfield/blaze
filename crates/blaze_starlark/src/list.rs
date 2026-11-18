//! Immutable lists (Java `StarlarkList.immutableCopyOf`, `ImmutableStarlarkList`).
//!
//! Java: net/starlark/java/eval/StarlarkList.java (copyOf, immutableCopyOf, wrap),
//! ImmutableStarlarkList.java (isImmutable, mutators), MutableStarlarkList.java (isImmutable).
//! The list state comes from starlark-rust BLAZE PATCH 0016
//! (`starlark::values::list::alloc_immutable_list`, `list_is_immutable`).

use starlark::values::Heap;
use starlark::values::Value;

/// Java `StarlarkList.immutableCopyOf(elems)`: an immutable list of the elements of `iterable`.
/// A list that is already immutable is returned as is (Java `copyOf(null, elems)`); the empty
/// result is the shared frozen empty list. Elements are not copied or frozen: an immutable list
/// may hold mutable values, as in Java. Every mutation of the result fails with "trying to
/// mutate a frozen list value".
// Java: net/starlark/java/eval/StarlarkList.java:168 StarlarkList.immutableCopyOf
// Java: net/starlark/java/eval/StarlarkList.java:144 StarlarkList.copyOf
pub fn list_immutable_copy_of<'v>(
    heap: Heap<'v>,
    iterable: Value<'v>,
) -> starlark::Result<Value<'v>> {
    if list_is_immutable(iterable) == Some(true) {
        return Ok(iterable);
    }
    let elems: Vec<Value<'v>> = iterable.iterate(heap)?.collect();
    Ok(starlark::values::list::alloc_immutable_list(heap, &elems))
}

/// Java `StarlarkList.isImmutable` for a list, `None` when `v` is not a list. Frozen lists and
/// lists from [`list_immutable_copy_of`] are immutable without looking at their elements
/// (ImmutableStarlarkList.isImmutable); other lists are mutable.
// Java: net/starlark/java/eval/ImmutableStarlarkList.java:25 ImmutableStarlarkList.isImmutable
// Java: net/starlark/java/eval/MutableStarlarkList.java:46 MutableStarlarkList.isImmutable
pub fn list_is_immutable(v: Value) -> Option<bool> {
    starlark::values::list::list_is_immutable(v)
}
