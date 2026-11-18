//! Port of `lib/actions/cache/ActionCache.java`: the action cache interface, its `Entry` and
//! `Entry.Builder`, and `vfs/OutputPermissions.java` (which the entry digest includes).

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

use blaze_util::{Fingerprint, check_argument, check_state};
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

pub use blaze_protos::blaze::ActionCacheStatistics;
pub use blaze_protos::blaze::action_cache_statistics::{MissDetail, MissReason};

use super::digest_utils::DigestUtils;
use super::metadata_digest_utils::{MetadataDigestUtils, file_artifact_value_to_string, is_proxy};
use super::output_metadata_store::IoOrInterruptedException;
use crate::actions::action_input::ActionInput;
use crate::actions::file_artifact_value::FileArtifactValue;
use crate::actions::input_metadata_provider::TreeArtifactValue;

/// Port of `OutputPermissions`: the permissions of action outputs.
///
/// PORT: belongs to `lib/vfs`; it lives here until `blaze_vfs` ports it.
// Java: com/google/devtools/build/lib/vfs/OutputPermissions.java:17 OutputPermissions
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutputPermissions {
    /// `READONLY(0555)`.
    Readonly,
    /// `WRITABLE(0755)`.
    Writable,
}

impl OutputPermissions {
    // Java: com/google/devtools/build/lib/vfs/OutputPermissions.java:27 OutputPermissions.getPermissionsMode
    pub fn get_permissions_mode(self) -> i32 {
        match self {
            OutputPermissions::Readonly => 0o555,
            OutputPermissions::Writable => 0o755,
        }
    }
}

/// An interface defining a cache of already-executed Actions.
///
/// The name of this class is misleading; it doesn't cache the actual actions, only a fingerprint
/// of all action properties that matter for cache invalidation (action key, path and contents of
/// input and outputs files, environment variables, execution properties, and certain flags), so
/// we can tell if we need to rerun an action given the current state of the file system.
///
/// Each action entry uses the path of the action's primary output as the key.
// Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:60 ActionCache
pub trait ActionCache: Send + Sync {
    /// Updates the cache entry for the specified key.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:63 ActionCache.put
    fn put(&self, key: &str, entry: ActionCacheEntry);

    /// Returns the cache entry for the specified key, or `None` if not found.
    ///
    /// If an entry exists but is corrupted, returns [`ActionCacheEntry::corrupted`]. Callers
    /// should check [`ActionCacheEntry::is_corrupted`] before inspecting anything else on the
    /// entry.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:73 ActionCache.get
    fn get(&self, key: &str) -> Option<ActionCacheEntry>;

    /// Removes entry from cache.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:76 ActionCache.remove
    fn remove(&self, key: &str);

    /// Removes entry from cache that matches the predicate.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:79 ActionCache.removeIf
    fn remove_if(&self, predicate: &dyn Fn(&ActionCacheEntry) -> bool);

    /// Give persistent cache implementations a notification to write to disk. Returns the size in
    /// bytes of the serialized cache.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:437 ActionCache.save
    fn save(&self) -> io::Result<i64>;

    /// Clear the action cache, closing all opened file handle.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:440 ActionCache.clear
    fn clear(&self);

    /// Returns an `ActionCache` with the same backing directory, but whose contents may have been
    /// garbage collected.
    ///
    /// May be safely interrupted. Upon interruption, this instance, including its backing
    /// directory, remains valid. Otherwise, the return value may be the current instance or a
    /// different one, depending on whether garbage collection was deemed necessary. If a
    /// different instance is returned, the current instance must not be used further. Thus, safe
    /// usage of this method looks like `action_cache = action_cache.trim(threshold, max_age)`.
    ///
    /// `threshold`: the fraction of stale entries required to trigger garbage collection;
    /// `max_age`: the age at which entries are considered stale.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:458 ActionCache.trim
    fn trim(
        self: Arc<Self>,
        threshold: f32,
        max_age: Duration,
    ) -> Result<Arc<dyn ActionCache>, IoOrInterruptedException>;

    /// Dumps the action cache into a human-readable format.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:461 ActionCache.dump
    fn dump(&self, out: &mut dyn Write);

    /// The number of entries in the cache.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:464 ActionCache.size
    fn size(&self) -> i32;

