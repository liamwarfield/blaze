//! The `platform_common` module and the platform providers.
//!
//! Java: rules/platform/PlatformCommon.java (starlarkbuildapi/platform/PlatformCommonApi.java),
//! analysis/platform/ToolchainInfo.java (provider and struct-like instances),
//! analysis/TemplateVariableInfo.java, analysis/platform/PlatformInfo.java,
//! ConstraintSettingInfo.java, ConstraintValueInfo.java and ToolchainTypeInfo.java (`PROVIDER`).

use std::cmp::Ordering;
use std::fmt;

use allocative::Allocative;
use blaze_starlark::structs::StructureMarker;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Arguments;
use starlark::eval::Evaluator;
use starlark::starlark_complex_value;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::Demand;
use starlark::values::Freeze;
use starlark::values::FreezeResult;
use starlark::values::Freezer;
use starlark::values::FrozenValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLifetimeless;
use starlark::values::ValueLike;
use starlark::values::starlark_value;
use starlark_map::StarlarkHasher;

use crate::attribute::Attribute;
use crate::starlark_defs::builtin_providers::BuiltinConstructor;
use crate::starlark_defs::builtin_providers::BuiltinProvider;
use crate::starlark_defs::builtin_providers::BuiltinProviderData;
use crate::starlark_defs::builtin_providers::BuiltinProviderId;
use crate::starlark_defs::builtin_providers::not_callable;
use crate::starlark_defs::util::errorf;

// -------------------------------------------------------------------------------------------
// Provider identities and values.

// Java: com/google/devtools/build/lib/analysis/TemplateVariableInfo.java:34 TemplateVariableInfo.PROVIDER
pub const TEMPLATE_VARIABLE_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "TemplateVariableInfo",
    value_class: "TemplateVariableInfo",
};

// Java: com/google/devtools/build/lib/analysis/platform/ToolchainInfo.java:48 ToolchainInfo.PROVIDER
pub const TOOLCHAIN_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "ToolchainInfo",
    value_class: "ToolchainInfo",
};

// Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:71 PlatformInfo.PROVIDER
pub const PLATFORM_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "PlatformInfo",
    value_class: "PlatformInfo",
};

// Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:35 ConstraintSettingInfo.PROVIDER
pub const CONSTRAINT_SETTING_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "ConstraintSettingInfo",
    value_class: "ConstraintSettingInfo",
};

// Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfo.java:38 ConstraintValueInfo.PROVIDER
pub const CONSTRAINT_VALUE_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "ConstraintValueInfo",
    value_class: "ConstraintValueInfo",
};

// Java: com/google/devtools/build/lib/analysis/platform/ToolchainTypeInfo.java:35 ToolchainTypeInfo.PROVIDER
pub const TOOLCHAIN_TYPE_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "ToolchainTypeInfo",
    value_class: "ToolchainTypeInfo",
};

/// Java `TemplateVariableInfo.PROVIDER`.
pub const fn template_variable_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: TEMPLATE_VARIABLE_INFO,
        // PORT: analysis-time (needs TemplateVariableInfo instances).
        constructor: BuiltinConstructor::Unported(
            "TemplateVariableInfo.Provider.templateVariableInfo",
        ),
    })
}

/// Java `ToolchainInfo.PROVIDER`.
pub const fn toolchain_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: TOOLCHAIN_INFO,
        constructor: BuiltinConstructor::Native(toolchain_info_constructor),
    })
}

/// Java `PlatformInfo.PROVIDER` (an anonymous `BuiltinProvider` without `selfCall`).
pub const fn platform_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: PLATFORM_INFO,
        constructor: BuiltinConstructor::Native(not_callable),
    })
}

/// Java `ConstraintSettingInfo.PROVIDER` (no `selfCall`).
pub const fn constraint_setting_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: CONSTRAINT_SETTING_INFO,
        constructor: BuiltinConstructor::Native(not_callable),
    })
}

/// Java `ConstraintValueInfo.PROVIDER` (no `selfCall`).
pub const fn constraint_value_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: CONSTRAINT_VALUE_INFO,
        constructor: BuiltinConstructor::Native(not_callable),
    })
}

