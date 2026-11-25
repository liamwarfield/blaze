//! BAZEL_FETCH_ALL: gather and fetch all the repositories from MODULE.bazel resolution and
//! extensions evaluation (`bazel fetch`, `bazel fetch --all`, `bazel fetch --configure`).
//!
//! Java: com/google/devtools/build/lib/bazel/bzlmod/BazelFetchAllFunction.java and
//! BazelFetchAllValue.java. PORT: the key and value are here with the function (not in
//! blaze_bzlmod, the crate of the Java package), because the function needs
//! `RepoDefinitionValue` (this crate) and the value is only read by the fetch command.

use std::fmt;

use blaze_bzlmod::skyframe::BazelDepGraphKey;
use blaze_bzlmod::skyframe::keys::SingleExtensionValue;
use blaze_cmdline::RepositoryName;
use blaze_sky_loading::repos::{RepositoryDirectoryKey, RepositoryDirectoryValue};
use blaze_skyframe::{
    ComputeError, Environment, FunctionHermeticity, Key, SkyFunctionName, SkyKey,
};

use crate::repository::repo_definition::{RepoDefinitionKey, RepoDefinitionValue};

// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:173 SkyFunctions.BAZEL_FETCH_ALL
pub const BAZEL_FETCH_ALL: SkyFunctionName = SkyFunctionName("BAZEL_FETCH_ALL");

/// Empty result of running Bazel fetch all dependencies, to indicate that all repos have been
/// fetched successfully.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelFetchAllValue.java:29 BazelFetchAllValue
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BazelFetchAllValue {
    pub repos_to_vendor: Vec<RepositoryName>,
}

impl BazelFetchAllValue {
    /// Creates a key from the given repository name.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelFetchAllValue.java:35 BazelFetchAllValue.key
    pub fn key(configure_enabled: bool) -> BazelFetchAllKey {
        BazelFetchAllKey(configure_enabled)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelFetchAllValue.java:39 BazelFetchAllValue.create
    pub fn create(repos_to_vendor: Vec<RepositoryName>) -> BazelFetchAllValue {
        BazelFetchAllValue { repos_to_vendor }
    }
}

/// Key type for BazelFetchAllValue: whether only `configure` repos are fetched. `Debug` prints
/// Java's `AbstractSkyKey.toString`, `BAZEL_FETCH_ALL:<arg>`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelFetchAllValue.java:46 BazelFetchAllValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct BazelFetchAllKey(pub bool);

impl fmt::Debug for BazelFetchAllKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", BAZEL_FETCH_ALL, self.0)
    }
}

impl Key for BazelFetchAllKey {
    type Value = BazelFetchAllValue;
    const FUNCTION: SkyFunctionName = BAZEL_FETCH_ALL;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:174 SkyFunctions.BAZEL_FETCH_ALL (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: a record of the list of repositories.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelFetchAllValue.java:34 BazelFetchAllValue
    fn values_equal(old: &BazelFetchAllValue, new: &BazelFetchAllValue) -> bool {
        old == new
    }
}

/// Gather and fetch all the repositories from MODULE.bazel resolution and extensions evaluation.
/// If this is fetch configure, only configure repos will be fetched and returned.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelFetchAllFunction.java:46 BazelFetchAllFunction.compute
pub fn compute(
    key: &BazelFetchAllKey,
    env: &mut Environment<'_>,
) -> Result<Option<BazelFetchAllValue>, ComputeError> {
    // Collect all the repos we want to fetch here

    // 1. Run resolution and collect the dependency graph repos except for main
    let Some(dep_graph_value) = env.get_value(BazelDepGraphKey)? else {
        return Ok(None);
    };
    let mut repos_to_fetch: Vec<RepositoryName> = dep_graph_value
        .canonical_repo_name_lookup
        .keys()
        .filter(|repo| !repo.is_main())
        .cloned()
        .collect();

    // 2. Run every extension found in the modules & collect its generated repos
    // ImmutableSet of the keys, in the table's row order.
    let mut single_extension_keys = Vec::new();
    for id in dep_graph_value.extension_usages_table.row_key_set() {
        let key = SingleExtensionValue::key(id.clone());
        if !single_extension_keys.contains(&key) {
            single_extension_keys.push(key);
        }
    }
    let single_extension_values =
        env.get_values_and_exceptions(&sky_keys(&single_extension_keys))?;
    for single_extension_key in &single_extension_keys {
        let Some(single_extension_value) = single_extension_values.get_typed(single_extension_key)
        else {
            return Ok(None);
        };
        repos_to_fetch.extend(
            single_extension_value
                .canonical_repo_name_to_internal_names
                .keys()
                .cloned(),
        );
    }

    // 3. If this is fetch configure, get repo rules and only collect repos marked as configure
    let fetch_configure = key.0;
    if fetch_configure {
        let mut repo_definition_keys = Vec::new();
        for repo in &repos_to_fetch {
            let key = RepoDefinitionKey(repo.clone());
            if !repo_definition_keys.contains(&key) {
                repo_definition_keys.push(key);
            }
        }
        repos_to_fetch.clear(); // empty this list to only add configured repos
        let repo_definition_values =
            env.get_values_and_exceptions(&sky_keys(&repo_definition_keys))?;
        for repo_rule_key in &repo_definition_keys {
            let Some(repo_definition_value) = repo_definition_values.get_typed(repo_rule_key)
            else {
                return Ok(None);
            };
            if let RepoDefinitionValue::Found(repo_definition) = &*repo_definition_value
                && repo_definition.repo_rule.configure
            {
                repos_to_fetch.push(repo_rule_key.0.clone());
            }
        }
    }

    // 4. Fetch all the collected repos
    let mut should_vendor: Vec<RepositoryName> = Vec::new();
    let mut repo_delegator_keys = Vec::new();
    for repo in &repos_to_fetch {
        let key = RepositoryDirectoryKey::key(repo.clone());
        if !repo_delegator_keys.contains(&key) {
            repo_delegator_keys.push(key);
        }
    }
    let repo_dir_values = env.get_values_and_exceptions(&sky_keys(&repo_delegator_keys))?;
    for repo_delegator_key in &repo_delegator_keys {
        let Some(repo_dir_value) = repo_dir_values.get_typed(repo_delegator_key) else {
            return Ok(None);
        };
        if let RepositoryDirectoryValue::Success {
            exclude_from_vendoring: false,
            ..
        } = &*repo_dir_value
        {
            should_vendor.push(repo_delegator_key.0.clone());
        }
    }

    Ok(Some(BazelFetchAllValue::create(should_vendor)))
}

fn sky_keys<K: Key + Clone>(keys: &[K]) -> Vec<SkyKey> {
    keys.iter().cloned().map(SkyKey::new).collect()
}
