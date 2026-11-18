//! Ports the static methods and the `Selector` / `SelectorList` classes of
//! `packages/BuildType.java` (the type constants are [`Type`] variants).

use std::fmt;
use std::sync::Arc;
use std::sync::LazyLock;

use blaze_cmdline::Label;
use blaze_starlark::error::bare_message;
use blaze_starlark::label::LabelValue;
use indexmap::IndexMap;
use indexmap::IndexSet;
use starlark::values::Heap;
use starlark::values::Value;

use crate::attribute::Attribute;
use crate::types::attr_value::AttrValue;
use crate::types::conversion_exception::ConversionException;
use crate::types::label_converter::LabelConverter;
use crate::types::select::SelectorList;
use crate::types::select::SelectorValue;
use crate::types::type_::Type;
use crate::types::type_::convert_label;
use crate::types::type_::convert_label_string;
use crate::types::type_::what_str;

/// If the given attribute type is non-configurable, returns the reason why.
// Java: com/google/devtools/build/lib/packages/BuildType.java:348 BuildType.maybeGetNonConfigurableReason
pub fn maybe_get_non_configurable_reason(type_: Type) -> Option<&'static str> {
    // Java: com/google/devtools/build/lib/packages/BuildType.java:125 BuildType.whyNotConfigurable
    match type_ {
        Type::License => {
            Some("loading phase license checking logic assumes non-configurable values")
        }
        Type::Output => Some("output paths are part of the static graph structure"),
        _ => None,
    }
}

/// Variation of [`Type::convert`] that supports `select()` expressions: returns a
/// [`AttrValue::SelectorList`] for a selector expression, else the converted native value.
/// With `simplify_unconditional_selects`, unconditional selects become plain values; returns
/// `None` iff that simplification yields Java's null.
// Java: com/google/devtools/build/lib/packages/BuildType.java:165 BuildType.selectableConvert
pub fn selectable_convert<'v>(
    type_: Type,
    x: Value<'v>,
    what: Option<&dyn fmt::Display>,
    context: Option<&LabelConverter>,
    simplify_unconditional_selects: bool,
) -> Result<Option<AttrValue>, ConversionException> {
    let Some(selector_list) = SelectorList::from_value(x) else {
        return type_.convert(x, what, context).map(Some);
    };
    let selector_list_elements: Vec<Value<'v>> = selector_list.get_elements().collect();
    let as_selector_list = || -> Result<Option<AttrValue>, ConversionException> {
        Ok(Some(AttrValue::SelectorList(Arc::new(
            BuildTypeSelectorList::from_starlark(&selector_list_elements, what, context, type_)?,
        ))))
    };
    if !simplify_unconditional_selects {
        return as_selector_list();
    }
    if selector_list_elements.len() > 1 && type_.concat(Vec::new()).is_none() {
        return Err(ConversionException::new(format!(
            "type '{type_}' doesn't support select concatenation"
        )));
    }
    // Note: a Vec of Option, because we may insert a null into it; the default value of an
    // unconditional Selector<T> is null if the SelectorValue value is None and the native type's
    // default value is null.
    let mut values: Vec<Option<AttrValue>> = Vec::with_capacity(selector_list_elements.len());
    for element in &selector_list_elements {
        if let Some(selector_value) = SelectorValue::from_value(*element) {
            let dictionary: Vec<(Value<'v>, Value<'v>)> = selector_value.get_dictionary().collect();
            if dictionary.len() != 1 {
                // Cannot simplify: selectorValue has multiple branches.
                return as_selector_list();
            }
            let selector = BuildTypeSelector::from_starlark(
                &dictionary,
                what,
                context,
                type_,
                selector_value.get_no_match_error(),
            )?;
            if !selector.is_unconditional() {
                // Cannot simplify: the only branch is not the default condition.
                return as_selector_list();
            }
            values.push(selector.get_default().cloned());
        } else {
            values.push(Some(type_.convert(*element, what, context)?));
        }
    }
    if values.len() == 1 {
        Ok(values.pop().unwrap())
    } else {
        Ok(type_.concat(
            values
                .into_iter()
                .map(|v| v.expect("NullPointerException: null in select concatenation"))
                .collect(),
        ))
    }
}

