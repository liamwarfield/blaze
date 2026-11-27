//! Ports of `skyframe/ActionLookupConflictFindingFunction.java`,
//! `skyframe/ActionLookupConflictFindingValue.java` and
//! `skyframe/TopLevelActionLookupConflictFindingFunction.java`: after action conflicts were
//! found, which top-level keys are transitively free of them.

use std::fmt;
use std::sync::Arc;

use blaze_actions::analysis::workspace_status_action::BUILD_INFO_FUNCTION_NAME;
use blaze_actions::{ActionLookupKeyRef, ActionRef, Artifact};
use blaze_analysis::analysis_services::WorkspaceStatusKey;
use blaze_analysis::aspect_key_creator::{
    ASPECT, AspectKey, TOP_LEVEL_ASPECTS, TopLevelAspectsKey,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::{CONFIGURED_TARGET, ConfiguredTargetKey};
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_skyframe::precomputed::Precomputed;
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, FunctionHermeticity, Key, SkyFunction,
    SkyFunctionException, SkyFunctionName, SkyKey, SkyValue, Transience,
};
use indexmap::{IndexMap, IndexSet};

use crate::action_conflict_exception::ActionConflicts;

/// `SkyFunctions.ACTION_LOOKUP_CONFLICT_FINDING`.
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:97 SkyFunctions.ACTION_LOOKUP_CONFLICT_FINDING
pub const ACTION_LOOKUP_CONFLICT_FINDING: SkyFunctionName =
    SkyFunctionName("ACTION_LOOKUP_CONFLICT_DETECTION");
/// `SkyFunctions.TOP_LEVEL_ACTION_LOOKUP_CONFLICT_FINDING`.
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:99 SkyFunctions.TOP_LEVEL_ACTION_LOOKUP_CONFLICT_FINDING
pub const TOP_LEVEL_ACTION_LOOKUP_CONFLICT_FINDING: SkyFunctionName =
    SkyFunctionName("TOP_LEVEL_ACTION_LOOKUP_CONFLICT_DETECTION");

/// `ArtifactConflictFinder.ACTION_CONFLICTS`: the conflicts the conflict-finding functions read,
/// injected by `SkyframeExecutor.filterActionConflictsForConfiguredTargetsAndAspects`
/// (`ACTION_CONFLICTS.set(injectable(), actionConflicts)`). Its node is a dependency of every
/// `ACTION_LOOKUP_CONFLICT_DETECTION` node, so a new set of conflicts invalidates the verdicts a
/// kept graph still holds (review round 2, S2-4).
// Java: com/google/devtools/build/lib/skyframe/ArtifactConflictFinder.java:43 ArtifactConflictFinder.ACTION_CONFLICTS
pub static ACTION_CONFLICTS: Precomputed<ActionConflictsValue> =
    Precomputed::new("action_conflicts");

/// The value of [`ACTION_CONFLICTS`]: Java's
/// `ImmutableMap<ActionAnalysisMetadata, ActionConflictException>`.
#[derive(Clone, Debug)]
pub struct ActionConflictsValue(pub Arc<ActionConflicts>);

impl PartialEq for ActionConflictsValue {
    /// `ImmutableMap.equals` (`Maps.equalsImpl`): the same keys (actions by identity) with equal
    /// values, in any order.
    ///
    /// PORT: Java's `ActionConflictException` does not override `equals` (identity); the port's
    /// exceptions are copied, so two are equal when their message (which names the artifact and
    /// both actions) is.
    // Java: com/google/common/collect/Maps.java equalsImpl (Guava)
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || self.0.len() == other.0.len()
                && self.0.entries().all(|(action, exception)| {
                    other
                        .0
                        .get(action)
                        .is_some_and(|o| o.get_message() == exception.get_message())
                })
    }
}

/// A marker for an `ActionLookupValue` which is known to be transitively error-free from action
/// conflict issues.
// Java: com/google/devtools/build/lib/skyframe/ActionLookupConflictFindingValue.java:31 ActionLookupConflictFindingValue
#[derive(Clone, Debug, Default)]
pub struct ActionLookupConflictFindingValue;

