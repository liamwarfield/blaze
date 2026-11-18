//! Port of `lib/remote/RemoteAction.java`.

use std::sync::Arc;

use blaze_exec::actions::{InputMapping, Spawn};
use blaze_exec::exec::SpawnExecutionContext;

use crate::common::network_time::NetworkTime;
use crate::common::remote_action_execution_context::RemoteActionExecutionContext;
use crate::common::remote_cache_client::ActionKey;
use crate::common::remote_path_resolver::RemotePathResolver;
use crate::merkletree::MerkleTree;
use crate::reapi::{Action, Command, Digest};

/// A value class representing an action which can be executed remotely.
///
/// Terminology note: "action" is used here in the remote execution protocol sense, which is
/// equivalent to a Bazel "spawn" (a Bazel "action" being a higher-level concept).
///
/// PORT: the spawn and its execution context are borrowed for `'a` (Java holds references), as
/// in [`RemoteActionExecutionContext`].
// Java: com/google/devtools/build/lib/remote/RemoteAction.java:36 RemoteAction
pub struct RemoteAction<'a> {
    spawn: &'a dyn Spawn,
    spawn_execution_context: &'a dyn SpawnExecutionContext,
    remote_action_execution_context: RemoteActionExecutionContext<'a>,
    remote_path_resolver: Arc<dyn RemotePathResolver>,
    merkle_tree: MerkleTree,
    command_hash: Digest,
    command: Command,
    action: Action,
    action_key: ActionKey,
}

impl std::fmt::Debug for RemoteAction<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteAction")
            .field("action_key", &self.action_key)
            .field("command_hash", &self.command_hash)
            .field("merkle_tree", &self.merkle_tree)
            .finish_non_exhaustive()
    }
}

impl<'a> RemoteAction<'a> {
    /// `new RemoteAction(spawn, spawnExecutionContext, remoteActionExecutionContext,
    /// remotePathResolver, merkleTree, commandHash, command, action, actionKey)`.
    ///
    /// PORT: package-private in Java; `pub` here because the test files of the crate are
    /// separate crates.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:48 RemoteAction.<init>
    pub fn new(
        spawn: &'a dyn Spawn,
        spawn_execution_context: &'a dyn SpawnExecutionContext,
        remote_action_execution_context: RemoteActionExecutionContext<'a>,
        remote_path_resolver: Arc<dyn RemotePathResolver>,
        merkle_tree: MerkleTree,
        command_hash: Digest,
        command: Command,
        action: Action,
        action_key: ActionKey,
    ) -> RemoteAction<'a> {
        RemoteAction {
            spawn,
            spawn_execution_context,
            remote_action_execution_context,
            remote_path_resolver,
            merkle_tree,
            command_hash,
            command,
            action,
            action_key,
        }
    }

    /// `getRemoteActionExecutionContext()`.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:69 RemoteAction.getRemoteActionExecutionContext
    pub fn get_remote_action_execution_context(&self) -> &RemoteActionExecutionContext<'a> {
        &self.remote_action_execution_context
    }

    /// `getSpawnExecutionContext()`.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:73 RemoteAction.getSpawnExecutionContext
    pub fn get_spawn_execution_context(&self) -> &'a dyn SpawnExecutionContext {
        self.spawn_execution_context
    }

    /// Returns the `Spawn` that owns this action.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:78 RemoteAction.getSpawn
    pub fn get_spawn(&self) -> &'a dyn Spawn {
        self.spawn
    }

    /// Returns the sum of file sizes plus protobuf sizes used to represent the inputs of this
    /// action.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:85 RemoteAction.getInputBytes
    pub fn get_input_bytes(&self) -> i64 {
        self.merkle_tree.input_bytes()
    }

    /// Returns the number of input files of this action.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:90 RemoteAction.getInputFiles
    pub fn get_input_files(&self) -> i64 {
        self.merkle_tree.input_files()
    }

    /// Returns the id this is action.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:95 RemoteAction.getActionId
    pub fn get_action_id(&self) -> &str {
        &self.action_key.digest().hash
    }

    /// Returns the `ActionKey` of this action.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:100 RemoteAction.getActionKey
    pub fn get_action_key(&self) -> &ActionKey {
        &self.action_key
    }

    /// Returns underlying `Action` of this remote action.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:105 RemoteAction.getAction
    pub fn get_action(&self) -> &Action {
        &self.action
    }

    /// `getCommandHash()`.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:109 RemoteAction.getCommandHash
    pub fn get_command_hash(&self) -> &Digest {
        &self.command_hash
    }

    /// `getCommand()`.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:113 RemoteAction.getCommand
    pub fn get_command(&self) -> &Command {
        &self.command
    }

    /// `getRemotePathResolver()`.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:117 RemoteAction.getRemotePathResolver
    pub fn get_remote_path_resolver(&self) -> &Arc<dyn RemotePathResolver> {
        &self.remote_path_resolver
    }

    /// `getMerkleTree()`.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:121 RemoteAction.getMerkleTree
    pub fn get_merkle_tree(&self) -> &MerkleTree {
        &self.merkle_tree
    }

    /// Returns a `SortedMap` which maps from input paths for remote action to `ActionInput`.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:129 RemoteAction.getInputMap
    pub fn get_input_map(&self, will_access_repeatedly: bool) -> Arc<InputMapping> {
        self.remote_path_resolver
            .get_input_mapping(self.spawn_execution_context, will_access_repeatedly)
    }

    /// Returns the `NetworkTime` instance used to measure the network time during the action
    /// execution.
    // Java: com/google/devtools/build/lib/remote/RemoteAction.java:137 RemoteAction.getNetworkTime
    pub fn get_network_time(&self) -> &Arc<NetworkTime> {
        self.remote_action_execution_context.get_network_time()
    }
}