/// Converts the build-language-typed `build_lang_value` of `attr` to a native value via
/// [`selectable_convert`], sorting lists of order-independent attributes.
// PORT: Java's list interner is dropped (memory only).
// Java: com/google/devtools/build/lib/packages/BuildType.java:226 BuildType.convertFromBuildLangType
pub fn convert_from_build_lang_type<'v>(
    rule_class: &str,
    attr: &Attribute,
    build_lang_value: Value<'v>,
    label_converter: Option<&LabelConverter>,
    simplify_unconditional_selects: bool,
) -> Result<Option<AttrValue>, ConversionException> {
    if SelectorList::from_value(build_lang_value).is_some() && !attr.is_configurable() {
        return Err(ConversionException::new(format!(
            "attribute \"{}\" is not configurable",
            attr.get_name()
        )));
    }
    let context = AttributeConversionContext {
        attr_name: attr.get_name(),
        attr_owner: rule_class,
    };
    let mut converted = selectable_convert(
        attr.get_type(),
        build_lang_value,
        Some(&context),
        label_converter,
        simplify_unconditional_selects,
    )?;
    if attr.is_order_independent() {
        // Java `converted instanceof List`: Ordering.natural().sortedCopy.
        match &mut converted {
            Some(AttrValue::StringList(l)) => l.sort_by(|a, b| java_string_compare(a, b)),
            Some(AttrValue::IntList(l)) => l.sort(),
            Some(AttrValue::LabelList(l)) => l.sort(),
            _ => {}
        }
    }
    // PORT: the list interner is memory only.
    Ok(converted)
}

/// Java `String.compareTo`: lexicographic order of UTF-16 code units.
pub(crate) fn java_string_compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Copies a Starlark select converting label strings to Label objects.
// Java: com/google/devtools/build/lib/packages/BuildType.java:280 BuildType.copyAndLiftSelectorList
fn copy_and_lift_selector_list<'v>(
    type_: Type,
    x: &SelectorList<'v>,
    what: Option<&dyn fmt::Display>,
    context: Option<&LabelConverter>,
    heap: Heap<'v>,
) -> Result<Value<'v>, ConversionException> {
    let elements: Vec<Value<'v>> = x.get_elements().collect();
    if elements.len() > 1 && type_.concat(Vec::new()).is_none() {
        return Err(ConversionException::new(format!(
            "type '{type_}' doesn't support select concatenation"
        )));
    }
    let mut builder: Vec<Value<'v>> = Vec::with_capacity(elements.len());
    for elem in elements {
        if let Some(selector_value) = SelectorValue::from_value(elem) {
            // ImmutableMap.Builder.buildKeepingLast: first position, last value.
            let mut new_map: IndexMap<Label, Value<'v>> = IndexMap::new();
            for (k, v) in selector_value.get_dictionary() {
                let key = convert_label(k, what, context)?;
                let value = if v.is_none() {
                    v
                } else {
                    let what_string = what_str(what);
                    let branch = SelectBranchMessage {
                        what: &what_string,
                        key: &key,
                    };
                    type_.copy_and_lift_starlark_value(v, Some(&branch), context, heap)?
                };
                new_map.insert(key, value);
            }
            let dictionary: Vec<(Value<'v>, Value<'v>)> = new_map
                .into_iter()
                .map(|(k, v)| (heap.alloc(LabelValue(k)), v))
                .collect();
            builder.push(heap.alloc(SelectorValue::new(
                dictionary,
                selector_value.get_no_match_error().to_owned(),
            )));
        } else {
            builder.push(type_.copy_and_lift_starlark_value(elem, what, context, heap)?);
        }
    }
    match SelectorList::of(&builder) {
        Ok(l) => Ok(heap.alloc(l)),
        Err(e) => Err(ConversionException::new(bare_message(&e))),
    }
}

/// Copies a Starlark value to immutable ones and converts label strings to Labels, type
/// checking it against `attr` (used before passing values to initializers).
// Java: com/google/devtools/build/lib/packages/BuildType.java:321 BuildType.copyAndLiftStarlarkValue
pub fn copy_and_lift_starlark_value<'v>(
    attr_owner: &str,
    attr: &Attribute,
    starlark_value: Value<'v>,
    label_converter: Option<&LabelConverter>,
    heap: Heap<'v>,
) -> Result<Value<'v>, ConversionException> {
    let context = AttributeConversionContext {
        attr_name: attr.get_name(),
        attr_owner,
    };
    if let Some(selector_list) = SelectorList::from_value(starlark_value) {
        if !attr.is_configurable() {
            return Err(ConversionException::new(format!(
                "attribute \"{}\" is not configurable",
                attr.get_name()
            )));
        }
        copy_and_lift_selector_list(
            attr.get_type(),
            selector_list,
            Some(&context),
            label_converter,
            heap,
        )
    } else {
        attr.get_type().copy_and_lift_starlark_value(
            starlark_value,
            Some(&context),
            label_converter,
            heap,
        )
    }
}

