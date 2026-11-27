//! Provider instances (`Info`) as frozen Starlark values.
//!
//! Java: packages/Info.java, packages/NativeInfo.java, packages/StarlarkInfo.java.
//!
//! Every `Info` is a Starlark value (Java: `Info extends StarlarkValue`). Blaze stores each one
//! as an [`OwnedFrozenValue`] (which keeps its frozen heap alive), so the same object is seen by
//! Starlark (`dep[CcInfo]`) and by Rust (`downcast_ref`). Two kinds:
//! - Starlark provider instances: `blaze_starlark::structs::StarlarkInfo` values created by
//!   calling a `provider()`; their key is the key of their provider.
//! - Native infos (DefaultInfo, PlatformInfo, ToolchainInfo, ...): Rust Starlark value types
//!   that `provide` an [`InfoProviderKey`] from `StarlarkValue::provide`.
//!
//! Native code that creates an info allocates it with [`freeze_native_info`] (a fresh frozen
//! heap) or on the frozen heap of the rule's module.

use blaze_packages::rule_class::ProviderKey;
use blaze_packages::starlark_defs::builtin_providers::builtin_provider_key;
use blaze_packages::starlark_defs::provider::provider_view;
use blaze_starlark::structs::StarlarkInfo;
use starlark::any::ProvidesStaticType;
use starlark::values::{
    AllocFrozenValue, FrozenHeap, FrozenValue, Heap, OwnedFrozenValue, StarlarkValue, Value,
    ValueLike,
};
use starlark_map::StarlarkHasher;
use std::hash::{Hash, Hasher};

/// What a native info value `provide`s so that [`info_provider_key`] can find its key
/// (Java `Info.getProvider().getKey()`).
#[derive(Clone, Debug, PartialEq, Eq, ProvidesStaticType)]
pub struct InfoProviderKey(pub ProviderKey);

/// Java `Info.getProvider().getKey()` of `v`, or `None` if `v` is not an `Info`.
// Java: com/google/devtools/build/lib/packages/Info.java:55 Info.getProvider
// PORT: the creation location (Java `Info.getCreationLocation`) is passed to the one reader,
// StarlarkRuleConfiguredTargetUtil.infoError (starlark_rule_configured_target_util.rs).
// Java: com/google/devtools/build/lib/packages/Info.java:61 Info.getCreationLocation
// Java: com/google/devtools/build/lib/packages/StarlarkInfo.java:47 StarlarkInfo.getCreationLocation
pub fn info_provider_key(v: Value) -> Option<ProviderKey> {
    if let Some(k) = v.request_value::<InfoProviderKey>() {
        return Some(k.0);
    }
    if let Some(info) = StarlarkInfo::from_value(v) {
        let provider = info.provider();
        if let Some(view) = provider_view(provider) {
            return view.key;
        }
        return builtin_provider_key(provider);
    }
    None
}

/// Java `x instanceof Info`.
pub fn is_info(v: Value) -> bool {
    info_provider_key(v).is_some()
        || StarlarkInfo::from_value(v).is_some()
        || v.request_value::<InfoProviderKey>().is_some()
}

/// Java `instanceof` for the provider interface names of the builtins' parameter table
/// (starlark-rust BLAZE PATCH 0062, round 3): `Info`, `NativeInfo`, `Provider` and
/// `structure` (Java's `Structure`, implemented by `StructImpl`, whose instances are
/// [`StarlarkInfo`] values).
// Java: com/google/devtools/build/lib/packages/Info.java:52 Info
// Java: com/google/devtools/build/lib/packages/NativeInfo.java:39 NativeInfo
// Java: com/google/devtools/build/lib/packages/Provider.java:36 Provider
// Java: net/starlark/java/eval/Structure.java:24 Structure
pub fn java_interface_matcher(name: &str, v: Value) -> Option<bool> {
    match name {
        "Info" => Some(is_info(v)),
        "NativeInfo" => Some(
            StarlarkInfo::from_value(v).is_none() && v.request_value::<InfoProviderKey>().is_some(),
        ),
        "Provider" => Some(provider_view(v).is_some() || builtin_provider_key(v).is_some()),
        "structure" => Some(StarlarkInfo::from_value(v).is_some()),
        _ => None,
    }
}

/// Allocates a native info on a new frozen heap.
pub fn freeze_native_info<T: AllocFrozenValue>(info: T) -> OwnedFrozenValue {
    let heap = FrozenHeap::new();
    let v: FrozenValue = heap.alloc(info);
    let heap_ref = heap.into_ref();
    // SAFETY: `v` was allocated on `heap_ref`.
    unsafe { OwnedFrozenValue::new(heap_ref, v) }
}

/// Downcasts a frozen info to a native info type.
pub fn downcast_info<T: starlark::values::StarlarkValue<'static>>(
    v: &OwnedFrozenValue,
) -> Option<&T> {
    // SAFETY: the returned reference borrows `v`, which keeps the frozen heap alive.
    unsafe { v.unchecked_frozen_value() }.downcast_ref::<T>()
}

// -------------------------------------------------------------------------------------------
// NativeInfo: equality, hashing and repr by legacy fields.