/// Java `ToolchainTypeInfo.PROVIDER` (no `selfCall`; not exposed by `platform_common`).
pub const fn toolchain_type_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: TOOLCHAIN_TYPE_INFO,
        constructor: BuiltinConstructor::Native(not_callable),
    })
}

// -------------------------------------------------------------------------------------------
// ToolchainInfo instances

/// A `ToolchainInfo` instance (Java `ToolchainInfo`, a `StructImpl` whose fields are the
/// constructor's keyword arguments, sorted by name).
// Java: com/google/devtools/build/lib/analysis/platform/ToolchainInfo.java:41 ToolchainInfo
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct ToolchainInfoGen<V: ValueLifetimeless> {
    /// Sorted by name, names unique.
    values: Vec<(String, V)>,
}

starlark_complex_value!(pub ToolchainInfo);

// SAFETY: `repr(C)`, and `FrozenValue`/`Value` have the same layout.
unsafe impl<'v> Coerce<ToolchainInfoGen<Value<'v>>> for ToolchainInfoGen<FrozenValue> {}

impl<'v> Freeze for ToolchainInfo<'v> {
    type Frozen = FrozenToolchainInfo;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenToolchainInfo> {
        Ok(ToolchainInfoGen {
            values: self.values.freeze(freezer)?,
        })
    }
}

impl<'v> ToolchainInfo<'v> {
    /// Java `new ToolchainInfo(values, location)`.
    // PORT: the creation location is not kept.
    // Java: com/google/devtools/build/lib/analysis/platform/ToolchainInfo.java:70 ToolchainInfo.<init>
    pub fn new(values: Vec<(String, Value<'v>)>) -> ToolchainInfo<'v> {
        ToolchainInfoGen {
            values: copy_values(values),
        }
    }
}

/// Java `ToolchainInfo.copyValues`: Starlark names, sorted by key.
// Java: com/google/devtools/build/lib/analysis/platform/ToolchainInfo.java:87 ToolchainInfo.copyValues
fn copy_values(values: Vec<(String, Value)>) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::with_capacity(values.len());
    for (k, v) in values {
        let name = Attribute::get_starlark_name(&k);
        // Java: ImmutableSortedMap.Builder.buildOrThrow rejects duplicate keys.
        if let Some((_, first)) = out.iter().find(|(n, _)| *n == name) {
            panic!(
                "IllegalArgumentException: Multiple entries with same key: {name}={} and {name}={}",
                first.to_str(),
                v.to_str()
            );
        }
        // PORT: Starlark.fromJava is the identity on Starlark values.
        out.push((name, v));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

impl<'v, V: ValueLike<'v>> ToolchainInfoGen<V> {
    /// Java `getValue(name)`.
    // Java: com/google/devtools/build/lib/analysis/platform/ToolchainInfo.java:96 ToolchainInfo.getValue
    pub fn get_value(&self, name: &str) -> Option<Value<'v>> {
        self.values
            .binary_search_by(|(k, _)| k.as_str().cmp(name))
            .ok()
            .map(|i| self.values[i].1.to_value())
    }

    /// Java `getFieldNames()` (sorted).
    // Java: com/google/devtools/build/lib/analysis/platform/ToolchainInfo.java:101 ToolchainInfo.getFieldNames
    pub fn get_field_names(&self) -> impl Iterator<Item = &str> {
        self.values.iter().map(|(k, _)| k.as_str())
    }

    /// Java `StructImpl.getErrorMessageForUnknownField`.
    // Java: com/google/devtools/build/lib/packages/StructImpl.java:86 StructImpl.getErrorMessageForUnknownField
    pub fn get_error_message_for_unknown_field(&self, name: &str) -> String {
        format!(
            "'ToolchainInfo' value has no field or method '{}'\nAvailable attributes: {}",
            name,
            self.get_field_names().collect::<Vec<_>>().join(", ")
        )
    }
}

impl<'v, V: ValueLike<'v>> fmt::Display for ToolchainInfoGen<V> {
    // Java: com/google/devtools/build/lib/packages/StructImpl.java:139 StructImpl.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "struct(")?;
        for (i, (k, v)) in self.values.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{k} = {}", v.to_value().to_repr())?;
        }
        write!(f, ")")
    }
}