/// `ActionLookupConflictFindingValue.Key`.
// Java: com/google/devtools/build/lib/skyframe/ActionLookupConflictFindingValue.java:50 ActionLookupConflictFindingValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ActionLookupConflictFindingKey(pub ActionLookupKeyRef);

impl ActionLookupConflictFindingKey {
    // Java: com/google/devtools/build/lib/skyframe/ActionLookupConflictFindingValue.java:37 ActionLookupConflictFindingValue.key
    pub fn key(lookup_key: ActionLookupKeyRef) -> Self {
        ActionLookupConflictFindingKey(lookup_key)
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionLookupConflictFindingValue.java:41 ActionLookupConflictFindingValue.key
    pub fn key_of_artifact(artifact: &Artifact) -> Self {
        blaze_util::check_argument!(!artifact.is_source_artifact(), "{}", artifact);
        Self::key(
            artifact
                .get_generating_action_key()
                .get_action_lookup_key()
                .clone(),
        )
    }

    /// `argument()`.
    pub fn argument(&self) -> &ActionLookupKeyRef {
        &self.0
    }
}

impl fmt::Debug for ActionLookupConflictFindingKey {
    // Java: com/google/devtools/build/skyframe/AbstractSkyKey.java:59 AbstractSkyKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", ACTION_LOOKUP_CONFLICT_FINDING, self.0)
    }
}

impl Key for ActionLookupConflictFindingKey {
    type Value = ActionLookupConflictFindingValue;
    const FUNCTION: SkyFunctionName = ACTION_LOOKUP_CONFLICT_FINDING;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:98 SkyFunctions.ACTION_LOOKUP_CONFLICT_FINDING (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: the function returns the singleton `ActionLookupConflictFindingValue.INSTANCE`,
    /// equal to itself.
    // Java: com/google/devtools/build/lib/skyframe/ActionLookupConflictFindingValue.java:34 ActionLookupConflictFindingValue.INSTANCE
    fn values_equal(
        _old: &ActionLookupConflictFindingValue,
        _new: &ActionLookupConflictFindingValue,
    ) -> bool {
        true
    }
}

/// `TopLevelActionLookupConflictFindingFunction.Key`: a top-level key and the output groups
/// whose artifacts are checked.
///
/// PORT: of `TopLevelArtifactContext` only `outputGroups` is kept (see
/// `execution_seam::top_level_output_groups`).
// Java: com/google/devtools/build/lib/skyframe/TopLevelActionLookupConflictFindingFunction.java:70 TopLevelActionLookupConflictFindingFunction.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TopLevelActionLookupConflictFindingKey {
    /// `actionLookupKey()`.
    pub action_lookup_key: ActionLookupKeyRef,
    /// `topLevelArtifactContext().outputGroups()`.
    pub output_groups: Vec<String>,
}

impl TopLevelActionLookupConflictFindingKey {
    // Java: com/google/devtools/build/lib/skyframe/TopLevelActionLookupConflictFindingFunction.java:72 TopLevelActionLookupConflictFindingFunction.Key.create
    pub fn create(action_lookup_key: ActionLookupKeyRef, output_groups: &[String]) -> Self {
        TopLevelActionLookupConflictFindingKey {
            action_lookup_key,
            output_groups: output_groups.to_vec(),
        }
    }
}

impl fmt::Debug for TopLevelActionLookupConflictFindingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Key{{actionLookupKey={}, topLevelArtifactContext=TopLevelArtifactContext{{outputGroups={:?}}}}}",
            self.action_lookup_key, self.output_groups
        )
    }
}