    /// Accounts one cache hit.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:467 ActionCache.accountHit
    fn account_hit(&self);

    /// Accounts one cache miss for the given reason.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:470 ActionCache.accountMiss
    fn account_miss(&self, reason: MissReason);

    /// Populates the given builder with statistics.
    ///
    /// The extracted values are not guaranteed to be a consistent snapshot of the metrics
    /// tracked by the action cache. Therefore, even if it is safe to call this function at any
    /// point in time, this should only be called once there are no actions running.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:479 ActionCache.mergeIntoActionCacheStatistics
    fn merge_into_action_cache_statistics(&self, builder: &mut ActionCacheStatistics);

    /// Resets the current statistics to zero.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:482 ActionCache.resetStatistics
    fn reset_statistics(&self);

    /// Duration it took to load the action cache. Might be `None` if not loaded in this
    /// invocation.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:486 ActionCache.getLoadTime
    fn get_load_time(&self) -> Option<Duration> {
        None
    }
}

/// An action cache entry (`ActionCache.Entry`).
///
/// Java's unique `CORRUPTED` instance is [`ActionCacheEntry::corrupted`]: an entry without a
/// digest.
// Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:82 ActionCache.Entry
#[derive(Clone, Debug, PartialEq)]
pub struct ActionCacheEntry {
    // Digest of all relevant properties of the action for cache invalidation purposes.
    // None if the entry is corrupted.
    digest: Option<Vec<u8>>,

    // List of input paths discovered by the action.
    // None if the action does not discover inputs.
    discovered_input_paths: Option<Vec<String>>,

    // Output metadata.
    // Only present when building without the bytes, and even then, only for remotely stored files.
    output_file_metadata: IndexMap<String, FileArtifactValue>,
    output_tree_metadata: IndexMap<String, SerializableTreeArtifactValue>,
    proxy_outputs: Vec<String>,
}

impl ActionCacheEntry {
    /// `Entry.CORRUPTED`: the entry standing for a corrupted cache entry.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:84 ActionCache.Entry.CORRUPTED
    pub fn corrupted() -> ActionCacheEntry {
        ActionCacheEntry::new(None, None, IndexMap::new(), IndexMap::new(), Vec::new())
    }

    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:101 ActionCache.Entry.<init>
    pub(crate) fn new(
        digest: Option<Vec<u8>>,
        discovered_input_paths: Option<Vec<String>>,
        output_file_metadata: IndexMap<String, FileArtifactValue>,
        output_tree_metadata: IndexMap<String, SerializableTreeArtifactValue>,
        proxy_outputs: Vec<String>,
    ) -> ActionCacheEntry {
        ActionCacheEntry {
            digest,
            discovered_input_paths,
            output_file_metadata,
            output_tree_metadata,
            proxy_outputs,
        }
    }

    /// Returns whether this cache entry is corrupted and should be ignored.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:115 ActionCache.Entry.isCorrupted
    pub fn is_corrupted(&self) -> bool {
        self.digest.is_none()
    }

    /// Returns a digest encoding all relevant properties of the action for cache invalidation
    /// purposes.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:123 ActionCache.Entry.getDigest
    pub fn get_digest(&self) -> &[u8] {
        check_state!(!self.is_corrupted());
        self.digest.as_deref().unwrap()
    }

    /// Returns whether the action discovers inputs.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:129 ActionCache.Entry.discoversInputs
    pub fn discovers_inputs(&self) -> bool {
        check_state!(!self.is_corrupted());
        self.discovered_input_paths.is_some()
    }

    /// Returns the list of discovered input paths, or `None` if the action does not discover
    /// inputs.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:138 ActionCache.Entry.getDiscoveredInputPaths
    pub fn get_discovered_input_paths(&self) -> Option<&[String]> {
        check_state!(!self.is_corrupted());
        self.discovered_input_paths.as_deref()
    }

    /// Gets the metadata of an output file.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:145 ActionCache.Entry.getOutputFile
    pub fn get_output_file(&self, output: &dyn ActionInput) -> Option<&FileArtifactValue> {
        check_state!(!self.is_corrupted());
        self.output_file_metadata
            .get(output.get_exec_path_string().as_str())
    }

    /// Gets the metadata of all output files.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:151 ActionCache.Entry.getOutputFiles
    pub fn get_output_files(&self) -> &IndexMap<String, FileArtifactValue> {
        check_state!(!self.is_corrupted());
        &self.output_file_metadata
    }

