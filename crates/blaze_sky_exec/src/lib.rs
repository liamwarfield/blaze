//! Port of Bazel's execution-phase SkyFunctions (`lib/skyframe`: `ActionExecutionFunction`,
//! `SkyframeActionExecutor`, `ArtifactFunction`, `ArtifactNestedSetFunction`,
//! `CompletionFunction`/`TargetCompletor`, `TestCompletionFunction`, `BuildDriverFunction`).
//!
//! The analysis crates plug in through [`analysis_seam::AnalysisSeam`]; actions execute through
//! `blaze_exec::actions::ExecutableAction` implementations registered with
//! `blaze_exec::actions::register_executable_action`.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]
// Ported signatures win: `ActionExecutionException` is Java's exception carried by value.

// Ported signatures and Java's control flow win (PORTING.md "Style"): maps keyed by `Artifact`
// (Java `Map<Artifact, ...>`), exceptions carried by value, Java method names (`from_*`), Java's
// branch structure.
#![allow(
    clippy::mutable_key_type,
    clippy::result_large_err,
    clippy::large_enum_variant,
    clippy::type_complexity,
    clippy::wrong_self_convention,
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::if_same_then_else
)]

pub mod action_execution_function;
pub mod action_execution_value;
pub mod analysis_seam;
pub mod artifact_function;
pub mod build_driver_function;
pub mod completion_function;
pub mod events;
pub mod keys;
pub mod recursive_filesystem_traversal_function;
pub mod skyframe_action_executor;
pub mod test_completion_function;

use std::sync::Arc;

use blaze_skyframe::EvaluatorBuilder;

use crate::action_execution_function::ActionExecutionFunction;
use crate::analysis_seam::AnalysisSeam;
use crate::keys::{
    ActionExecutionKey, ArtifactKey, ArtifactNestedSetKey, AspectCompletionKey, BuildDriverKey,
    TargetCompletionKey, TestCompletionKey,
};
use crate::recursive_filesystem_traversal_function::DirectoryArtifactTraversalRequest;
use crate::skyframe_action_executor::SkyframeActionExecutor;
use crate::test_completion_function::UnrunnableTestReporter;

/// Registers the execution SkyFunctions (`ACTION_EXECUTION`, `ARTIFACT`, `ARTIFACT_NESTED_SET`,
/// `RECURSIVE_FILESYSTEM_TRAVERSAL`, `TARGET_COMPLETION`, `ASPECT_COMPLETION`,
/// `TEST_COMPLETION`, `BUILD_DRIVER`) on `builder`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:600 SkyframeExecutor.skyFunctions
pub fn register_execution_functions(
    builder: EvaluatorBuilder,
    executor: Arc<SkyframeActionExecutor>,
    seam: Arc<dyn AnalysisSeam>,
    unrunnable_test_reporter: Option<Arc<UnrunnableTestReporter>>,
) -> EvaluatorBuilder {
    let completion_executor = executor.clone();
    let traversal_executor = executor.clone();
    let action_execution = Arc::new(ActionExecutionFunction {
        skyframe_action_executor: executor,
        seam: seam.clone(),
    });
    let seam_tc = seam.clone();
    let seam_ac = seam.clone();
    let aspect_completion_executor = completion_executor.clone();
    let seam_test = seam.clone();
    // Java: SkyframeExecutor.skyFunctions: `new RecursiveFilesystemTraversalFunction(
    // syscallCache, directories.getOutputBase().getRelative(LabelConstants.EXTERNAL_PATH_PREFIX))`.
    let recursive_filesystem_traversal = Arc::new(
        recursive_filesystem_traversal_function::RecursiveFilesystemTraversalFunction::new(
            traversal_executor.get_external_directory().clone(),
        ),
    );
    let build_driver = Arc::new(build_driver_function::BuildDriverFunction::new(seam));
    builder
        .typed_function::<ActionExecutionKey, _>(move |k, env| action_execution.compute(k, env))
        .typed_function::<ArtifactKey, _>(artifact_function::compute_artifact)
        .typed_function::<ArtifactNestedSetKey, _>(artifact_function::compute_artifact_nested_set)
        .typed_function::<DirectoryArtifactTraversalRequest, _>(move |k, env| {
            recursive_filesystem_traversal.compute(k, env)
        })
        .typed_function::<TargetCompletionKey, _>(move |k, env| {
            completion_function::compute_target_completion(&completion_executor, &*seam_tc, k, env)
        })
        .typed_function::<AspectCompletionKey, _>(move |k, env| {
            completion_function::compute_aspect_completion(
                &aspect_completion_executor,
                &*seam_ac,
                k,
                env,
            )
        })
        .typed_function::<TestCompletionKey, _>(move |k, env| {
            test_completion_function::compute_test_completion(
                &*seam_test,
                unrunnable_test_reporter.as_deref(),
                k,
                env,
            )
        })
        .typed_function::<BuildDriverKey, _>(move |k, env| build_driver.compute(k, env))
}