impl Key for TopLevelActionLookupConflictFindingKey {
    type Value = ActionLookupConflictFindingValue;
    const FUNCTION: SkyFunctionName = TOP_LEVEL_ACTION_LOOKUP_CONFLICT_FINDING;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:100 SkyFunctions.TOP_LEVEL_ACTION_LOOKUP_CONFLICT_FINDING (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: the function returns the singleton `ActionLookupConflictFindingValue.INSTANCE`,
    /// equal to itself.
    // Java: com/google/devtools/build/lib/skyframe/ActionLookupConflictFindingValue.java:34 ActionLookupConflictFindingValue.INSTANCE
    fn values_equal(
        _old: &ActionLookupConflictFindingValue,
        _new: &ActionLookupConflictFindingValue,
    ) -> bool {
        true
    }
}

/// `TopLevelActionLookupConflictFindingFunction.keys`.
// Java: com/google/devtools/build/lib/skyframe/TopLevelActionLookupConflictFindingFunction.java:64 TopLevelActionLookupConflictFindingFunction.keys
pub fn keys(
    keys: &[ActionLookupKeyRef],
    output_groups: &[String],
) -> Vec<TopLevelActionLookupConflictFindingKey> {
    keys.iter()
        .map(|k| TopLevelActionLookupConflictFindingKey::create(k.clone(), output_groups))
        .collect()
}

/// The actions of the `ActionLookupValue` of `lookup_key` (`env.getValue(lookupKey)`), `None`
/// when the value is missing.
///
/// PORT: the `ActionLookupKey`s that are graph nodes are `ConfiguredTargetKey`, `AspectKey`,
/// `TopLevelAspectsKey` and the workspace status key; the others (action templates, the coverage
/// report) are not ported.
fn get_action_lookup_value_actions(
    lookup_key: &ActionLookupKeyRef,
    env: &mut Environment<'_>,
) -> Result<Option<Vec<ActionRef>>, ComputeError> {
    match lookup_key.function_name() {
        name if name == CONFIGURED_TARGET.0 => {
            let ct_key = lookup_key
                .as_any()
                .downcast_ref::<ConfiguredTargetKey>()
                .unwrap_or_else(|| {
                    panic!("ClassCastException: {lookup_key} is not a ConfiguredTargetKey")
                });
            let Some(value) = env.get_value(ct_key.clone())? else {
                return Ok(None);
            };
            Ok(Some(value.get_actions().to_vec()))
        }
        BUILD_INFO_FUNCTION_NAME => {
            let Some(value) = env.get_value(WorkspaceStatusKey)? else {
                return Ok(None);
            };
            Ok(Some(value.actions.clone()))
        }
        // Java: AspectValue extends BasicActionLookupValue.
        name if name == ASPECT.0 => {
            let aspect_key = lookup_key
                .as_any()
                .downcast_ref::<AspectKey>()
                .unwrap_or_else(|| panic!("ClassCastException: {lookup_key} is not an AspectKey"));
            let Some(value) = env.get_value(aspect_key.clone())? else {
                return Ok(None);
            };
            Ok(Some(value.get_actions().to_vec()))
        }
        // Java: TopLevelAspectsValue implements ActionLookupValue (no actions).
        name if name == TOP_LEVEL_ASPECTS.0 => {
            let top_level_key = lookup_key
                .as_any()
                .downcast_ref::<TopLevelAspectsKey>()
                .unwrap_or_else(|| {
                    panic!("ClassCastException: {lookup_key} is not a TopLevelAspectsKey")
                });
            let Some(value) = env.get_value(top_level_key.clone())? else {
                return Ok(None);
            };
            Ok(Some(value.get_actions().to_vec()))
        }
        other => blaze_util::unported!(
            "ActionLookupConflictFindingFunction.compute",
            "ActionLookupValue of a {other} key ({lookup_key})"
        ),
    }
}

/// Check all transitive actions of an `ActionLookupValue` for action conflicts.
// Java: com/google/devtools/build/lib/skyframe/ActionLookupConflictFindingFunction.java:38 ActionLookupConflictFindingFunction
pub struct ActionLookupConflictFindingFunction;

impl SkyFunction for ActionLookupConflictFindingFunction {
    // Java: com/google/devtools/build/lib/skyframe/ActionLookupConflictFindingFunction.java:43 ActionLookupConflictFindingFunction.compute
    fn compute(
        &self,
        sky_key: &SkyKey,
        env: &mut Environment<'_>,
    ) -> blaze_skyframe::ComputeResult {
        let action_conflicts: Option<Arc<ActionConflictsValue>> = ACTION_CONFLICTS.get(env)?;
        let key = sky_key
            .downcast::<ActionLookupConflictFindingKey>()
            .expect("ACTION_LOOKUP_CONFLICT_DETECTION key");
        let lookup_key = key.argument();
        let al_value_actions = get_action_lookup_value_actions(lookup_key, env)?;
        if env.values_missing() {
            // PORT: skipped, BugReport.sendNonFatalBugReport ("Unexpected missing action lookup
            // value during action conflict finding") (bug reports).
            return Ok(None);
        }
        let al_value_actions = al_value_actions.expect("value is not missing");
        let action_conflicts: Arc<ActionConflictsValue> =
            action_conflicts.expect("the precomputed value is not missing");
        let action_conflicts: &ActionConflicts = &action_conflicts.0;

        // CompactHashSet: insertion order.
        let mut dep_keys: IndexSet<SkyKey> = IndexSet::new();
        for action in &al_value_actions {
            if let Some(conflict) = action_conflicts.get(action) {
                // ActionConflictFunctionException
                return Err(
                    SkyFunctionException::new(conflict.clone(), Transience::Persistent).into(),
                );
            }
            for dep_key in convert_artifacts(&action.get_inputs()) {
                dep_keys.insert(SkyKey::new(dep_key));
            }
        }
        // Avoid silly cycles.
        dep_keys.shift_remove(sky_key);

        let dep_keys: Vec<SkyKey> = dep_keys.into_iter().collect();
        let result = env.get_values_and_exceptions(&dep_keys)?;
        if env.values_missing() {
            return Ok(None);
        }
        for key in &dep_keys {
            if result.get(key).is_none() {
                return Ok(None);
            }
        }
        Ok(Some(Arc::new(ActionLookupConflictFindingValue) as SkyValue))
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionLookupConflictFindingFunction.java:86 ActionLookupConflictFindingFunction.extractTag
    fn extract_tag(&self, sky_key: &SkyKey) -> Option<String> {
        // Label.print(((ConfiguredTargetKey) skyKey.argument()).getLabel())
        let key = sky_key.downcast::<ActionLookupConflictFindingKey>()?;
        Some(match key.argument().get_label() {
            Some(label) => label.to_string(),
            None => "(unknown)".to_owned(),
        })
    }
}

// Java: com/google/devtools/build/lib/skyframe/ActionLookupConflictFindingFunction.java:78 ActionLookupConflictFindingFunction.convertArtifacts
fn convert_artifacts(artifacts: &NestedSet<Artifact>) -> Vec<ActionLookupConflictFindingKey> {
    artifacts
        .to_list()
        .iter()
        .filter(|a| !a.is_source_artifact())
        .map(ActionLookupConflictFindingKey::key_of_artifact)
        .collect()
}

/// Checks to see if any artifacts to be built by this `ActionLookupKey` transitively depend on
/// actions from an `ActionLookupValue` that has an action in conflict with another. If so, none
/// of this key's artifacts will be built.
// Java: com/google/devtools/build/lib/skyframe/TopLevelActionLookupConflictFindingFunction.java:39 TopLevelActionLookupConflictFindingFunction
pub struct TopLevelActionLookupConflictFindingFunction;

impl SkyFunction for TopLevelActionLookupConflictFindingFunction {
    // Java: com/google/devtools/build/lib/skyframe/TopLevelActionLookupConflictFindingFunction.java:42 TopLevelActionLookupConflictFindingFunction.compute
    fn compute(
        &self,
        sky_key: &SkyKey,
        env: &mut Environment<'_>,
    ) -> blaze_skyframe::ComputeResult {
        let key = sky_key
            .downcast::<TopLevelActionLookupConflictFindingKey>()
            .expect("TOP_LEVEL_ACTION_LOOKUP_CONFLICT_DETECTION key");
        // CompletionFunction.getValueAndArtifactsToBuild(key, env)
        // Java: com/google/devtools/build/lib/skyframe/CompletionFunction.java:364 CompletionFunction.getValueAndArtifactsToBuild
        let all_output_groups = get_all_artifacts_to_build(&key.action_lookup_key, env)?;
        if env.values_missing() {
            return Ok(None);
        }
        // TopLevelArtifactHelper.getAllArtifactsToBuild(value.getConfiguredObject(),
        // topLevelContext).getAllArtifacts()
        let all_output_groups = all_output_groups.expect("value is not missing");
        let mut all_artifacts = NestedSetBuilder::stable_order();
        for output_group in &key.output_groups {
            if let Some(artifacts) = all_output_groups.get(output_group) {
                all_artifacts.add_transitive(artifacts);
            }
        }
        let dep_keys: Vec<SkyKey> = convert_artifacts(&all_artifacts.build())
            .into_iter()
            .map(SkyKey::new)
            .collect();
        // GraphTraversingHelper.declareDependenciesAndCheckIfValuesMissingMaybeWithExceptions
        // Java: com/google/devtools/build/skyframe/GraphTraversingHelper.java:110 GraphTraversingHelper.declareDependenciesAndCheckIfValuesMissingMaybeWithExceptions
        let result = env.get_values_and_exceptions(&dep_keys)?;
        if env.values_missing() {
            return Ok(None);
        }
        for key in &dep_keys {
            if result.get(key).is_none() {
                return Ok(None);
            }
        }
        Ok(Some(Arc::new(ActionLookupConflictFindingValue) as SkyValue))
    }