    /// Gets the metadata of an output tree.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:158 ActionCache.Entry.getOutputTree
    pub fn get_output_tree(
        &self,
        output: &dyn ActionInput,
    ) -> Option<&SerializableTreeArtifactValue> {
        check_state!(!self.is_corrupted());
        self.output_tree_metadata
            .get(output.get_exec_path_string().as_str())
    }

    /// Gets the metadata of all output trees.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:164 ActionCache.Entry.getOutputTrees
    pub fn get_output_trees(&self) -> &IndexMap<String, SerializableTreeArtifactValue> {
        check_state!(!self.is_corrupted());
        &self.output_tree_metadata
    }

    /// Returns a list of exec path strings for proxied outputs.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:172 ActionCache.Entry.getProxyOutputs
    pub fn get_proxy_outputs(&self) -> &[String] {
        check_state!(!self.is_corrupted());
        &self.proxy_outputs
    }

    /// Returns whether this entry stores any output metadata.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:178 ActionCache.Entry.hasOutputMetadata
    pub fn has_output_metadata(&self) -> bool {
        check_state!(!self.is_corrupted());
        !self.output_file_metadata.is_empty()
            || !self.output_tree_metadata.is_empty()
            || !self.proxy_outputs.is_empty()
    }

    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:196 ActionCache.Entry.dump
    pub(crate) fn dump(&self, out: &mut dyn Write) {
        // PORT: PrintStream swallows write errors.
        if self.is_corrupted() {
            let _ = writeln!(out, "  CORRUPTED");
            return;
        }
        let _ = writeln!(
            out,
            "  digest = {}",
            ActionCacheEntry::format_digest(self.digest.as_deref().unwrap())
        );
        if let Some(discovered_input_paths) = &self.discovered_input_paths {
            let _ = writeln!(out, "  discoveredInputPaths =");
            let mut sorted = discovered_input_paths.clone();
            sorted.sort();
            for path in sorted {
                let _ = writeln!(out, "    {}", path);
            }
        }

        if !self.output_file_metadata.is_empty() {
            let _ = writeln!(out, "  outputFileMetadata =");
            let mut sorted: Vec<&String> = self.output_file_metadata.keys().collect();
            sorted.sort();
            for path in sorted {
                let _ = writeln!(
                    out,
                    "    {} = {}",
                    path,
                    file_artifact_value_to_string(&self.output_file_metadata[path])
                );
            }
        }

        if !self.output_tree_metadata.is_empty() {
            let _ = writeln!(out, "  outputTreeMetadata =");
            let mut sorted: Vec<&String> = self.output_tree_metadata.keys().collect();
            sorted.sort();
            for path in sorted {
                let _ = writeln!(out, "    {} = {}", path, self.output_tree_metadata[path]);
            }
        }
    }

    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:224 ActionCache.Entry.formatDigest
    fn format_digest(digest: &[u8]) -> String {
        hex::encode(digest)
    }
}

/// Guava `MoreObjects.toStringHelper` renders a `byte[]` with `Arrays.deepToString`.
fn signed_bytes_to_string(bytes: &[u8]) -> String {
    let parts: Vec<String> = bytes.iter().map(|b| (*b as i8).to_string()).collect();
    format!("[{}]", parts.join(", "))
}

fn string_list_to_string(list: &[String]) -> String {
    format!("[{}]", list.join(", "))
}

// Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:186 ActionCache.Entry.toString
impl fmt::Display for ActionCacheEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let files: Vec<String> = self
            .output_file_metadata
            .iter()
            .map(|(k, v)| format!("{}={}", k, file_artifact_value_to_string(v)))
            .collect();
        let trees: Vec<String> = self
            .output_tree_metadata
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        write!(
            f,
            "Entry{{digest={}, discoveredInputPaths={}, outputFileMetadata={{{}}}, outputTreeMetadata={{{}}}, proxyOutputs={}}}",
            match &self.digest {
                Some(d) => signed_bytes_to_string(d),
                None => "null".to_string(),
            },
            match &self.discovered_input_paths {
                Some(p) => string_list_to_string(p),
                None => "null".to_string(),
            },
            files.join(", "),
            trees.join(", "),
            string_list_to_string(&self.proxy_outputs)
        )
    }
}

