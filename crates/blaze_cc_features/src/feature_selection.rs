//! Port of `com.google.devtools.build.lib.rules.cpp.FeatureSelection`.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use crate::cc_toolchain_features::{
    ActionConfig, CrosstoolSelectable, FeatureConfiguration, colliding_provides_error,
};
use crate::errors::CollidingProvidesException;

/// Implements the feature selection algorithm.
///
/// Feature selection is done by first enabling all features reachable by an 'implies' edge, and
/// then iteratively pruning features that have unmet requirements.
///
/// Selectables are identified by their index in the toolchain's selectable list.
// Java: com/google/devtools/build/lib/rules/cpp/FeatureSelection.java:43 FeatureSelection
pub(crate) struct FeatureSelection<'a> {
    /// The selectables Bazel would like to enable; either because they are supported and
    /// generally useful, or because the user required them (for example through the command
    /// line).
    requested_selectables: IndexSet<usize>,

    /// All features requested by the user regardless of whether they exist in CROSSTOOLs or not.
    requested_features: IndexSet<String>,

    /// The currently enabled selectable; during feature selection, we first put all selectables
    /// reachable via an 'implies' edge into the enabled selectable set, and than prune that set
    /// from selectables that have unmet requirements.
    enabled: HashSet<usize>,

    /// All features and action configs in the order in which they were specified in the
    /// configuration.
    selectables: &'a [CrosstoolSelectable],

    /// Maps an action's name to the ActionConfig.
    action_configs_by_action_name: Arc<IndexMap<String, Arc<ActionConfig>>>,

    /// Maps from a selectable to a set of all the selectables it has a direct 'implies' edge to.
    implies: &'a [Vec<usize>],

    /// Maps from a selectable to all features that have an direct 'implies' edge to this
    /// selectable.
    implied_by: &'a [Vec<usize>],

    /// Maps from a selectable to a set of selectable sets.
    requires: &'a [Vec<IndexSet<usize>>],

    /// Maps from a string to the set of selectables that 'provide' it.
    provides: &'a IndexMap<String, Vec<usize>>,

    /// Maps from a selectable to all selectables that have a requirement referencing it.
    required_by: &'a [Vec<usize>],

    /// Location of the cc_toolchain in use.
    cc_toolchain_path: PathFragment,
}

