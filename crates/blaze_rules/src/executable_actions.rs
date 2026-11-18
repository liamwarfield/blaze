//! Registration of the execution half (`blaze_exec::actions::ExecutableAction`) of the action
//! classes this crate defines, and of the action contexts they read.
//!
//! Not a port of a Java class: Java dispatches `Action.execute` virtually; Blaze finds the
//! implementation through `blaze_exec::actions::as_executable`, which needs each class registered
//! once (see `blaze_exec::actions::action_execution`).

use std::any::TypeId;
use std::sync::Arc;

use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_exec::actions::action_execution::{ActionContextRef, register_executable_action};

use crate::cpp::cpp_compile_action::CppCompileAction;
use crate::python::py_builtins::CopyWithoutCachingAction;

/// Registers the `ExecutableAction` implementation of every action class of this crate
/// (idempotent): `CppCompileAction` and `PyBuiltins.CopyWithoutCachingAction`.
pub fn register_executable_actions() {
    register_executable_action::<CppCompileAction>();
    register_executable_action::<CopyWithoutCachingAction>();
}

/// The action contexts the C++ actions look up (`CppIncludeExtractionContext` and the
/// `BuildLanguageOptions` seam); register them in the executor's
/// `ActionContextRegistry::get_context_any`. `artifact_factory` is the command's
/// `SkyframeBuildView.getArtifactFactory()`; `experimental_sibling_repository_layout` is
/// `BuildLanguageOptions.experimentalSiblingRepositoryLayout`.
pub fn executable_action_contexts(
    artifact_factory: Arc<ArtifactFactory>,
    experimental_sibling_repository_layout: bool,
) -> Vec<(TypeId, ActionContextRef)> {
    crate::cpp::cpp_include_extraction_context::cpp_action_contexts(
        artifact_factory,
        experimental_sibling_repository_layout,
    )
}