/// Serializable representation of [`TreeArtifactValue`] (`ActionCache.Entry
/// .SerializableTreeArtifactValue`).
// PORT: named without its `ActionCacheEntry` prefix; the inner name is already unique.
// Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:229 ActionCache.Entry.SerializableTreeArtifactValue
#[derive(Clone, Debug, PartialEq)]
pub struct SerializableTreeArtifactValue {
    pub child_values: IndexMap<String, FileArtifactValue>,
    pub archived_file_value: Option<FileArtifactValue>,
    pub resolved_path: Option<PathFragment>,
}

impl SerializableTreeArtifactValue {
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:233 ActionCache.Entry.SerializableTreeArtifactValue.<init>
    pub fn new(
        child_values: IndexMap<String, FileArtifactValue>,
        archived_file_value: Option<FileArtifactValue>,
        resolved_path: Option<PathFragment>,
    ) -> SerializableTreeArtifactValue {
        SerializableTreeArtifactValue {
            child_values,
            archived_file_value,
            resolved_path,
        }
    }

    /// `childValues()`.
    pub fn child_values(&self) -> &IndexMap<String, FileArtifactValue> {
        &self.child_values
    }

    /// `archivedFileValue()`.
    pub fn archived_file_value(&self) -> Option<&FileArtifactValue> {
        self.archived_file_value.as_ref()
    }

    /// `resolvedPath()`.
    pub fn resolved_path(&self) -> Option<&PathFragment> {
        self.resolved_path.as_ref()
    }

    /// Creates a [`SerializableTreeArtifactValue`] from a [`TreeArtifactValue`] by collecting
    /// children and archived artifact which are remote.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:243 ActionCache.Entry.SerializableTreeArtifactValue.create
    pub fn create(tree_metadata: &TreeArtifactValue) -> SerializableTreeArtifactValue {
        let mut child_values = IndexMap::new();
        for (child, value) in &tree_metadata.child_values {
            // Only save remote tree file
            if !value.is_remote() {
                continue;
            }
            let tree_relative_path = child
                .as_artifact()
                .and_then(|a| a.get_parent_relative_path())
                .unwrap_or_else(|| {
                    panic!("tree artifact child {} is not a TreeFileArtifact", child)
                });
            let key = tree_relative_path.get_path_string().to_string();
            if child_values.insert(key.clone(), value.clone()).is_some() {
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {}",
                    key
                );
            }
        }

        // Only save remote archived artifact
        let archived_file_value = tree_metadata
            .archived_representation
            .as_ref()
            .filter(|(_, archived_file_value)| archived_file_value.is_remote())
            .map(|(_, archived_file_value)| archived_file_value.clone());

        let resolved_path = tree_metadata.resolved_path.clone();

        SerializableTreeArtifactValue::new(child_values, archived_file_value, resolved_path)
    }
}

// Java record toString: `SerializableTreeArtifactValue[childValues=..., ...]`.
impl fmt::Display for SerializableTreeArtifactValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let children: Vec<String> = self
            .child_values
            .iter()
            .map(|(k, v)| format!("{}={}", k, file_artifact_value_to_string(v)))
            .collect();
        write!(
            f,
            "SerializableTreeArtifactValue[childValues={{{}}}, archivedFileValue={}, resolvedPath={}]",
            children.join(", "),
            match &self.archived_file_value {
                Some(v) => format!("Optional[{}]", file_artifact_value_to_string(v)),
                None => "Optional.empty".to_string(),
            },
            match &self.resolved_path {
                Some(p) => format!("Optional[{}]", p),
                None => "Optional.empty".to_string(),
            }
        )
    }
}

/// A builder for an action cache entry (`ActionCache.Entry.Builder`).
///
/// Artifacts are passed as `&dyn ActionInput` whose `as_artifact()` is `Some`.
// Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:266 ActionCache.Entry.Builder
pub struct ActionCacheEntryBuilder {
    action_key: String,

    // Combined input and output metadata.
    metadata_map: HashMap<String, Option<FileArtifactValue>>,

    client_env: IndexMap<String, String>,

    action_execution_salt: String,

    // Discovered inputs.
    // None if the action does not discover inputs.
    discovered_input_paths: Option<Vec<String>>,