impl<'a> FeatureSelection<'a> {
    // Java: com/google/devtools/build/lib/rules/cpp/FeatureSelection.java:112 FeatureSelection.<init>
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        requested_features: IndexSet<String>,
        selectables_by_name: &IndexMap<String, usize>,
        selectables: &'a [CrosstoolSelectable],
        provides: &'a IndexMap<String, Vec<usize>>,
        implies: &'a [Vec<usize>],
        implied_by: &'a [Vec<usize>],
        requires: &'a [Vec<IndexSet<usize>>],
        required_by: &'a [Vec<usize>],
        action_configs_by_action_name: Arc<IndexMap<String, Arc<ActionConfig>>>,
        cc_toolchain_path: PathFragment,
    ) -> FeatureSelection<'a> {
        let mut requested_selectables = IndexSet::new();
        for name in &requested_features {
            if let Some(&i) = selectables_by_name.get(name) {
                requested_selectables.insert(i);
            }
        }
        FeatureSelection {
            requested_selectables,
            requested_features,
            enabled: HashSet::new(),
            selectables,
            action_configs_by_action_name,
            implies,
            implied_by,
            requires,
            provides,
            required_by,
            cc_toolchain_path,
        }
    }

    /// Returns a [`FeatureConfiguration`] that reflects the set of activated features and action
    /// configs.
    // Java: com/google/devtools/build/lib/rules/cpp/FeatureSelection.java:145 FeatureSelection.run
    pub(crate) fn run(mut self) -> Result<FeatureConfiguration, CollidingProvidesException> {
        let requested: Vec<usize> = self.requested_selectables.iter().copied().collect();
        for selectable in requested {
            self.enable_all_implied_by(selectable);
        }

        self.disable_unsupported_activatables();
        let mut enabled_activatables_in_order: Vec<usize> = Vec::new();
        for i in 0..self.selectables.len() {
            if self.enabled.contains(&i) {
                enabled_activatables_in_order.push(i);
            }
        }

        let enabled_features_in_order = enabled_activatables_in_order
            .iter()
            .filter_map(|&i| match &self.selectables[i] {
                CrosstoolSelectable::Feature(f) => Some(f.clone()),
                CrosstoolSelectable::ActionConfig(_) => None,
            })
            .collect();
        let enabled_action_configs_in_order =
            enabled_activatables_in_order
                .iter()
                .filter_map(|&i| match &self.selectables[i] {
                    CrosstoolSelectable::ActionConfig(a) => Some(a.clone()),
                    CrosstoolSelectable::Feature(_) => None,
                });

        // provides.keys(): each key repeated once per providing selectable.
        for (provided, providers) in self.provides {
            for _ in 0..providers.len() {
                let mut conflicts: Vec<&str> = Vec::new();
                for &selectable_providing_string in providers {
                    if enabled_activatables_in_order.contains(&selectable_providing_string) {
                        conflicts.push(self.selectables[selectable_providing_string].get_name());
                    }
                }

                if conflicts.len() > 1 {
                    return Err(CollidingProvidesException {
                        message: colliding_provides_error(provided, &conflicts.join(" ")),
                    });
                }
            }
        }

        let mut enabled_action_config_names = IndexSet::new();
        for action_config in enabled_action_configs_in_order {
            enabled_action_config_names.insert(action_config.get_action_name().to_string());
        }

        Ok(FeatureConfiguration::new(
            self.requested_features,
            enabled_features_in_order,
            enabled_action_config_names,
            self.action_configs_by_action_name,
            self.cc_toolchain_path,
        ))
    }

    /// Transitively and unconditionally enable all selectables implied by the given selectable
    /// and the selectable itself to the enabled selectable set.
    // Java: com/google/devtools/build/lib/rules/cpp/FeatureSelection.java:204 FeatureSelection.enableAllImpliedBy
    fn enable_all_implied_by(&mut self, selectable: usize) {
        if self.enabled.contains(&selectable) {
            return;
        }
        self.enabled.insert(selectable);
        for &implied in &self.implies[selectable] {
            self.enable_all_implied_by(implied);
        }
    }

    /// Remove all unsupported features from the enabled feature set.
    // Java: com/google/devtools/build/lib/rules/cpp/FeatureSelection.java:215 FeatureSelection.disableUnsupportedActivatables
    fn disable_unsupported_activatables(&mut self) {
        // PORT: Java seeds the queue in HashSet order. Every predicate in isSatisfied is monotone
        // in the enabled set, so the result (the greatest fixpoint) does not depend on the order;
        // index order is used.
        let mut seed: Vec<usize> = self.enabled.iter().copied().collect();
        seed.sort_unstable();
        let mut check: VecDeque<usize> = seed.into();
        while let Some(selectable) = check.pop_front() {
            self.check_activatable(selectable);
        }
    }

    /// Check if the given selectable is still satisfied within the set of currently enabled
    /// selectables.
    ///
    /// If it is not, remove the selectable from the set of enabled selectables, and re-check all
    /// selectables that may now also become disabled.
    // Java: com/google/devtools/build/lib/rules/cpp/FeatureSelection.java:229 FeatureSelection.checkActivatable
    fn check_activatable(&mut self, selectable: usize) {
        if !self.enabled.contains(&selectable) || self.is_satisfied(selectable) {
            return;
        }
        self.enabled.remove(&selectable);

        // Once we disable a selectable, we have to re-check all selectables that can be affected
        // by that removal.
        // 1. A selectable that implied the current selectable is now going to be disabled.
        for &implies_current in &self.implied_by[selectable] {
            self.check_activatable(implies_current);
        }
        // 2. A selectable that required the current selectable may now be disabled, depending on
        // whether the requirement was optional.
        for &requires_current in &self.required_by[selectable] {
            self.check_activatable(requires_current);
        }
        // 3. A selectable that this selectable implied may now be disabled if no other
        // selectables also implies it.
        for &implied in &self.implies[selectable] {
            self.check_activatable(implied);
        }
    }

    /// Returns whether all requirements of the selectable are met in the set of currently
    /// enabled selectables.
    // Java: com/google/devtools/build/lib/rules/cpp/FeatureSelection.java:257 FeatureSelection.isSatisfied
    fn is_satisfied(&self, selectable: usize) -> bool {
        (self.requested_selectables.contains(&selectable)
            || self.is_implied_by_enabled_activatable(selectable))
            && self.all_implications_enabled(selectable)
            && self.all_requirements_met(selectable)
    }

    /// Returns whether a currently enabled selectable implies the given selectable.
    // Java: com/google/devtools/build/lib/rules/cpp/FeatureSelection.java:264 FeatureSelection.isImpliedByEnabledActivatable
    fn is_implied_by_enabled_activatable(&self, selectable: usize) -> bool {
        self.implied_by[selectable]
            .iter()
            .any(|i| self.enabled.contains(i))
    }

    /// Returns whether all implications of the given feature are enabled.
    // Java: com/google/devtools/build/lib/rules/cpp/FeatureSelection.java:269 FeatureSelection.allImplicationsEnabled
    fn all_implications_enabled(&self, selectable: usize) -> bool {
        for implied in &self.implies[selectable] {
            if !self.enabled.contains(implied) {
                return false;
            }
        }
        true
    }

    /// Returns whether all requirements are enabled.
    ///
    /// This implies that for any of the selectable sets all of the specified selectable are
    /// enabled.
    // Java: com/google/devtools/build/lib/rules/cpp/FeatureSelection.java:283 FeatureSelection.allRequirementsMet
    fn all_requirements_met(&self, feature: usize) -> bool {
        if self.requires[feature].is_empty() {
            return true;
        }
        for requires_all_of in &self.requires[feature] {
            let mut requirement_met = true;
            for required in requires_all_of {
                if !self.enabled.contains(required) {
                    requirement_met = false;
                    break;
                }
            }
            if requirement_met {
                return true;
            }
        }
        false
    }
}
