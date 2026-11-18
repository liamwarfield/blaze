//! Ports `packages/Type.java`, `packages/Types.java` and the type constants of
//! `packages/BuildType.java` as one closed enum (crates/blaze_packages/DESIGN.md "Types").
//!
//! Java's `Type<T>` instances become [`Type`] variants; converted values are [`AttrValue`]s.
//! Java identity comparisons (`type == BuildType.LABEL`) are `==` on the enum.

use std::fmt;

use blaze_cmdline::Label;
use blaze_starlark::depset::Depset;
use blaze_starlark::label::LabelValue;
use indexmap::IndexMap;
use indexmap::IndexSet;
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::dict::AllocDict;
use starlark::values::dict::DictRef;
use starlark::values::list::AllocList;
use starlark::values::list::ListRef;
use starlark::values::set::SetRef;
use starlark::values::tuple::TupleRef;

use crate::attribute::Attribute;
use crate::types::attr_value::AttrValue;
use crate::types::conversion_exception::ConversionException;
use crate::types::label_converter::LabelConverter;
use crate::types::license::License;
use crate::types::tri_state::TriState;

/// Classifications of labels by their usage (Java `Type.LabelClass`).
// Java: com/google/devtools/build/lib/packages/Type.java:213 Type.LabelClass
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LabelClass {
    /// Used for types which are not labels.
    None,
    /// Used for types which use labels to declare a dependency.
    Dependency,
    /// Used for types which use labels to reference another target but do not declare a
    /// dependency, in cases where doing so would cause a dependency cycle.
    NondepReference,
    /// Used for types which declare a dependency, but the dependency should not be configured.
    GenqueryScopeReference,
    /// Used for types which use labels to declare an output path.
    Output,
}

/// An attribute type: one variant per Java `Type` constant (`Type.*`, `Types.*`, `BuildType.*`).
// Java: com/google/devtools/build/lib/packages/Type.java:89 Type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Type {
    // Java: com/google/devtools/build/lib/packages/Type.java:270 Type.INTEGER
    Integer,
    // Java: com/google/devtools/build/lib/packages/Type.java:274 Type.STRING
    String,
    // Java: com/google/devtools/build/lib/packages/Type.java:285 Type.STRING_NO_INTERN
    StringNoIntern,
    // Java: com/google/devtools/build/lib/packages/Type.java:288 Type.BOOLEAN
    Boolean,
    // Java: com/google/devtools/build/lib/packages/Types.java:36 Types.STRING_LIST
    StringList,
    // Java: com/google/devtools/build/lib/packages/Types.java:40 Types.STRING_SET
    StringSet,
    // Java: com/google/devtools/build/lib/packages/Types.java:44 Types.INTEGER_LIST
    IntegerList,
    // Java: com/google/devtools/build/lib/packages/Types.java:48 Types.STRING_DICT
    StringDict,
    // Java: com/google/devtools/build/lib/packages/Types.java:53 Types.STRING_LIST_DICT
    StringListDict,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:60 BuildType.LABEL
    Label,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:63 BuildType.LABEL_DICT_UNARY
    LabelDictUnary,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:67 BuildType.LABEL_KEYED_STRING_DICT
    LabelKeyedStringDict,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:70 BuildType.LABEL_LIST
    LabelList,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:74 BuildType.LABEL_LIST_DICT
    LabelListDict,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:83 BuildType.NODEP_LABEL
    NodepLabel,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:86 BuildType.NODEP_LABEL_LIST
    NodepLabelList,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:89 BuildType.DORMANT_LABEL
    DormantLabel,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:93 BuildType.DORMANT_LABEL_LIST
    DormantLabelList,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:105 BuildType.GENQUERY_SCOPE_TYPE
    GenqueryScopeType,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:113 BuildType.GENQUERY_SCOPE_TYPE_LIST
    GenqueryScopeTypeList,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:120 BuildType.LICENSE
    License,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:123 BuildType.OUTPUT
    Output,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:132 BuildType.OUTPUT_LIST
    OutputList,
    // Java: com/google/devtools/build/lib/packages/BuildType.java:135 BuildType.TRISTATE
    Tristate,
}

/// Java `Type.LabelVisitor`: receives each label of a value and the attribute it came from.
// PORT: Java passes a @Nullable Label; callers never visit null labels of set values, so the
// label is a reference here.
// Java: com/google/devtools/build/lib/packages/Type.java:196 Type.LabelVisitor
pub type LabelVisitor<'a> = dyn FnMut(&Label, Option<&Attribute>) + 'a;

impl Type {
    /// Java `instanceof Type.ListType`.
    pub fn is_list_type(self) -> bool {
        matches!(
            self,
            Type::StringList
                | Type::IntegerList
                | Type::LabelList
                | Type::NodepLabelList
                | Type::DormantLabelList
                | Type::GenqueryScopeTypeList
                | Type::OutputList
        )
    }

    /// Java `instanceof Type.SetType`.
    pub fn is_set_type(self) -> bool {
        matches!(self, Type::StringSet)
    }

    /// Java `instanceof Type.DictType` (including `BuildType.LabelKeyedDictType`).
    pub fn is_dict_type(self) -> bool {
        matches!(
            self,
            Type::StringDict
                | Type::StringListDict
                | Type::LabelDictUnary
                | Type::LabelKeyedStringDict
                | Type::LabelListDict
        )
    }

    /// For collection types, the element type; `None` otherwise.
    // Java: com/google/devtools/build/lib/packages/Type.java:288 Type.getListElementType
    pub fn get_list_element_type(self) -> Option<Type> {
        match self {
            Type::StringList | Type::StringSet => Some(Type::String),
            Type::IntegerList => Some(Type::Integer),
            Type::LabelList => Some(Type::Label),
            Type::NodepLabelList => Some(Type::NodepLabel),
            Type::DormantLabelList => Some(Type::DormantLabel),
            Type::GenqueryScopeTypeList => Some(Type::GenqueryScopeType),
            Type::OutputList => Some(Type::Output),
            _ => None,
        }
    }

