//! Conversions between the three forms of a repo rule or tag attribute value.
//!
//! - Starlark values (`Value`), as passed to and returned from Starlark;
//! - converted attribute values ([`AttrValue`], Java's "native" attribute value objects after
//!   `Type.convert`);
//! - [`AttributeValue`], the plain-data form `RepoSpec` / `AttributeValues` store (Java stores
//!   the Starlark objects `Attribute.valueToStarlark` returns).

use blaze_bzlmod_model::AttributeValue;
use blaze_packages::types::AttrValue;
use blaze_starlark::label::LabelValue;
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::dict::AllocDict;
use starlark::values::dict::DictRef;
use starlark::values::list::AllocList;
use starlark::values::list::ListRef;
use starlark::values::tuple::TupleRef;

/// The Starlark object for a stored attribute value (Java: the stored object itself).
///
/// PORT: Java's lists here are immutable `StarlarkList`s and dicts immutable `Dict`s; Blaze
/// allocates ordinary (mutable) ones on the evaluation heap.
pub fn attribute_value_to_starlark<'v>(value: &AttributeValue, heap: Heap<'v>) -> Value<'v> {
    match value {
        AttributeValue::None => Value::new_none(),
        AttributeValue::Bool(b) => Value::new_bool(*b),
        AttributeValue::Int(i) => heap.alloc(*i),
        AttributeValue::String(s) => heap.alloc_str(s).to_value(),
        AttributeValue::Label(l) => heap.alloc(LabelValue(l.clone())),
        AttributeValue::List(items) => heap.alloc(AllocList(
            items.iter().map(|i| attribute_value_to_starlark(i, heap)),
        )),
        AttributeValue::Dict(entries) => heap.alloc(AllocDict(entries.iter().map(|(k, v)| {
            (
                attribute_value_to_starlark(k, heap),
                attribute_value_to_starlark(v, heap),
            )
        }))),
    }
}

/// `Starlark.repr` of a stored attribute value (Java calls it on the stored Starlark object).
// Java: net/starlark/java/eval/Starlark.java:530 Starlark.repr
pub fn starlark_repr(value: &AttributeValue) -> String {
    starlark::environment::Module::with_temp_heap(|module| {
        attribute_value_to_starlark(value, module.heap()).to_repr()
    })
}

/// A Starlark value as stored data, deep-copying containers (Java
/// `ModuleExtensionEvalStarlarkThreadContext.deepCloneAttrValue`). Fails with Java's message
/// for values that cannot be attribute values.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalStarlarkThreadContext.java:158 ModuleExtensionEvalStarlarkThreadContext.deepCloneAttrValue
pub fn starlark_to_attribute_value(x: Value<'_>) -> Result<AttributeValue, String> {
    if x.is_none() {
        return Ok(AttributeValue::None);
    }
    if let Some(b) = x.unpack_bool() {
        return Ok(AttributeValue::Bool(b));
    }
    if let Some(i) = x.unpack_i32() {
        return Ok(AttributeValue::Int(i as i64));
    }
    if let Some(s) = x.unpack_str() {
        return Ok(AttributeValue::String(s.to_owned()));
    }
    if let Some(l) = x.downcast_ref::<LabelValue>() {
        return Ok(AttributeValue::Label(l.0.clone()));
    }
    // Mutable Starlark values have to be cloned deeply.
    if let Some(dict) = DictRef::from_value(x) {
        let mut entries = Vec::with_capacity(dict.len());
        for (k, v) in dict.iter() {
            entries.push((
                starlark_to_attribute_value(k)?,
                starlark_to_attribute_value(v)?,
            ));
        }
        return Ok(AttributeValue::Dict(entries));
    }
    if let Some(list) = ListRef::from_value(x) {
        return Ok(AttributeValue::List(
            list.iter()
                .map(starlark_to_attribute_value)
                .collect::<Result<_, _>>()?,
        ));
    }
    if let Some(tuple) = TupleRef::from_value(x) {
        return Ok(AttributeValue::List(
            tuple
                .iter()
                .map(starlark_to_attribute_value)
                .collect::<Result<_, _>>()?,
        ));
    }
    if x.get_type() == "int" {
        // PORT: Java StarlarkInt is arbitrary-precision; values outside i32 are kept as i64
        // when they fit (AttributeValue::Int).
        if let Ok(s) = x.to_str().parse::<i64>() {
            return Ok(AttributeValue::Int(s));
        }
    }
    Err(format!(
        "unexpected Starlark value: {} (of type {})",
        x.to_repr(),
        x.get_type()
    ))
}

/// The stored form of a converted attribute value: Java `Attribute.valueToStarlark(nativeValue)`.
// Java: com/google/devtools/build/lib/packages/Attribute.java:2442 Attribute.valueToStarlark
pub fn attr_value_to_attribute_value(value: Option<&AttrValue>) -> AttributeValue {
    let Some(value) = value else {
        // Starlark.fromJava(null) is None.
        return AttributeValue::None;
    };
    let s = |s: &String| AttributeValue::String(s.clone());
    let l = |l: &blaze_cmdline::Label| AttributeValue::Label(l.clone());
    match value {
        AttrValue::Int(i) => AttributeValue::Int(*i as i64),
        AttrValue::String(v) => s(v),
        AttrValue::Bool(b) => AttributeValue::Bool(*b),
        AttrValue::Label(v) => l(v),
        AttrValue::StringList(v) => AttributeValue::List(v.iter().map(s).collect()),
        // Until Starlark gains a set data type, shallow-convert Java sets to Starlark lists.
        AttrValue::StringSet(v) => AttributeValue::List(v.iter().map(s).collect()),
        AttrValue::IntList(v) => {
            AttributeValue::List(v.iter().map(|i| AttributeValue::Int(*i as i64)).collect())
        }
        AttrValue::LabelList(v) => AttributeValue::List(v.iter().map(l).collect()),
        AttrValue::StringDict(m) => {
            AttributeValue::Dict(m.iter().map(|(k, v)| (s(k), s(v))).collect())
        }
        AttrValue::StringListDict(m) => AttributeValue::Dict(
            m.iter()
                .map(|(k, v)| (s(k), AttributeValue::List(v.iter().map(s).collect())))
                .collect(),
        ),
        AttrValue::LabelDictUnary(m) => {
            AttributeValue::Dict(m.iter().map(|(k, v)| (s(k), l(v))).collect())
        }
        AttrValue::LabelKeyedStringDict(m) => {
            AttributeValue::Dict(m.iter().map(|(k, v)| (l(k), s(v))).collect())
        }
        AttrValue::LabelListDict(m) => AttributeValue::Dict(
            m.iter()
                .map(|(k, v)| (s(k), AttributeValue::List(v.iter().map(l).collect())))
                .collect(),
        ),
        // Convert TriState to integer (same as in query output and native.existing_rules())
        AttrValue::TriState(t) => AttributeValue::Int(t.to_int() as i64),
        other => blaze_util::unported!(
            "Attribute.valueToStarlark",
            "attribute value {other:?} in a repo rule or tag"
        ),
    }
}
