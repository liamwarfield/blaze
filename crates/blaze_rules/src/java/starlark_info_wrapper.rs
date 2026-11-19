//! `rules/java/StarlarkInfoWrapper.java`: shared access to the fields of a wrapped Starlark
//! provider instance (or struct).
//!
//! PORT: Java's wrapped `StructImpl` lives as long as the provider; the Rust wrappers borrow the
//! Starlark value for the duration of the native call that wraps it (every Java caller wraps,
//! reads and drops within one `java_common` call). Java's `RuleErrorException(EvalException)`
//! is carried as the `starlark::Error` of the `EvalException`: at the Starlark boundary Java
//! reports it as `new EvalException(cause)` with the same message.

use blaze_actions::Artifact;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_starlark::structs::StarlarkInfo;
use starlark::values::{Heap, Value};

use crate::cpp::util::{errorf, sequence_elements};

/// Java `StructImpl.getValue(key)`: the field's value, `None` if the struct has no such field.
// Java: com/google/devtools/build/lib/packages/StructImpl.java:40 StructImpl.getValue
pub fn struct_get_value<'v>(underlying: Value<'v>, key: &str, heap: Heap<'v>) -> Option<Value<'v>> {
    if let Some(info) = StarlarkInfo::from_value(underlying) {
        return info.get_field(key);
    }
    // A native struct (e.g. `ToolchainInfo`): its Starlark fields.
    underlying.get_attr(key, heap).ok().flatten()
}

/// `noneIfNull(value)`: Starlark `None` for a missing field.
// Java: com/google/devtools/build/lib/rules/java/StarlarkInfoWrapper.java:64 StarlarkInfoWrapper.noneIfNull
pub fn none_if_null<'v>(value: Option<Value<'v>>) -> Value<'v> {
    value.unwrap_or_else(Value::new_none)
}

/// The Java class name `Starlark.classType` reports for the field types the wrappers read.
fn class_type(want: &str) -> &str {
    want
}

/// Base class for sharing utility code between wrapped Starlark provider instances.
// Java: com/google/devtools/build/lib/rules/java/StarlarkInfoWrapper.java:29 StarlarkInfoWrapper
#[derive(Clone, Copy)]
pub struct StarlarkInfoWrapper<'v> {
    pub underlying: Value<'v>,
    pub heap: Heap<'v>,
}