    // Java ImmutableMap.Builder (duplicate keys fail in build()).
    output_file_metadata: Vec<(String, FileArtifactValue)>,
    output_tree_metadata: Vec<(String, SerializableTreeArtifactValue)>,

    proxy_outputs: Vec<String>,

    // Settings that affect the outcome of an action but aren't captured in the file metadata.
    output_permissions: OutputPermissions,
    use_archived_tree_artifacts: bool,
}

fn is_tree_artifact(input: &dyn ActionInput) -> bool {
    input.as_artifact().is_some_and(|a| a.is_tree_artifact())
}

fn is_child_of_declared_directory(input: &dyn ActionInput) -> bool {
    input
        .as_artifact()
        .is_some_and(|a| a.is_child_of_declared_directory())
}

impl ActionCacheEntryBuilder {
    /// Creates a new builder.
    ///
    /// `discovers_inputs`: whether the action discovers inputs; `output_permissions`: the
    /// requested output permissions; `use_archived_tree_artifacts`: whether archived tree
    /// artifacts are enabled.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:298 ActionCache.Entry.Builder.<init>
    pub fn new(
        action_key: impl Into<String>,
        discovers_inputs: bool,
        client_env: IndexMap<String, String>,
        action_execution_salt: impl Into<String>,
        output_permissions: OutputPermissions,
        use_archived_tree_artifacts: bool,
    ) -> ActionCacheEntryBuilder {
        ActionCacheEntryBuilder {
            action_key: action_key.into(),
            metadata_map: HashMap::new(),
            client_env,
            action_execution_salt: action_execution_salt.into(),
            discovered_input_paths: if discovers_inputs {
                Some(Vec::new())
            } else {
                None
            },
            output_file_metadata: Vec::new(),
            output_tree_metadata: Vec::new(),
            proxy_outputs: Vec::new(),
            output_permissions,
            use_archived_tree_artifacts,
        }
    }

    /// Adds metadata of an input file.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:315 ActionCache.Entry.Builder.addInputFile
    pub fn add_input_file(
        &mut self,
        artifact: &dyn ActionInput,
        metadata: Option<FileArtifactValue>,
    ) -> &mut Self {
        self.add_input_file_save_exec_path(artifact, metadata, /* save_exec_path= */ false)
    }

    /// Adds metadata of an input file (`addInputFile(artifact, metadata, saveExecPath)`).
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:322 ActionCache.Entry.Builder.addInputFile
    pub fn add_input_file_save_exec_path(
        &mut self,
        artifact: &dyn ActionInput,
        metadata: Option<FileArtifactValue>,
        save_exec_path: bool,
    ) -> &mut Self {
        let exec_path = artifact.get_exec_path_string();
        if save_exec_path && let Some(discovered_input_paths) = &mut self.discovered_input_paths {
            discovered_input_paths.push(exec_path.clone());
        }
        self.metadata_map.insert(exec_path, metadata);
        self
    }

    /// Adds an output file.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:334 ActionCache.Entry.Builder.addOutputFile
    pub fn add_output_file(
        &mut self,
        output: &dyn ActionInput,
        metadata: FileArtifactValue,
    ) -> &mut Self {
        self.add_output_file_save_metadata(output, metadata, /* save_file_metadata= */ false)
    }

    /// Adds an output file (`addOutputFile(output, metadata, saveFileMetadata)`).
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:340 ActionCache.Entry.Builder.addOutputFile
    pub fn add_output_file_save_metadata(
        &mut self,
        output: &dyn ActionInput,
        metadata: FileArtifactValue,
        save_file_metadata: bool,
    ) -> &mut Self {
        check_argument!(
            !is_tree_artifact(output) && !is_child_of_declared_directory(output),
            "Must use addOutputTree to save tree artifacts and their children: {}",
            output.get_exec_path_string()
        );
        let exec_path = output.get_exec_path_string();
        // Only save remote and proxy file metadata.
        if save_file_metadata {
            if metadata.is_remote() {
                self.output_file_metadata
                    .push((exec_path.clone(), metadata.clone()));
            } else if is_proxy(&metadata) {
                self.proxy_outputs.push(exec_path.clone());
            }
        }
        self.metadata_map.insert(exec_path, Some(metadata));
        self
    }

