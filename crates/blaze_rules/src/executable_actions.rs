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
use blaze_vfs::Path;

use crate::cpp::cpp_compile_action::CppCompileAction;
use crate::cpp::cpp_compile_action_template::CppCompileActionTemplate;
use crate::python::py_builtins::CopyWithoutCachingAction;

/// Registers the `ExecutableAction` implementation of every action class of this crate
/// (idempotent): `CppCompileAction` and `PyBuiltins.CopyWithoutCachingAction`; and of its
/// action templates (`blaze_exec::actions::ActionTemplate`): `CppCompileActionTemplate`.
pub fn register_executable_actions() {
    register_executable_action::<CppCompileAction>();
    register_executable_action::<CopyWithoutCachingAction>();
    register_executable_action::<crate::genquery::QueryResultAction>();
    register_executable_action::<crate::java::java_compile_action::JavaCompileAction>();
    register_executable_action::<crate::java::java_header_compile_action::JavaHeaderCompileAction>(
    );
    blaze_exec::actions::register_action_template::<CppCompileActionTemplate>();
}

/// The action contexts the C++ actions look up (`CppIncludeExtractionContext`,
/// `CppIncludeScanningContext` and the `BuildLanguageOptions` seam); register them in the
/// executor's `ActionContextRegistry::get_context_any`. `artifact_factory` is the command's
/// `SkyframeBuildView.getArtifactFactory()`; `experimental_sibling_repository_layout` is
/// `BuildLanguageOptions.experimentalSiblingRepositoryLayout`; `exec_root` is the execution root
/// and `output_path` `directories.getOutputPath(workspaceName)` (`<exec_root>/bazel-out`).
pub fn executable_action_contexts(
    artifact_factory: Arc<ArtifactFactory>,
    experimental_sibling_repository_layout: bool,
    exec_root: Path,
    output_path: Path,
) -> Vec<(TypeId, ActionContextRef)> {
    let mut contexts = crate::cpp::cpp_include_extraction_context::cpp_action_contexts(
        artifact_factory.clone(),
        experimental_sibling_repository_layout,
    );
    // Java: com/google/devtools/build/lib/includescanning/IncludeScanningModule.java:96 IncludeScanningModule.registerActionContexts
    contexts.push(crate::includescanning::include_scanning_context(
        exec_root,
        output_path,
        artifact_factory,
    ));
    // Java: com/google/devtools/build/lib/bazel/rules/BazelRulesModule.java:979 BazelRulesModule.registerActionContexts
    contexts.push(crate::java::java_compile_action_context::java_action_context());
    contexts
}
