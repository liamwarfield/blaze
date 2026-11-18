//! `CompletionFunction` with `TargetCompletor`: the `TARGET_COMPLETION` SkyFunction.

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_exec::actions::ActionExecutionException;
use blaze_skyframe::{ComputeError, Environment, SkyFunctionException, SkyKey, Transience};
use indexmap::IndexSet;

use crate::analysis_seam::{AnalysisSeam, ConfiguredTargetView};
use crate::artifact_function::{ArtifactValue, SourceArtifactException};
use crate::events::TargetCompleteEvent;
use crate::keys::{TargetCompletionKey, artifact_key};

/// The (marker) value of a completed top-level target.
// Java: com/google/devtools/build/lib/skyframe/TargetCompletionValue.java:33 TargetCompletionValue
#[derive(Clone, Debug, Default)]
pub struct TargetCompletionValue;

/// The output groups of a target as `TopLevelArtifactHelper.getAllArtifactsToBuild` sees them:
/// each requested group present in the target's `OutputGroupInfo`, with its artifacts.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:192 TopLevelArtifactHelper.getAllArtifactsToBuild
pub fn get_all_artifacts_to_build(
    target: &dyn ConfiguredTargetView,
    output_groups: &[String],
) -> Vec<(String, Vec<Artifact>)> {
    let groups = target.get_output_groups();
    let mut result = Vec::new();
    for name in output_groups {
        if let Some(set) = groups.get(name) {
            result.push((name.clone(), set.to_list().as_ref().clone()));
        }
    }
    result
}

/// An input file problem of a top-level target (`InputFileErrorException`).
// Java: com/google/devtools/build/lib/skyframe/CompletionFunction.java:458 CompletionFunction.InputFileErrorException
#[derive(Clone, Debug)]
pub struct InputFileErrorException {
    pub message: String,
}

impl std::fmt::Display for InputFileErrorException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for InputFileErrorException {}

/// `CompletionFunction.compute` for targets.
// Java: com/google/devtools/build/lib/skyframe/CompletionFunction.java:153 CompletionFunction.compute
pub fn compute_target_completion(
    seam: &dyn AnalysisSeam,
    key: &TargetCompletionKey,
    env: &mut Environment<'_>,
) -> Result<Option<TargetCompletionValue>, ComputeError> {
    // Java: com/google/devtools/build/lib/skyframe/TargetCompletor.java:66 TargetCompletor.getValueAndArtifactsToBuild
    let Some(value) = seam.get_configured_target(&key.action_lookup_key, env)? else {
        return Ok(None);
    };
    let artifacts_to_build = get_all_artifacts_to_build(&*value, &key.output_groups);
    if env.values_missing() {
        return Ok(None);
    }
    let all_artifacts: IndexSet<Artifact> = artifacts_to_build
        .iter()
        .flat_map(|(_, a)| a.iter().cloned())
        .collect();
    let keys: Vec<SkyKey> = all_artifacts.iter().map(artifact_key).collect();
    let input_deps = env.get_values_and_exceptions(&keys)?;

    let mut first_action_execution_exception: Option<ActionExecutionException> = None;
    let mut root_causes = Vec::new();
    let mut missing_messages = Vec::new();
    for (input, k) in all_artifacts.iter().zip(&keys) {
        let catch = |e: &blaze_skyframe::SkyError| {
            if let Some(x) = e.downcast_ref::<ActionExecutionException>() {
                return Some(Err(x.clone()));
            }
            e.downcast_ref::<SourceArtifactException>()
                .map(|x| Ok(x.clone()))
        };
        let got = match k.downcast::<crate::keys::ArtifactKey>() {
            Some(ak) => input_deps
                .get_or_throw_with(ak, catch)
                .map(|v| v.map(|v| Some((*v).clone()))),
            None => input_deps
                .get_or_throw_with(
                    k.downcast::<crate::keys::ActionExecutionKey>().unwrap(),
                    catch,
                )
                .map(|v| v.map(|_| None)),
        };
        match got {
            Ok(None) => continue,
            Ok(Some(Some(ArtifactValue::Missing(m)))) => {
                // Java: com/google/devtools/build/lib/skyframe/CompletionFunction.java:343 CompletionFunction.handleSourceFileError
                missing_messages.push(m.get_failure_detail().message.clone());
                root_causes.push(input.get_owner());
            }
            Ok(Some(_)) => {}
            Err(Err(e)) => {
                for c in e.get_root_causes() {
                    root_causes.push(Some(c.label.clone()));
                }
                // Prefer a catastrophic exception as the one we propagate.
                let replace = match &first_action_execution_exception {
                    None => true,
                    Some(f) => !f.is_catastrophe() && e.is_catastrophe(),
                };
                if replace {
                    first_action_execution_exception = Some(e);
                }
            }
            Err(Ok(e)) => {
                missing_messages.push(e.get_failure_detail().message.clone());
                root_causes.push(input.get_owner());
            }
        }
    }

    if !root_causes.is_empty() || first_action_execution_exception.is_some() {
        // Java: com/google/devtools/build/lib/skyframe/CompletionFunction.java:331 CompletionFunction.postFailedEvent
        env.listener().post(Arc::new(TargetCompleteEvent {
            label: value.get_label(),
            configuration_checksum: value.get_configuration_checksum(),
            root_causes: root_causes.iter().flatten().cloned().collect(),
            output_groups: artifacts_to_build.clone(),
            is_test: key.will_test,
            failed: true,
        }));
        if let Some(e) = first_action_execution_exception {
            return Err(SkyFunctionException::new(e, Transience::Persistent).into());
        }
        // Java: com/google/devtools/build/lib/skyframe/TargetCompletor.java:118 TargetCompletor.getLocationIdentifier
        // PORT: the target's Location (its BUILD file position) is not part of the seam; the
        // label form of the identifier is used.
        let message = format!(
            "{} {} input file(s) do not exist",
            value.get_label(),
            missing_messages.len()
        );
        env.listener()
            .handle(&blaze_events::Event::error(message.clone()));
        return Err(SkyFunctionException::new(
            InputFileErrorException { message },
            Transience::Persistent,
        )
        .into());
    }

    // Only check for missing values *after* reporting errors.
    if env.values_missing() {
        return Ok(None);
    }

    // Java: com/google/devtools/build/lib/skyframe/TargetCompletor.java:89 TargetCompletor.createSucceeded
    env.listener().post(Arc::new(TargetCompleteEvent {
        label: value.get_label(),
        configuration_checksum: value.get_configuration_checksum(),
        root_causes: Vec::new(),
        output_groups: artifacts_to_build,
        is_test: key.will_test,
        failed: false,
    }));
    Ok(Some(TargetCompletionValue))
}