    /// For dict types, the key type (Java `DictType.getKeyType`).
    // Java: com/google/devtools/build/lib/packages/Type.java:540 Type.DictType.getKeyType
    pub fn get_key_type(self) -> Option<Type> {
        match self {
            Type::StringDict
            | Type::StringListDict
            | Type::LabelDictUnary
            | Type::LabelListDict => Some(Type::String),
            Type::LabelKeyedStringDict => Some(Type::Label),
            _ => None,
        }
    }

    /// For dict types, the value type (Java `DictType.getValueType`).
    // Java: com/google/devtools/build/lib/packages/Type.java:544 Type.DictType.getValueType
    pub fn get_value_type(self) -> Option<Type> {
        match self {
            Type::StringDict | Type::LabelKeyedStringDict => Some(Type::String),
            Type::StringListDict => Some(Type::StringList),
            Type::LabelDictUnary => Some(Type::Label),
            Type::LabelListDict => Some(Type::LabelList),
            _ => None,
        }
    }

    /// Returns the class of labels contained by this type, if any.
    // Java: com/google/devtools/build/lib/packages/Type.java:233 Type.getLabelClass
    pub fn get_label_class(self) -> LabelClass {
        match self {
            Type::Label
            | Type::LabelList
            | Type::LabelDictUnary
            | Type::LabelKeyedStringDict
            | Type::LabelListDict => LabelClass::Dependency,
            Type::NodepLabel | Type::NodepLabelList => LabelClass::NondepReference,
            Type::DormantLabel
            | Type::DormantLabelList
            | Type::GenqueryScopeType
            | Type::GenqueryScopeTypeList => LabelClass::GenqueryScopeReference,
            Type::Output | Type::OutputList => LabelClass::Output,
            Type::Integer
            | Type::String
            | Type::StringNoIntern
            | Type::Boolean
            | Type::StringList
            | Type::StringSet
            | Type::IntegerList
            | Type::StringDict
            | Type::StringListDict
            | Type::License
            | Type::Tristate => LabelClass::None,
        }
    }

    /// Returns whether the specified type is a label type or not.
    // Java: com/google/devtools/build/lib/packages/BuildType.java:142 BuildType.isLabelType
    pub fn is_label_type(self) -> bool {
        self.get_label_class() != LabelClass::None
    }

    /// The default value for this type; `None` iff no default is defined (labels, outputs).
    // Java: com/google/devtools/build/lib/packages/Type.java:190 Type.getDefaultValue
    pub fn get_default_value(self) -> Option<AttrValue> {
        match self {
            Type::Integer => Some(AttrValue::Int(0)),
            Type::String | Type::StringNoIntern => Some(AttrValue::String(String::new())),
            Type::Boolean => Some(AttrValue::Bool(false)),
            Type::StringList => Some(AttrValue::StringList(Vec::new())),
            Type::StringSet => Some(AttrValue::StringSet(IndexSet::new())),
            Type::IntegerList => Some(AttrValue::IntList(Vec::new())),
            Type::StringDict => Some(AttrValue::StringDict(Default::default())),
            Type::StringListDict => Some(AttrValue::StringListDict(Default::default())),
            Type::LabelDictUnary => Some(AttrValue::LabelDictUnary(Default::default())),
            Type::LabelKeyedStringDict => Some(AttrValue::LabelKeyedStringDict(Default::default())),
            Type::LabelListDict => Some(AttrValue::LabelListDict(Default::default())),
            Type::LabelList
            | Type::NodepLabelList
            | Type::DormantLabelList
            | Type::GenqueryScopeTypeList
            | Type::OutputList => Some(AttrValue::LabelList(Vec::new())),
            // Labels have no default value.
            Type::Label
            | Type::NodepLabel
            | Type::DormantLabel
            | Type::GenqueryScopeType
            | Type::Output => None,
            Type::License => Some(AttrValue::License(License::no_license())),
            Type::Tristate => Some(AttrValue::TriState(TriState::Auto)),
        }
    }

