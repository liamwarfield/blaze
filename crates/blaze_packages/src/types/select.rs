//! Ports `packages/SelectorValue.java` and `packages/SelectorList.java`: the Starlark values
//! returned by `select({...})` and by `+`/`|` on selects.
//!
//! These are Starlark values (Java stores Starlark objects here): dictionary keys are Label
//! values or label strings, values are arbitrary Starlark values. Rule instantiation converts
//! them to [`crate::types::BuildTypeSelectorList`].
//!
//! The `select()` global itself is registered by the BUILD globals; it calls [`select_impl`].
//!
//! Java's `HasBinary` lets `{...} | select(...)` dispatch to the select: `radd` for `+`, and
//! `rbit_or` (starlark-rust patch 0014) for `|`.

use std::fmt;

use allocative::Allocative;
use blaze_starlark::label::LabelValue;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::starlark_complex_value;
use starlark::values::Freeze;
use starlark::values::FreezeResult;
use starlark::values::Freezer;
use starlark::values::FrozenValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueError;
use starlark::values::ValueLifetimeless;
use starlark::values::ValueLike;
use starlark::values::dict::DictRef;
use starlark::values::starlark_value;

use crate::types::build_type::DEFAULT_CONDITION_KEY;
use crate::types::label_converter::LabelConverter;

fn eval_err(msg: String) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// The value of `select({...})` (Java `SelectorValue`, Starlark type `selector`).
// Java: com/google/devtools/build/lib/packages/SelectorValue.java:44 SelectorValue
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct SelectorValueGen<V: ValueLifetimeless> {
    /// Keys are Label values or label strings; non-empty.
    dictionary: Vec<(V, V)>,
    /// Java `Class<?> type`: the native type of the first value. PORT: the Starlark type name
    /// of the first dictionary value (`"list"`, `"dict"`, ...).
    type_: String,
    no_match_error: String,
}

starlark_complex_value!(pub SelectorValue);

// SAFETY: `repr(C)` and FrozenValue/Value have the same layout.
unsafe impl<'v> Coerce<SelectorValueGen<Value<'v>>> for SelectorValueGen<FrozenValue> {}

impl<'v> SelectorValue<'v> {
    // Java: com/google/devtools/build/lib/packages/SelectorValue.java:54 SelectorValue.<init>
    pub fn new(dictionary: Vec<(Value<'v>, Value<'v>)>, no_match_error: String) -> Self {
        blaze_util::check_argument!(!dictionary.is_empty());
        // TODO(adonovan): doesn't this assume all the elements have the same type?
        let type_ = dictionary[0].1.get_type().to_owned();
        SelectorValueGen {
            dictionary,
            type_,
            no_match_error,
        }
    }
}

impl<'v, V: ValueLike<'v>> SelectorValueGen<V> {
    // Java: com/google/devtools/build/lib/packages/SelectorValue.java:62 SelectorValue.getDictionary
    pub fn get_dictionary(&self) -> impl Iterator<Item = (Value<'v>, Value<'v>)> + '_ {
        self.dictionary
            .iter()
            .map(|(k, v)| (k.to_value(), v.to_value()))
    }

    // Java: com/google/devtools/build/lib/packages/SelectorValue.java:66 SelectorValue.getType
    pub fn get_type(&self) -> &str {
        &self.type_
    }

    /// The custom no-match error, or `""`.
    // Java: com/google/devtools/build/lib/packages/SelectorValue.java:74 SelectorValue.getNoMatchError
    pub fn get_no_match_error(&self) -> &str {
        &self.no_match_error
    }

    /// A copy of this value on `heap` (starlark-rust binary operators do not receive the
    /// operand's own `Value`).
    fn to_heap(&self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc(SelectorValueGen {
            dictionary: self
                .dictionary
                .iter()
                .map(|(k, v)| (k.to_value(), v.to_value()))
                .collect(),
            type_: self.type_.clone(),
            no_match_error: self.no_match_error.clone(),
        })
    }
}

impl<'v> Freeze for SelectorValue<'v> {
    type Frozen = FrozenSelectorValue;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenSelectorValue> {
        Ok(SelectorValueGen {
            dictionary: self.dictionary.freeze(freezer)?,
            type_: self.type_,
            no_match_error: self.no_match_error,
        })
    }
}

impl<'v, V: ValueLike<'v>> fmt::Display for SelectorValueGen<V> {
    // Java: com/google/devtools/build/lib/packages/SelectorValue.java:89 SelectorValue.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "select({{")?;
        for (i, (k, v)) in self.dictionary.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}: {}", k.to_value().to_repr(), v.to_value().to_repr())?;
        }
        write!(f, "}})")
    }
}