/// Java `BuildType.AttributeConversionContext`: "attribute '<name>' of '<owner>'".
// Java: com/google/devtools/build/lib/packages/BuildType.java:358 BuildType.AttributeConversionContext
#[derive(Debug, Clone, Copy)]
pub struct AttributeConversionContext<'a> {
    pub attr_name: &'a str,
    pub attr_owner: &'a str,
}

impl fmt::Display for AttributeConversionContext<'_> {
    // Java: com/google/devtools/build/lib/packages/BuildType.java:374 BuildType.AttributeConversionContext.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "attribute '{}' of '{}'", self.attr_name, self.attr_owner)
    }
}

/// Lazy string message to pass as the `what` when converting a select branch value (Java
/// `BuildType.SelectBranchMessage`).
// Java: com/google/devtools/build/lib/packages/BuildType.java:655 BuildType.SelectBranchMessage
struct SelectBranchMessage<'a> {
    what: &'a str,
    key: &'a Label,
}

impl fmt::Display for SelectBranchMessage<'_> {
    // Java: com/google/devtools/build/lib/packages/BuildType.java:665 BuildType.SelectBranchMessage.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "each branch in select expression of {} (including '{}')",
            self.what, self.key
        )
    }
}

/// A configurable attribute value: an order-preserved list of selects (direct values are
/// stored as selects with only a default condition) (Java `BuildType.SelectorList`).
// Java: com/google/devtools/build/lib/packages/BuildType.java:579 BuildType.SelectorList
#[derive(Debug, Clone, PartialEq)]
pub struct BuildTypeSelectorList {
    original_type: Type,
    elements: Vec<BuildTypeSelector>,
}

impl BuildTypeSelectorList {
    /// Converts the elements of a Starlark `select()` expression (each a `SelectorValue` or a
    /// direct value) to `original_type`.
    // Java: com/google/devtools/build/lib/packages/BuildType.java:584 BuildType.SelectorList.<init>
    pub fn from_starlark<'v>(
        x: &[Value<'v>],
        what: Option<&dyn fmt::Display>,
        context: Option<&LabelConverter>,
        original_type: Type,
    ) -> Result<Self, ConversionException> {
        if x.len() > 1 && original_type.concat(Vec::new()).is_none() {
            return Err(ConversionException::new(format!(
                "type '{original_type}' doesn't support select concatenation"
            )));
        }
        let mut builder: Vec<BuildTypeSelector> = Vec::with_capacity(x.len());
        for elem in x {
            if let Some(selector_value) = SelectorValue::from_value(*elem) {
                let dictionary: Vec<(Value<'v>, Value<'v>)> =
                    selector_value.get_dictionary().collect();
                builder.push(BuildTypeSelector::from_starlark(
                    &dictionary,
                    what,
                    context,
                    original_type,
                    selector_value.get_no_match_error(),
                )?);
            } else {
                let direct_value = original_type.convert(*elem, what, context)?;
                // Java: new Selector<>(ImmutableMap.of(DEFAULT_CONDITION_KEY, directValue), ...);
                // converting the already-converted value again is the identity.
                let key = convert_label_string(DEFAULT_CONDITION_KEY, what, context)?;
                let default_condition_pos = if key == *DEFAULT_CONDITION_LABEL {
                    0
                } else {
                    -1
                };
                builder.push(BuildTypeSelector::new(
                    vec![key],
                    vec![Some(direct_value)],
                    original_type,
                    String::new(),
                    IndexSet::new(),
                    default_condition_pos,
                ));
            }
        }
        Ok(BuildTypeSelectorList {
            original_type,
            elements: builder,
        })
    }

    // Java: com/google/devtools/build/lib/packages/BuildType.java:607 BuildType.SelectorList.<init>
    pub fn new(elements: Vec<BuildTypeSelector>, original_type: Type) -> Self {
        BuildTypeSelectorList {
            original_type,
            elements,
        }
    }

