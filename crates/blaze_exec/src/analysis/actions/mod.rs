//! Execution (`Action.execute`) of the `blaze_actions` action classes: the execute halves of
//! `lib/analysis/actions/*`, `lib/analysis/{SourceManifestAction,RepoMappingManifestAction}`,
//! `lib/analysis/starlark/UnresolvedSymlinkAction`, `lib/actions/RunfilesTreeAction`,
//! `lib/rules/cpp/SolibSymlinkAction`, `lib/bazel/BazelWorkspaceStatusModule` and the action
//! contexts they use (`FileWriteActionContext`/`FileWriteStrategy`,
//! `TemplateExpansionContext`/`LocalTemplateExpansionStrategy`, `SymlinkTreeActionContext`,
//! `WorkspaceStatusAction.Context`).
//!
//! Call [`register`] once at startup; register the contexts from [`default_action_contexts`]
//! (plus a [`bazel_workspace_status_module::WorkspaceStatusActionContext`]) in the executor's
//! `ActionContextRegistry::get_context_any`.

// The seam's `ActionExecutionException` carries the Java exception's fields (large).
#![allow(clippy::result_large_err)]

pub mod abstract_file_write_action;
pub mod bazel_workspace_status_module;
pub mod build_info_file_write_action;
pub mod file_system_utils;
pub mod file_write_action_context;
pub mod file_write_strategy;
pub mod local_template_expansion_strategy;
pub mod runfiles_tree_action;
pub mod solib_symlink_action;
pub mod spawn_action;
pub mod symlink_action;
pub mod symlink_tree_action;
pub mod template_expansion_action;
pub mod unresolved_symlink_action;

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;

use blaze_actions::actions::action::{Action, InputMetadataProvider as KeyInputMetadataProvider};
use blaze_actions::actions::artifact::Artifact as BlazeArtifact;
use blaze_collect::{NestedSet, NestedSetBuilder, NestedSetNode, Order};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_vfs::Path;

use crate::actions::action_adapters::{ActionMetadataSnapshot, artifact_input, as_blaze_artifact};
use crate::actions::action_execution::{ActionContextRef, register_executable_action};
use crate::actions::{ActionInputRef, InputMetadataProvider};
use crate::exec::action_execution_context::ActionExecutionContext;
use crate::exec::spawn_runner::ProgressStatus;
use crate::exec::spawn_strategy_resolver::SpawnStrategyResolver;

use file_write_action_context::FileWriteActionContext;
use local_template_expansion_strategy::{LocalTemplateExpansionStrategy, TemplateExpansionContext};
use symlink_tree_action::SymlinkTreeActionContext;

/// Registers the [`ExecutableAction`](crate::actions::ExecutableAction) implementation of every
/// `blaze_actions` action class this module executes (idempotent).
pub fn register() {
    use blaze_actions::actions::runfiles_tree_action::RunfilesTreeAction;
    use blaze_actions::analysis::actions::binary_file_write_action::BinaryFileWriteAction;
    use blaze_actions::analysis::actions::build_info_file_write_action::BuildInfoFileWriteAction;
    use blaze_actions::analysis::actions::file_write_action::FileWriteAction;
    use blaze_actions::analysis::actions::lazy_write_nested_set_of_tuple_action::LazyWriteNestedSetOfTupleAction;
    use blaze_actions::analysis::actions::lazy_write_paths_file_action::LazyWritePathsFileAction;
    use blaze_actions::analysis::actions::parameter_file_write_action::ParameterFileWriteAction;
    use blaze_actions::analysis::actions::spawn_action::SpawnAction;
    use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
    use blaze_actions::analysis::actions::symlink_tree_action::SymlinkTreeAction;
    use blaze_actions::analysis::actions::template_expansion_action::TemplateExpansionAction;
    use blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingManifestAction;
    use blaze_actions::analysis::source_manifest_action::SourceManifestAction;
    use blaze_actions::analysis::starlark::unresolved_symlink_action::UnresolvedSymlinkAction;
    use blaze_actions::bazel::bazel_workspace_status_module::BazelWorkspaceStatusAction;
    use blaze_actions::rules::cpp::solib_symlink_action::SolibSymlinkAction;

    register_executable_action::<FileWriteAction>();
    register_executable_action::<BinaryFileWriteAction>();
    register_executable_action::<ParameterFileWriteAction>();
    register_executable_action::<LazyWritePathsFileAction>();
    register_executable_action::<LazyWriteNestedSetOfTupleAction>();
    register_executable_action::<SourceManifestAction>();
    register_executable_action::<RepoMappingManifestAction>();
    register_executable_action::<TemplateExpansionAction>();
    register_executable_action::<BuildInfoFileWriteAction>();
    register_executable_action::<SymlinkAction>();
    register_executable_action::<SolibSymlinkAction>();
    register_executable_action::<UnresolvedSymlinkAction>();
    register_executable_action::<SymlinkTreeAction>();
    register_executable_action::<RunfilesTreeAction>();
    register_executable_action::<SpawnAction>();
    register_executable_action::<BazelWorkspaceStatusAction>();
}