    /// Converts a legal Starlark value `x` into a value of this type. `what` describes `x` in
    /// error messages (Java `Object what`, `None` is Java null); `label_converter` must be given
    /// to parse non-canonical label strings.
    // Java: com/google/devtools/build/lib/packages/Type.java:110 Type.convert
    pub fn convert<'v>(
        self,
        x: Value<'v>,
        what: Option<&dyn fmt::Display>,
        label_converter: Option<&LabelConverter>,
    ) -> Result<AttrValue, ConversionException> {
        self.convert_impl(x, what, label_converter, None)
    }

    /// [`Type::convert`] with the heap of `x`, which starlark-rust needs to iterate Starlark
    /// `set` and `range` values (Java Iterables). Use it wherever a heap is available.
    // Java: com/google/devtools/build/lib/packages/Type.java:110 Type.convert
    pub fn convert_with_heap<'v>(
        self,
        x: Value<'v>,
        what: Option<&dyn fmt::Display>,
        label_converter: Option<&LabelConverter>,
        heap: Heap<'v>,
    ) -> Result<AttrValue, ConversionException> {
        self.convert_impl(x, what, label_converter, Some(heap))
    }

    // Java: com/google/devtools/build/lib/packages/Type.java:110 Type.convert
    fn convert_impl<'v>(
        self,
        x: Value<'v>,
        what: Option<&dyn fmt::Display>,
        label_converter: Option<&LabelConverter>,
        heap: Option<Heap<'v>>,
    ) -> Result<AttrValue, ConversionException> {
        match self {
            Type::Integer => convert_integer(x, what).map(AttrValue::Int),
            Type::Boolean => convert_boolean(x, what, label_converter).map(AttrValue::Bool),
            Type::String | Type::StringNoIntern => convert_string(self, x, what),
            Type::Label | Type::NodepLabel | Type::DormantLabel | Type::GenqueryScopeType => {
                convert_label(x, what, label_converter).map(AttrValue::Label)
            }
            Type::Output => convert_output(x, what, label_converter).map(AttrValue::Label),
            Type::License => convert_license(x, what),
            Type::Tristate => convert_tristate(x, what, label_converter).map(AttrValue::TriState),
            Type::StringSet => convert_set(self, x, what, label_converter, heap),
            Type::StringList
            | Type::IntegerList
            | Type::LabelList
            | Type::NodepLabelList
            | Type::DormantLabelList
            | Type::GenqueryScopeTypeList
            | Type::OutputList => convert_list(self, x, what, label_converter, heap),
            Type::StringDict
            | Type::StringListDict
            | Type::LabelDictUnary
            | Type::LabelKeyedStringDict
            | Type::LabelListDict => convert_dict(self, x, what, label_converter, heap),
        }
    }

    /// Like [`Type::convert`], but converts Starlark `None` to Java null (`Ok(None)`).
    // Java: com/google/devtools/build/lib/packages/Type.java:166 Type.convertOptional
    pub fn convert_optional<'v>(
        self,
        x: Value<'v>,
        what: Option<&dyn fmt::Display>,
        label_converter: Option<&LabelConverter>,
    ) -> Result<Option<AttrValue>, ConversionException> {
        self.convert_optional_with_default(x, what, label_converter, None)
    }

    /// Like [`Type::convert`], but converts Starlark `None` to `default_value`.
    // Java: com/google/devtools/build/lib/packages/Type.java:152 Type.convertOptional
    pub fn convert_optional_with_default<'v>(
        self,
        x: Value<'v>,
        what: Option<&dyn fmt::Display>,
        label_converter: Option<&LabelConverter>,
        default_value: Option<AttrValue>,
    ) -> Result<Option<AttrValue>, ConversionException> {
        if x.is_none() {
            return Ok(default_value);
        }
        self.convert(x, what, label_converter).map(Some)
    }

    /// Copies a Starlark value to an immutable one, converting label strings to Labels and type
    /// checking it (Java `Type.copyAndLiftStarlarkValue`).
    // PORT: Java returns immutable Starlark lists and dicts; starlark-rust values allocated here
    // are ordinary (mutable) lists and dicts.
    // Java: com/google/devtools/build/lib/packages/Type.java:126 Type.copyAndLiftStarlarkValue
    pub fn copy_and_lift_starlark_value<'v>(
        self,
        x: Value<'v>,
        what: Option<&dyn fmt::Display>,
        label_converter: Option<&LabelConverter>,
        heap: Heap<'v>,
    ) -> Result<Value<'v>, ConversionException> {
        match self {
            // Java: com/google/devtools/build/lib/packages/Type.java:768 Type.ListType.copyAndLiftStarlarkValue
            Type::StringList
            | Type::IntegerList
            | Type::LabelList
            | Type::NodepLabelList
            | Type::DormantLabelList
            | Type::GenqueryScopeTypeList
            | Type::OutputList => {
                let converted = self.convert_with_heap(x, what, label_converter, heap)?;
                Ok(native_to_starlark(&converted, heap))
            }
            // Java: com/google/devtools/build/lib/packages/Type.java:846 Type.SetType.copyAndLiftStarlarkValue
            Type::StringSet => {
                let converted = self.convert_with_heap(x, what, label_converter, heap)?;
                Ok(native_to_starlark(&converted, heap))
            }
            // Java: com/google/devtools/build/lib/packages/Type.java:583 Type.DictType.copyAndLiftStarlarkValue
            Type::StringDict
            | Type::StringListDict
            | Type::LabelDictUnary
            | Type::LabelKeyedStringDict
            | Type::LabelListDict => {
                let Some(o) = DictRef::from_value(x) else {
                    return Err(ConversionException::of_type(self, x, what));
                };
                let key_type = self.get_key_type().unwrap();
                let value_type = self.get_value_type().unwrap();
                let mut result: Vec<(Value<'v>, Value<'v>)> = Vec::with_capacity(o.len());
                for (k, v) in o.iter() {
                    let k2 = key_type.copy_and_lift_starlark_value(
                        k,
                        Some(&"dict key element"),
                        label_converter,
                        heap,
                    )?;
                    let v2 = value_type.copy_and_lift_starlark_value(
                        v,
                        Some(&"dict value element"),
                        label_converter,
                        heap,
                    )?;
                    result.push((k2, v2));
                }
                // LinkedHashMap.put then Dict.immutableCopyOf: AllocDict keeps the first position
                // and the last value of a repeated key, as LinkedHashMap does.
                Ok(heap.alloc(AllocDict(result)))
            }
            // Java: com/google/devtools/build/lib/packages/BuildType.java:515 BuildType.LicenseType.copyAndLiftStarlarkValue
            Type::License => {
                Type::StringList.copy_and_lift_starlark_value(x, what, label_converter, heap)
            }
            _ => {
                // Nones are valid as the Starlark representation of the internal null value
                // (used for certain types' default values).
                if x.is_none() {
                    return Ok(x);
                }
                let converted = self.convert_with_heap(x, what, label_converter, heap)?;
                Ok(native_to_starlark(&converted, heap))
            }
        }
    }

    /// True if `value` has this type's representation (Java: `cast` succeeds).
    pub fn is_instance(self, value: &AttrValue) -> bool {
        match self {
            Type::Integer => matches!(value, AttrValue::Int(_)),
            Type::String | Type::StringNoIntern => matches!(value, AttrValue::String(_)),
            Type::Boolean => matches!(value, AttrValue::Bool(_)),
            Type::Label
            | Type::NodepLabel
            | Type::DormantLabel
            | Type::GenqueryScopeType
            | Type::Output => matches!(value, AttrValue::Label(_)),
            Type::StringList => matches!(value, AttrValue::StringList(_)),
            Type::StringSet => matches!(value, AttrValue::StringSet(_)),
            Type::IntegerList => matches!(value, AttrValue::IntList(_)),
            Type::LabelList
            | Type::NodepLabelList
            | Type::DormantLabelList
            | Type::GenqueryScopeTypeList
            | Type::OutputList => matches!(value, AttrValue::LabelList(_)),
            Type::StringDict => matches!(value, AttrValue::StringDict(_)),
            Type::StringListDict => matches!(value, AttrValue::StringListDict(_)),
            Type::LabelDictUnary => matches!(value, AttrValue::LabelDictUnary(_)),
            Type::LabelKeyedStringDict => matches!(value, AttrValue::LabelKeyedStringDict(_)),
            Type::LabelListDict => matches!(value, AttrValue::LabelListDict(_)),
            Type::License => matches!(value, AttrValue::License(_)),
            Type::Tristate => matches!(value, AttrValue::TriState(_)),
        }
    }

    /// Java `Type.cast`: checks that `value` has this type's representation (panics otherwise,
    /// as Java's ClassCastException).
    // Java: com/google/devtools/build/lib/packages/Type.java:180 Type.cast
    pub fn cast(self, value: &AttrValue) -> &AttrValue {
        if !self.is_instance(value) {
            panic!("ClassCastException: {value:?} is not a value of type {self}");
        }
        value
    }

    /// Invokes `visitor` for each label of `value`, an instance of this type.
    // Java: com/google/devtools/build/lib/packages/Type.java:210 Type.visitLabels
    pub fn visit_labels(
        self,
        visitor: &mut LabelVisitor<'_>,
        value: &AttrValue,
        context: Option<&Attribute>,
    ) {
        // Non-label scalars, lists of non-labels (Java: ListType/SetType return when the element
        // label class is NONE) and dicts without labels (DictType checks its label class) visit
        // nothing.
        if self.get_label_class() == LabelClass::None {
            return;
        }
        match value {
            // Java: com/google/devtools/build/lib/packages/BuildType.java:397 BuildType.LabelType.visitLabels
            // Java: com/google/devtools/build/lib/packages/BuildType.java:547 BuildType.OutputType.visitLabels
            AttrValue::Label(l) if !self.is_list_type() && !self.is_dict_type() => {
                visitor(l, context)
            }
            // Java: com/google/devtools/build/lib/packages/Type.java:706 Type.ListType.visitLabels
            AttrValue::LabelList(ls) if self.is_list_type() => {
                for l in ls {
                    visitor(l, context);
                }
            }
            // Java: com/google/devtools/build/lib/packages/Type.java:504 Type.DictType.visitLabels
            AttrValue::LabelDictUnary(m) if self == Type::LabelDictUnary => {
                for l in m.values() {
                    visitor(l, context);
                }
            }
            AttrValue::LabelKeyedStringDict(m) if self == Type::LabelKeyedStringDict => {
                for l in m.keys() {
                    visitor(l, context);
                }
            }
            AttrValue::LabelListDict(m) if self == Type::LabelListDict => {
                for ls in m.values() {
                    for l in ls {
                        visitor(l, context);
                    }
                }
            }
            _ => panic!("ClassCastException: {value:?} is not a value of type {self}"),
        }
    }

    /// Concatenation (`+` for scalars and lists, `|` for dicts); `None` if unsupported.
    // Java: com/google/devtools/build/lib/packages/Type.java:245 Type.concat
    pub fn concat(self, elements: Vec<AttrValue>) -> Option<AttrValue> {
        match self {
            // Java: com/google/devtools/build/lib/packages/Type.java:378 Type.IntegerType.concat
            Type::Integer => {
                let mut sum: i64 = 0;
                for elem in elements {
                    sum = sum.wrapping_add(into_int(elem) as i64);
                }
                // Perform narrowing conversion to ensure that the result remains in the signed
                // 32-bit range. This means that s=select(0x7fffffff); s+s may yield a negative
                // result.
                Some(AttrValue::Int(sum as i32))
            }
            // Java: com/google/devtools/build/lib/packages/Type.java:478 Type.StringType.concat
            Type::String | Type::StringNoIntern => {
                let mut s = String::new();
                for elem in elements {
                    s.push_str(&into_string(elem));
                }
                Some(AttrValue::String(s))
            }
            // Java: com/google/devtools/build/lib/packages/Type.java:774 Type.ListType.concat
            Type::StringList => Some(AttrValue::StringList(
                elements
                    .into_iter()
                    .flat_map(|e| match e {
                        AttrValue::StringList(l) => l,
                        other => panic!("ClassCastException: {other:?}"),
                    })
                    .collect(),
            )),
            Type::IntegerList => Some(AttrValue::IntList(
                elements
                    .into_iter()
                    .flat_map(|e| match e {
                        AttrValue::IntList(l) => l,
                        other => panic!("ClassCastException: {other:?}"),
                    })
                    .collect(),
            )),
            Type::LabelList
            | Type::NodepLabelList
            | Type::DormantLabelList
            | Type::GenqueryScopeTypeList
            | Type::OutputList => Some(AttrValue::LabelList(
                elements
                    .into_iter()
                    .flat_map(|e| match e {
                        AttrValue::LabelList(l) => l,
                        other => panic!("ClassCastException: {other:?}"),
                    })
                    .collect(),
            )),
            // Java: com/google/devtools/build/lib/packages/Type.java:852 Type.SetType.concat
            Type::StringSet => Some(AttrValue::StringSet(
                elements
                    .into_iter()
                    .flat_map(|e| match e {
                        AttrValue::StringSet(l) => l,
                        other => panic!("ClassCastException: {other:?}"),
                    })
                    .collect(),
            )),
            // Java: com/google/devtools/build/lib/packages/Type.java:604 Type.DictType.concat
            // ImmutableMap.Builder.buildKeepingLast: each key once, at its first position, with
            // its last value (IndexMap::insert / extend).
            Type::StringDict => {
                let mut m = IndexMap::new();
                for e in elements {
                    match e {
                        AttrValue::StringDict(d) => m.extend(d),
                        other => panic!("ClassCastException: {other:?}"),
                    }
                }
                Some(AttrValue::StringDict(m))
            }
            Type::StringListDict => {
                let mut m = IndexMap::new();
                for e in elements {
                    match e {
                        AttrValue::StringListDict(d) => m.extend(d),
                        other => panic!("ClassCastException: {other:?}"),
                    }
                }
                Some(AttrValue::StringListDict(m))
            }
            Type::LabelDictUnary => {
                let mut m = IndexMap::new();
                for e in elements {
                    match e {
                        AttrValue::LabelDictUnary(d) => m.extend(d),
                        other => panic!("ClassCastException: {other:?}"),
                    }
                }
                Some(AttrValue::LabelDictUnary(m))
            }
            Type::LabelKeyedStringDict => {
                let mut m = IndexMap::new();
                for e in elements {
                    match e {
                        AttrValue::LabelKeyedStringDict(d) => m.extend(d),
                        other => panic!("ClassCastException: {other:?}"),
                    }
                }
                Some(AttrValue::LabelKeyedStringDict(m))
            }
            Type::LabelListDict => {
                let mut m = IndexMap::new();
                for e in elements {
                    match e {
                        AttrValue::LabelListDict(d) => m.extend(d),
                        other => panic!("ClassCastException: {other:?}"),
                    }
                }
                Some(AttrValue::LabelListDict(m))
            }
            // Java: com/google/devtools/build/lib/packages/Type.java:245 Type.concat (default)
            Type::Boolean
            | Type::Label
            | Type::NodepLabel
            | Type::DormantLabel
            | Type::GenqueryScopeType
            | Type::Output
            | Type::License
            | Type::Tristate => None,
        }
    }

    /// Converts a value of this type into a tag set.
    // Java: com/google/devtools/build/lib/packages/Type.java:257 Type.toTagSet
    pub fn to_tag_set(self, value: Option<&AttrValue>, name: &str) -> IndexSet<String> {
        match self {
            // Java: com/google/devtools/build/lib/packages/Type.java:433 Type.BooleanType.toTagSet
            Type::Boolean => {
                let Some(value) = value else {
                    panic!(
                        "IllegalStateException: Illegal tag conversion from null on Attribute \
                         {name}."
                    );
                };
                let tag = if into_bool(value.clone()) {
                    name.to_owned()
                } else {
                    format!("no{name}")
                };
                IndexSet::from([tag])
            }
            // Java: com/google/devtools/build/lib/packages/Type.java:484 Type.StringType.toTagSet
            Type::String | Type::StringNoIntern => {
                let Some(value) = value else {
                    panic!(
                        "IllegalStateException: Illegal tag conversion from null on Attribute \
                         {name}."
                    );
                };
                IndexSet::from([into_string(value.clone())])
            }
            // Java: com/google/devtools/build/lib/packages/Type.java:656 Type.CollectionType.toTagSet
            t if t.is_list_type() || t.is_set_type() => {
                let Some(items) = value else {
                    panic!(
                        "IllegalStateException: Illegal tag conversion from null on Attribute\
                         {name}."
                    );
                };
                let mut tags = IndexSet::new();
                match items {
                    AttrValue::StringList(l) => tags.extend(l.iter().cloned()),
                    AttrValue::StringSet(l) => tags.extend(l.iter().cloned()),
                    AttrValue::IntList(l) => tags.extend(l.iter().map(|i| i.to_string())),
                    AttrValue::LabelList(l) => {
                        tags.extend(l.iter().map(|l| l.get_canonical_form()))
                    }
                    other => panic!("ClassCastException: {other:?}"),
                }
                tags
            }
            _ => panic!(
                "UnsupportedOperationException: Attribute {name} does not support tag conversion."
            ),
        }
    }
}

