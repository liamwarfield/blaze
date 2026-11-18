//! Port of `lib/exec/SpawnStrategyPolicy.java`.

use blaze_protos::blaze::strategy_policy::MnemonicPolicy;
use indexmap::{IndexMap, IndexSet};

/// Policy for filtering spawn strategies.
// Java: com/google/devtools/build/lib/exec/SpawnStrategyPolicy.java:26 SpawnStrategyPolicy
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnStrategyPolicy {
    /// `AllowAllStrategiesPolicy`: allows all strategies - effectively a no-op strategy.
    AllowAll,
    /// `SpawnStrategyPolicyImpl`: enforces a real strategy policy based on provided config.
    Impl {
        per_mnemonic_allow_list: IndexMap<String, IndexSet<String>>,
        default_allow_list: IndexSet<String>,
    },
}

impl SpawnStrategyPolicy {
    /// Creates new policy from proto descriptor. Empty proto policy implies everything allowed.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyPolicy.java:35 SpawnStrategyPolicy.create
    pub fn create(policy: &MnemonicPolicy) -> SpawnStrategyPolicy {
        if *policy == MnemonicPolicy::default() {
            return SpawnStrategyPolicy::AllowAll;
        }

        // ImmutableMap.Builder + buildKeepingLast: first position, last value.
        let mut per_mnemonic_allow_list: IndexMap<String, IndexSet<String>> = IndexMap::new();
        for strategies_for_mnemonic in &policy.strategy_allowlist {
            per_mnemonic_allow_list.insert(
                strategies_for_mnemonic.mnemonic.clone().unwrap_or_default(),
                strategies_for_mnemonic.strategy.iter().cloned().collect(),
            );
        }
        SpawnStrategyPolicy::Impl {
            per_mnemonic_allow_list,
            default_allow_list: policy.default_allowlist.iter().cloned().collect(),
        }
    }

    /// Returns result of applying policy to per-mnemonic strategies.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyPolicy.java:58 SpawnStrategyPolicy.apply
    pub fn apply_for_mnemonic(&self, mnemonic: &str, strategies: &[String]) -> Vec<String> {
        match self {
            SpawnStrategyPolicy::AllowAll => strategies.to_vec(),
            // Java: com/google/devtools/build/lib/exec/SpawnStrategyPolicy.java:82 SpawnStrategyPolicyImpl.apply
            SpawnStrategyPolicy::Impl {
                per_mnemonic_allow_list,
                default_allow_list,
            } => {
                let allow_list = per_mnemonic_allow_list
                    .get(mnemonic)
                    .unwrap_or(default_allow_list);
                strategies
                    .iter()
                    .filter(|s| allow_list.contains(*s))
                    .cloned()
                    .collect()
            }
        }
    }

    /// Returns result of applying default policy to strategies.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyPolicy.java:63 SpawnStrategyPolicy.apply
    pub fn apply(&self, strategies: &[String]) -> Vec<String> {
        match self {
            SpawnStrategyPolicy::AllowAll => strategies.to_vec(),
            // Java: com/google/devtools/build/lib/exec/SpawnStrategyPolicy.java:89 SpawnStrategyPolicyImpl.apply
            SpawnStrategyPolicy::Impl {
                default_allow_list, ..
            } => strategies
                .iter()
                .filter(|s| default_allow_list.contains(*s))
                .cloned()
                .collect(),
        }
    }
}