#[starlark_value(type = "selector")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for SelectorValueGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    // Java: com/google/devtools/build/lib/packages/SelectorValue.java:94 SelectorValue.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        let Some(that) = SelectorValue::from_value(other) else {
            return Ok(false);
        };
        // ImmutableMap.equals: the same entries, in any order.
        if self.dictionary.len() != that.dictionary.len()
            || self.type_ != that.type_
            || self.no_match_error != that.no_match_error
        {
            return Ok(false);
        }
        for (k, v) in &self.dictionary {
            let mut found = false;
            for (k2, v2) in &that.dictionary {
                if k.to_value().equals(*k2)? {
                    found = v.to_value().equals(*v2)?;
                    break;
                }
            }
            if !found {
                return Ok(false);
            }
        }
        Ok(true)
    }

    // Java: com/google/devtools/build/lib/packages/SelectorValue.java:83 SelectorValue.binaryOp
    fn add(&self, rhs: Value<'v>, heap: Heap<'v>) -> Option<starlark::Result<Value<'v>>> {
        let this = Operand::of_selector_value(self.to_heap(heap), &self.type_);
        binary_op(this, "+", rhs, true, heap)
    }

    fn radd(&self, lhs: Value<'v>, heap: Heap<'v>) -> Option<starlark::Result<Value<'v>>> {
        let this = Operand::of_selector_value(self.to_heap(heap), &self.type_);
        binary_op(this, "+", lhs, false, heap)
    }

    fn bit_or(&self, rhs: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let this = Operand::of_selector_value(self.to_heap(heap), &self.type_);
        match binary_op(this, "|", rhs, true, heap) {
            Some(r) => r,
            None => ValueError::unsupported_with(self, "|", rhs),
        }
    }

    // starlark-rust patch 0014: `{...} | select(...)` (Java HasBinary, thisLeft = false).
    fn rbit_or(&self, lhs: Value<'v>, heap: Heap<'v>) -> Option<starlark::Result<Value<'v>>> {
        let this = Operand::of_selector_value(self.to_heap(heap), &self.type_);
        binary_op(this, "|", lhs, false, heap)
    }
}

/// A concatenation of native values and selects (Java `SelectorList`, Starlark type `select`).
// Java: com/google/devtools/build/lib/packages/SelectorList.java:61 SelectorList
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct SelectorListGen<V: ValueLifetimeless> {
    /// Java `Class<?> type`. PORT: the Starlark type name of the native values.
    type_: String,
    /// Each element is a native value or a [`SelectorValue`].
    elements: Vec<V>,
}

starlark_complex_value!(pub SelectorList);

// SAFETY: `repr(C)` and FrozenValue/Value have the same layout.
unsafe impl<'v> Coerce<SelectorListGen<Value<'v>>> for SelectorListGen<FrozenValue> {}

// Java: com/google/devtools/build/lib/packages/SelectorList.java:168 SelectorList.NATIVE_LIST_TYPE
// Java: com/google/devtools/build/lib/packages/SelectorList.java:191 SelectorList.isListType
fn is_list_type(type_: &str) -> bool {
    // Java `List.class.isAssignableFrom`: StarlarkList and Tuple are Java Lists.
    type_ == "list" || type_ == "tuple"
}

// Java: com/google/devtools/build/lib/packages/SelectorList.java:195 SelectorList.canConcatenate
fn can_concatenate(type1: &str, type2: &str) -> bool {
    if type1 == type2 {
        return true;
    }
    is_list_type(type1) && is_list_type(type2)
}

/// An operand of `SelectorList.of`: its native type, its type name for messages, and the
/// elements it contributes.
struct Operand<'v> {
    native_type: String,
    type_name: String,
    elements: Vec<Value<'v>>,
}

