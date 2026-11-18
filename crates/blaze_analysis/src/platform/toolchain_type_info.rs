//! Port of `analysis/platform/ToolchainTypeInfo.java`.

use std::fmt;
use std::hash::Hash;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::starlark_defs::platform_common::TOOLCHAIN_TYPE_INFO;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::StructureMarker;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use starlark::values::ValueLike;
use starlark::values::{Demand, Heap, NoSerialize, StarlarkValue, Value, starlark_value};
use starlark_map::StarlarkHasher;

use crate::info::InfoProviderKey;

/// A provider that supplies information about a specific toolchain type.
// Java: com/google/devtools/build/lib/analysis/platform/ToolchainTypeInfo.java:30 ToolchainTypeInfo
// Java: com/google/devtools/build/lib/analysis/platform/ToolchainTypeInfo.java:76 ToolchainTypeInfo.hashCode
// Java: com/google/devtools/build/lib/analysis/platform/ToolchainTypeInfo.java:81 ToolchainTypeInfo.equals
#[derive(Clone, Debug, PartialEq, Eq, Hash, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ToolchainTypeInfo {
    #[allocative(skip)]
    type_label: Label,
    #[allocative(skip)]
    none_found_error: Option<String>,
}

starlark_simple_value!(ToolchainTypeInfo);

impl ToolchainTypeInfo {
    /// Name used in Starlark for accessing this provider.
    // Java: com/google/devtools/build/lib/analysis/platform/ToolchainTypeInfo.java:32 ToolchainTypeInfo.STARLARK_NAME
    pub const STARLARK_NAME: &'static str = "ToolchainTypeInfo";

    /// Java `PROVIDER.getKey()`.
    pub fn provider_key() -> ProviderKey {
        TOOLCHAIN_TYPE_INFO.key()
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ToolchainTypeInfo.java:41 ToolchainTypeInfo.create
    pub fn create_with(type_label: Label, none_found_error: Option<String>) -> ToolchainTypeInfo {
        ToolchainTypeInfo {
            type_label,
            none_found_error,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ToolchainTypeInfo.java:46 ToolchainTypeInfo.create
    pub fn create(type_label: Label) -> ToolchainTypeInfo {
        Self::create_with(type_label, None)
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ToolchainTypeInfo.java:61 ToolchainTypeInfo.typeLabel
    pub fn type_label(&self) -> &Label {
        &self.type_label
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ToolchainTypeInfo.java:66 ToolchainTypeInfo.noneFoundError
    pub fn none_found_error(&self) -> Option<&str> {
        self.none_found_error.as_deref()
    }
}

impl fmt::Display for ToolchainTypeInfo {
    // Java: com/google/devtools/build/lib/analysis/platform/ToolchainTypeInfo.java:71 ToolchainTypeInfo.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ToolchainTypeInfo({})", self.type_label)
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/platform/ToolchainTypeInfoApi.java:31 ToolchainTypeInfoApi
#[starlark_value(type = "ToolchainTypeInfo")]
impl<'v> StarlarkValue<'v> for ToolchainTypeInfo {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "type_label" => Some(heap.alloc(LabelValue(self.type_label.clone()))),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        attribute == "type_label"
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["type_label".to_owned()]
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<ToolchainTypeInfo>()
            .is_some_and(|o| o == self))
    }

    // Java: NativeInfo.isImmutable is true (hashable).
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.type_label.get_canonical_form().hash(hasher);
        self.none_found_error.hash(hasher);
        Ok(())
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(TOOLCHAIN_TYPE_INFO.key()));
        demand.provide_value(StructureMarker { native_info: true });
    }
}

/// The type label of a `ToolchainTypeInfo` value (for `ctx.toolchains[...]` keys, Java
/// `StarlarkToolchainContext.transformKey`'s `ToolchainTypeInfo` case).
pub fn toolchain_type_label_from_value(v: Value) -> Option<Label> {
    v.downcast_ref::<ToolchainTypeInfo>()
        .map(|t| t.type_label().clone())
}
