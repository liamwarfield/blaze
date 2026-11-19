//! The `lib/actions` types spawn execution reads (seam with `blaze_actions`), plus the
//! spawn-side classes this crate owns (`Spawn`, `SimpleSpawn`, `Spawns`, `SpawnResult`,
//! `SpawnMetrics`, `ResourceSet`, `ExecException`s) and the action cache (`cache`).
//!
//! # Seam with `blaze_actions`
//!
//! `blaze_exec` is above `blaze_actions` in the crate DAG, so it cannot be implemented *by*
//! `blaze_actions`; instead, once `blaze_exec` depends on `blaze_actions`, `blaze_exec` adds
//! adapter impls of these traits for `blaze_actions` types (the orphan rule allows it because the
//! traits are local). What `blaze_actions` must provide for that:
//!
//! - [`ActionInput`] for `Artifact` (+ `TreeFileArtifact`, `ArchivedTreeArtifact`,
//!   `ParamFileActionInput`, runfiles-tree artifacts): exec path, `isDirectory`, `isSymlink`; and
//!   [`Artifact`]: `isSourceArtifact`, `isTreeArtifact`, `isRunfilesTree`, `isFileset`,
//!   `isConstantMetadata`, `isChildOfDeclaredDirectory`, `getParent`,
//!   `getParentRelativePath`, `getRootRelativePath`, root exec path. Inputs are compared by exec
//!   path ([`ActionInputRef`]).
//! - [`VirtualActionInput`] for `ParamFileActionInput` and other virtual inputs (`writeTo`).
//! - [`ActionExecutionMetadata`] for every action that produces spawns: mnemonic, progress
//!   message, [`ActionOwner`] (label, configuration checksum, description, exec properties),
//!   primary output, execution [`PlatformInfo`], `prettyPrint`, `describe`.
//! - [`Spawn`]: `SpawnAction.getSpawn` & co. build a [`SimpleSpawn`] (or their own impl) with
//!   arguments, environment, execution info, input and tool `NestedSet<ActionInputRef>`,
//!   outputs and a [`ResourceSetOrBuilder`].
//! - [`InputMetadataProvider`] (from the action's input metadata: `ActionInputMap` /
//!   Skyframe artifact values), returning [`FileArtifactValue`], [`TreeArtifactValue`],
//!   [`RunfilesArtifactValue`] (+ a [`RunfilesTree`] impl for `RunfilesTree`) and
//!   [`FilesetOutputTree`].
//! - For the action cache (`action_cache_checker::ActionCacheChecker`): the `Action` view in
//!   `action_cache_checker` (key, inputs, outputs, client env, flags).
//!
//! [`FileArtifactValue`] here is a seam copy of `lib/actions/FileArtifactValue.java`'s variants;
//! convert from (or replace with) the `blaze_actions` port when wiring.

pub mod action_adapters;
pub mod action_cache_checker;
pub mod action_execution;
pub mod action_execution_metadata;
pub mod action_input;
pub mod action_input_prefetcher;
pub mod action_output_directory_helper;
pub mod cache;
pub mod exec_exception;
pub mod execution_requirements;
pub mod file_artifact_value;
pub mod input_metadata_provider;
pub mod path_mapper;
pub mod resource_manager;
pub mod resource_set;
pub mod simple_spawn;
pub mod spawn;
pub mod spawn_metrics;
pub mod spawn_result;
pub mod spawn_strategy;
pub mod spawns;
pub mod virtual_action_input;

pub use action_cache_checker::{ActionCacheChecker, CacheCheckAction, OutputChecker};
pub use action_execution::{
    ActionExecutionException, ActionResult, ExecutableAction, as_executable,
    register_executable_action,
};
pub use action_execution_metadata::{
    ActionExecutionMetadata, ActionOwner, FakeOwner, PlatformInfo,
};
pub use action_input::{
    ActionInput, ActionInputRef, Artifact, BasicActionInput, HierarchicalPath, InputMapping,
};
pub use exec_exception::{
    EnvironmentalExecException, ExecException, SpawnExecException, UserExecException,
};
pub use file_artifact_value::{FileArtifactValue, FileContentsProxy, FileStateType};
pub use input_metadata_provider::{
    FilesetOutputSymlink, FilesetOutputTree, InputMetadataProvider, RunfilesArtifactValue,
    RunfilesTree, SymlinkEntry, TreeArtifactValue, expand_artifacts,
};
pub use path_mapper::PathMapper;
pub use resource_set::{ResourceSet, ResourceSetOrBuilder};
pub use simple_spawn::SimpleSpawn;
pub use spawn::{Spawn, SpawnDescribable};
pub use spawn_metrics::{ExecKind, SpawnMetrics};
pub use spawn_result::{POSIX_TIMEOUT_EXIT_CODE, SpawnResult, SpawnResultBuilder, Status};
pub use virtual_action_input::{
    BasicVirtualActionInput, EmptyActionInput, VirtualActionInput, empty_marker,
};
