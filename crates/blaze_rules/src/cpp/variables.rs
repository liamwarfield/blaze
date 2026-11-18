//! Starlark values stored in `CcToolchainVariables` maps (Java `MapVariables` holds the raw
//! `Object`s and `asVariableValue` wraps them on lookup), converted to
//! `blaze_cc_features::cc_toolchain_variables::VariableObject`.
//!
//! PORT: Java keeps the Starlark objects and reads them lazily; Starlark values here live on a
//! per-target heap that is frozen and dropped, so the values are converted when the variables
//! are created. All values rules_cc passes are deeply immutable once passed (strings, `File`s,
//! depsets, frozen/unchanged lists, provider structs), so the conversion is equivalent. A value
//! of a type Java's `asVariableValue` cannot wrap fails (Java: `ClassCastException`) when it is
//! read, as in Java.

use std::fmt;
use std::sync::Arc;

use blaze_actions::analysis::starlark::starlark_file::artifact_from_value;
use blaze_cc_features::actions::{InputMetadataProvider, PathMapper};
use blaze_cc_features::cc_toolchain_variables::{
    Structure, VariableObject, VariableValue, VariableValueAdapter,
};
use blaze_cc_features::errors::{EvalException, ExpansionException};
use blaze_starlark::depset::Depset;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::StarlarkInfo;
use indexmap::IndexMap;
use starlark::values::ValueLike;
use starlark::values::dict::DictRef;
use starlark::values::list::ListRef;
use starlark::values::tuple::TupleRef;
use starlark::values::{Heap, Value};

use super::cc_artifact::to_cc_artifact;
use super::util::errorf;

/// Java `asVariableValue(o)`'s input conversion: a Starlark value as the raw variables object.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:826 CcToolchainVariables.asVariableValue
pub fn to_variable_object<'v>(v: Value<'v>, heap: Heap<'v>) -> starlark::Result<VariableObject> {
    if v.is_none() {
        return Ok(VariableObject::None);
    }
    if let Some(b) = v.unpack_bool() {
        return Ok(VariableObject::Boolean(b));
    }
    if let Some(s) = v.unpack_str() {
        return Ok(VariableObject::string(s));
    }
    if let Some(label) = v.downcast_ref::<LabelValue>() {
        return Ok(VariableObject::Label(Arc::from(
            label.label().expand_to_command_line().as_str(),
        )));
    }
    if let Some(artifact) = artifact_from_value(v) {
        return Ok(VariableObject::Artifact(to_cc_artifact(&artifact)));
    }
    // Java `Iterable`: StarlarkList, Tuple, RangeList, Dict (its keys), StarlarkSet.
    if let Some(l) = ListRef::from_value(v) {
        return iterable(l.content().iter().copied(), heap);
    }
    if let Some(t) = TupleRef::from_value(v) {
        return iterable(t.content().iter().copied(), heap);
    }
    if let Some(d) = DictRef::from_value(v) {
        let keys: Vec<Value<'v>> = d.keys().collect();
        return iterable(keys.into_iter(), heap);
    }
    if matches!(v.get_type(), "range" | "set") {
        let items: Vec<Value<'v>> = v.iterate(heap)?.collect();
        return iterable(items.into_iter(), heap);
    }
    if let Some(d) = Depset::from_value(v) {
        let set = d.to_nested_set(&mut |e: Value<'v>| to_variable_object(e, heap))?;
        return Ok(VariableObject::NestedSet(set));
    }
    if let Some(s) = StarlarkInfo::from_value(v) {
        let mut fields = IndexMap::new();
        for (name, value) in s.fields() {
            fields.insert(name.to_owned(), to_variable_object(value, heap)?);
        }
        return Ok(VariableObject::Structure(Arc::new(ConvertedStructure {
            type_name: v.get_type().to_owned(),
            truth: v.to_bool(),
            fields,
        })));
    }
    // Java: `default -> (VariableValue) o`.
    Ok(VariableObject::Value(VariableValue::Custom(Arc::new(
        UnwrappableValue {
            java_type: v.get_type().to_owned(),
        },
    ))))
}

fn iterable<'v>(
    items: impl Iterator<Item = Value<'v>>,
    heap: Heap<'v>,
) -> starlark::Result<VariableObject> {
    let mut converted = Vec::new();
    for item in items {
        converted.push(to_variable_object(item, heap)?);
    }
    Ok(VariableObject::list(converted))
}

/// `Dict.cast(buildVariables, String.class, Object.class, "vars")` converted.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:140 CcStarlarkInternal.getCcToolchainVariables
pub fn cast_vars<'v>(
    x: Value<'v>,
    what: &str,
    heap: Heap<'v>,
) -> starlark::Result<IndexMap<String, VariableObject>> {
    let entries = super::util::dict_cast_string_object(x, what)?;
    let mut result = IndexMap::new();
    for (k, v) in entries {
        result.insert(k, to_variable_object(v, heap)?);
    }
    Ok(result)
}

/// A Starlark struct or provider instance captured for `StarlarkStructureAdapter`.
#[derive(Debug)]
pub struct ConvertedStructure {
    type_name: String,
    truth: bool,
    fields: IndexMap<String, VariableObject>,
}

impl Structure for ConvertedStructure {
    // Java: com/google/devtools/build/lib/packages/StarlarkInfo.java:57 StarlarkInfo.getValue
    fn get_value(&self, name: &str) -> Result<Option<VariableObject>, EvalException> {
        Ok(self.fields.get(name).cloned())
    }

    fn starlark_type(&self) -> String {
        self.type_name.clone()
    }

    fn truth(&self) -> bool {
        self.truth
    }
}

/// A value Java's `asVariableValue` would cast to `VariableValue` and fail on
/// (`ClassCastException`).
#[derive(Debug)]
struct UnwrappableValue {
    java_type: String,
}

impl UnwrappableValue {
    fn fail(&self) -> ! {
        panic!(
            "ClassCastException: class of Starlark type '{}' cannot be cast to class \
             com.google.devtools.build.lib.rules.cpp.CcToolchainVariables$VariableValue",
            self.java_type
        )
    }
}

impl VariableValueAdapter for UnwrappableValue {
    fn get_variable_type_name(&self) -> String {
        self.fail()
    }

    fn is_truthy(&self) -> bool {
        self.fail()
    }

    fn get_field_value(
        &self,
        _variable_name: &str,
        _field: &str,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        _path_mapper: &dyn PathMapper,
        _throw_on_missing_variable: bool,
    ) -> Result<Option<VariableValue>, ExpansionException> {
        self.fail()
    }

    fn get_string_value(
        &self,
        _variable_name: &str,
        _path_mapper: &dyn PathMapper,
    ) -> Result<String, ExpansionException> {
        self.fail()
    }
}

impl fmt::Display for UnwrappableValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}>", self.java_type)
    }
}

/// Java `EvalException` from `blaze_cc_features` as a Starlark error.
pub fn eval_error(e: EvalException) -> starlark::Error {
    errorf(e.message)
}

/// Java `ExpansionException` (an `EvalException`) as a Starlark error.
pub fn expansion_error(e: ExpansionException) -> starlark::Error {
    errorf(e.message)
}
