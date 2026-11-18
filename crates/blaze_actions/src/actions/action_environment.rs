//! Port of `lib/actions/ActionEnvironment.java`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use blaze_util::Fingerprint;
use indexmap::{IndexMap, IndexSet};

/// Environment variables for build or test actions.
///
/// The action environment consists of two parts.
///
/// 1. All the environment variables with a fixed value, stored in a map.
/// 2. All the environment variables inherited from the client environment, stored in a set.
///
/// Inherited environment variables must be declared in the Action interface (see
/// `Action#getClientEnvironmentVariables`), so that the dependency on the client environment is
/// known to the execution framework for correct incremental builds.
///
/// By splitting the environment, we can handle environment variable changes more efficiently -
/// the dependency of the action on the environment variable are tracked in Skyframe (and in the
/// action cache), such that Bazel knows exactly which actions it needs to rerun, and does not
/// have to reanalyze the entire dependency graph.
// Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:48 ActionEnvironment
// PORT: Java's three subclasses (Empty, Simple, Compound) are one enum; ImmutableMap/ImmutableSet
// iteration order (insertion order) is kept with IndexMap/IndexSet. Interning is not ported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionEnvironment {
    // Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:155 ActionEnvironment.EmptyActionEnvironment
    Empty,
    // Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:177 ActionEnvironment.SimpleActionEnvironment
    Simple {
        fixed_env: IndexMap<String, String>,
        inherited_env: IndexSet<String>,
    },
    // Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:215 ActionEnvironment.CompoundActionEnvironment
    Compound {
        base: Arc<ActionEnvironment>,
        fixed_vars: IndexMap<String, String>,
    },
}

/// Java `ActionEnvironment.EMPTY`.
// Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:50 ActionEnvironment.EMPTY
pub const EMPTY: ActionEnvironment = ActionEnvironment::Empty;

impl ActionEnvironment {
    /// Creates a new action environment. The order in which the environments are combined is
    /// undefined, so callers need to take care that the key set of the `fixed_env` map and the
    /// set of `inherited_env` elements are disjoint.
    // Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:56 ActionEnvironment.create
    pub fn create(
        fixed_env: IndexMap<String, String>,
        inherited_env: IndexSet<String>,
    ) -> ActionEnvironment {
        if fixed_env.is_empty() && inherited_env.is_empty() {
            return ActionEnvironment::Empty;
        }
        ActionEnvironment::Simple {
            fixed_env,
            inherited_env,
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:56 ActionEnvironment.create
    pub fn create_fixed(fixed_env: IndexMap<String, String>) -> ActionEnvironment {
        ActionEnvironment::create(fixed_env, IndexSet::new())
    }

    /// Splits the given map into a map of variables with a fixed value, and a set of variables
    /// that should be inherited, the latter of which are identified by having a `null` value in
    /// the given map. Returns these two parts as a new `ActionEnvironment` instance.
    // Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:80 ActionEnvironment.split
    pub fn split<'a>(
        env: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
    ) -> ActionEnvironment {
        // Care needs to be taken that the two sets don't overlap - the order in which the two
        // parts are combined later is undefined.
        let mut fixed_env: BTreeMap<String, String> = BTreeMap::new();
        let mut inherited_env: BTreeSet<String> = BTreeSet::new();
        for (key, value) in env {
            match value {
                Some(v) => {
                    fixed_env.insert(key.to_string(), v.to_string());
                }
                None => {
                    inherited_env.insert(key.to_string());
                }
            }
        }
        ActionEnvironment::create(
            fixed_env.into_iter().collect(),
            inherited_env.into_iter().collect(),
        )
    }

    /// Returns the 'fixed' part of the environment, i.e., those environment variables that are set
    /// to fixed values and their values. This should only be used for testing and to compute the
    /// cache keys of actions. Use [`Self::resolve`] instead to get the complete environment.
    // Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:100 ActionEnvironment.getFixedEnv
    pub fn get_fixed_env(&self) -> IndexMap<String, String> {
        match self {
            ActionEnvironment::Empty => IndexMap::new(),
            ActionEnvironment::Simple { fixed_env, .. } => fixed_env.clone(),
            // Java: ImmutableMap.builder().putAll(base).putAll(fixedVars).buildKeepingLast().
            // PORT: Guava's buildKeepingLast keeps each key at the position of its first
            // occurrence with the last value; IndexMap::insert does the same.
            ActionEnvironment::Compound { base, fixed_vars } => {
                let mut m = base.get_fixed_env();
                for (k, v) in fixed_vars {
                    m.insert(k.clone(), v.clone());
                }
                m
            }
        }
    }

    /// Returns the 'inherited' part of the environment, i.e., those environment variables that
    /// are inherited from the client environment and therefore have no fixed value here. This
    /// should only be used for testing and to compute the cache keys of actions. Use
    /// [`Self::resolve`] instead to get the complete environment.
    // Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:108 ActionEnvironment.getInheritedEnv
    pub fn get_inherited_env(&self) -> IndexSet<String> {
        match self {
            ActionEnvironment::Empty => IndexSet::new(),
            ActionEnvironment::Simple { inherited_env, .. } => inherited_env.clone(),
            ActionEnvironment::Compound { base, .. } => base.get_inherited_env(),
        }
    }

    /// Returns an upper bound on the combined size of the fixed and inherited environments. A call
    /// to [`Self::resolve`] may add less entries than this number if environment variables are
    /// contained in both the fixed and the inherited environment.
    // Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:115 ActionEnvironment.estimatedSize
    pub fn estimated_size(&self) -> usize {
        match self {
            ActionEnvironment::Empty => 0,
            ActionEnvironment::Simple {
                fixed_env,
                inherited_env,
            } => fixed_env.len() + inherited_env.len(),
            ActionEnvironment::Compound { base, fixed_vars } => {
                base.estimated_size() + fixed_vars.len()
            }
        }
    }

    /// Resolves the action environment and adds the resulting entries to the given `result` map,
    /// by looking up any inherited env variables in the given `client_env`.
    ///
    /// We pass in a map to mutate to avoid creating and merging intermediate maps.
    // Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:123 ActionEnvironment.resolve
    pub fn resolve(
        &self,
        result: &mut IndexMap<String, String>,
        client_env: &IndexMap<String, String>,
    ) {
        for (k, v) in self.get_fixed_env() {
            result.insert(k, v);
        }
        for var in self.get_inherited_env() {
            if let Some(value) = client_env.get(&var) {
                result.insert(var, value.clone());
            }
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:134 ActionEnvironment.addTo
    pub fn add_to(&self, f: &mut Fingerprint) {
        let fixed = self.get_fixed_env();
        f.add_string_map(fixed.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let inherited = self.get_inherited_env();
        f.add_strings(inherited.iter());
    }

    /// Returns a copy of the environment with the given fixed variables added to it,
    /// *overwriting* any existing occurrences of those variables.
    // Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:143 ActionEnvironment.withAdditionalFixedVariables
    pub fn with_additional_fixed_variables(
        self: &Arc<Self>,
        fixed_vars: IndexMap<String, String>,
    ) -> Arc<ActionEnvironment> {
        if fixed_vars.is_empty() {
            return Arc::clone(self);
        }
        if **self == ActionEnvironment::Empty {
            return Arc::new(ActionEnvironment::Simple {
                fixed_env: fixed_vars,
                inherited_env: IndexSet::new(),
            });
        }
        Arc::new(ActionEnvironment::Compound {
            base: Arc::clone(self),
            fixed_vars,
        })
    }
}