/// The action contexts Bazel registers for these actions, keyed as
/// [`crate::exec::action_execution_context::get_context`] looks them up:
///
/// - `SpawnStrategyResolver` (`ExecutionTool`: `register(SpawnStrategyResolver.class, new
///   SpawnStrategyResolver())`), as `Arc<SpawnStrategyResolver>`;
/// - `FileWriteActionContext` → `FileWriteStrategy` (`StandaloneModule`), as
///   `Arc<Arc<dyn FileWriteActionContext>>`;
/// - `TemplateExpansionContext` → `LocalTemplateExpansionStrategy` (`StandaloneModule`), as
///   `Arc<Arc<dyn TemplateExpansionContext>>`;
/// - `SymlinkTreeActionContext` → `SymlinkTreeStrategy(env.getOutputService(),
///   env.getWorkspaceName())` (`ExecutionTool`) with Bazel's `LocalOutputService`, as
///   `Arc<Arc<dyn SymlinkTreeActionContext>>`.
///
/// `WorkspaceStatusAction.Context` needs the command's options; build it with
/// [`bazel_workspace_status_module::BazelWorkspaceStatusActionContext::new`] and register it
/// under [`bazel_workspace_status_module::workspace_status_context_entry`].
pub fn default_action_contexts(workspace_name: &str) -> Vec<(TypeId, ActionContextRef)> {
    let file_write: Arc<dyn FileWriteActionContext> =
        Arc::new(file_write_strategy::FileWriteStrategy::new());
    let template: Arc<dyn TemplateExpansionContext> =
        Arc::new(LocalTemplateExpansionStrategy::new());
    let symlink_tree: Arc<dyn SymlinkTreeActionContext> = Arc::new(
        crate::exec::symlink_tree_strategy::SymlinkTreeStrategy::new(
            Arc::new(crate::exec::symlink_tree_strategy::LocalOutputService),
            workspace_name.to_string(),
        ),
    );
    vec![
        (
            TypeId::of::<SpawnStrategyResolver>(),
            Arc::new(SpawnStrategyResolver::new()) as ActionContextRef,
        ),
        (
            TypeId::of::<Arc<dyn FileWriteActionContext>>(),
            Arc::new(file_write) as ActionContextRef,
        ),
        (
            TypeId::of::<Arc<dyn TemplateExpansionContext>>(),
            Arc::new(template) as ActionContextRef,
        ),
        (
            TypeId::of::<Arc<dyn SymlinkTreeActionContext>>(),
            Arc::new(symlink_tree) as ActionContextRef,
        ),
    ]
}

