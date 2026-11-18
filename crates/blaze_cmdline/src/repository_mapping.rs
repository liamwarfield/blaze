//! Port of `RepositoryMapping`.

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

use indexmap::IndexMap;

use crate::interner::stable_hash;
use crate::repository_name::{MAIN, RepositoryName};
use crate::string_utilities::did_you_mean;

/// The mapping from apparent repo name to canonical repo name, from the viewpoint of a context
/// repo (Java `RepositoryMapping`).
///
/// Entries keep insertion order (Java `ImmutableMap`). Equality and hashing ignore entry order,
/// as Java's map equality does, and ignore any cached inverse map.
// Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:34 RepositoryMapping
#[derive(Clone)]
pub struct RepositoryMapping(Arc<RepositoryMappingInner>);

struct RepositoryMappingInner {
    entries: IndexMap<String, RepositoryName>,
    context_repo: RepositoryName,
    hash: u64,
    /// Set by `with_cached_inverse_map` (Java: the anonymous subclass overriding `getInverse`).
    inverse: Option<HashMap<RepositoryName, String>>,
}

/// An empty repo mapping with the main repo as the context repo (it maps "" to the main repo).
// Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:36 RepositoryMapping.EMPTY
pub static EMPTY: LazyLock<RepositoryMapping> =
    LazyLock::new(|| RepositoryMapping::create([("", MAIN.clone())], MAIN.clone()));

// Java ImmutableMap.hashCode: sum of entry hash codes, so independent of order.
fn entries_hash(entries: &IndexMap<String, RepositoryName>) -> u64 {
    entries.iter().fold(0u64, |acc, (k, v)| {
        acc.wrapping_add(stable_hash(k.as_str()) ^ v.cached_hash())
    })
}

impl RepositoryMapping {
    // Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:44 RepositoryMapping.<init>
    fn new(
        entries: IndexMap<String, RepositoryName>,
        entries_hash_code: u64,
        context_repo: RepositoryName,
        inverse: Option<HashMap<RepositoryName, String>>,
    ) -> Self {
        let hash = entries_hash_code
            .wrapping_mul(31)
            .wrapping_add(context_repo.cached_hash());
        RepositoryMapping(Arc::new(RepositoryMappingInner {
            entries,
            context_repo,
            hash,
            inverse,
        }))
    }

    /// Returns all the entries in this repo mapping, in insertion order.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:52 RepositoryMapping.entries
    pub fn entries(&self) -> &IndexMap<String, RepositoryName> {
        &self.0.entries
    }

    /// The context repo of this repository mapping.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:60 RepositoryMapping.contextRepo
    pub fn context_repo(&self) -> &RepositoryName {
        &self.0.context_repo
    }

    /// Creates a mapping. Later duplicate keys replace earlier values in the first key's position.
    // PORT: Java ImmutableMap.of throws on duplicate keys; callers never pass duplicates.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:82 RepositoryMapping.create
    pub fn create<K: Into<String>>(
        entries: impl IntoIterator<Item = (K, RepositoryName)>,
        context_repo: RepositoryName,
    ) -> RepositoryMapping {
        let entries: IndexMap<String, RepositoryName> =
            entries.into_iter().map(|(k, v)| (k.into(), v)).collect();
        let hash = entries_hash(&entries);
        Self::create_with_known_entries_hash_code(entries, hash, context_repo)
    }

    // Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:94 RepositoryMapping.createWithKnownEntriesHashCode
    fn create_with_known_entries_hash_code(
        entries: IndexMap<String, RepositoryName>,
        entries_hash_code: u64,
        context_repo: RepositoryName,
    ) -> RepositoryMapping {
        Self::new(entries, entries_hash_code, context_repo, None)
    }

    /// Returns a mapping with the given additional mappings; on conflicts the existing mappings
    /// take precedence.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:108 RepositoryMapping.withAdditionalMappings
    pub fn with_additional_mappings<K: Into<String>>(
        &self,
        additional_mappings: impl IntoIterator<Item = (K, RepositoryName)>,
    ) -> RepositoryMapping {
        // ImmutableMap.Builder.buildKeepingLast: last value wins, first position is kept.
        let mut merged: IndexMap<String, RepositoryName> = IndexMap::new();
        for (k, v) in additional_mappings {
            merged.insert(k.into(), v);
        }
        for (k, v) in self.entries() {
            merged.insert(k.clone(), v.clone());
        }
        Self::create(merged, self.context_repo().clone())
    }

    /// Like [`RepositoryMapping::with_additional_mappings`] with another mapping's entries (its
    /// owner repo is ignored).
    // Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:124 RepositoryMapping.withAdditionalMappings
    pub fn with_additional_mappings_from(
        &self,
        additional_mappings: Option<&RepositoryMapping>,
    ) -> RepositoryMapping {
        match additional_mappings {
            None => self.with_additional_mappings(std::iter::empty::<(String, RepositoryName)>()),
            Some(m) => self
                .with_additional_mappings(m.entries().iter().map(|(k, v)| (k.clone(), v.clone()))),
        }
    }

    /// Returns the canonical repository name for the apparent name `pre_mapping_name`, or a
    /// not-visible name (with a "did you mean" suffix) if there is no entry.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:133 RepositoryMapping.get
    pub fn get(&self, pre_mapping_name: &str) -> RepositoryName {
        if let Some(canonical_repo_name) = self.entries().get(pre_mapping_name) {
            return canonical_repo_name.clone();
        }
        RepositoryName::create_unvalidated(pre_mapping_name).to_non_visible_with_suffix(
            self.context_repo(),
            &did_you_mean(pre_mapping_name, self.entries().keys().map(|k| k.as_str())),
        )
    }

    /// Returns the first apparent name that maps to the given canonical name, if any.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:145 RepositoryMapping.getInverse
    pub fn get_inverse(&self, post_mapping_name: &RepositoryName) -> Option<String> {
        if let Some(inverse) = &self.0.inverse {
            // Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:166 RepositoryMapping.withCachedInverseMap.getInverse
            return inverse.get(post_mapping_name).cloned();
        }
        self.entries()
            .iter()
            .find(|(_, v)| *v == post_mapping_name)
            .map(|(k, _)| k.clone())
    }

    /// Returns a mapping with identical contents whose inverse lookups are cached.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:158 RepositoryMapping.withCachedInverseMap
    pub fn with_cached_inverse_map(&self) -> RepositoryMapping {
        let mut inverse = HashMap::with_capacity(self.entries().len());
        for (k, v) in self.entries() {
            inverse.entry(v.clone()).or_insert_with(|| k.clone());
        }
        Self::new(
            self.0.entries.clone(),
            entries_hash(&self.0.entries),
            self.0.context_repo.clone(),
            Some(inverse),
        )
    }
}

// Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:65 RepositoryMapping.equals
impl PartialEq for RepositoryMapping {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (self.0.entries == other.0.entries && self.0.context_repo == other.0.context_repo)
    }
}

impl Eq for RepositoryMapping {}

// Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:73 RepositoryMapping.hashCode
impl Hash for RepositoryMapping {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.0.hash);
    }
}

// Java: com/google/devtools/build/lib/cmdline/RepositoryMapping.java:78 RepositoryMapping.toString
impl fmt::Display for RepositoryMapping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RepositoryMapping{entries={")?;
        for (i, (k, v)) in self.entries().iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{}={}", k, v)?;
        }
        write!(f, "}}, contextRepo={}}}", self.context_repo())
    }
}

impl fmt::Debug for RepositoryMapping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
