//! `repo_metadata`: the object a repository rule's implementation function returns to describe
//! its reproducibility.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/starlark/RepoMetadata.java.
//!
//! PORT: Java keeps `attrsForReproducibility` as a Starlark `Dict<String, Object>`; Starlark
//! values cannot outlive their heap here, so each value is stored as a
//! [`ReproducibilityValue`]: an attribute-like value as its [`AttributeValue`], any other value
//! by its `Starlark.repr`.

use std::fmt;

use allocative::Allocative;
use blaze_bzlmod_model::AttributeValue;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::dict::DictRef;
use starlark::values::starlark_value;

use crate::attr_values::starlark_to_attribute_value;

/// Whether the fetched contents of a repo are reproducible, hence cacheable.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/RepoMetadata.java:37 RepoMetadata.Reproducibility
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reproducibility {
    Yes,
    No,
}

/// A value of `attrsForReproducibility` (Java: any Starlark `Object`).
///
/// The map is read by `RepositoryResolvedEvent`, which compares its values with the repo's
/// attribute values (`Object.equals`) and prints them (`Starlark.repr`). An attribute-like value
/// (`None`, a bool, an int, a string, a `Label`, a list or dict of those) is stored converted
/// and compares as the attribute value would; any other value (a tuple, a float, a struct, a
/// function, ...) equals no attribute value in Java (`Tuple.equals` only takes tuples,
/// `StarlarkFloat.equals` only floats, ...) and is kept as its repr.
#[derive(Clone, Debug, PartialEq)]
pub enum ReproducibilityValue {
    Attribute(AttributeValue),
    /// `Starlark.repr(value)` of a value that no attribute value equals.
    Other(String),
}

impl ReproducibilityValue {
    /// Stores a Starlark value (see the type documentation).
    pub fn of(value: Value<'_>) -> ReproducibilityValue {
        if is_attribute_like(value)
            && let Ok(converted) = starlark_to_attribute_value(value)
        {
            return ReproducibilityValue::Attribute(converted);
        }
        ReproducibilityValue::Other(value.to_repr())
    }
}

/// Whether `value` and everything in it is a value an attribute can hold as is: `None`, a bool,
/// an int, a string, a `Label`, or a list or dict of those. A tuple is not (Java compares it
/// with the attribute's list and finds them different).
fn is_attribute_like(value: Value<'_>) -> bool {
    if value.is_none() || value.unpack_bool().is_some() || value.unpack_str().is_some() {
        return true;
    }
    if value.unpack_i32().is_some() {
        return true;
    }
    if value
        .downcast_ref::<blaze_starlark::label::LabelValue>()
        .is_some()
    {
        return true;
    }
    // `ListRef` is a list only; a tuple falls through to `false`.
    if let Some(list) = starlark::values::list::ListRef::from_value(value) {
        return list.iter().all(is_attribute_like);
    }
    if let Some(dict) = DictRef::from_value(value) {
        return dict
            .iter()
            .all(|(k, v)| is_attribute_like(k) && is_attribute_like(v));
    }
    false
}

/// The Starlark object returned from a `repository_rule`'s implementation function.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/RepoMetadata.java:30 RepoMetadata
#[derive(Clone, Debug)]
pub struct RepoMetadata {
    pub reproducible: Reproducibility,
    pub attrs_for_reproducibility: IndexMap<String, ReproducibilityValue>,
}

impl RepoMetadata {
    pub fn new(
        reproducible: Reproducibility,
        attrs_for_reproducibility: IndexMap<String, ReproducibilityValue>,
    ) -> RepoMetadata {
        RepoMetadata {
            reproducible,
            attrs_for_reproducibility,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/RepoMetadata.java:33 RepoMetadata.NONREPRODUCIBLE
    pub fn nonreproducible() -> RepoMetadata {
        RepoMetadata::new(Reproducibility::No, IndexMap::new())
    }
}

/// `Dict.cast(x, String.class, Object.class, what)` followed by the conversion of the values to
/// stored form. `Err` is the message of Java's `EvalException`.
///
/// Used for `repository_ctx.repo_metadata(attrs_for_reproducibility = ...)` and for a dict
/// returned by the implementation function (`RepositoryFetchFunction`, `what = "return value"`).
// Java: net/starlark/java/eval/Dict.java:659 Dict.cast
pub fn attrs_from_starlark_dict(
    x: Value<'_>,
    what: &str,
) -> Result<IndexMap<String, ReproducibilityValue>, String> {
    let Some(dict) = DictRef::from_value(x) else {
        return Err(format!("got {} for '{}', want dict", x.get_type(), what));
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
    let mut out = IndexMap::with_capacity(dict.len());
    for (k, v) in dict.iter() {
        out.insert(
            k.unpack_str().unwrap_or_default().to_owned(),
            ReproducibilityValue::of(v),
        );
    }
    Ok(out)
}

/// The Starlark value of type `repo_metadata` wrapping a [`RepoMetadata`].
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct RepoMetadataValue(#[allocative(skip)] pub RepoMetadata);

starlark_simple_value!(RepoMetadataValue);

impl fmt::Display for RepoMetadataValue {
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr (default)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object com.google.devtools.build.lib.bazel.repository.starlark.RepoMetadata>",
        )
    }
}

#[starlark_value(type = "repo_metadata")]
impl<'v> StarlarkValue<'v> for RepoMetadataValue {}
