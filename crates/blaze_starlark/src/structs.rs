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
use std::hash::Hasher;

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
    // Types whose Java class keeps StarlarkValue's default (`isImmutable` false) whether or not
    // the value is frozen: `RangeList` and `BuiltinFunction` (a global such as `len` is frozen
    // here).
    if matches!(v.get_type(), "range" | "builtin_function_or_method") {
        return false;
    }
    // A provider decides by itself: `StarlarkProvider.isImmutable` is `isExported()` (it provides
    // `DeeplyImmutable` once exported); a `BuiltinProvider` (`struct`, `DefaultInfo`, ...) keeps
    // the default, false, though it is frozen here.
    if provider_info(v).is_some() {
        return v.request_value::<DeeplyImmutable>().is_some();
    }
    if v.unpack_frozen().is_some() {
        return true;
    }
    match v.get_type() {
        "NoneType" | "bool" | "int" | "float" | "string" => return true,
        // Java: StarlarkList.isImmutable (ImmutableStarlarkList / frozen lists).
        "list" => return crate::list::list_is_immutable(v) == Some(true),
        // Java: Dict.isImmutable (Dict.immutableCopyOf / frozen dicts).
        "dict" => return crate::dict::dict_is_immutable(v) == Some(true),
        // Java: StarlarkSet.isImmutable (StarlarkSet.immutableCopyOf / frozen sets).
        "set" => return crate::set::set_is_immutable(v) == Some(true),
        // Java: StarlarkFunction.isImmutable is true (StarlarkFunction.java:273); a
        // BuiltinFunction keeps StarlarkValue's default, false.
        "function" => return true,
        "builtin_function_or_method" => return false,
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
    // Java: the call-stack name of a builtin is `getName()`: `BuiltinProvider.getName` is "struct"
    // (a traceback reads `Error in struct: ...`), not the repr `<function struct>`.
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:127 BuiltinProvider.getName
    fn name_for_call_stack(&self, _me: Value<'v>) -> String {
        "struct".to_owned()
    }

    fn invoke(
        &self,
        me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        // Java: StructProvider.createStruct(Dict kwargs): `struct` takes no positional
        // parameter, so any positional argument is rejected by the builtin's argument check.
        // Java: net/starlark/java/eval/BuiltinFunction.java:154 BuiltinFunction.getArgumentVector
        if args.positions(eval.heap())?.next().is_some() {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "struct() got unexpected positional argument"
            )));
        }
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
    /// Java `StarlarkInfoWithMessage.unknownFieldError`: a format with one `%s` (the field
    /// name); `None` for a plain struct (the provider's message).
    unknown_field_error: Option<String>,
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
        StarlarkInfoGen {
            provider,
            fields,
            unknown_field_error: None,
        }
    }

    /// Java `StarlarkInfoWithMessage.createWithCustomMessage(provider, values,
    /// unknownFieldError)`: a struct whose unknown-field error is `unknown_field_error` (a format
    /// with one `%s`, the field name), followed by the list of fields (`ctx.attr`, the `attr`
    /// of a Starlark transition).
    // Java: com/google/devtools/build/lib/packages/StarlarkInfoWithMessage.java:66 StarlarkInfoWithMessage.createWithCustomMessage
    pub fn with_message(
        provider: Value<'v>,
        fields: Vec<(String, Value<'v>)>,
        unknown_field_error: &str,
    ) -> Self {
        let mut info = Self::new(provider, fields);
        info.unknown_field_error = Some(unknown_field_error.to_owned());
        info
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

    /// Java: StructImpl.getErrorMessageForUnknownField (+ allAttributesSuffix), or
    /// StarlarkInfoWithMessage's format.
    // Java: com/google/devtools/build/lib/packages/StarlarkInfoWithMessage.java:51 StarlarkInfoWithMessage.getErrorMessageForUnknownField
    pub fn unknown_field_message(&self, name: &str) -> String {
        let head = match &self.unknown_field_error {
            // Java String.format(unknownFieldError, name): the formats have one `%s`.
            Some(format) => format.replacen("%s", name, 1),
            None => format!(
                "'{}' value has no field or method '{}'",
                self.printable_provider_name(),
                name
            ),
        };
        format!(
            "{head}\nAvailable attributes: {}",
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
            unknown_field_error: self.unknown_field_error,
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

    // Java: net/starlark/java/eval/Structure.java:80 Structure.setField (StarlarkInfo does not
    // override it; reached from EvalUtils.setField because a struct is a Structure).
    fn set_attr(&self, _attribute: &str, _new_value: Value<'v>) -> starlark::Result<()> {
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "{} value does not support field assignment",
            Self::TYPE
        )))
    }

    // Java: com/google/devtools/build/lib/packages/StructImpl.java:86 StructImpl.getErrorMessageForUnknownField
    // (raised by net/starlark/java/eval/Starlark.java:1006 Starlark.getattr; starlark-rust patch 0013)
    fn get_attr_error_message(&self, attribute: &str) -> Option<String> {
        Some(self.unknown_field_message(attribute))
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
        // Java: net/starlark/java/eval/StarlarkValue.java:104 StarlarkValue.checkHashable, which
        // StarlarkInfo inherits: a struct is hashable iff it is immutable (an exported provider
        // and immutable field values; a frozen list counts, a mutable one does not).
        // PORT: starlark-rust has one hook for Java's checkHashable and hashCode.
        if !self.is_immutable() {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "unhashable type: 'struct'"
            )));
        }
        self.printable_provider_name().hash(hasher);
        for (k, v) in &self.fields {
            k.hash(hasher);
            java_hash_code(v.to_value(), hasher)?;
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
    // Java StarlarkInfo.plus: a StarlarkInfoNoSchema (the provider's message).
    Ok(heap.alloc(StarlarkInfoGen {
        provider: x.provider.to_value(),
        fields: z,
        unknown_field_error: None,
    }))
}

/// Java `Object.hashCode` of a struct field value, as `StarlarkInfo.hashCode` uses it: lists and
/// tuples hash by their elements in order (`StarlarkList.hashCode`, `Tuple.hashCode`), dicts by
/// the order-independent sum of their entries (`Map.hashCode`), where Starlark's own `hash`
/// rejects lists and dicts.
// Java: net/starlark/java/eval/StarlarkList.java:254 StarlarkList.hashCode
// Java: net/starlark/java/eval/Dict.java Dict.hashCode
// W4-analysis: lead-applied, root-approved.
fn java_hash_code(v: Value, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
    if let Some(list) = starlark::values::list::ListRef::from_value(v) {
        "list".hash(hasher);
        for e in list.iter() {
            java_hash_code(e, hasher)?;
        }
        return Ok(());
    }
    if let Some(t) = TupleRef::from_value(v) {
        "tuple".hash(hasher);
        for e in t.content() {
            java_hash_code(*e, hasher)?;
        }
        return Ok(());
    }
    if let Some(d) = starlark::values::dict::DictRef::from_value(v) {
        "dict".hash(hasher);
        let mut sum: u64 = 0;
        for (k, e) in d.iter() {
            let mut entry = StarlarkHasher::new();
            java_hash_code(k, &mut entry)?;
            java_hash_code(e, &mut entry)?;
            sum = sum.wrapping_add(entry.finish());
        }
        sum.hash(hasher);
        return Ok(());
    }
    v.write_hash(hasher)
}