    // Java: com/google/devtools/build/lib/skyframe/TopLevelActionLookupConflictFindingFunction.java:60 TopLevelActionLookupConflictFindingFunction.extractTag
    fn extract_tag(&self, sky_key: &SkyKey) -> Option<String> {
        let key = sky_key.downcast::<TopLevelActionLookupConflictFindingKey>()?;
        Some(match key.action_lookup_key.get_label() {
            Some(label) => label.to_string(),
            None => "(unknown)".to_owned(),
        })
    }
}

/// `env.getValue(key.actionLookupKey())` and `TopLevelArtifactHelper.getAllArtifactsToBuild` of
/// `value.getConfiguredObject()` for a top-level key: a `ConfiguredTargetKey` (the
/// `ConfiguredTarget`) or an `AspectKey` (the `AspectValue`, a `ConfiguredAspect`).
// Java: com/google/devtools/build/lib/skyframe/CompletionFunction.java:364 CompletionFunction.getValueAndArtifactsToBuild
fn get_all_artifacts_to_build(
    action_lookup_key: &ActionLookupKeyRef,
    env: &mut Environment<'_>,
) -> Result<Option<IndexMap<String, NestedSet<Artifact>>>, ComputeError> {
    if let Some(aspect_key) = action_lookup_key.as_any().downcast_ref::<AspectKey>() {
        let Some(value) = env.get_value(aspect_key.clone())? else {
            return Ok(None);
        };
        return Ok(Some(
            crate::execution_seam::get_all_artifacts_to_build_of_aspect(&value),
        ));
    }
    let ct_key = action_lookup_key
        .as_any()
        .downcast_ref::<ConfiguredTargetKey>()
        .unwrap_or_else(|| {
            panic!(
                "ClassCastException: {action_lookup_key} is neither a ConfiguredTargetKey nor an \
                 AspectKey"
            )
        });
    let Some(value) = env.get_value(ct_key.clone())? else {
        return Ok(None);
    };
    let configured_target: &ConfiguredTarget = value.get_configured_target();
    Ok(Some(crate::execution_seam::get_all_artifacts_to_build(
        configured_target,
    )))
}

/// Registers `ACTION_LOOKUP_CONFLICT_DETECTION` and `TOP_LEVEL_ACTION_LOOKUP_CONFLICT_DETECTION`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:760 SkyframeExecutor.skyFunctions
pub fn register(builder: EvaluatorBuilder) -> EvaluatorBuilder {
    builder
        .function(
            ACTION_LOOKUP_CONFLICT_FINDING,
            ActionLookupConflictFindingFunction,
        )
        .function(
            TOP_LEVEL_ACTION_LOOKUP_CONFLICT_FINDING,
            TopLevelActionLookupConflictFindingFunction,
        )
}
