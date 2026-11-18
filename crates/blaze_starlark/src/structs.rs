//! `struct` and the struct-like base that provider instances build on.
//!
//! Java: com/google/devtools/build/lib/packages/StructImpl.java (equality, hash, repr),
//! StarlarkInfo.java, StarlarkInfoNoSchema.java (sorted field table, `+`),
//! StructProvider.java (`struct(**kwargs)`), Provider.java (unknown-field message).
//!
//! [`StarlarkInfo`] is generic over its provider: `struct()` uses the [`StructProvider`] value;
//! `blaze_packages` creates instances of Starlark/native providers with
//! [`StarlarkInfo::new`], passing its own provider value. A provider value describes itself to
//! this crate by implementing `StarlarkValue::provide` with a [`ProviderInfo`].

use std::cmp::Ordering;
use std::fmt;
use std::hash::Hash;

use allocative::Allocative;
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
use starlark::values::ValueLike;
use starlark::values::starlark_value;
use starlark::values::tuple::TupleRef;
use starlark_map::StarlarkHasher;

/// What a provider value tells this crate about itself (Java `Provider.getPrintableName`,
/// `isExported`). Provided via `StarlarkValue::provide`:
/// `demand.provide_value(ProviderInfo { .. })`.
#[derive(Debug, Clone, PartialEq, Eq, ProvidesStaticType)]
pub struct ProviderInfo {
    pub printable_name: String,
    pub exported: bool,
}

/// Marker provided by struct-like values (Java `Structure`): `json.encode` encodes them as
/// objects of their `dir()` fields. `native_info` is Java's `NativeInfo` (callable fields are
/// skipped when encoding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ProvidesStaticType)]
pub struct StructureMarker {
    pub native_info: bool,
}

/// Marker provided by Blaze values that are deeply immutable once created (Label, depset,
/// File, …), for depset element checks (Java `StarlarkValue.isImmutable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ProvidesStaticType)]
pub struct DeeplyImmutable;

/// Provider information of `v`, if it describes itself.
pub fn provider_info(v: Value) -> Option<ProviderInfo> {
    v.request_value::<ProviderInfo>()
}

/// Java `Starlark.isImmutable`, as far as starlark-rust can tell: frozen values, scalars and
/// strings, tuples of immutables, struct-like values of exported providers with immutable
/// fields, and values providing [`DeeplyImmutable`].
pub fn is_immutable(v: Value) -> bool {
    if v.unpack_frozen().is_some() {
        return true;
    }
    match v.get_type() {
        "NoneType" | "bool" | "int" | "float" | "string" => return true,
        "list" | "dict" | "set" | "function" => return false,
        _ => {}
    }
    if let Some(t) = TupleRef::from_value(v) {
        return t.content().iter().all(|x| is_immutable(*x));
    }
    if let Some(s) = StarlarkInfo::from_value(v) {
        return s.is_immutable();
    }
    v.request_value::<DeeplyImmutable>().is_some()
}

// -------------------------------------------------------------------------------------------
// StructProvider: the `struct` global.

/// Java `StructProvider.STRUCT`: the callable `struct` and the provider of its instances.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StructProvider;

starlark_simple_value!(StructProvider);

impl fmt::Display for StructProvider {
    // Java: BuiltinProvider.repr -> "<function struct>"
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<function struct>")
    }
}

#[starlark_value(type = "Provider")]
impl<'v> StarlarkValue<'v> for StructProvider {
    fn invoke(
        &self,
        me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        // Java: StructProvider.createStruct(Dict kwargs)
        args.no_positional_args(eval.heap())?;
        let fields = args
            .names_map()?
            .into_iter()
            .map(|(k, v)| (k.as_str().to_owned(), v))
            .collect();
        Ok(eval.heap().alloc(StarlarkInfo::new(me, fields)))
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other.downcast_ref::<StructProvider>().is_some())
    }

    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        "struct".hash(hasher);
        Ok(())
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(ProviderInfo {
            printable_name: "struct".to_owned(),
            exported: true,
        });
    }
}

/// Registers `struct` (Java: StarlarkGlobalsImpl puts it in the .bzl, .scl and cquery
/// environments).
#[starlark_module]
pub fn register_struct(builder: &mut GlobalsBuilder) {
    const r#struct: StructProvider = StructProvider;
}

// -------------------------------------------------------------------------------------------
// StarlarkInfo

/// A struct-like value: a provider and a field table sorted by name (Java `StarlarkInfoNoSchema`).
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct StarlarkInfoGen<V: starlark::values::ValueLifetimeless> {
    provider: V,
    /// Sorted by name, names unique.
    fields: Vec<(String, V)>,
}

starlark_complex_value!(pub StarlarkInfo);

// SAFETY: `repr(C)` and FrozenValue/Value have the same layout.
unsafe impl<'v> Coerce<StarlarkInfoGen<Value<'v>>> for StarlarkInfoGen<FrozenValue> {}

impl<'v> StarlarkInfo<'v> {
    /// Java: StarlarkInfoNoSchema(provider, values) — fields sorted by name. Later duplicates
    /// replace earlier ones (callers pass maps).
    pub fn new(provider: Value<'v>, mut fields: Vec<(String, Value<'v>)>) -> Self {
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        fields.dedup_by(|later, earlier| {
            if later.0 == earlier.0 {
                earlier.1 = later.1;
                true
            } else {
                false
            }
        });
        StarlarkInfoGen { provider, fields }
    }
}

