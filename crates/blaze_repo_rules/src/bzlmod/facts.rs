//! The Starlark side of `Facts`: converting the `facts` argument of `extension_metadata` to
//! [`Facts`] (`Facts.validateAndCreate`), and the `Facts` value that `module_ctx.facts` returns.
//!
//! Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java. The plain data and its lockfile
//! serialization (`FactsAdapter`) are `blaze_bzlmod_model::facts`.

use std::cmp::Ordering;
use std::fmt;

use allocative::Allocative;
use blaze_bzlmod_model::FactValue;
use blaze_bzlmod_model::Facts;
use blaze_bzlmod_model::facts::starlark_float_repr;
use blaze_starlark::printer::quote;
use num_bigint::BigInt;
use starlark::any::ProvidesStaticType;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::FrozenHeap;
use starlark::values::FrozenValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::dict::AllocDict;
use starlark::values::dict::DictRef;
use starlark::values::float::StarlarkFloat;
use starlark::values::list::ListRef;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::tuple::TupleRef;

// This limit only exists to prevent pathological uses of facts, which are meant to be
// human-readable and friendly to VCS merges.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:77 Facts.MAX_FACTS_DEPTH
const MAX_FACTS_DEPTH: i32 = 7;

/// Java `String.compareTo` (UTF-16 code unit order), the order of a `TreeMap<String, _>`.
pub(crate) fn java_string_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `Dict.cast(value, String.class, Object.class, what)`: checks that `value` is a dict with
/// string keys. The error is the `EvalException` message.
// Java: net/starlark/java/eval/Dict.java:659 Dict.cast
pub(crate) fn cast_string_keyed_dict<'v>(
    value: Value<'v>,
    what: &str,
) -> Result<DictRef<'v>, String> {
    let Some(dict) = DictRef::from_value(value) else {
        return Err(format!(
            "got {} for '{}', want dict",
            value.get_type(),
            what
        ));
    };
    for (k, v) in dict.iter() {
        if k.unpack_str().is_none() {
            // TODO(adonovan): change message to "found <K2, V2> entry",
            // without suggesting that the entire dict is <K2, V2>.
            return Err(format!(
                "got dict<{}, {}> for '{}', want dict<string, unknown>",
                k.get_type(),
                v.get_type(),
                what
            ));
        }
    }
    Ok(dict)
}

/// `Facts.validateAndCreate(value)`. The error is the `EvalException` message.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:64 Facts.validateAndCreate
pub fn validate_and_create(value: Value<'_>) -> Result<Facts, String> {
    cast_string_keyed_dict(value, "facts")?;
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:80 Facts.validateAndNormalize
    match validate_and_normalize(value, MAX_FACTS_DEPTH)? {
        FactValue::Dict(entries) => Ok(Facts::create_unchecked(entries)),
        _ => unreachable!("a dict normalizes to a dict"),
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:85 Facts.validateAndNormalize
fn validate_and_normalize(facts: Value<'_>, remaining_depth: i32) -> Result<FactValue, String> {
    if remaining_depth < 0 {
        return Err(format!(
            "Facts cannot be nested more than {MAX_FACTS_DEPTH} levels deep"
        ));
    }
    // Only permit types that can be serialized to JSON and ensure that they contain no information
    // not captured by equality by sorting dicts.
    if let Some(s) = facts.unpack_str() {
        return Ok(FactValue::String(s.to_owned()));
    }
    if facts.is_none() {
        return Ok(FactValue::None);
    }
    if let Some(b) = facts.unpack_bool() {
        return Ok(FactValue::Bool(b));
    }
    if let Some(f) = facts.downcast_ref::<StarlarkFloat>() {
        return Ok(FactValue::Float(f.0));
    }
    if facts.get_type() == "int" {
        let i = facts
            .to_str()
            .parse::<BigInt>()
            .expect("the decimal form of a Starlark int");
        return Ok(FactValue::Int(i));
    }
    if let Some(list) = ListRef::from_value(facts) {
        let mut normalized_list = Vec::with_capacity(list.len());
        for item in list.iter() {
            normalized_list.push(validate_and_normalize(item, remaining_depth - 1)?);
        }
        return Ok(FactValue::List(normalized_list));
    }
    if let Some(tuple) = TupleRef::from_value(facts) {
        // Turn a tuple into a list since JSON does not have a tuple type.
        let mut normalized_list = Vec::with_capacity(tuple.len());
        for item in tuple.iter() {
            normalized_list.push(validate_and_normalize(item, remaining_depth - 1)?);
        }
        return Ok(FactValue::List(normalized_list));
    }
    if let Some(dict) = DictRef::from_value(facts) {
        // TreeMap<String, Object>.
        let mut builder: Vec<(String, FactValue)> = Vec::with_capacity(dict.len());
        for (k, v) in dict.iter() {
            let Some(string) = k.unpack_str() else {
                // Starlark.repr(Map.Entry) is "<repr(key)>: <repr(value)>".
                return Err(format!(
                    "Facts keys must be strings, got '{}: {}' ({})",
                    k.to_repr(),
                    v.to_repr(),
                    k.get_type()
                ));
            };
            let value = validate_and_normalize(v, remaining_depth - 1)?;
            match builder.binary_search_by(|(e, _)| java_string_cmp(e, string)) {
                Ok(i) => builder[i].1 = value,
                Err(i) => builder.insert(i, (string.to_owned(), value)),
            }
        }
        return Ok(FactValue::Dict(builder));
    }
    Err(format!(
        "'{}' ({}) is not supported in facts",
        facts.to_repr(),
        facts.get_type()
    ))
}

/// `Starlark.repr(facts.value())`.
// Java: net/starlark/java/eval/Printer.java:160 Printer.repr
pub fn facts_repr(facts: &Facts) -> String {
    let mut out = String::new();
    repr_entries(facts.value(), &mut out);
    out
}

fn repr_entries(entries: &[(String, FactValue)], out: &mut String) {
    out.push('{');
    for (i, (k, v)) in entries.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&quote(k));
        out.push_str(": ");
        repr_fact_value(v, out);
    }
    out.push('}');
}