/// Java's `String.valueOf(what)`: the description, or `"null"`.
pub(crate) fn what_str(what: Option<&dyn fmt::Display>) -> String {
    match what {
        Some(w) => w.to_string(),
        None => "null".to_owned(),
    }
}

/// The description of an element being converted: "element N of <what>" (Java
/// `Type.CollectionType.ConversionContext`).
// Java: com/google/devtools/build/lib/packages/Type.java:675 Type.CollectionType.ConversionContext
struct ConversionContext<'a> {
    what: Option<&'a dyn fmt::Display>,
    index: usize,
}

impl fmt::Display for ConversionContext<'_> {
    // Java: com/google/devtools/build/lib/packages/Type.java:688 Type.CollectionType.ConversionContext.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "element {} of {}", self.index, what_str(self.what))
    }
}

// Java: com/google/devtools/build/lib/packages/Type.java:357 Type.IntegerType.convert
fn convert_integer(
    x: Value<'_>,
    what: Option<&dyn fmt::Display>,
) -> Result<i32, ConversionException> {
    if x.get_type() == "int" {
        // assert signed 32-bit
        return match x.unpack_i32() {
            Some(i) => Ok(i),
            None => {
                let prefix = match what {
                    Some(w) => format!("for {w}, "),
                    None => String::new(),
                };
                Err(ConversionException::new(format!(
                    "{prefix}got {}, want value in signed 32-bit range",
                    x.to_repr()
                )))
            }
        };
    }
    Err(ConversionException::of_type(Type::Integer, x, what))
}