/// `actionExecutionContext.getContext(T.class)` for a trait-object context registered as
/// `Arc<Arc<dyn Trait>>`; Java dereferences the result without a null check, so a missing
/// context is the same `NullPointerException`.
pub(crate) fn get_required_context<T: ?Sized + Send + Sync + 'static>(
    ctx: &dyn ActionExecutionContext,
    name: &str,
) -> Arc<T>
where
    Arc<T>: Send + Sync,
{
    match crate::exec::action_execution_context::get_context::<Arc<T>>(ctx) {
        Some(c) => (*c).clone(),
        None => panic!("NullPointerException: getContext({name}.class) returned null"),
    }
}

/// `actionExecutionContext.getInputPath(artifact)`: `ArtifactPathResolver.IdentityResolver.toPath`
/// returns `artifact.getPath()` for an `Artifact` (a source artifact resolves to its source root,
/// not to the execroot).
///
/// PORT: the context's own `get_input_path` default resolves every input against the execroot,
/// which is Java's behaviour only for non-artifact inputs; this is the artifact case.
// Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:401 ActionExecutionContext.getInputPath
// Java: com/google/devtools/build/lib/actions/ArtifactPathResolver.java:75 IdentityResolver.toPath
// Java: com/google/devtools/build/lib/actions/ArtifactPathResolver.java:107 TransformResolver.toPath
pub fn get_input_path(ctx: &dyn ActionExecutionContext, artifact: &BlazeArtifact) -> Path {
    // IdentityResolver: artifact.getPath(); TransformResolver (an action file system):
    // fileSystem.getPath(artifact.getPath().asFragment()).
    ctx.convert_path(&artifact.get_path())
}

/// `getEventHandler().post(new RunningActionEvent(action, strategy))`: `RunningActionEvent` is
/// what `SpawnExecutingEvent.postTo` posts, so it goes through the context's progress seam.
// Java: com/google/devtools/build/lib/actions/RunningActionEvent.java:27 RunningActionEvent.<init>
pub(crate) fn post_running_action_event(
    ctx: &dyn ActionExecutionContext,
    action: &dyn Action,
    strategy: &str,
) {
    let metadata = ActionMetadataSnapshot::of(action);
    ctx.post_progress(&*metadata, &ProgressStatus::Executing(strategy.to_string()));
}

/// A `FailureDetail` with the given message and category.
pub(crate) fn failure_detail(message: &str, category: failure_detail::Category) -> FailureDetail {
    FailureDetail {
        message: message.to_string(),
        category: Some(category),
    }
}

/// `FailureDetail.newBuilder().setExecution(Execution.newBuilder().setCode(code))` (no message).
pub(crate) fn execution_category(
    code: failure_details::execution::Code,
) -> failure_detail::Category {
    failure_detail::Category::Execution(failure_details::Execution { code: code as i32 })
}

/// `Throwable.toString()` of an `IOException` (`ExecException(Throwable cause)` uses it as the
/// message): `java.io.IOException: <message>`.
///
/// PORT: Java prints the concrete exception class (`FileNotFoundException`,
/// `AccessDeniedException`, ...); Rust I/O errors carry no class, so the base class is printed
/// (same convention as `sandbox_module.rs`).
pub(crate) fn io_exception_to_string(e: &std::io::Error) -> String {
    format!("java.io.IOException: {e}")
}

/// `new EnvironmentalExecException(IOException cause, Execution.Code code)`: the message is the
/// cause's `toString()`, the failure detail carries only the code.
// Java: com/google/devtools/build/lib/actions/EnvironmentalExecException.java:33 EnvironmentalExecException.<init>
pub(crate) fn environmental_exec_exception(
    e: &std::io::Error,
    code: failure_details::execution::Code,
) -> crate::actions::ExecException {
    crate::actions::EnvironmentalExecException {
        message: io_exception_to_string(e),
        failure_detail: failure_detail("", execution_category(code)),
        lost_inputs: None,
    }
    .into()
}

/// The execution layer's `InputMetadataProvider` as the key/command-line expansion one of
/// `blaze_actions` (which only reads tree artifact children).
pub(crate) struct KeyMetadataAdapter<'a>(pub &'a dyn InputMetadataProvider);