impl<'v, V: ValueLike<'v>> StarlarkInfoGen<V> {
    pub fn provider(&self) -> Value<'v> {
        self.provider.to_value()
    }

    /// Field names, sorted (Java `getFieldNames`).
    pub fn field_names(&self) -> impl Iterator<Item = &str> {
        self.fields.iter().map(|(k, _)| k.as_str())
    }

    /// Java `getValue(name)`.
    pub fn get_field(&self, name: &str) -> Option<Value<'v>> {
        self.fields
            .binary_search_by(|(k, _)| k.as_str().cmp(name))
            .ok()
            .map(|i| self.fields[i].1.to_value())
    }

    pub fn fields(&self) -> impl Iterator<Item = (&str, Value<'v>)> {
        self.fields.iter().map(|(k, v)| (k.as_str(), v.to_value()))
    }

    fn printable_provider_name(&self) -> String {
        provider_info(self.provider.to_value())
            .map(|p| p.printable_name)
            .unwrap_or_else(|| "struct".to_owned())
    }

    /// Java: StarlarkInfoNoSchema.isImmutable
    pub fn is_immutable(&self) -> bool {
        let exported = provider_info(self.provider.to_value()).is_none_or(|p| p.exported);
        exported && self.fields.iter().all(|(_, v)| is_immutable(v.to_value()))
    }

    /// Java: StructImpl.getErrorMessageForUnknownField (+ allAttributesSuffix).
    pub fn unknown_field_message(&self, name: &str) -> String {
        format!(
            "'{}' value has no field or method '{}'\nAvailable attributes: {}",
            self.printable_provider_name(),
            name,
            self.field_names().collect::<Vec<_>>().join(", ")
        )
    }
}

impl<'v> Freeze for StarlarkInfo<'v> {
    type Frozen = FrozenStarlarkInfo;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenStarlarkInfo> {
        Ok(StarlarkInfoGen {
            provider: self.provider.freeze(freezer)?,
            fields: self.fields.freeze(freezer)?,
        })
    }
}

impl<'v, V: ValueLike<'v>> fmt::Display for StarlarkInfoGen<V> {
    // Java: StructImpl.repr — "struct(a = 1, b = 2)", fields in sorted order.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "struct(")?;
        for (i, (k, v)) in self.fields.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{k} = {}", v.to_value().to_repr())?;
        }
        write!(f, ")")
    }
}

#[starlark_value(type = "struct")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for StarlarkInfoGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.get_field(attribute)
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        self.get_field(attribute).is_some()
    }

    fn dir_attr(&self) -> Vec<String> {
        self.field_names().map(str::to_owned).collect()
    }

    // Java: StructImpl.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        let Some(other) = StarlarkInfo::from_value(other) else {
            return Ok(false);
        };
        if !self.provider.to_value().equals(other.provider())? {
            return Ok(false);
        }
        if self.fields.len() != other.fields.len() {
            return Ok(false);
        }
        for ((ka, va), (kb, vb)) in self.fields.iter().zip(other.fields.iter()) {
            if ka != kb || !va.to_value().equals(*vb)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    // Java: StructImpl.hashCode (provider, then sorted field names and values). The provider's
    // own hash is not used, so unhashable provider values do not make instances unhashable.
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.printable_provider_name().hash(hasher);
        for (k, v) in &self.fields {
            k.hash(hasher);
            v.to_value().write_hash(hasher)?;
        }
        Ok(())
    }

    fn compare(&self, other: Value<'v>) -> starlark::Result<Ordering> {
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "unsupported comparison: struct <=> {}",
            other.get_type()
        )))
    }

    // Java: StarlarkInfoNoSchema.binaryOp(PLUS)
    fn add(&self, rhs: Value<'v>, heap: Heap<'v>) -> Option<starlark::Result<Value<'v>>> {
        let other = StarlarkInfo::from_value(rhs)?;
        Some(plus(self, other, heap))
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(StructureMarker { native_info: false });
    }
}

fn plus<'v, V: ValueLike<'v>>(
    x: &StarlarkInfoGen<V>,
    y: &StarlarkInfo<'v>,
    heap: Heap<'v>,
) -> starlark::Result<Value<'v>> {
    if !x.provider.to_value().equals(y.provider())? {
        return Err(starlark::Error::new_other(anyhow::anyhow!(
            "Cannot use '+' operator on instances of different providers ({} and {})",
            x.printable_provider_name(),
            y.printable_provider_name()
        )));
    }
    let mut z: Vec<(String, Value<'v>)> = Vec::with_capacity(x.fields.len() + y.fields.len());
    let (mut xi, mut yi) = (0, 0);
    while xi < x.fields.len() && yi < y.fields.len() {
        let (xk, xv) = (&x.fields[xi].0, x.fields[xi].1.to_value());
        let (yk, yv) = (&y.fields[yi].0, y.fields[yi].1);
        match xk.cmp(yk) {
            Ordering::Less => {
                z.push((xk.clone(), xv));
                xi += 1;
            }
            Ordering::Greater => {
                z.push((yk.clone(), yv));
                yi += 1;
            }
            Ordering::Equal => {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "cannot add struct instances with common field '{xk}'"
                )));
            }
        }
    }
    z.extend(
        x.fields[xi..]
            .iter()
            .map(|(k, v)| (k.clone(), v.to_value())),
    );
    z.extend(y.fields[yi..].iter().map(|(k, v)| (k.clone(), *v)));
    Ok(heap.alloc(StarlarkInfoGen {
        provider: x.provider.to_value(),
        fields: z,
    }))
}
