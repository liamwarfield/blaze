//! Port of `analysis/constraints/EnvironmentCollection.java`: a collection of environments,
//! grouped by the environment groups they belong to.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_packages::target::environment_group::EnvironmentLabels;

/// An environment and the group it belongs to (Java `EnvironmentCollection.EnvironmentWithGroup`).
// Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:51 EnvironmentCollection.EnvironmentWithGroup
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentWithGroup {
    pub environment: Label,
    pub group: Arc<EnvironmentLabels>,
}

impl EnvironmentWithGroup {
    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:58 EnvironmentCollection.EnvironmentWithGroup.create
    pub fn create(environment: Label, group: Arc<EnvironmentLabels>) -> EnvironmentWithGroup {
        EnvironmentWithGroup { environment, group }
    }
}

/// Contains a set of environments, grouped by the environment groups they belong to. Java keeps
/// an `ImmutableListMultimap<EnvironmentLabels, Label>`: keys in insertion order, each key's
/// values grouped under it.
// Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:37 EnvironmentCollection
#[derive(Clone, Debug, Default)]
pub struct EnvironmentCollection {
    map: Vec<(Arc<EnvironmentLabels>, Vec<Label>)>,
}

impl EnvironmentCollection {
    /// Java `EnvironmentCollection.EMPTY`.
    pub fn empty() -> EnvironmentCollection {
        EnvironmentCollection { map: Vec::new() }
    }

    /// Returns the environments in this collection, regardless of group (Java `map.values()`).
    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:67 EnvironmentCollection.getEnvironments
    pub fn get_environments(&self) -> Vec<Label> {
        self.map
            .iter()
            .flat_map(|(_, envs)| envs.iter().cloned())
            .collect()
    }

    /// Returns the environments in this collection that belong to the given group.
    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:74 EnvironmentCollection.getEnvironments
    pub fn get_environments_of(&self, group: &EnvironmentLabels) -> Vec<Label> {
        self.map
            .iter()
            .find(|(g, _)| **g == *group)
            .map(|(_, envs)| envs.clone())
            .unwrap_or_default()
    }

    /// Returns the environment groups represented by this collection.
    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:81 EnvironmentCollection.getGroups
    pub fn get_groups(&self) -> Vec<Arc<EnvironmentLabels>> {
        self.map.iter().map(|(g, _)| g.clone()).collect()
    }

    /// Whether `group` is one of [`EnvironmentCollection::get_groups`].
    pub fn contains_group(&self, group: &EnvironmentLabels) -> bool {
        self.map.iter().any(|(g, _)| **g == *group)
    }

    /// Returns the environments in this collection, along with their groups.
    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:88 EnvironmentCollection.getGroupedEnvironments
    pub fn get_grouped_environments(&self) -> Vec<EnvironmentWithGroup> {
        let mut builder: Vec<EnvironmentWithGroup> = Vec::new();
        for (group, envs) in &self.map {
            for env in envs {
                let e = EnvironmentWithGroup::create(env.clone(), group.clone());
                // ImmutableSet.Builder: duplicates are dropped.
                if !builder.contains(&e) {
                    builder.push(e);
                }
            }
        }
        builder
    }

    /// Returns true if this collection has no environments.
    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:97 EnvironmentCollection.isEmpty
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl PartialEq for EnvironmentCollection {
    // ImmutableListMultimap equality considers the order of each value list but not the order of
    // keys. Additionally check equality of the keys as a list to reflect ordering.
    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:107 EnvironmentCollection.equals
    fn eq(&self, that: &EnvironmentCollection) -> bool {
        self.map.len() == that.map.len()
            && self
                .map
                .iter()
                .zip(that.map.iter())
                .all(|((ga, ea), (gb, eb))| **ga == **gb && ea == eb)
    }
}

impl fmt::Display for EnvironmentCollection {
    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:121 EnvironmentCollection.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // PORT: Java also prints the multimap's hash code.
        let size: usize = self.map.iter().map(|(_, e)| e.len()).sum();
        write!(f, "EnvironmentCollection{{size={size}, map={{")?;
        for (i, (group, envs)) in self.map.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            let envs: Vec<String> = envs.iter().map(|e| e.to_string()).collect();
            write!(f, "{}=[{}]", group, envs.join(", "))?;
        }
        write!(f, "}}}}")
    }
}

/// Builder for [`EnvironmentCollection`].
// Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:130 EnvironmentCollection.Builder
#[derive(Default)]
pub struct EnvironmentCollectionBuilder {
    // ImmutableListMultimap.Builder allows duplicate values, which we don't want.
    added_labels: HashSet<Label>,
    map_builder: Vec<(Arc<EnvironmentLabels>, Vec<Label>)>,
}

impl EnvironmentCollectionBuilder {
    pub fn new() -> EnvironmentCollectionBuilder {
        EnvironmentCollectionBuilder::default()
    }

    fn map_put(&mut self, group: &Arc<EnvironmentLabels>, environment: Label) {
        match self.map_builder.iter_mut().find(|(g, _)| **g == **group) {
            Some((_, envs)) => envs.push(environment),
            None => self.map_builder.push((group.clone(), vec![environment])),
        }
    }

    /// Inserts the given environment / owning group pair.
    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:137 EnvironmentCollection.Builder.put
    pub fn put(&mut self, group: &Arc<EnvironmentLabels>, environment: Label) -> &mut Self {
        if self.added_labels.insert(environment.clone()) {
            self.map_put(group, environment);
        }
        self
    }

    /// Inserts the given set of environments, all belonging to the specified group.
    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:146 EnvironmentCollection.Builder.putAll
    pub fn put_all(
        &mut self,
        group: &Arc<EnvironmentLabels>,
        environments: impl IntoIterator<Item = Label>,
    ) -> &mut Self {
        for env in environments {
            if self.added_labels.insert(env.clone()) {
                self.map_put(group, env);
            }
        }
        self
    }

    /// Inserts the contents of another [`EnvironmentCollection`] into this one.
    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:157 EnvironmentCollection.Builder.putAll
    pub fn put_all_collection(&mut self, other: &EnvironmentCollection) -> &mut Self {
        for (group, envs) in &other.map {
            for env in envs {
                if self.added_labels.insert(env.clone()) {
                    self.map_put(group, env.clone());
                }
            }
        }
        self
    }

    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentCollection.java:166 EnvironmentCollection.Builder.build
    pub fn build(&self) -> EnvironmentCollection {
        // PORT: Java interns the result (identity only).
        EnvironmentCollection {
            map: self.map_builder.clone(),
        }
    }
}
