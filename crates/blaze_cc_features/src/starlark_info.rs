//! A Starlark-free view of the `StarlarkInfo` structs that rules_cc's `cc_toolchain_config_lib.bzl`
//! builds (`feature(...)`, `flag_set(...)`, ...) and of the `CcToolchainConfigInfo` provider
//! instance that holds them.
//!
//! PORT: Java reads these through `com.google.devtools.build.lib.packages.StarlarkInfo` and
//! `net.starlark.java.eval.Sequence`. The Starlark wrapper crate converts frozen Starlark values
//! into this tree (or implements the same lookups); tests build it from Starlark evaluated with
//! starlark-rust or from the oracle's `json.encode` output.

use std::sync::Arc;

use indexmap::IndexMap;

use crate::actions::ArtifactRef;
use crate::errors::EvalException;

/// A Starlark value as far as `CcToolchainFeaturesLib` inspects it.
#[derive(Clone, Debug)]
pub enum StarlarkInfoValue {
    None,
    Bool(bool),
    Int(i64),
    String(String),
    /// A Starlark sequence (list or tuple).
    List(Vec<StarlarkInfoValue>),
    /// A struct or provider instance (`StarlarkInfo`).
    Info(Arc<StarlarkInfo>),
    /// A `File`.
    Artifact(ArtifactRef),
    /// Any other value, by its Starlark type name.
    Other(String),
}

impl StarlarkInfoValue {
    /// Java: `Starlark.type(x)`.
    pub fn starlark_type(&self) -> String {
        match self {
            StarlarkInfoValue::None => "NoneType".to_string(),
            StarlarkInfoValue::Bool(_) => "bool".to_string(),
            StarlarkInfoValue::Int(_) => "int".to_string(),
            StarlarkInfoValue::String(_) => "string".to_string(),
            StarlarkInfoValue::List(_) => "list".to_string(),
            StarlarkInfoValue::Info(_) => "struct".to_string(),
            StarlarkInfoValue::Artifact(_) => "File".to_string(),
            StarlarkInfoValue::Other(t) => t.clone(),
        }
    }

    /// Converts the output of Bazel's `json.encode` of a struct tree (objects become structs).
    /// Struct printable names and creation locations are unknown in JSON.
    pub fn from_json_like(value: &JsonLike) -> StarlarkInfoValue {
        match value {
            JsonLike::Null => StarlarkInfoValue::None,
            JsonLike::Bool(b) => StarlarkInfoValue::Bool(*b),
            JsonLike::Int(i) => StarlarkInfoValue::Int(*i),
            JsonLike::String(s) => StarlarkInfoValue::String(s.clone()),
            JsonLike::Array(items) => StarlarkInfoValue::List(
                items
                    .iter()
                    .map(StarlarkInfoValue::from_json_like)
                    .collect(),
            ),
            JsonLike::Object(fields) => {
                let mut info = StarlarkInfo::new("struct", "<unknown>");
                for (k, v) in fields {
                    info.fields
                        .insert(k.clone(), StarlarkInfoValue::from_json_like(v));
                }
                StarlarkInfoValue::Info(Arc::new(info))
            }
        }
    }
}

/// A dependency-free JSON tree, so that callers need not share a JSON crate with this one.
#[derive(Clone, Debug)]
pub enum JsonLike {
    Null,
    Bool(bool),
    Int(i64),
    String(String),
    Array(Vec<JsonLike>),
    Object(Vec<(String, JsonLike)>),
}

/// A struct or provider instance: Java's `StarlarkInfo`.
// Java: com/google/devtools/build/lib/packages/StarlarkInfo.java:25 StarlarkInfo
#[derive(Clone, Debug)]
pub struct StarlarkInfo {
    /// Java: `info.getProvider().getPrintableName()`.
    pub provider_printable_name: String,
    /// Java: `info.getCreationLocation()` rendered with `Location.toString()`.
    pub creation_location: String,
    pub fields: IndexMap<String, StarlarkInfoValue>,
}

impl StarlarkInfo {
    pub fn new(provider_printable_name: &str, creation_location: &str) -> StarlarkInfo {
        StarlarkInfo {
            provider_printable_name: provider_printable_name.to_string(),
            creation_location: creation_location.to_string(),
            fields: IndexMap::new(),
        }
    }

    /// Java: `StarlarkInfo.getValue(name)`; `None` where Java returns null.
    pub fn get_value(&self, name: &str) -> Option<&StarlarkInfoValue> {
        self.fields.get(name)
    }

    /// Java: `Structure.getValue(name, String.class)`.
    pub fn get_string_value(&self, name: &str) -> Result<String, EvalException> {
        match self.get_value(name) {
            Some(StarlarkInfoValue::String(s)) => Ok(s.clone()),
            Some(other) => Err(EvalException::new(format!(
                "for {} field, got {}, want string",
                name,
                other.starlark_type()
            ))),
            None => Err(EvalException::new(format!(
                "'{}' value has no field or method '{}'",
                self.provider_printable_name, name
            ))),
        }
    }
}

/// Java: `Sequence.cast(x, StarlarkInfo.class / String.class, what)` and `noneableCast`.
// Java: net/starlark/java/eval/Sequence.java:132 Sequence.cast
pub(crate) fn sequence_cast<'v>(
    x: &'v StarlarkInfoValue,
    want: &str,
    what: &str,
    noneable: bool,
) -> Result<&'v [StarlarkInfoValue], EvalException> {
    if noneable && matches!(x, StarlarkInfoValue::None) {
        return Ok(&[]);
    }
    let StarlarkInfoValue::List(items) = x else {
        return Err(EvalException::new(format!(
            "for {}, got {}, want sequence",
            what,
            x.starlark_type()
        )));
    };
    for (i, elem) in items.iter().enumerate() {
        let ok = match want {
            "string" => matches!(elem, StarlarkInfoValue::String(_)),
            "struct" => matches!(elem, StarlarkInfoValue::Info(_)),
            _ => unreachable!(),
        };
        if !ok {
            return Err(EvalException::new(format!(
                "at index {} of {}, got element of type {}, want {}",
                i,
                what,
                elem.starlark_type(),
                want
            )));
        }
    }
    Ok(items)
}