impl<'v> Operand<'v> {
    // Java: com/google/devtools/build/lib/packages/SelectorList.java:182 SelectorList.getNativeType
    // Java: com/google/devtools/build/lib/packages/SelectorList.java:172 SelectorList.getTypeName
    fn of(value: Value<'v>) -> Operand<'v> {
        if let Some(l) = SelectorList::from_value(value) {
            Operand {
                native_type: l.type_.clone(),
                type_name: format!("select of {}", l.type_),
                elements: l.get_elements().collect(),
            }
        } else if let Some(s) = SelectorValue::from_value(value) {
            Operand {
                native_type: s.type_.clone(),
                type_name: format!("select of {}", s.type_),
                elements: vec![value],
            }
        } else {
            Operand {
                native_type: value.get_type().to_owned(),
                type_name: value.get_type().to_owned(),
                elements: vec![value],
            }
        }
    }

    fn of_selector_value(value: Value<'v>, type_: &str) -> Operand<'v> {
        Operand {
            native_type: type_.to_owned(),
            type_name: format!("select of {type_}"),
            elements: vec![value],
        }
    }

    fn of_selector_list<V: ValueLike<'v>>(list: &SelectorListGen<V>) -> Operand<'v> {
        Operand {
            native_type: list.type_.clone(),
            type_name: format!("select of {}", list.type_),
            elements: list.elements.iter().map(|v| v.to_value()).collect(),
        }
    }
}

// Java: com/google/devtools/build/lib/packages/SelectorList.java:126 SelectorList.of
fn of_operands<'v>(operands: Vec<Operand<'v>>) -> starlark::Result<SelectorList<'v>> {
    blaze_util::check_argument!(!operands.is_empty());
    let mut elements: Vec<Value<'v>> = Vec::new();
    for operand in &operands {
        elements.extend(operand.elements.iter().copied());
        let first = &operands[0];
        if !can_concatenate(&first.native_type, &operand.native_type) {
            return Err(eval_err(format!(
                "Cannot combine incompatible types ({}, {})",
                first.type_name, operand.type_name
            )));
        }
    }
    Ok(SelectorListGen {
        type_: operands[0].native_type.clone(),
        elements,
    })
}

// Java: com/google/devtools/build/lib/packages/SelectorList.java:165 SelectorList.binaryOpToken
fn binary_op_token(native_type: &str) -> &'static str {
    if native_type == "dict" { "|" } else { "+" }
}

/// Java `SelectorList.binaryOp(op, that, thisLeft)`: `None` when `op` does not apply to `that`.
// Java: com/google/devtools/build/lib/packages/SelectorList.java:157 SelectorList.binaryOp
fn binary_op<'v>(
    this: Operand<'v>,
    op: &str,
    that: Value<'v>,
    this_left: bool,
    heap: Heap<'v>,
) -> Option<starlark::Result<Value<'v>>> {
    let that = Operand::of(that);
    if op != binary_op_token(&that.native_type) {
        return None;
    }
    let operands = if this_left {
        vec![this, that]
    } else {
        vec![that, this]
    };
    Some(of_operands(operands).map(|l| heap.alloc(l)))
}

impl<'v> SelectorList<'v> {
    // Java: com/google/devtools/build/lib/packages/SelectorList.java:68 SelectorList.<init>
    pub fn new(type_: String, elements: Vec<Value<'v>>) -> Self {
        SelectorListGen { type_, elements }
    }

    /// Creates a "wrapper" list that consists of a single select.
    // Java: com/google/devtools/build/lib/packages/SelectorList.java:116 SelectorList.of
    pub fn of_selector(selector: Value<'v>) -> Self {
        let type_ = match SelectorValue::from_value(selector) {
            Some(s) => s.type_.clone(),
            None => panic!(
                "ClassCastException: {} is not a selector",
                selector.get_type()
            ),
        };
        SelectorListGen {
            type_,
            elements: vec![selector],
        }
    }

    /// Creates a list from non-empty `values` (native values, selects or selector lists over the
    /// same type).
    // Java: com/google/devtools/build/lib/packages/SelectorList.java:126 SelectorList.of
    pub fn of(values: &[Value<'v>]) -> starlark::Result<Self> {
        of_operands(values.iter().map(|v| Operand::of(*v)).collect())
    }

    /// Wraps a single value in a `select()` whose default condition maps to it.
    // Java: com/google/devtools/build/lib/packages/SelectorList.java:150 SelectorList.wrapSingleValue
    pub fn wrap_single_value(obj: Value<'v>, heap: Heap<'v>) -> Self {
        let key = heap.alloc(DEFAULT_CONDITION_KEY);
        let selector = heap.alloc(SelectorValue::new(vec![(key, obj)], String::new()));
        SelectorList::of_selector(selector)
    }

    /// Concatenates two values (native, select or selector list).
    // Java: com/google/devtools/build/lib/packages/SelectorList.java:161 SelectorList.concat
    pub fn concat(x: Value<'v>, y: Value<'v>) -> starlark::Result<Self> {
        Self::of(&[x, y])
    }
}

impl<'v, V: ValueLike<'v>> SelectorListGen<V> {
    /// The elements in order; each is a native value or a [`SelectorValue`].
    // Java: com/google/devtools/build/lib/packages/SelectorList.java:77 SelectorList.getElements
    pub fn get_elements(&self) -> impl Iterator<Item = Value<'v>> + '_ {
        self.elements.iter().map(|v| v.to_value())
    }