impl<'v> StarlarkInfoWrapper<'v> {
    // Java: com/google/devtools/build/lib/rules/java/StarlarkInfoWrapper.java:33 StarlarkInfoWrapper.<init>
    pub fn new(underlying: Value<'v>, heap: Heap<'v>) -> StarlarkInfoWrapper<'v> {
        StarlarkInfoWrapper { underlying, heap }
    }

    /// `getUnderlyingValue(key, type)` without the cast: `None` for a missing field or `None`.
    // Java: com/google/devtools/build/lib/rules/java/StarlarkInfoWrapper.java:37 StarlarkInfoWrapper.getUnderlyingValue
    pub fn get_underlying_value(&self, key: &str) -> Option<Value<'v>> {
        let v = struct_get_value(self.underlying, key, self.heap)?;
        if v.is_none() { None } else { Some(v) }
    }

    /// `StructImpl.getValue(key, type)`'s failed cast.
    // Java: com/google/devtools/build/lib/packages/StructImpl.java:49 StructImpl.getValue
    pub fn cast_error(key: &str, got: Value<'v>, want: &str) -> starlark::Error {
        errorf(format!(
            "for {key} field, got {}, want {}",
            got.get_type(),
            class_type(want)
        ))
    }

    /// `getUnderlyingValue(key, String.class)`.
    pub fn get_underlying_string(&self, key: &str) -> starlark::Result<Option<String>> {
        match self.get_underlying_value(key) {
            None => Ok(None),
            Some(v) => match v.unpack_str() {
                Some(s) => Ok(Some(s.to_owned())),
                None => Err(Self::cast_error(key, v, "string")),
            },
        }
    }

    /// `getUnderlyingValue(key, Boolean.class)`.
    pub fn get_underlying_bool(&self, key: &str) -> starlark::Result<Option<bool>> {
        match self.get_underlying_value(key) {
            None => Ok(None),
            Some(v) => match v.unpack_bool() {
                Some(b) => Ok(Some(b)),
                None => Err(Self::cast_error(key, v, "bool")),
            },
        }
    }

    /// `getUnderlyingValue(key, Artifact.class)`.
    pub fn get_underlying_artifact(&self, key: &str) -> starlark::Result<Option<Artifact>> {
        match self.get_underlying_value(key) {
            None => Ok(None),
            Some(v) => {
                match blaze_actions::analysis::starlark::starlark_file::artifact_from_value(v) {
                    Some(a) => Ok(Some(a)),
                    None => Err(Self::cast_error(key, v, "File")),
                }
            }
        }
    }

    /// `getUnderlyingNestedSet(key, Artifact.class)`: `Depset.noneableCast(noneIfNull(value))`.
    // Java: com/google/devtools/build/lib/rules/java/StarlarkInfoWrapper.java:50 StarlarkInfoWrapper.getUnderlyingNestedSet
    pub fn get_underlying_nested_set_artifacts(
        &self,
        key: &str,
    ) -> starlark::Result<NestedSet<Artifact>> {
        match struct_get_value(self.underlying, key, self.heap) {
            None => Ok(NestedSetBuilder::empty_set(Order::STABLE_ORDER)),
            Some(v) if v.is_none() => Ok(NestedSetBuilder::empty_set(Order::STABLE_ORDER)),
            Some(v) => blaze_analysis::rule::depsets::depset_to_artifacts(v, key),
        }
    }

    /// `getUnderlyingNestedSet(key, String.class)`.
    // Java: com/google/devtools/build/lib/rules/java/StarlarkInfoWrapper.java:50 StarlarkInfoWrapper.getUnderlyingNestedSet
    pub fn get_underlying_nested_set_strings(
        &self,
        key: &str,
    ) -> starlark::Result<NestedSet<String>> {
        match struct_get_value(self.underlying, key, self.heap) {
            None => Ok(NestedSetBuilder::empty_set(Order::STABLE_ORDER)),
            Some(v) if v.is_none() => Ok(NestedSetBuilder::empty_set(Order::STABLE_ORDER)),
            Some(v) => blaze_analysis::rule::depsets::depset_to_strings(v, key),
        }
    }

    /// `getUnderlyingSequence(key, type)`: `Sequence.noneableCast(noneIfNull(value))` as the
    /// element values (the caller casts them).
    // Java: com/google/devtools/build/lib/rules/java/StarlarkInfoWrapper.java:59 StarlarkInfoWrapper.getUnderlyingSequence
    pub fn get_underlying_sequence(&self, key: &str) -> starlark::Result<Vec<Value<'v>>> {
        match struct_get_value(self.underlying, key, self.heap) {
            None => Ok(Vec::new()),
            Some(v) if v.is_none() => Ok(Vec::new()),
            Some(v) => {
                if !crate::cpp::util::is_sequence(v) {
                    return Err(errorf(format!(
                        "for {key}, got {}, want sequence",
                        v.get_type()
                    )));
                }
                sequence_elements(v, self.heap)
            }
        }
    }

    /// `getUnderlyingSequence(key, Artifact.class)`.
    pub fn get_underlying_sequence_artifacts(&self, key: &str) -> starlark::Result<Vec<Artifact>> {
        match struct_get_value(self.underlying, key, self.heap) {
            None => Ok(Vec::new()),
            Some(v) if v.is_none() => Ok(Vec::new()),
            Some(v) => crate::cpp::util::sequence_cast_artifacts(v, key, self.heap),
        }
    }
}