    /// A syntactically order-preserved list of all values and selectors for this attribute.
    // Java: com/google/devtools/build/lib/packages/BuildType.java:615 BuildType.SelectorList.getSelectors
    pub fn get_selectors(&self) -> &[BuildTypeSelector] {
        &self.elements
    }

    // Java: com/google/devtools/build/lib/packages/BuildType.java:623 BuildType.SelectorList.getOriginalType
    pub fn get_original_type(&self) -> Type {
        self.original_type
    }

    /// The labels of all configurability keys across all selects in this expression.
    // Java: com/google/devtools/build/lib/packages/BuildType.java:628 BuildType.SelectorList.getKeyLabels
    pub fn get_key_labels(&self) -> IndexSet<Label> {
        let mut keys = IndexSet::new();
        for selector in &self.elements {
            selector.for_each(|label, _value| {
                if !BuildTypeSelector::is_default_condition_label(label) {
                    keys.insert(label.clone());
                }
            });
        }
        keys
    }
}

/// Java `BuildType.Selector.DEFAULT_CONDITION_KEY`.
// Java: com/google/devtools/build/lib/packages/BuildType.java:678 BuildType.Selector.DEFAULT_CONDITION_KEY
pub const DEFAULT_CONDITION_KEY: &str = "//conditions:default";

// Java: com/google/devtools/build/lib/packages/BuildType.java:680 BuildType.Selector.DEFAULT_CONDITION_LABEL
static DEFAULT_CONDITION_LABEL: LazyLock<Label> =
    LazyLock::new(|| Label::parse_canonical_unchecked(DEFAULT_CONDITION_KEY));

/// The entries of one select expression, in the order initially specified: condition labels
/// and values of the attribute's native type (Java `BuildType.Selector`).
// Java: com/google/devtools/build/lib/packages/BuildType.java:675 BuildType.Selector
#[derive(Debug, Clone, PartialEq)]
pub struct BuildTypeSelector {
    original_type: Type,
    labels: Vec<Label>,
    /// `None` where an entry maps to `None` and the type has no default value (Java null).
    values: Vec<Option<AttrValue>>,
    conditions_with_default_values: IndexSet<Label>,
    no_match_error: String,
    default_condition_pos: i32,
}