    /// Adds an output tree.
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:361 ActionCache.Entry.Builder.addOutputTree
    pub fn add_output_tree(
        &mut self,
        output: &dyn ActionInput,
        metadata: &TreeArtifactValue,
    ) -> &mut Self {
        self.add_output_tree_save_metadata(output, metadata, /* save_tree_metadata= */ false)
    }

    /// Adds an output tree (`addOutputTree(output, metadata, saveTreeMetadata)`).
    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:367 ActionCache.Entry.Builder.addOutputTree
    pub fn add_output_tree_save_metadata(
        &mut self,
        output: &dyn ActionInput,
        metadata: &TreeArtifactValue,
        save_tree_metadata: bool,
    ) -> &mut Self {
        check_argument!(
            is_tree_artifact(output),
            "artifact must be a tree artifact: {}",
            output.get_exec_path_string()
        );
        let exec_path = output.get_exec_path_string();
        if save_tree_metadata {
            if !metadata.child_values.is_empty()
                && metadata.child_values.iter().all(|(_, v)| is_proxy(v))
            {
                self.proxy_outputs.push(output.get_exec_path_string());
            } else {
                self.output_tree_metadata.push((
                    exec_path.clone(),
                    SerializableTreeArtifactValue::create(metadata),
                ));
            }
        }
        // Java: TreeArtifactValue.getMetadata() is never null.
        let tree_metadata = metadata.metadata.clone().unwrap_or_else(|| {
            panic!(
                "TreeArtifactValue.getMetadata() is missing for {}",
                exec_path
            )
        });
        self.metadata_map.insert(exec_path, Some(tree_metadata));
        self
    }

    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:384 ActionCache.Entry.Builder.build
    pub fn build(&self) -> ActionCacheEntry {
        ActionCacheEntry::new(
            Some(ActionCacheEntryBuilder::compute_digest(
                &self.action_key,
                self.discovered_input_paths.is_some(),
                &self.metadata_map,
                &self.client_env,
                &self.action_execution_salt,
                self.output_permissions,
                self.use_archived_tree_artifacts,
            )),
            self.discovered_input_paths.clone(),
            build_or_throw(&self.output_file_metadata),
            build_or_throw(&self.output_tree_metadata),
            self.proxy_outputs.clone(),
        )
    }

    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:400 ActionCache.Entry.Builder.computeDigest
    fn compute_digest(
        action_key: &str,
        discovers_inputs: bool,
        metadata_map: &HashMap<String, Option<FileArtifactValue>>,
        client_env: &IndexMap<String, String>,
        action_execution_salt: &str,
        output_permissions: OutputPermissions,
        use_archived_tree_artifacts: bool,
    ) -> Vec<u8> {
        let mut fp = Fingerprint::new();
        // PORT: StringEncoding internal form (Java strings here are in Bazel's internal encoding).
        fp.add_internal_string(action_key);
        fp.add_boolean(discovers_inputs);
        fp.add_bytes(&MetadataDigestUtils::from_metadata(metadata_map));
        fp.add_bytes(&ActionCacheEntryBuilder::compute_map_digest(client_env));
        fp.add_internal_string(action_execution_salt);
        fp.add_int(output_permissions.get_permissions_mode());
        fp.add_boolean(use_archived_tree_artifacts);
        fp.digest_and_reset()
    }

    // Java: com/google/devtools/build/lib/actions/cache/ActionCache.java:419 ActionCache.Entry.Builder.computeMapDigest
    fn compute_map_digest(map: &IndexMap<String, String>) -> Vec<u8> {
        let mut result = Vec::new();
        let mut fp = Fingerprint::new();
        for (key, value) in map {
            fp.add_internal_string(key);
            fp.add_internal_string(value);
            result = DigestUtils::combine_unordered(result, fp.digest_and_reset());
        }
        result
    }
}

/// Guava `ImmutableMap.Builder.buildOrThrow`: fails on duplicate keys.
fn build_or_throw<V: Clone>(entries: &[(String, V)]) -> IndexMap<String, V> {
    let mut map = IndexMap::with_capacity(entries.len());
    for (key, value) in entries {
        if map.insert(key.clone(), value.clone()).is_some() {
            panic!(
                "IllegalArgumentException: Multiple entries with same key: {}",
                key
            );
        }
    }
    map
}