// Java: com/google/devtools/build/lib/packages/Type.java:411 Type.BooleanType.convert
fn convert_boolean(
    x: Value<'_>,
    what: Option<&dyn fmt::Display>,
    label_converter: Option<&LabelConverter>,
) -> Result<bool, ConversionException> {
    if let Some(b) = x.unpack_bool() {
        return Ok(b);
    }
    // Conversion to boolean must also tolerate integers of 0 and 1 only.
    if let Ok(AttrValue::Int(x_as_integer)) = Type::Integer.convert(x, what, label_converter) {
        if x_as_integer == 0 {
            return Ok(false);
        } else if x_as_integer == 1 {
            return Ok(true);
        }
    }
    Err(ConversionException::expected(
        "one of [False, True, 0, 1]",
        x,
        what,
    ))
}

// Java: com/google/devtools/build/lib/packages/Type.java:469 Type.StringType.convert
fn convert_string(
    type_: Type,
    x: Value<'_>,
    what: Option<&dyn fmt::Display>,
) -> Result<AttrValue, ConversionException> {
    match x.unpack_str() {
        // PORT: String.intern is memory only.
        Some(s) => Ok(AttrValue::String(s.to_owned())),
        None => Err(ConversionException::of_type(type_, x, what)),
    }
}

/// Java `BuildType.LABEL.convert`.
// Java: com/google/devtools/build/lib/packages/BuildType.java:412 BuildType.LabelType.convert
pub(crate) fn convert_label(
    x: Value<'_>,
    what: Option<&dyn fmt::Display>,
    label_converter: Option<&LabelConverter>,
) -> Result<Label, ConversionException> {
    if let Some(l) = x.downcast_ref::<LabelValue>() {
        return Ok(l.label().clone());
    }
    let Some(s) = x.unpack_str() else {
        return Err(ConversionException::of_type(Type::String, x, what));
    };
    convert_label_string(s, what, label_converter)
}

