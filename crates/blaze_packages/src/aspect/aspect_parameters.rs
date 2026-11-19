//! Port of `packages/AspectParameters.java`.

use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::LazyLock;

use indexmap::IndexMap;

/// Values of some attributes of rules, passed to aspects (Java `AspectParameters`).
///
/// Java keeps an `ImmutableMultimap<String, String>` (an `ImmutableListMultimap`): keys in order
/// of first insertion, each key's values in insertion order, duplicates kept. Equality and hash
/// are the multimap's (`asMap()` equality: key order does not matter, value order does).
// Java: com/google/devtools/build/lib/packages/AspectParameters.java:42 AspectParameters
#[derive(Clone)]
pub struct AspectParameters {
    attributes: Arc<IndexMap<String, Vec<String>>>,
}

/// Java `AspectParameters.EMPTY`.
// Java: com/google/devtools/build/lib/packages/AspectParameters.java:50 AspectParameters.EMPTY
pub static EMPTY: LazyLock<AspectParameters> = LazyLock::new(|| AspectParameters {
    attributes: Arc::new(IndexMap::new()),
});

impl AspectParameters {
    /// Java `AspectParameters.EMPTY`.
    pub fn empty() -> AspectParameters {
        EMPTY.clone()
    }

    // Java: com/google/devtools/build/lib/packages/AspectParameters.java:52 AspectParameters.create
    fn create(attributes: IndexMap<String, Vec<String>>) -> AspectParameters {
        if attributes.is_empty() {
            return AspectParameters::empty();
        }
        AspectParameters {
            attributes: Arc::new(attributes),
        }
    }

    /// Java `new AspectParameters.Builder()`.
    pub fn builder() -> AspectParametersBuilder {
        AspectParametersBuilder::new()
    }

    /// Returns the values for the key, or an empty collection if the key is missing.
    // Java: com/google/devtools/build/lib/packages/AspectParameters.java:82 AspectParameters.getAttribute
    pub fn get_attribute(&self, key: &str) -> &[String] {
        match self.attributes.get(key) {
            Some(values) => values,
            None => &[],
        }
    }

    /// Java `getAttributes()` as the multimap's `asMap()` view (keys in first-insertion order,
    /// values in insertion order).
    // Java: com/google/devtools/build/lib/packages/AspectParameters.java:86 AspectParameters.getAttributes
    pub fn get_attributes(&self) -> &IndexMap<String, Vec<String>> {
        &self.attributes
    }

    /// Java `getAttributes().entries()`: every (key, value) pair, grouped by key.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.attributes
            .iter()
            .flat_map(|(k, vs)| vs.iter().map(move |v| (k.as_str(), v.as_str())))
    }

    /// Like [`AspectParameters::get_attribute`], but asserts that there is exactly one value
    /// (Guava `Iterables.getOnlyElement`, which throws otherwise).
    // Java: com/google/devtools/build/lib/packages/AspectParameters.java:96 AspectParameters.getOnlyValueOfAttribute
    pub fn get_only_value_of_attribute(&self, key: &str) -> &str {
        let values = self.get_attribute(key);
        match values {
            [only] => only,
            [] => panic!("NoSuchElementException"),
            // Guava Iterators.getOnlyElement: "expected one element but was: <a, b, ...>" (at
            // most five elements, then ", ...").
            _ => {
                let mut shown: Vec<&str> = values.iter().take(5).map(String::as_str).collect();
                if values.len() > 5 {
                    shown.push("...");
                }
                panic!(
                    "IllegalArgumentException: expected one element but was: <{}>",
                    shown.join(", ")
                )
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/AspectParameters.java:100 AspectParameters.isEmpty
    pub fn is_empty(&self) -> bool {
        *self == *EMPTY
    }
}

impl PartialEq for AspectParameters {
    // Java: com/google/devtools/build/lib/packages/AspectParameters.java:106 AspectParameters.equals
    // PORT: `IndexMap` equality ignores key order and compares each key's value list in order,
    // as Guava's `ListMultimap` equality (`asMap().equals`).
    fn eq(&self, other: &AspectParameters) -> bool {
        Arc::ptr_eq(&self.attributes, &other.attributes) || self.attributes == other.attributes
    }
}

impl Eq for AspectParameters {}

impl Hash for AspectParameters {
    // Java: com/google/devtools/build/lib/packages/AspectParameters.java:118 AspectParameters.hashCode
    // PORT: consistent with the order-insensitive equality: entries hashed in key order.
    fn hash<H: Hasher>(&self, state: &mut H) {
        let mut entries: Vec<(&String, &Vec<String>)> = self.attributes.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        entries.len().hash(state);
        for (k, vs) in entries {
            k.hash(state);
            vs.hash(state);
        }
    }
}

impl fmt::Display for AspectParameters {
    /// Java `ImmutableMultimap.toString()`: `{a=[1, 2], b=[3]}`.
    // Java: com/google/devtools/build/lib/packages/AspectParameters.java:123 AspectParameters.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("{")?;
        for (i, (k, vs)) in self.attributes.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{k}=[{}]", vs.join(", "))?;
        }
        f.write_str("}")
    }
}

impl fmt::Debug for AspectParameters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A builder for [`AspectParameters`] (Java `AspectParameters.Builder`).
// Java: com/google/devtools/build/lib/packages/AspectParameters.java:60 AspectParameters.Builder
#[derive(Debug, Clone, Default)]
pub struct AspectParametersBuilder {
    attributes: IndexMap<String, Vec<String>>,
}

impl AspectParametersBuilder {
    pub fn new() -> Self {
        AspectParametersBuilder::default()
    }

    /// Adds a new pair of attribute-value.
    // Java: com/google/devtools/build/lib/packages/AspectParameters.java:66 AspectParameters.Builder.addAttribute
    pub fn add_attribute(&mut self, name: &str, value: &str) -> &mut Self {
        self.attributes
            .entry(name.to_owned())
            .or_default()
            .push(value.to_owned());
        self
    }

    /// Creates a new instance of [`AspectParameters`].
    // Java: com/google/devtools/build/lib/packages/AspectParameters.java:74 AspectParameters.Builder.build
    pub fn build(&self) -> AspectParameters {
        AspectParameters::create(self.attributes.clone())
    }
}
