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
    AllocFrozenValue, FrozenHeap, FrozenValue, OwnedFrozenValue, Value, ValueLike,
};

/// What a native info value `provide`s so that [`info_provider_key`] can find its key
/// (Java `Info.getProvider().getKey()`).
#[derive(Clone, Debug, PartialEq, Eq, ProvidesStaticType)]
pub struct InfoProviderKey(pub ProviderKey);

/// Java `Info.getProvider().getKey()` of `v`, or `None` if `v` is not an `Info`.
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