impl KeyInputMetadataProvider for KeyMetadataAdapter<'_> {
    // Java: com/google/devtools/build/lib/actions/InputMetadataProvider.java:56 InputMetadataProvider.getTreeMetadata
    fn get_tree_children(&self, tree: &BlazeArtifact) -> Option<Vec<BlazeArtifact>> {
        let value = self.0.get_tree_metadata(&artifact_input(tree))?;
        Some(
            value
                .get_children()
                .iter()
                .map(|c| match as_blaze_artifact(c) {
                    Some(a) => a.clone(),
                    None => blaze_util::unported!(
                        "TreeArtifactValue.getChildren",
                        "tree child {} is not a blaze_actions Artifact",
                        c.0.get_exec_path()
                    ),
                })
                .collect(),
        )
    }
}

/// Converts a `NestedSet<Artifact>` into the `NestedSet<ActionInput>` a spawn holds, keeping the
/// node structure (and sharing of sub-sets) so that consumers that walk the DAG, such as the
/// compact execution log, see the same nodes Java does (Java upcasts the same object).
// `NestedSetNode` hashes and compares by node identity (Java `Node.equals`), not by the
// artifacts' interior state.
///
/// The conversion is memoized for the process: every spawn that holds a node of one
/// `NestedSet<Artifact>` (for example the depset of headers shared by many compile actions) gets
/// the same converted node, as every Java spawn holds the same object, so that
/// `CompactSpawnLogContext` logs a shared set once and refers to it by its entry ID. The memo is
/// keyed by node and order: sets that share a node but not an order (every empty set, equal
/// singletons, a set that wraps a single transitive set of another order) are different Java
/// objects with different orders.
#[allow(clippy::mutable_key_type)]
pub fn to_action_input_set(set: &NestedSet<BlazeArtifact>) -> NestedSet<ActionInputRef> {
    convert_set(set)
}

/// The key of a converted set: Java's `NestedSet` object is its node and its order.
type ConvertedSetKey = (NestedSetNode<BlazeArtifact>, Order);

/// The converted set of each `NestedSet<Artifact>` (see [`to_action_input_set`]).
#[allow(clippy::mutable_key_type)]
static ACTION_INPUT_SETS: std::sync::LazyLock<
    parking_lot::Mutex<HashMap<ConvertedSetKey, NestedSet<ActionInputRef>>>,
> = std::sync::LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

/// Converts `set`, its sub-sets first. The memo's lock is held only to look up and to publish a
/// converted set, not across a conversion, so conversions of different actions run in parallel.
/// When two threads convert the same set at once, the first to publish wins and both return
/// its result, so every holder still shares one converted node.
#[allow(clippy::mutable_key_type)]
fn convert_set(set: &NestedSet<BlazeArtifact>) -> NestedSet<ActionInputRef> {
    let order = set.get_order();
    let key: ConvertedSetKey = (set.to_node(), order);
    if let Some(s) = ACTION_INPUT_SETS.lock().get(&key) {
        return s.clone();
    }
    let converted = if set.is_empty() {
        NestedSetBuilder::empty_set(order)
    } else if set.is_singleton() {
        NestedSetBuilder::create(order, [artifact_input(set.get_singleton())])
    } else {
        // PORT: NestedSet's constructor lays out a node as its transitive members followed by its
        // direct members for every order but NAIVE_LINK_ORDER (direct first), which is what the
        // builder rebuilds from the node's non-leaf and leaf successors.
        let mut b = NestedSetBuilder::new_builder(order);
        for child in set.get_non_leaves() {
            let c = convert_set(&child);
            b.add_transitive(&c);
        }
        for leaf in set.get_leaves() {
            b.add(artifact_input(&leaf));
        }
        b.build()
    };
    ACTION_INPUT_SETS
        .lock()
        .entry(key)
        .or_insert(converted)
        .clone()
}