#[starlark_value(type = "ToolchainInfo")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for ToolchainInfoGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.get_value(attribute)
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        self.get_value(attribute).is_some()
    }

    fn dir_attr(&self) -> Vec<String> {
        self.get_field_names().map(str::to_owned).collect()
    }

    // Java: com/google/devtools/build/lib/packages/StructImpl.java:97 StructImpl.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        let Some(other) = ToolchainInfo::from_value(other) else {
            return Ok(false);
        };
        if self.values.len() != other.values.len() {
            return Ok(false);
        }
        for ((ka, va), (kb, vb)) in self.values.iter().zip(other.values.iter()) {
            if ka != kb || !va.to_value().equals(*vb)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    // Java: StarlarkValue.checkHashable — StructImpl is not immutable.
    fn write_hash(&self, _hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        Err(errorf("unhashable type: 'ToolchainInfo'"))
    }

    fn compare(&self, other: Value<'v>) -> starlark::Result<Ordering> {
        Err(errorf(format!(
            "unsupported comparison: ToolchainInfo <=> {}",
            other.get_type()
        )))
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(StructureMarker { native_info: false });
    }
}

/// Java `ToolchainInfo.Provider.toolchainInfo(kwargs, thread)` (`extraKeywords` only).
// Java: com/google/devtools/build/lib/analysis/platform/ToolchainInfo.java:58 ToolchainInfo.Provider.toolchainInfo
fn toolchain_info_constructor<'v>(
    _id: &BuiltinProviderId,
    _me: Value<'v>,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    // Java: BuiltinFunction argument processing with no positional parameters.
    // Java: net/starlark/java/eval/BuiltinFunction.java:422 BuiltinFunction.ArgumentProcessor.call
    if args.positions(eval.heap())?.next().is_some() {
        return Err(errorf("ToolchainInfo() got unexpected positional argument"));
    }
    let kwargs: Vec<(String, Value<'v>)> = args
        .names_map()?
        .into_iter()
        .map(|(k, v)| (k.as_str().to_owned(), v))
        .collect();
    Ok(eval.heap().alloc(ToolchainInfo::new(kwargs)))
}

// -------------------------------------------------------------------------------------------
// platform_common

/// The `platform_common` module (Java `PlatformCommon`).
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct PlatformCommon;

starlark_simple_value!(PlatformCommon);

impl fmt::Display for PlatformCommon {
    // Java: StarlarkValue.repr default (PlatformCommon has no repr).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<unknown object com.google.devtools.build.lib.rules.platform.PlatformCommon>"
        )
    }
}

/// The `structField` methods of `PlatformCommonApi`, in declaration order.
const PLATFORM_COMMON_FIELDS: [&str; 5] = [
    "TemplateVariableInfo",
    "ToolchainInfo",
    "PlatformInfo",
    "ConstraintSettingInfo",
    "ConstraintValueInfo",
];