impl BuildTypeSelector {
    /// Converts a `select()` dictionary (Starlark keys and values) to a selector of
    /// `original_type`, with a custom no-match error (`""` for the default message).
    // Java: com/google/devtools/build/lib/packages/BuildType.java:704 BuildType.Selector.<init>
    pub fn from_starlark<'v>(
        x: &[(Value<'v>, Value<'v>)],
        what: Option<&dyn fmt::Display>,
        context: Option<&LabelConverter>,
        original_type: Type,
        no_match_error: &str,
    ) -> Result<Self, ConversionException> {
        let mut labels: Vec<Label> = Vec::with_capacity(x.len());
        let mut values: Vec<Option<AttrValue>> = Vec::with_capacity(x.len());
        let mut default_values_builder: IndexSet<Label> = IndexSet::new();
        let mut default_condition_pos: i32 = -1;
        for (pos, (k, v)) in x.iter().enumerate() {
            let key = convert_label(*k, what, context)?;
            let value = if v.is_none() {
                // { "//condition": None } is the same as not setting the value.
                default_values_builder.insert(key.clone());
                original_type.get_default_value()
            } else {
                match what {
                    None => Some(original_type.convert(*v, None, context)?),
                    Some(w) => {
                        let what_string = w.to_string();
                        let select_branch = SelectBranchMessage {
                            what: &what_string,
                            key: &key,
                        };
                        Some(original_type.convert(*v, Some(&select_branch), context)?)
                    }
                }
            };
            if key == *DEFAULT_CONDITION_LABEL {
                default_condition_pos = pos as i32;
            }
            labels.push(key);
            values.push(value);
        }
        Ok(BuildTypeSelector {
            original_type,
            labels,
            values,
            conditions_with_default_values: default_values_builder,
            no_match_error: no_match_error.to_owned(),
            default_condition_pos,
        })
    }

    /// Creates a selector from raw values.
    // Java: com/google/devtools/build/lib/packages/BuildType.java:744 BuildType.Selector.<init>
    pub fn new(
        labels: Vec<Label>,
        values: Vec<Option<AttrValue>>,
        original_type: Type,
        no_match_error: String,
        conditions_with_default_values: IndexSet<Label>,
        default_condition_pos: i32,
    ) -> Self {
        BuildTypeSelector {
            original_type,
            labels,
            values,
            conditions_with_default_values,
            no_match_error,
            default_condition_pos,
        }
    }

    // Java: com/google/devtools/build/lib/packages/BuildType.java:760 BuildType.Selector.hasDefault
    pub fn has_default(&self) -> bool {
        self.default_condition_pos >= 0
    }

    /// The value to use when none of the attribute's selection keys match.
    // Java: com/google/devtools/build/lib/packages/BuildType.java:766 BuildType.Selector.getDefault
    pub fn get_default(&self) -> Option<&AttrValue> {
        if self.default_condition_pos < 0 {
            None
        } else {
            self.values[self.default_condition_pos as usize].as_ref()
        }
    }

    /// All the values, in the order initially specified.
    // Java: com/google/devtools/build/lib/packages/BuildType.java:776 BuildType.Selector.valuesCopy
    pub fn values_copy(&self) -> Vec<Option<AttrValue>> {
        self.values.clone()
    }

    /// The branches, in the order initially specified.
    // Java: com/google/devtools/build/lib/packages/BuildType.java:789 BuildType.Selector.mapCopy
    pub fn map_copy(&self) -> IndexMap<Label, Option<AttrValue>> {
        self.labels
            .iter()
            .cloned()
            .zip(self.values.iter().cloned())
            .collect()
    }

    /// Passes each entry to `consumer`, in the order initially specified.
    // Java: com/google/devtools/build/lib/packages/BuildType.java:806 BuildType.Selector.forEach
    pub fn for_each(&self, mut consumer: impl FnMut(&Label, Option<&AttrValue>)) {
        for (label, value) in self.labels.iter().zip(self.values.iter()) {
            consumer(label, value.as_ref());
        }
    }

    /// Like [`BuildTypeSelector::for_each`], stopping at the first error.
    // Java: com/google/devtools/build/lib/packages/BuildType.java:821 BuildType.Selector.forEachExceptionally
    pub fn for_each_exceptionally<E>(
        &self,
        mut consumer: impl FnMut(&Label, Option<&AttrValue>) -> Result<(), E>,
    ) -> Result<(), E> {
        for (label, value) in self.labels.iter().zip(self.values.iter()) {
            consumer(label, value.as_ref())?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/BuildType.java:829 BuildType.Selector.getNumEntries
    pub fn get_num_entries(&self) -> usize {
        self.labels.len()
    }

    // Java: com/google/devtools/build/lib/packages/BuildType.java:837 BuildType.Selector.getOriginalType
    pub fn get_original_type(&self) -> Type {
        self.original_type
    }

    /// True if this selector has the structure `{"//conditions:default": ...}`.
    // Java: com/google/devtools/build/lib/packages/BuildType.java:845 BuildType.Selector.isUnconditional
    pub fn is_unconditional(&self) -> bool {
        self.labels.len() == 1 && self.default_condition_pos >= 0
    }

    /// True if an explicit value is set for `condition` (vs. `{"//condition": None}`).
    // Java: com/google/devtools/build/lib/packages/BuildType.java:853 BuildType.Selector.isValueSet
    pub fn is_value_set(&self, condition: &Label) -> bool {
        !self.conditions_with_default_values.contains(condition)
    }

    /// The custom no-match error, or `""`.
    // Java: com/google/devtools/build/lib/packages/BuildType.java:861 BuildType.Selector.getNoMatchError
    pub fn get_no_match_error(&self) -> &str {
        &self.no_match_error
    }

    /// True for the default condition label (`//conditions:default`).
    // Java: com/google/devtools/build/lib/packages/BuildType.java:869 BuildType.Selector.isDefaultConditionLabel
    pub fn is_default_condition_label(label: &Label) -> bool {
        *label == *DEFAULT_CONDITION_LABEL
    }

    /// Java `BuildType.Selector.DEFAULT_CONDITION_LABEL`.
    pub fn default_condition_label() -> Label {
        DEFAULT_CONDITION_LABEL.clone()
    }
}
