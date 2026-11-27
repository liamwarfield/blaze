//! REPOSITORY_ENVIRONMENT_VARIABLE: the effective value of an environment variable for repository
//! rules and module extensions.
//!
//! Java: com/google/devtools/build/lib/skyframe/RepoEnvironmentFunction.java.
//!
//! Skyframe function that provides the effective value for an environment variable in the context
//! of repository rules and module extensions. This will be the value from the repo environment as
//! provided by `CommandEnvironment#getRepoEnv()`.
//!
//! PORT: Java reads the environment from an `AtomicReference` whose entries are injected per
//! variable for incremental efficiency; Blaze evaluates one command per evaluator, so the function
//! is registered with the command's repo environment.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use blaze_bzlmod::skyframe::precomputed::EnvironmentVariableValue;
use blaze_skyframe::Environment;
use blaze_skyframe::Interrupted;
use blaze_skyframe::Key;
use blaze_skyframe::SkyFunctionName;
use blaze_skyframe::SkyKey;
use blaze_skyframe::{ComputeError, FunctionHermeticity};
use indexmap::IndexMap;
use indexmap::IndexSet;

// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:30 SkyFunctions.REPOSITORY_ENVIRONMENT_VARIABLE
pub const REPOSITORY_ENVIRONMENT_VARIABLE: SkyFunctionName =
    SkyFunctionName("REPOSITORY_ENVIRONMENT_VARIABLE");

/// The SkyKey for the environment variable named by the argument. `Debug` is
/// `REPOSITORY_ENVIRONMENT_VARIABLE:<name>` (Java `AbstractSkyKey.toString`).
// Java: com/google/devtools/build/lib/skyframe/RepoEnvironmentFunction.java:66 RepoEnvironmentFunction.Key
#[derive(Clone, PartialEq, Eq, Hash)]
// PORT: skipped, a struct key (Key::FUNCTION; no interning): RepoEnvironmentFunction.Key.create, RepoEnvironmentFunction.Key.intern, RepoEnvironmentFunction.Key.functionName, RepoEnvironmentFunction.Key.getSkyKeyInterner
pub struct RepoEnvironmentKey(pub String);

impl RepoEnvironmentKey {
    /// Returns the SkyKey to invoke this function for the environment variable `variable`.
    // Java: com/google/devtools/build/lib/skyframe/RepoEnvironmentFunction.java:60 RepoEnvironmentFunction.key
    pub fn key(variable: &str) -> RepoEnvironmentKey {
        RepoEnvironmentKey(variable.to_owned())
    }

    pub fn argument(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RepoEnvironmentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", REPOSITORY_ENVIRONMENT_VARIABLE, self.0)
    }
}

impl Key for RepoEnvironmentKey {
    type Value = EnvironmentVariableValue;
    const FUNCTION: SkyFunctionName = REPOSITORY_ENVIRONMENT_VARIABLE;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:31 SkyFunctions.REPOSITORY_ENVIRONMENT_VARIABLE (createNonHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
    /// Java: `RepoEnvironmentFunction` returns an `EnvironmentVariableValue`, a record of one nullable string.
    // Java: com/google/devtools/build/lib/skyframe/EnvironmentVariableValue.java:28 EnvironmentVariableValue
    fn values_equal(old: &EnvironmentVariableValue, new: &EnvironmentVariableValue) -> bool {
        old == new
    }
}

// PORT: skipped, the function reads the command's repo environment from the RepoServices it is given (no constructor state): RepoEnvironmentFunction.<init>
// Java: com/google/devtools/build/lib/skyframe/RepoEnvironmentFunction.java:55 RepoEnvironmentFunction.compute
pub fn compute(
    repo_env: &Arc<IndexMap<String, String>>,
    key: &RepoEnvironmentKey,
    _env: &mut Environment<'_>,
) -> Result<Option<EnvironmentVariableValue>, ComputeError> {
    Ok(Some(EnvironmentVariableValue(
        repo_env.get(&key.0).cloned(),
    )))
}

/// Returns a map of environment variable key => values, getting them from Skyframe. Returns
/// `None` if and only if some dependencies from Skyframe still need to be resolved.
// Java: com/google/devtools/build/lib/skyframe/RepoEnvironmentFunction.java:99 RepoEnvironmentFunction.getEnvironmentView
pub fn get_environment_view(
    env: &mut Environment<'_>,
    keys: &IndexSet<String>,
) -> Result<Option<BTreeMap<String, Option<String>>>, Interrupted> {
    let sky_keys: Vec<RepoEnvironmentKey> =
        keys.iter().map(|k| RepoEnvironmentKey::key(k)).collect();
    let dyn_keys: Vec<SkyKey> = sky_keys.iter().cloned().map(SkyKey::new).collect();
    let values = env.get_values_and_exceptions(&dyn_keys)?;
    if env.values_missing() {
        return Ok(None);
    }

    let mut result = BTreeMap::new();
    for key in &sky_keys {
        let Some(value) = values.get_typed(key) else {
            return Ok(None);
        };
        result.insert(key.argument().to_owned(), value.0.clone());
    }
    Ok(Some(result))
}