/// The string case of `BuildType.LABEL.convert`.
// Java: com/google/devtools/build/lib/packages/BuildType.java:419 BuildType.LabelType.convert
pub(crate) fn convert_label_string(
    s: &str,
    what: Option<&dyn fmt::Display>,
    label_converter: Option<&LabelConverter>,
) -> Result<Label, ConversionException> {
    let parsed = match label_converter {
        None => Label::parse_canonical(s),
        Some(lc) => lc.convert(s),
    };
    parsed.map_err(|e| {
        ConversionException::new(format!(
            "invalid label '{}' in {}: {}",
            s,
            what_str(what),
            e.get_message()
        ))
    })
}

// Java: com/google/devtools/build/lib/packages/BuildType.java:562 BuildType.OutputType.convert
fn convert_output(
    x: Value<'_>,
    what: Option<&dyn fmt::Display>,
    label_converter: Option<&LabelConverter>,
) -> Result<Label, ConversionException> {
    let result = convert_label(x, what, label_converter)?;
    let Some(label_converter) = label_converter else {
        panic!("NullPointerException: OutputType.convert without a label converter");
    };
    if *result.get_package_identifier() != label_converter.get_base_package() {
        // Java: "label '" + x + "'": a string as is, a Label by its toString (canonical form).
        let x_str = match x.unpack_str() {
            Some(s) => s.to_owned(),
            None => result.get_canonical_form(),
        };
        return Err(ConversionException::new(format!(
            "label '{x_str}' is not in the current package"
        )));
    }
    Ok(result)
}

// Java: com/google/devtools/build/lib/packages/BuildType.java:504 BuildType.LicenseType.convert
fn convert_license(
    x: Value<'_>,
    what: Option<&dyn fmt::Display>,
) -> Result<AttrValue, ConversionException> {
    let license_strings = into_string_list(Type::StringList.convert(x, what, None)?);
    License::parse_license(&license_strings)
        .map(AttrValue::License)
        .map_err(|e| ConversionException::new(e.0))
}

// Java: com/google/devtools/build/lib/packages/BuildType.java:900 BuildType.TriStateType.convert
fn convert_tristate(
    x: Value<'_>,
    what: Option<&dyn fmt::Display>,
    label_converter: Option<&LabelConverter>,
) -> Result<TriState, ConversionException> {
    // PORT: Java first accepts a TriState object; it is not a Starlark value.
    if let Some(b) = x.unpack_bool() {
        return Ok(if b { TriState::Yes } else { TriState::No });
    }
    let x_as_integer = convert_integer(x, what)?;
    let _ = label_converter;
    match x_as_integer {
        -1 => Ok(TriState::Auto),
        1 => Ok(TriState::Yes),
        0 => Ok(TriState::No),
        _ => Err(ConversionException::of_type(
            Type::Tristate,
            x,
            Some(&"TriState values is not one of [-1, 0, 1]"),
        )),
    }
}

/// The elements of a Java `Iterable` (list, tuple) or `Depset`, or `None` if `x` is neither.
fn iterable_elements<'v>(
    x: Value<'v>,
    heap: Option<Heap<'v>>,
) -> Result<Option<Vec<Value<'v>>>, ConversionException> {
    if let Some(l) = ListRef::from_value(x) {
        return Ok(Some(l.content().to_vec()));
    }
    if let Some(t) = TupleRef::from_value(x) {
        return Ok(Some(t.content().to_vec()));
    }
    if let Some(d) = Depset::from_value(x) {
        return d
            .to_list()
            .map(Some)
            .map_err(|e| ConversionException::new(e.to_string()));
    }
    // starlark-rust patch 0015: sets iterate without a heap.
    if let Some(set) = SetRef::from_value(x) {
        return Ok(Some(set.iter().collect()));
    }
    match x.get_type() {
        // range is a Java Iterable; starlark-rust iterates it only with a heap.
        "range" | "set" => match heap {
            Some(heap) => iterate_with_heap(x, heap).map(Some),
            None => blaze_util::unported!(
                "Type.ListType.convert of a range or set without a heap (use convert_with_heap)"
            ),
        },
        _ => Ok(None),
    }
}

