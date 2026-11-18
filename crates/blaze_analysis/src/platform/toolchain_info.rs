//! Port of `analysis/platform/ToolchainInfo.java` (the analysis-phase instances, which are
//! `Info`s: they provide [`InfoProviderKey`]).

use std::cmp::Ordering;
use std::fmt;

use allocative::Allocative;
use blaze_packages::attribute::Attribute;
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::starlark_defs::builtin_providers::{
    BuiltinConstructor, BuiltinProvider, BuiltinProviderData, BuiltinProviderId,
};
use blaze_packages::starlark_defs::platform_common::TOOLCHAIN_INFO;
use blaze_starlark::structs::StructureMarker;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::eval::{Arguments, Evaluator};
use starlark::starlark_complex_value;
use starlark::values::{
    Demand, Freeze, FreezeResult, Freezer, FrozenValue, Heap, NoSerialize, StarlarkValue, Trace,
    Value, ValueLifetimeless, ValueLike, starlark_value,
};
use starlark_map::StarlarkHasher;

use crate::info::InfoProviderKey;

/// A provider that supplies information about a specific language toolchain, including what
/// platform constraints are required for execution and for the target platform.
///
/// Unusually, ToolchainInfo exposes both its StarlarkCallable-annotated fields and a Map of
/// additional fields to Starlark code. Also, these are not disjoint.
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

/// Name used in Starlark for accessing this provider.
// Java: com/google/devtools/build/lib/analysis/platform/ToolchainInfo.java:45 ToolchainInfo.STARLARK_NAME
pub const STARLARK_NAME: &str = "ToolchainInfo";

impl<'v> ToolchainInfo<'v> {
    /// Constructs a ToolchainInfo. The `values` map itself is not retained.
    // PORT: the creation location is not kept.
    // Java: com/google/devtools/build/lib/analysis/platform/ToolchainInfo.java:68 ToolchainInfo.<init>
    pub fn new(values: Vec<(String, Value<'v>)>) -> ToolchainInfo<'v> {
        ToolchainInfoGen {
            values: copy_values(values),
        }
    }
}

/// Preprocesses a map of field values to convert the field names and field values to
/// Starlark-acceptable names and types. Entries are ordered by key.
// Java: com/google/devtools/build/lib/analysis/platform/ToolchainInfo.java:84 ToolchainInfo.copyValues
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

impl<V: ValueLifetimeless> ToolchainInfoGen<V> {
    /// Java `ToolchainInfo.PROVIDER.getKey()`.
    pub fn provider_key() -> ProviderKey {
        TOOLCHAIN_INFO.key()
    }
}

impl<'v, V: ValueLike<'v>> ToolchainInfoGen<V> {
    // Java: com/google/devtools/build/lib/analysis/platform/ToolchainInfo.java:93 ToolchainInfo.getValue
    pub fn get_value(&self, name: &str) -> Option<Value<'v>> {
        self.values
            .binary_search_by(|(k, _)| k.as_str().cmp(name))
            .ok()
            .map(|i| self.values[i].1.to_value())
    }

    // Java: com/google/devtools/build/lib/analysis/platform/ToolchainInfo.java:98 ToolchainInfo.getFieldNames
    pub fn get_field_names(&self) -> impl Iterator<Item = &str> {
        self.values.iter().map(|(k, _)| k.as_str())
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

fn errorf(msg: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
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
        demand.provide_value(InfoProviderKey(TOOLCHAIN_INFO.key()));
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

/// Java `ToolchainInfo.PROVIDER`, whose constructor creates analysis-phase [`ToolchainInfo`]s.
// Java: com/google/devtools/build/lib/analysis/platform/ToolchainInfo.java:48 ToolchainInfo.PROVIDER
pub const fn provider() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: TOOLCHAIN_INFO,
        constructor: BuiltinConstructor::Native(toolchain_info_constructor),
    })
}