fn repr_fact_value(v: &FactValue, out: &mut String) {
    match v {
        FactValue::None => out.push_str("None"),
        FactValue::Bool(b) => out.push_str(if *b { "True" } else { "False" }),
        FactValue::Int(i) => out.push_str(&i.to_string()),
        FactValue::Float(f) => out.push_str(&starlark_float_repr(*f)),
        FactValue::String(s) => out.push_str(&quote(s)),
        FactValue::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                repr_fact_value(item, out);
            }
            out.push(']');
        }
        FactValue::Dict(entries) => repr_entries(entries, out),
    }
}

/// Allocates a fact value on a frozen heap (Java's facts are immutable Starlark objects).
pub fn alloc_frozen_fact_value(v: &FactValue, heap: &FrozenHeap) -> FrozenValue {
    match v {
        FactValue::None => FrozenValue::new_none(),
        FactValue::Bool(b) => FrozenValue::new_bool(*b),
        FactValue::Int(i) => heap.alloc(i.clone()),
        FactValue::Float(f) => heap.alloc(*f),
        FactValue::String(s) => heap.alloc(s.as_str()),
        FactValue::List(items) => heap.alloc(
            items
                .iter()
                .map(|i| alloc_frozen_fact_value(i, heap))
                .collect::<Vec<FrozenValue>>(),
        ),
        FactValue::Dict(entries) => alloc_frozen_dict(entries, heap),
    }
}

fn alloc_frozen_dict(entries: &[(String, FactValue)], heap: &FrozenHeap) -> FrozenValue {
    heap.alloc(AllocDict(
        entries
            .iter()
            .map(|(k, v)| (heap.alloc(k.as_str()), alloc_frozen_fact_value(v, heap)))
            .collect::<Vec<(FrozenValue, FrozenValue)>>(),
    ))
}

/// The `Facts` Starlark value (`module_ctx.facts`): dict-like access to the facts.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:59 Facts
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct FactsValue {
    /// The facts as data (for `debugPrint`).
    #[allocative(skip)]
    facts: Facts,
    /// `value()`: a frozen dict on the extension's frozen heap.
    #[allocative(skip)]
    value: FrozenValue,
}

starlark_simple_value!(FactsValue);

impl FactsValue {
    /// The value of `facts`, allocated on `heap`.
    pub fn new(facts: Facts, heap: &FrozenHeap) -> FactsValue {
        let value = alloc_frozen_dict(facts.value(), heap);
        FactsValue { facts, value }
    }

    fn dict<'v>(&self) -> DictRef<'v> {
        DictRef::from_value(self.value.to_value()).expect("facts are a dict")
    }
}

impl fmt::Debug for FactsValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Facts({:?})", self.facts)
    }
}

impl fmt::Display for FactsValue {
    // Don't leak the contents to Starlark.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:157 Facts.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Facts(<opaque, inspect with print()>)")
    }
}

#[starlark_value(type = "Facts")]
impl<'v> StarlarkValue<'v> for FactsValue {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("Facts", facts_methods);
        Some(RES.methods())
    }

    // Print the contents for debugging purposes.
    // PORT: Java's `debugPrint` (used by `print()`); starlark-rust's `print()` uses `str()`, so
    // `str(facts)` also shows the contents here (Java: the opaque repr).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:163 Facts.debugPrint
    fn collect_str(&self, collector: &mut String) {
        collector.push_str("Facts(");
        collector.push_str(&facts_repr(&self.facts));
        collector.push(')');
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:132 Facts.getIndex
    fn at(&self, index: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        match self.dict().get(index)? {
            Some(v) => Ok(v),
            // Java returns null from getIndex, which EvalUtils.index passes on as a null value.
            None => blaze_util::unported!(
                "Facts.getIndex",
                "key {} is not in the facts (Java yields a null value)",
                index.to_repr()
            ),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:137 Facts.containsKey
    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(self.dict().get(other)?.is_some())
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:176 Facts.checkHashable
    fn write_hash(
        &self,
        _hasher: &mut starlark::collections::StarlarkHasher,
    ) -> starlark::Result<()> {
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "unhashable type: '{}'",
            Self::TYPE
        )))
    }

    // PORT: skipped Facts.isImmutable: a `starlark_simple_value` is always frozen (immutable).
}

#[starlark_module]
fn facts_methods(builder: &mut MethodsBuilder) {
    /// Returns the value for `key` if it exists, or `default`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:152 Facts.get
    fn get<'v>(
        this: &FactsValue,
        key: &str,
        #[starlark(default = NoneType)] default: Value<'v>,
    ) -> starlark::Result<Value<'v>> {
        Ok(this.dict().get_str(key).unwrap_or(default))
    }
}