// Java: com/google/devtools/build/lib/packages/Type.java:730 Type.ListType.convert
fn convert_list<'v>(
    type_: Type,
    x: Value<'v>,
    what: Option<&dyn fmt::Display>,
    label_converter: Option<&LabelConverter>,
    heap: Option<Heap<'v>>,
) -> Result<AttrValue, ConversionException> {
    let Some(iterable) = iterable_elements(x, heap)? else {
        return Err(ConversionException::of_type(type_, x, what));
    };
    let elem_type = type_.get_list_element_type().unwrap();
    let mut result: Vec<AttrValue> = Vec::with_capacity(iterable.len());
    for (index, elem) in iterable.into_iter().enumerate() {
        let conversion_context = ConversionContext { what, index };
        result.push(elem_type.convert_impl(
            elem,
            Some(&conversion_context),
            label_converter,
            heap,
        )?);
    }
    Ok(match type_ {
        Type::StringList => AttrValue::StringList(result.into_iter().map(into_string).collect()),
        Type::IntegerList => AttrValue::IntList(result.into_iter().map(into_int).collect()),
        _ => AttrValue::LabelList(result.into_iter().map(into_label).collect()),
    })
}

// Java: com/google/devtools/build/lib/packages/Type.java:812 Type.SetType.convert
fn convert_set<'v>(
    type_: Type,
    x: Value<'v>,
    what: Option<&dyn fmt::Display>,
    label_converter: Option<&LabelConverter>,
    heap: Option<Heap<'v>>,
) -> Result<AttrValue, ConversionException> {
    if x.get_type() != "set" {
        return Err(ConversionException::of_type(type_, x, what));
    }
    // starlark-rust patch 0015: sets iterate without a heap.
    let set: Vec<Value<'v>> = match SetRef::from_value(x) {
        Some(set) => set.iter().collect(),
        None => return Err(ConversionException::of_type(type_, x, what)),
    };
    let elem_type = type_.get_list_element_type().unwrap();
    let mut result: IndexSet<String> = IndexSet::with_capacity(set.len());
    for (index, elem) in set.into_iter().enumerate() {
        let conversion_context = ConversionContext { what, index };
        let converted =
            elem_type.convert_impl(elem, Some(&conversion_context), label_converter, heap)?;
        // Java logs (and skips) a null element; converted elements are never null here.
        result.insert(into_string(converted));
    }
    Ok(AttrValue::StringSet(result))
}

/// The elements of an iterable Starlark value, iterated with `heap`.
fn iterate_with_heap<'v>(
    x: Value<'v>,
    heap: Heap<'v>,
) -> Result<Vec<Value<'v>>, ConversionException> {
    match x.iterate(heap) {
        Ok(it) => Ok(it.collect()),
        Err(e) => Err(ConversionException::new(e.to_string())),
    }
}

// Java: com/google/devtools/build/lib/packages/Type.java:565 Type.DictType.convert
// Java: com/google/devtools/build/lib/packages/BuildType.java:450 BuildType.LabelKeyedDictType.convert
fn convert_dict<'v>(
    type_: Type,
    x: Value<'v>,
    what: Option<&dyn fmt::Display>,
    label_converter: Option<&LabelConverter>,
    heap: Option<Heap<'v>>,
) -> Result<AttrValue, ConversionException> {
    let Some(o) = DictRef::from_value(x) else {
        return Err(ConversionException::of_type(type_, x, what));
    };
    let key_type = type_.get_key_type().unwrap();
    let value_type = type_.get_value_type().unwrap();
    // It's possible that #convert() calls transform non-equal keys into equal ones so we can't
    // just use ImmutableMap.Builder() here (that throws on collisions); IndexMap::insert keeps
    // the first position and the last value, as LinkedHashMap.put.
    let mut entries: Vec<(AttrValue, AttrValue)> = Vec::with_capacity(o.len());
    let input: Vec<(Value<'v>, Value<'v>)> = o.iter().collect();
    for (k, v) in &input {
        entries.push((
            key_type.convert_impl(*k, Some(&"dict key element"), label_converter, heap)?,
            value_type.convert_impl(*v, Some(&"dict value element"), label_converter, heap)?,
        ));
    }
    let result = match type_ {
        Type::StringDict => AttrValue::StringDict(
            entries
                .into_iter()
                .map(|(k, v)| (into_string(k), into_string(v)))
                .collect(),
        ),
        Type::StringListDict => AttrValue::StringListDict(
            entries
                .into_iter()
                .map(|(k, v)| (into_string(k), into_string_list(v)))
                .collect(),
        ),
        Type::LabelDictUnary => AttrValue::LabelDictUnary(
            entries
                .into_iter()
                .map(|(k, v)| (into_string(k), into_label(v)))
                .collect(),
        ),
        Type::LabelListDict => AttrValue::LabelListDict(
            entries
                .into_iter()
                .map(|(k, v)| (into_string(k), into_label_list(v)))
                .collect(),
        ),
        _ => {
            let result: IndexMap<Label, String> = entries
                .into_iter()
                .map(|(k, v)| (into_label(k), into_string(v)))
                .collect();
            if input.len() == result.len() {
                // No collisions found. Exit early.
                return Ok(AttrValue::LabelKeyedStringDict(result));
            }
            // Look for collisions in order to produce a nicer error message.
            let mut converted_from: IndexMap<Label, Vec<Value<'v>>> = IndexMap::new();
            for (original, _) in &input {
                let label = convert_label(*original, what, label_converter)?;
                converted_from.entry(label).or_default().push(*original);
            }
            let mut error_message = String::from("duplicate labels");
            if let Some(w) = what {
                error_message.push_str(" in ");
                error_message.push_str(&w.to_string());
            }
            error_message.push(':');
            let mut is_first_entry = true;
            for (label, originals) in &converted_from {
                if originals.len() == 1 {
                    continue;
                }
                if is_first_entry {
                    is_first_entry = false;
                } else {
                    error_message.push(',');
                }
                error_message.push(' ');
                error_message.push_str(&label.get_canonical_form());
                error_message.push_str(" (as [");
                let reprs: Vec<String> = originals.iter().map(|v| v.to_repr()).collect();
                error_message.push_str(&reprs.join(", "));
                error_message.push_str("])");
            }
            return Err(ConversionException::new(error_message));
        }
    };
    Ok(result)
}

