//! `repo_metadata`: the object a repository rule's implementation function returns to describe
//! its reproducibility.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/starlark/RepoMetadata.java.
//!
//! PORT: Java keeps `attrsForReproducibility` as a Starlark `Dict<String, Object>`; Starlark
//! values cannot outlive their heap here, so the values are stored as [`AttributeValue`]s
//! (converted with [`starlark_to_attribute_value`]).

use std::fmt;

use allocative::Allocative;
use blaze_bzlmod_model::AttributeValue;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
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

/// The Starlark object returned from a `repository_rule`'s implementation function.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/RepoMetadata.java:30 RepoMetadata
#[derive(Clone, Debug)]
pub struct RepoMetadata {
    pub reproducible: Reproducibility,
    pub attrs_for_reproducibility: IndexMap<String, AttributeValue>,
}

impl RepoMetadata {
    pub fn new(
        reproducible: Reproducibility,
        attrs_for_reproducibility: IndexMap<String, AttributeValue>,
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
) -> Result<IndexMap<String, AttributeValue>, String> {
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
        let value = match starlark_to_attribute_value(v) {
            Ok(value) => value,
            // Java keeps any Starlark value here; Blaze stores only attribute-like values.
            Err(e) => blaze_util::unported!(
                "RepoMetadata.attrsForReproducibility",
                "value of '{}' in {}: {}",
                k.unpack_str().unwrap_or_default(),
                what,
                e
            ),
        };
        out.insert(k.unpack_str().unwrap_or_default().to_owned(), value);
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