impl PlatformCommon {
    fn field(name: &str) -> Option<BuiltinProvider> {
        match name {
            // Java: com/google/devtools/build/lib/rules/platform/PlatformCommon.java:42 PlatformCommon.getMakeVariableProvider
            "TemplateVariableInfo" => Some(template_variable_info()),
            // Java: com/google/devtools/build/lib/rules/platform/PlatformCommon.java:47 PlatformCommon.getToolchainInfoConstructor
            "ToolchainInfo" => Some(toolchain_info()),
            // Java: com/google/devtools/build/lib/rules/platform/PlatformCommon.java:27 PlatformCommon.getPlatformInfoConstructor
            "PlatformInfo" => Some(platform_info()),
            // Java: com/google/devtools/build/lib/rules/platform/PlatformCommon.java:32 PlatformCommon.getConstraintSettingInfoConstructor
            "ConstraintSettingInfo" => Some(constraint_setting_info()),
            // Java: com/google/devtools/build/lib/rules/platform/PlatformCommon.java:37 PlatformCommon.getConstraintValueInfoConstructor
            "ConstraintValueInfo" => Some(constraint_value_info()),
            _ => None,
        }
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/platform/PlatformCommonApi.java:24 PlatformCommonApi
#[starlark_value(type = "platform_common")]
impl<'v> StarlarkValue<'v> for PlatformCommon {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        PlatformCommon::field(attribute).map(|p| heap.alloc(p))
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        PlatformCommon::field(attribute).is_some()
    }

    fn dir_attr(&self) -> Vec<String> {
        PLATFORM_COMMON_FIELDS
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    }

    // Java: StarlarkValue.checkHashable (isImmutable is false).
    fn write_hash(&self, _hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        Err(errorf("unhashable type: 'platform_common'"))
    }
}

/// Registers `platform_common` (Java `PlatformBootstrap`).
// Java: com/google/devtools/build/lib/starlarkbuildapi/platform/PlatformBootstrap.java:33 PlatformBootstrap.addBindingsToBuilder
#[starlark_module]
pub fn register_platform_common(builder: &mut GlobalsBuilder) {
    const platform_common: PlatformCommon = PlatformCommon;
}

#[cfg(test)]
mod tests {
    use crate::starlark_defs::provider::test_support::error_message;
    use crate::starlark_defs::provider::test_support::exec_bzl;
    use crate::starlark_defs::provider::test_support::string_global;

    // Expected values recorded from the oracle (Bazel 9.3.2).

    #[test]
    fn platform_common_members() {
        let m = exec_bzl(
            "platform_common_test_1.bzl",
            r#"
def _p(x):
    return "%s|%s" % (type(x), repr(x))
result = ",".join([
    _p(platform_common), str(dir(platform_common)),
    _p(platform_common.ToolchainInfo), _p(platform_common.TemplateVariableInfo),
    _p(platform_common.PlatformInfo), _p(platform_common.ConstraintSettingInfo),
    _p(platform_common.ConstraintValueInfo),
])
"#,
        )
        .unwrap();
        assert_eq!(
            string_global(&m, "result"),
            "platform_common|<unknown object com.google.devtools.build.lib.rules.platform.PlatformCommon>,\
             [\"ConstraintSettingInfo\", \"ConstraintValueInfo\", \"PlatformInfo\", \"TemplateVariableInfo\", \"ToolchainInfo\"],\
             Provider|<function ToolchainInfo>,Provider|<function TemplateVariableInfo>,\
             Provider|<function PlatformInfo>,Provider|<function ConstraintSettingInfo>,\
             Provider|<function ConstraintValueInfo>"
        );
    }

    #[test]
    fn toolchain_info_instances() {
        let m = exec_bzl(
            "platform_common_test_2.bzl",
            r#"
ti = platform_common.ToolchainInfo(a = 1, b = "x", _c = [1])
result = "|".join([
    type(ti), repr(ti), str(dir(ti)), str(ti.a),
    str(ti == platform_common.ToolchainInfo(a = 1, b = "x", _c = [1])),
    str({platform_common.ToolchainInfo: 3}[platform_common.ToolchainInfo]),
])
"#,
        )
        .unwrap();
        assert_eq!(
            string_global(&m, "result"),
            "ToolchainInfo|struct(_c = [1], a = 1, b = \"x\")|[\"_c\", \"a\", \"b\"]|1|True|3"
        );
    }

    #[test]
    fn toolchain_info_rejects_positional_arguments() {
        let e = error_message(exec_bzl(
            "platform_common_test_3.bzl",
            "x = platform_common.ToolchainInfo(1)\n",
        ));
        assert!(
            e.contains("ToolchainInfo() got unexpected positional argument"),
            "{e}"
        );
    }

    #[test]
    fn toolchain_info_is_unhashable() {
        let e = error_message(exec_bzl(
            "platform_common_test_4.bzl",
            "x = platform_common.ToolchainInfo(a = [1])\ny = {x: 1}\n",
        ));
        assert!(e.contains("unhashable type: 'ToolchainInfo'"), "{e}");
    }
}