/// A native value as a Starlark value (Java: the converted object itself is a Starlark value).
pub(crate) fn native_to_starlark<'v>(value: &AttrValue, heap: Heap<'v>) -> Value<'v> {
    // Elements are allocated before their container (no allocation while a container is being
    // allocated).
    let string = |s: &str| heap.alloc(s);
    let label = |l: &Label| heap.alloc(LabelValue(l.clone()));
    let list = |items: Vec<Value<'v>>| heap.alloc(AllocList(items));
    let dict = |items: Vec<(Value<'v>, Value<'v>)>| heap.alloc(AllocDict(items));
    match value {
        AttrValue::Int(i) => heap.alloc(*i),
        AttrValue::String(s) => string(s),
        AttrValue::Bool(b) => Value::new_bool(*b),
        AttrValue::Label(l) => label(l),
        // PORT: Java's TriState is not a Starlark value; its int form (Attribute.valueToStarlark).
        AttrValue::TriState(t) => heap.alloc(t.to_int()),
        AttrValue::StringList(l) => list(l.iter().map(|s| string(s)).collect()),
        AttrValue::IntList(l) => list(l.iter().map(|i| heap.alloc(*i)).collect()),
        AttrValue::LabelList(l) => list(l.iter().map(label).collect()),
        AttrValue::StringDict(m) => dict(m.iter().map(|(k, v)| (string(k), string(v))).collect()),
        AttrValue::StringListDict(m) => dict(
            m.iter()
                .map(|(k, v)| (string(k), list(v.iter().map(|s| string(s)).collect())))
                .collect(),
        ),
        AttrValue::LabelDictUnary(m) => {
            dict(m.iter().map(|(k, v)| (string(k), label(v))).collect())
        }
        AttrValue::LabelKeyedStringDict(m) => {
            dict(m.iter().map(|(k, v)| (label(k), string(v))).collect())
        }
        AttrValue::LabelListDict(m) => dict(
            m.iter()
                .map(|(k, v)| (string(k), list(v.iter().map(label).collect())))
                .collect(),
        ),
        // Java StarlarkSet.immutableCopyOf. PORT: a mutable starlark-rust set (as for lists and
        // dicts above); strings are always hashable.
        AttrValue::StringSet(l) => {
            let items: Vec<Value<'v>> = l.iter().map(|s| string(s)).collect();
            starlark::values::set::alloc_set(heap, items).expect("strings are hashable")
        }
        other => panic!("not a native attribute value: {other:?}"),
    }
}

pub(crate) fn into_int(v: AttrValue) -> i32 {
    match v {
        AttrValue::Int(i) => i,
        other => panic!("ClassCastException: {other:?} is not an int"),
    }
}

pub(crate) fn into_bool(v: AttrValue) -> bool {
    match v {
        AttrValue::Bool(b) => b,
        other => panic!("ClassCastException: {other:?} is not a boolean"),
    }
}

pub(crate) fn into_string(v: AttrValue) -> String {
    match v {
        AttrValue::String(s) => s,
        other => panic!("ClassCastException: {other:?} is not a string"),
    }
}

pub(crate) fn into_label(v: AttrValue) -> Label {
    match v {
        AttrValue::Label(l) => l,
        other => panic!("ClassCastException: {other:?} is not a label"),
    }
}

pub(crate) fn into_string_list(v: AttrValue) -> Vec<String> {
    match v {
        AttrValue::StringList(l) => l,
        other => panic!("ClassCastException: {other:?} is not a list(string)"),
    }
}

pub(crate) fn into_label_list(v: AttrValue) -> Vec<Label> {
    match v {
        AttrValue::LabelList(l) => l,
        other => panic!("ClassCastException: {other:?} is not a list(label)"),
    }
}

impl fmt::Display for Type {
    // Java: com/google/devtools/build/lib/packages/Type.java:183 Type.toString (and the
    // subclass overrides: "int", "string", "boolean", "list(E)", "set(E)", "dict(K, V)",
    // "label", "license", "output", "tristate").
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Integer => f.write_str("int"),
            Type::String | Type::StringNoIntern => f.write_str("string"),
            Type::Boolean => f.write_str("boolean"),
            Type::Label | Type::NodepLabel | Type::DormantLabel | Type::GenqueryScopeType => {
                f.write_str("label")
            }
            Type::License => f.write_str("license"),
            Type::Output => f.write_str("output"),
            Type::Tristate => f.write_str("tristate"),
            Type::StringSet => write!(f, "set({})", self.get_list_element_type().unwrap()),
            t if t.is_list_type() => write!(f, "list({})", t.get_list_element_type().unwrap()),
            t => write!(
                f,
                "dict({}, {})",
                t.get_key_type().unwrap(),
                t.get_value_type().unwrap()
            ),
        }
    }
}