/// Runs `f` with a temporary heap, typed as the heap of the values it reads. `equals`,
/// `write_hash` and `Display` of a native info have no heap, but its fields are computed
/// (`get_attr` allocates).
fn with_temporary_heap<'v, R: 'static>(f: impl FnOnce(Heap<'v>) -> R) -> R {
    Heap::temp(|heap| f(as_heap_of(heap)))
}

/// SAFETY of the callers: the values made on the temporary heap live only during
/// [`with_temporary_heap`]'s closure, whose result (`R: 'static`) holds none of them.
fn as_heap_of<'t, 'v>(heap: Heap<'t>) -> Heap<'v> {
    // SAFETY: see above; only the lifetime changes.
    unsafe { std::mem::transmute::<Heap<'t>, Heap<'v>>(heap) }
}

/// Java `NativeInfo.getLegacyFields` of a native info: its struct fields in
/// `getLegacyStarlarkMethodNames` order (sorted names), methods left out (Java skips
/// `BuiltinFunction` values; Rust methods are not fields), a field whose `getattr` fails `None`.
// Java: com/google/devtools/build/lib/packages/NativeInfo.java:64 NativeInfo.getLegacyStarlarkMethodNames
// Java: com/google/devtools/build/lib/packages/NativeInfo.java:69 NativeInfo.getLegacyFields
fn legacy_fields<'v, T: StarlarkValue<'v>>(this: &T, heap: Heap<'v>) -> Vec<(String, Value<'v>)> {
    let mut names = this.dir_attr();
    names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    names.dedup();
    names
        .into_iter()
        .map(|name| {
            let value = this.get_attr(&name, heap).unwrap_or_else(Value::new_none);
            (name, value)
        })
        .collect()
}

/// [`legacy_fields`] of a value known only as a `Value` (the other operand of `==`).
fn legacy_fields_of_value<'v>(v: Value<'v>, heap: Heap<'v>) -> Vec<(String, Value<'v>)> {
    let mut names = v.dir_attr();
    names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    names.dedup();
    let mut fields = Vec::with_capacity(names.len());
    for name in names {
        match v.get_attr(&name, heap) {
            Ok(Some(value)) if value.get_type() == "builtin_function_or_method" => {}
            Ok(Some(value)) => fields.push((name, value)),
            Ok(None) => {}
            // Java: an EvalException from getattr gives `None`.
            Err(_) => fields.push((name, Value::new_none())),
        }
    }
    fields
}

/// Java `NativeInfo.equals`: an info of the same provider with equal legacy fields. For the
/// native infos that do not override it (Java subclasses of `NativeInfo` without their own
/// `equals`); `key` is this info's provider key.
// Java: com/google/devtools/build/lib/packages/NativeInfo.java:97 NativeInfo.equals
pub fn native_info_equals<'v, T: StarlarkValue<'v>>(
    this: &T,
    key: &ProviderKey,
    other: Value<'v>,
) -> starlark::Result<bool> {
    if other
        .request_value::<InfoProviderKey>()
        .map(|k| k.0)
        .as_ref()
        != Some(key)
    {
        return Ok(false);
    }
    with_temporary_heap(|heap| {
        let x = legacy_fields(this, heap);
        let y = legacy_fields_of_value(other, heap);
        if x.len() != y.len() {
            return Ok(false);
        }
        for ((xn, xv), (yn, yv)) in x.iter().zip(y.iter()) {
            if xn != yn || !xv.equals(*yv)? {
                return Ok(false);
            }
        }
        Ok(true)
    })
}

/// Java `NativeInfo.hashCode` (`Objects.hashCode(getProvider(), getLegacyFields())`); a native
/// info is always hashable (Java `NativeInfo.isImmutable` is true), whatever its fields hold.
// Java: com/google/devtools/build/lib/packages/NativeInfo.java:115 NativeInfo.hashCode
pub fn native_info_write_hash<'v, T: StarlarkValue<'v>>(
    this: &T,
    key: &ProviderKey,
    hasher: &mut StarlarkHasher,
) -> starlark::Result<()> {
    key.hash(hasher);
    let fields: starlark::Result<Vec<(String, u64)>> = with_temporary_heap(|heap| {
        legacy_fields(this, heap)
            .into_iter()
            .map(|(name, value)| {
                let mut h = StarlarkHasher::new();
                blaze_starlark::structs::java_hash_code(value, &mut h)?;
                Ok((name, h.finish()))
            })
            .collect()
    });
    for (name, h) in fields? {
        name.hash(hasher);
        h.hash(hasher);
    }
    Ok(())
}

/// Java `NativeInfo.repr` (and `toString`, which is `Starlark.repr`): `struct(name = value, ...)`
/// over the legacy fields. Native infos call it from `StarlarkValue::collect_repr`, which `str`
/// also uses.
// Java: com/google/devtools/build/lib/packages/NativeInfo.java:125 NativeInfo.repr
// Java: com/google/devtools/build/lib/packages/NativeInfo.java:140 NativeInfo.toString
pub fn native_info_collect_repr<'v, T: StarlarkValue<'v>>(this: &T, collector: &mut String) {
    let text = with_temporary_heap(|heap| {
        let fields: Vec<String> = legacy_fields(this, heap)
            .into_iter()
            .map(|(name, value)| format!("{name} = {}", value.to_repr()))
            .collect();
        format!("struct({})", fields.join(", "))
    });
    collector.push_str(&text);
}
