//! Ports of `Facts` (as plain data) and `FactsAdapter`, with the parts of Starlark's
//! `json.encode`/`json.decode` and `StarlarkFloat` formatting that the lockfile depends on.

use num_bigint::BigInt;

use crate::gson::{JsonValue, JsonWriter};
use crate::version::java_string_compare;

/// A JSON-like Starlark value stored in [`Facts`]: `None`, bool, int, float, string, list, or
/// dict with string keys (tuples are normalized to lists).
#[derive(Debug, Clone, PartialEq)]
pub enum FactValue {
    None,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    String(String),
    List(Vec<FactValue>),
    /// Entries in insertion order (sorted by key after normalization).
    Dict(Vec<(String, FactValue)>),
}

/// User-provided JSON-like data attached to a module extension that is persisted across
/// reevaluations of the extension. Port of `Facts`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:53 Facts
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Facts {
    value: Vec<(String, FactValue)>,
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:76 Facts.MAX_FACTS_DEPTH
const MAX_FACTS_DEPTH: i32 = 7;

impl Facts {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:55 Facts.EMPTY
    pub fn empty() -> Facts {
        Facts { value: Vec::new() }
    }

    pub fn value(&self) -> &[(String, FactValue)] {
        &self.value
    }

    /// Validates (depth) and normalizes (sorts dicts) a dict value. The error is the
    /// `EvalException` message.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:59 Facts.validateAndCreate
    pub fn validate_and_create(value: FactValue) -> Result<Facts, String> {
        let FactValue::Dict(_) = &value else {
            return Err(format!(
                "got {} for 'facts', want dict",
                starlark_type(&value)
            ));
        };
        match validate_and_normalize(value, MAX_FACTS_DEPTH)? {
            FactValue::Dict(entries) => Ok(Facts { value: entries }),
            _ => unreachable!(),
        }
    }

    /// `createUnchecked`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:66 Facts.createUnchecked
    pub fn create_unchecked(value: Vec<(String, FactValue)>) -> Facts {
        Facts { value }
    }
}

fn starlark_type(v: &FactValue) -> &'static str {
    match v {
        FactValue::None => "NoneType",
        FactValue::Bool(_) => "bool",
        FactValue::Int(_) => "int",
        FactValue::Float(_) => "float",
        FactValue::String(_) => "string",
        FactValue::List(_) => "list",
        FactValue::Dict(_) => "dict",
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/Facts.java:85 Facts.validateAndNormalize
fn validate_and_normalize(facts: FactValue, remaining_depth: i32) -> Result<FactValue, String> {
    if remaining_depth < 0 {
        return Err(format!(
            "Facts cannot be nested more than {MAX_FACTS_DEPTH} levels deep"
        ));
    }
    Ok(match facts {
        FactValue::List(items) => FactValue::List(
            items
                .into_iter()
                .map(|i| validate_and_normalize(i, remaining_depth - 1))
                .collect::<Result<_, _>>()?,
        ),
        FactValue::Dict(entries) => {
            // TreeMap<String, Object>: sorted, later duplicates replace earlier ones.
            let mut sorted: Vec<(String, FactValue)> = Vec::new();
            for (k, v) in entries {
                let v = validate_and_normalize(v, remaining_depth - 1)?;
                match sorted.binary_search_by(|(e, _)| java_string_compare(e, &k)) {
                    Ok(i) => sorted[i].1 = v,
                    Err(i) => sorted.insert(i, (k, v)),
                }
            }
            FactValue::Dict(sorted)
        }
        other => other,
    })
}

/// Port of `FactsAdapter`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/FactsAdapter.java:34 FactsAdapter
pub struct FactsAdapter;

impl FactsAdapter {
    /// Encodes with Starlark `json.encode`, round-trips through Gson, and writes with `out`'s
    /// indentation. Gson's default `serializeNulls` (false) drops null dict entries.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/FactsAdapter.java:44 FactsAdapter.write
    pub fn write(out: &mut JsonWriter, facts: &Facts) {
        let json = json_encode(&FactValue::Dict(facts.value.clone()));
        out.json_value_with_gson_defaults(&json);
    }

    /// Reads with Gson, re-encodes compactly with Gson defaults (dropping null dict entries),
    /// decodes with Starlark `json.decode`, then validates. The error is the `IOException`
    /// message.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/FactsAdapter.java:58 FactsAdapter.read
    pub fn read(json: &JsonValue) -> Result<Facts, String> {
        let value = json_decode(json);
        Facts::validate_and_create(value).map_err(|e| format!("Failed to decode facts JSON: {e}"))
    }
}

/// Starlark `json.encode` followed by Gson's parse into a tree: dict keys are sorted (Java
/// `Arrays.sort` on strings), numbers keep Starlark's text.
// Java: net/starlark/java/lib/json/Json.java:140 Json.Encoder.encode
fn json_encode(v: &FactValue) -> JsonValue {
    match v {
        FactValue::None => JsonValue::Null,
        FactValue::Bool(b) => JsonValue::Bool(*b),
        FactValue::Int(i) => JsonValue::Number(i.to_string()),
        FactValue::Float(f) => {
            if !f.is_finite() {
                panic!(
                    "IllegalStateException: Unexpected error while serializing facts: cannot \
                     encode non-finite float {}",
                    starlark_float_repr(*f)
                );
            }
            JsonValue::Number(starlark_float_repr(*f))
        }
        FactValue::String(s) => JsonValue::String(s.clone()),
        FactValue::List(items) => JsonValue::Array(items.iter().map(json_encode).collect()),
        FactValue::Dict(entries) => {
            let mut sorted: Vec<&(String, FactValue)> = entries.iter().collect();
            sorted.sort_by(|a, b| java_string_compare(&a.0, &b.0));
            JsonValue::Object(
                sorted
                    .into_iter()
                    .map(|(k, v)| (k.clone(), json_encode(v)))
                    .collect(),
            )
        }
    }
}

/// Starlark `json.decode` of Gson's compact re-encoding of `json`.
// Java: net/starlark/java/lib/json/Json.java:560 Json.Decoder.parse
fn json_decode(json: &JsonValue) -> FactValue {
    match json {
        JsonValue::Null => FactValue::None,
        JsonValue::Bool(b) => FactValue::Bool(*b),
        JsonValue::Number(n) => {
            if n.contains(['.', 'e', 'E']) {
                FactValue::Float(n.parse::<f64>().expect("JSON number"))
            } else {
                FactValue::Int(n.parse::<BigInt>().expect("JSON number"))
            }
        }
        JsonValue::String(s) => FactValue::String(s.clone()),
        JsonValue::Array(items) => FactValue::List(items.iter().map(json_decode).collect()),
        JsonValue::Object(members) => FactValue::Dict(
            members
                .iter()
                // Gson's default serializeNulls drops null members before Starlark sees them.
                .filter(|(_, v)| *v != JsonValue::Null)
                .map(|(k, v)| (k.clone(), json_decode(v)))
                .collect(),
        ),
    }
}

/// `StarlarkFloat.toString` (`format(v, 'g')`): Java's `%.17g`, cleaned of trailing zeros.
// PORT: Java's Formatter starts from FloatingDecimal's digits, which are the shortest
// round-trip digits except in rare cases; this uses Rust's shortest digits.
// Java: net/starlark/java/eval/StarlarkFloat.java:120 StarlarkFloat.format
pub fn starlark_float_repr(v: f64) -> String {
    if !v.is_finite() {
        if v == f64::INFINITY {
            return "+inf".to_string();
        } else if v == f64::NEG_INFINITY {
            return "-inf".to_string();
        } else {
            return "nan".to_string();
        }
    }
    let sign = if v.is_sign_negative() { "-" } else { "" };
    let a = v.abs();
    if a == 0.0 {
        return format!("{sign}0.0");
    }
    // Shortest digits and decimal exponent: a = d.ddd x 10^exp.
    let e_form = format!("{a:e}");
    let (mantissa, exp) = e_form.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    if (-4..17).contains(&exp) {
        // Decimal format, then trailing zeros removed (keeping one digit after the point).
        let (int_part, frac_part) = if exp >= 0 {
            let int_len = (exp + 1) as usize;
            let mut int_part = digits.chars().take(int_len).collect::<String>();
            while int_part.len() < int_len {
                int_part.push('0');
            }
            let frac: String = digits.chars().skip(int_len).collect();
            (int_part, frac)
        } else {
            let zeros = "0".repeat((-exp - 1) as usize);
            ("0".to_string(), format!("{zeros}{digits}"))
        };
        let frac = frac_part.trim_end_matches('0');
        let frac = if frac.is_empty() { "0" } else { frac };
        format!("{sign}{int_part}.{frac}")
    } else {
        // Scientific: mantissa without trailing zeros (and without a bare '.'), exponent with at
        // least two digits.
        let first = &digits[..1];
        let rest = digits[1..].trim_end_matches('0');
        let mantissa = if rest.is_empty() {
            first.to_string()
        } else {
            format!("{first}.{rest}")
        };
        let exp_sign = if exp < 0 { '-' } else { '+' };
        format!("{sign}{mantissa}e{exp_sign}{:02}", exp.abs())
    }
}