    /// The native type contained by this expression.
    // Java: com/google/devtools/build/lib/packages/SelectorList.java:82 SelectorList.getType
    pub fn get_type(&self) -> &str {
        &self.type_
    }
}

impl<'v> Freeze for SelectorList<'v> {
    type Frozen = FrozenSelectorList;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenSelectorList> {
        Ok(SelectorListGen {
            type_: self.type_,
            elements: self.elements.freeze(freezer)?,
        })
    }
}

impl<'v, V: ValueLike<'v>> fmt::Display for SelectorListGen<V> {
    // Java: com/google/devtools/build/lib/packages/SelectorList.java:220 SelectorList.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sep = format!(" {} ", binary_op_token(&self.type_));
        for (i, e) in self.elements.iter().enumerate() {
            if i > 0 {
                f.write_str(&sep)?;
            }
            f.write_str(&e.to_value().to_repr())?;
        }
        Ok(())
    }
}

#[starlark_value(type = "select")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for SelectorListGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    // Java: com/google/devtools/build/lib/packages/SelectorList.java:230 SelectorList.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        let Some(that) = SelectorList::from_value(other) else {
            return Ok(false);
        };
        if self.type_ != that.type_ || self.elements.len() != that.elements.len() {
            return Ok(false);
        }
        for (a, b) in self.elements.iter().zip(that.elements.iter()) {
            if !a.to_value().equals(*b)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    // Java: com/google/devtools/build/lib/packages/SelectorList.java:157 SelectorList.binaryOp
    fn add(&self, rhs: Value<'v>, heap: Heap<'v>) -> Option<starlark::Result<Value<'v>>> {
        binary_op(Operand::of_selector_list(self), "+", rhs, true, heap)
    }

    fn radd(&self, lhs: Value<'v>, heap: Heap<'v>) -> Option<starlark::Result<Value<'v>>> {
        binary_op(Operand::of_selector_list(self), "+", lhs, false, heap)
    }

    fn bit_or(&self, rhs: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        match binary_op(Operand::of_selector_list(self), "|", rhs, true, heap) {
            Some(r) => r,
            None => ValueError::unsupported_with(self, "|", rhs),
        }
    }

    // starlark-rust patch 0014: `{...} | select(...)` (Java HasBinary, thisLeft = false).
    fn rbit_or(&self, lhs: Value<'v>, heap: Heap<'v>) -> Option<starlark::Result<Value<'v>>> {
        binary_op(Operand::of_selector_list(self), "|", lhs, false, heap)
    }
}

/// Implementation of the Starlark `select(x, no_match_error = '')` function. `label_converter`
/// is set when `--incompatible_resolve_select_keys_eagerly` is on (Java
/// `SelectLibrary.select` chooses it: the initializer's converter, else the BUILD file's
/// `TargetDefinitionContext.getLabelConverter()`, else `LabelConverter.forBzlEvaluatingThread`);
/// string keys are then resolved to Labels.
// Java: com/google/devtools/build/lib/packages/SelectorList.java:88 SelectorList.select
pub fn select_impl<'v>(
    dict: Value<'v>,
    no_match_error: &str,
    label_converter: Option<&LabelConverter>,
    heap: Heap<'v>,
) -> starlark::Result<Value<'v>> {
    let Some(dict) = DictRef::from_value(dict) else {
        // PORT: Java's @Param type check of `x`.
        return Err(eval_err(format!(
            "in call to select(), parameter 'x' got value of type '{}', want 'dict'",
            dict.get_type()
        )));
    };
    if dict.is_empty() {
        return Err(eval_err(
            "select({}) with an empty dictionary can never resolve because it includes no \
             conditions to match"
                .to_owned(),
        ));
    }
    let mut select_dict: Vec<(Value<'v>, Value<'v>)> = Vec::with_capacity(dict.len());
    for (key, value) in dict.iter() {
        let key = if key.downcast_ref::<LabelValue>().is_some() {
            key
        } else if let Some(label_string) = key.unpack_str() {
            match label_converter {
                Some(lc) => match lc.convert(label_string) {
                    Ok(label) => heap.alloc(LabelValue(label)),
                    Err(e) => {
                        return Err(eval_err(format!(
                            "invalid label in select(): {}",
                            e.get_message()
                        )));
                    }
                },
                None => key,
            }
        } else {
            return Err(eval_err(format!(
                "select: got {} for dict key, want a Label or label string",
                key.get_type()
            )));
        };
        // ImmutableMap.Builder.buildOrThrow: duplicate keys (strings that resolve to the same
        // label) are an IllegalArgumentException.
        for (k, v) in &select_dict {
            if k.equals(key)? {
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {}={} and {}={}",
                    key.to_str(),
                    value.to_str(),
                    k.to_str(),
                    v.to_str()
                );
            }
        }
        select_dict.push((key, value));
    }
    let selector = heap.alloc(SelectorValue::new(select_dict, no_match_error.to_owned()));
    Ok(heap.alloc(SelectorList::of_selector(selector)))
}
