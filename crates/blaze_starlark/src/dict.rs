//! Immutable dicts (Java `Dict.immutableCopyOf`).
//!
//! Java: net/starlark/java/eval/Dict.java (copyOf, immutableCopyOf, isImmutable, checkMutable
//! callers). The dict state comes from starlark-rust BLAZE PATCH 0018
//! (`starlark::values::dict::alloc_immutable_dict`, `dict_is_immutable`).

use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::dict::Dict;
use starlark::values::dict::DictRef;

/// Java `Dict.immutableCopyOf(m)`: an immutable dict with the entries of the dict `value`, in
/// order. A dict that is already immutable is returned as is (Java `copyOf(null, m)`); the empty
/// result is the shared frozen empty dict. Keys and values are not copied or frozen. Every
/// mutation of the result fails with "trying to mutate a frozen dict value". Fails when `value`
/// is not a dict (Java takes a `Map`).
// Java: net/starlark/java/eval/Dict.java:495 Dict.immutableCopyOf
// Java: net/starlark/java/eval/Dict.java:456 Dict.copyOf
pub fn dict_immutable_copy_of<'v>(heap: Heap<'v>, value: Value<'v>) -> starlark::Result<Value<'v>> {
    if dict_is_immutable(value) == Some(true) {
        return Ok(value);
    }
    let Some(d) = DictRef::from_value(value) else {
        return Err(starlark::Error::new_other(anyhow::anyhow!(
            "got value of type '{}', want 'dict'",
            value.get_type()
        )));
    };
    let content = d.iter_hashed().collect();
    drop(d);
    Ok(starlark::values::dict::alloc_immutable_dict(
        heap,
        Dict::new(content),
    ))
}

/// Java `Dict.isImmutable` for a dict, `None` when `v` is not a dict. Frozen dicts and dicts from
/// [`dict_immutable_copy_of`] are immutable without looking at their keys or values (Java: the
/// dict's mutability is frozen); other dicts are mutable.
// Java: net/starlark/java/eval/Dict.java:180 Dict.isImmutable
pub fn dict_is_immutable(v: Value) -> Option<bool> {
    starlark::values::dict::dict_is_immutable(v)
}
