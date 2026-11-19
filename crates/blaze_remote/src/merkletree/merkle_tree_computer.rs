//! Port of `lib/remote/merkletree/MerkleTreeComputer.java`.

use std::any::Any;
use std::borrow::Borrow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io;
use std::sync::{Arc, LazyLock, Weak};

use blaze_actions::Artifact as BlazeArtifact;
use blaze_collect::NestedSet;
use blaze_exec::actions::{
    ActionInput, ActionInputRef, FileArtifactValue, FileStateType, FilesetOutputTree,
    HierarchicalPath, InputMetadataProvider, PathMapper, RunfilesArtifactValue, Spawn,
    TreeArtifactValue, empty_marker,
};
use blaze_exec::exec::SpawnExecutionContext;
use blaze_exec::exec::action_execution_context::SpawnCacheError;
use blaze_protos::JavaMessage;
use blaze_vfs::{DirentType, Path, PathFragment, Symlinks};
use futures::future::LocalBoxFuture;
use parking_lot::Mutex;

use super::merkle_tree::{
    BlobKey, MerkleTree, MerkleTreeBlob, MerkleTreeRootOnly, MerkleTreeUploadable,
};
use super::merkle_tree_uploader::MerkleTreeUploader;
use crate::common::bulk_transfer_exception::BulkTransferException;
use crate::common::errors;
use crate::common::remote_action_execution_context::{CachePolicy, RemoteActionExecutionContext};
use crate::common::remote_path_resolver::RemotePathResolver;
use crate::reapi::{
    Digest, Directory, DirectoryNode, FileNode, NodeProperties, NodeProperty, SymlinkNode,
};
use crate::runtime::get_from_future;
use crate::scrubber::{Scrubber, SpawnScrubber};
use crate::util::digest_util::DigestUtil;
use crate::util::tracing_metadata_utils;

// This class achieves its goals via the following observations:
//
// * Incremental builds typically have a large number of cache hits and the only information about
//   a Merkle tree needed to check for a cache hit is the root digest. We can thus avoid
//   materializing the full tree unless the cache check fails.
// * Certain special artifacts are known to form self-contained Merkle trees that never intersect
//   with any other Merkle tree. This includes tree artifacts, runfiles directories and source
//   directories. The Merkle trees for such artifacts can be computed and uploaded to the remote
//   cache independently. Only their root digest has to be kept around for inclusion in other
//   Merkle trees.
// * FileArtifactValue's fully describe their contents and can thus be used as cache keys for
//   inter-build caches. By using a weak reference for the key, they can be cleaned up
//   automatically as soon as their contents change or they are no longer relevant.
// * Instead of basing caching decisions on a particular remote cache TTL, we can optimistically
//   assume that every blob that has been uploaded is still in the cache if we support a mode in
//   which all blobs can be forcibly recomputed and re-uploaded on missing digests.
// * While the inputs of a spawn naturally form a map of paths to contents, this map doesn't have
//   to be materialized in memory. Instead, it suffices to maintain a list of inputs that is
//   sorted by lazily computed paths. This drastically reduces peak memory usage.
// * When visiting the inputs of a spawn in hierarchical order, once a directory is left once,
//   it will never be entered again. At that point, the proto describing it can be built and
//   digested and intermediate structures are no longer needed.

/// An entry of the sorted inputs of a (sub-)tree (Java: `Map.Entry<PathFragment, ? extends
/// ActionInput>`). The input is `None` for an empty runfile.
///
/// PORT: Java computes the paths of a spawn's inputs lazily (`Lists.transform`) to reduce peak
/// memory usage; here the entries are materialized.
type SortedInput = (PathFragment, Option<ActionInputRef>);

/// `Predicate<PathFragment> isToolInput`.
type IsToolInput<'a> = dyn Fn(&PathFragment) -> bool + 'a;

/// The future of the root of a sub-tree (Java: `ListenableFuture<MerkleTree.RootOnly>`).
type SubTreeFuture<'a> = LocalBoxFuture<'a, io::Result<MerkleTreeRootOnly>>;

/// `SortedInputsSupplier`.
// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:920 MerkleTreeComputer.SortedInputsSupplier
type SortedInputsSupplier<'a> = Box<dyn FnOnce() -> io::Result<Vec<SortedInput>> + 'a>;

// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:140 MerkleTreeComputer.TOOL_NODE_PROPERTIES
fn tool_node_properties() -> NodeProperties {
    NodeProperties {
        properties: vec![NodeProperty {
            name: "bazel_tool_input".to_string(),
            value: String::new(),
        }],
        mtime: None,
        unix_mode: None,
    }
}

// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:144 MerkleTreeComputer.END_OF_INPUTS_SENTINEL
fn end_of_inputs_sentinel() -> SortedInput {
    (PathFragment::empty_fragment(), Some(empty_marker()))
}

// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:146 MerkleTreeComputer.ROOT_FAKE_PATH_SEGMENT
fn root_fake_path_segment() -> PathFragment {
    PathFragment::create("root")
}

// PORT: MERKLE_TREE_BUILD_POOL and MERKLE_TREE_UPLOAD_POOL (Java: MerkleTreeComputer.java:150 and
// :161) do not exist: the trees are built and uploaded by futures driven by the thread that
// calls `build_for_spawn` / `build_for_files` (docs/remote.md, "Futures").

/// The key of the persistent sub-tree caches (Java: `Object cacheKey`).
///
/// PORT: Java's caches have weak, identity-compared keys. The value that Java uses as key is
/// kept as follows:
/// - a `TreeArtifactValue` and a `RunfilesArtifactValue` (Java keys the latter by its
///   `getMetadata()` field, which lives exactly as long as the value) are handed out as `Arc`s by
///   the `InputMetadataProvider`: the key is a `Weak` of that `Arc`, compared by address, so it is
///   a weak identity key as in Java;
/// - the `FileArtifactValue` of a source directory is handed out by value, so there is no
///   instance to identify: weak-key cache → value-keyed map, cleared when the scrubber changes.
///   `FileArtifactValue` is `Eq` but not `Hash`, and a `DirectoryArtifactValue` is equal to
///   every other one with the same mtime, so the key is the value together with the path of the
///   directory (which is what is hashed).
enum SubTreeCacheKey {
    TreeArtifactValue(Weak<TreeArtifactValue>),
    RunfilesArtifactValue(Weak<RunfilesArtifactValue>),
    SourceDirectory {
        metadata: FileArtifactValue,
        path: Path,
    },
}

impl SubTreeCacheKey {
    /// Whether the weakly referenced key is still reachable.
    fn is_live(&self) -> bool {
        match self {
            SubTreeCacheKey::TreeArtifactValue(value) => value.strong_count() > 0,
            SubTreeCacheKey::RunfilesArtifactValue(value) => value.strong_count() > 0,
            SubTreeCacheKey::SourceDirectory { .. } => true,
        }
    }
}

impl PartialEq for SubTreeCacheKey {
    fn eq(&self, other: &SubTreeCacheKey) -> bool {
        match (self, other) {
            (SubTreeCacheKey::TreeArtifactValue(a), SubTreeCacheKey::TreeArtifactValue(b)) => {
                Weak::ptr_eq(a, b)
            }
            (
                SubTreeCacheKey::RunfilesArtifactValue(a),
                SubTreeCacheKey::RunfilesArtifactValue(b),
            ) => Weak::ptr_eq(a, b),
            (
                SubTreeCacheKey::SourceDirectory {
                    metadata: metadata1,
                    path: path1,
                },
                SubTreeCacheKey::SourceDirectory {
                    metadata: metadata2,
                    path: path2,
                },
            ) => path1 == path2 && metadata1 == metadata2,
            _ => false,
        }
    }
}

impl Eq for SubTreeCacheKey {}

impl Hash for SubTreeCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            SubTreeCacheKey::TreeArtifactValue(value) => {
                0u8.hash(state);
                (Weak::as_ptr(value) as usize).hash(state);
            }
            SubTreeCacheKey::RunfilesArtifactValue(value) => {
                1u8.hash(state);
                (Weak::as_ptr(value) as usize).hash(state);
            }
            SubTreeCacheKey::SourceDirectory { path, .. } => {
                2u8.hash(state);
                path.hash(state);
            }
        }
    }
}

/// `Cache<Object, MerkleTree.RootOnly>` built with `Caffeine.newBuilder().weakKeys()`.
struct SubTreeCache {
    inner: Mutex<SubTreeCacheInner>,
}

struct SubTreeCacheInner {
    map: HashMap<SubTreeCacheKey, MerkleTreeRootOnly>,
    /// The size at which the entries of collected keys are removed next (Caffeine removes them
    /// during its maintenance).
    purge_at: usize,
}

impl SubTreeCache {
    const INITIAL_PURGE_AT: usize = 64;

    fn new() -> SubTreeCache {
        SubTreeCache {
            inner: Mutex::new(SubTreeCacheInner {
                map: HashMap::new(),
                purge_at: SubTreeCache::INITIAL_PURGE_AT,
            }),
        }
    }

    /// `invalidateAll()`.
    fn invalidate_all(&self) {
        let mut inner = self.inner.lock();
        inner.map.clear();
        inner.purge_at = SubTreeCache::INITIAL_PURGE_AT;
    }

    /// `invalidate(key)`.
    fn invalidate(&self, key: &SubTreeCacheKey) {
        self.inner.lock().map.remove(key);
    }

    /// `getIfPresent(key)`.
    fn get_if_present(&self, key: &SubTreeCacheKey) -> Option<MerkleTreeRootOnly> {
        self.inner.lock().map.get(key).cloned()
    }

    /// `asMap().compute(key, remappingFunction)`.
    fn compute(
        &self,
        key: SubTreeCacheKey,
        remapping_function: impl FnOnce(Option<&MerkleTreeRootOnly>) -> MerkleTreeRootOnly,
    ) {
        let mut inner = self.inner.lock();
        if inner.map.len() >= inner.purge_at {
            inner.map.retain(|key, _| key.is_live());
            inner.purge_at = std::cmp::max(SubTreeCache::INITIAL_PURGE_AT, inner.map.len() * 2);
        }
        let new_root = remapping_function(inner.map.get(&key));
        inner.map.insert(key, new_root);
    }
}

// Keyed on FileArtifactValue-like object describing the contents of the subtree. Since weak keys
// imply identity comparison, the key type can't be just FileArtifactValue as not every aggregate
// input has a FileArtifactValue that is retained alongside its contents.
// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:168 MerkleTreeComputer.persistentToolSubTreeCache
static PERSISTENT_TOOL_SUB_TREE_CACHE: LazyLock<SubTreeCache> = LazyLock::new(SubTreeCache::new);
// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:170 MerkleTreeComputer.persistentNonToolSubTreeCache
static PERSISTENT_NON_TOOL_SUB_TREE_CACHE: LazyLock<SubTreeCache> =
    LazyLock::new(SubTreeCache::new);

// PORT: Java guards writes with `MerkleTreeComputer.class` and reads with double-checked locking;
// here every access takes the lock.
// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:174 MerkleTreeComputer.lastScrubber
static LAST_SCRUBBER: LazyLock<Mutex<Option<Scrubber>>> = LazyLock::new(|| Mutex::new(None));

/// `ArtifactPathResolver`, as far as this package uses it (`toPath`).
// Java: com/google/devtools/build/lib/actions/ArtifactPathResolver.java:30 ArtifactPathResolver
#[derive(Clone, Debug)]
pub(super) enum ArtifactPathResolver {
    /// `ArtifactPathResolver.forExecRoot(execRoot)`.
    ///
    /// PORT: ArtifactPathResolver is the exec root (missing hook:
    /// SpawnExecutionContext.getPathResolver); a spawn that runs on an action file system would
    /// have a `TransformResolver` in Java.
    ExecRoot(Path),
    /// `MerkleTreeComputer.PATH_ACTION_INPUT_RESOLVER`.
    PathActionInput,
}

// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:459 MerkleTreeComputer.PATH_ACTION_INPUT_RESOLVER
pub(super) const PATH_ACTION_INPUT_RESOLVER: ArtifactPathResolver =
    ArtifactPathResolver::PathActionInput;

impl ArtifactPathResolver {
    /// `toPath(actionInput)`.
    pub(super) fn to_path(&self, action_input: &dyn ActionInput) -> Path {
        artifact_path_resolver_to_path(self, action_input)
    }
}

/// `artifactPathResolver.toPath(actionInput)`.
fn artifact_path_resolver_to_path(
    artifact_path_resolver: &ArtifactPathResolver,
    action_input: &dyn ActionInput,
) -> Path {
    match artifact_path_resolver {
        // Java: com/google/devtools/build/lib/actions/ArtifactPathResolver.java:83 ArtifactPathResolver.IdentityResolver.toPath
        ArtifactPathResolver::ExecRoot(exec_root) => {
            // PORT: `actionInput instanceof Artifact artifact` is a `blaze_actions` artifact;
            // any other implementation of the execution layer's `Artifact` trait has no
            // `getPath()` and is resolved against the exec root like a plain input.
            if let Some(artifact) = action_input.as_any().downcast_ref::<BlazeArtifact>() {
                return artifact.get_path();
            }
            exec_root.get_relative(action_input.get_exec_path())
        }
        // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:462 MerkleTreeComputer.PATH_ACTION_INPUT_RESOLVER.toPath
        ArtifactPathResolver::PathActionInput => {
            match action_input.as_any().downcast_ref::<PathActionInput>() {
                Some(path_action_input) => path_action_input.get_path().clone(),
                None => panic!(
                    "ClassCastException: {action_input:?} cannot be cast to MerkleTreeComputer.PathActionInput"
                ),
            }
        }
    }
}

/// `artifact.getRunfilesPath()`.
// Java: com/google/devtools/build/lib/actions/Artifact.java:636 Artifact.getRunfilesPath
fn artifact_get_runfiles_path(artifact: &ActionInputRef) -> PathFragment {
    artifact
        .as_artifact()
        .expect("ClassCastException: not an Artifact")
        .get_runfiles_path()
}

/// `TreeFileArtifact.getParentRelativePath()`.
fn tree_file_artifact_get_parent_relative_path(child: &ActionInputRef) -> PathFragment {
    match child
        .as_artifact()
        .and_then(|artifact| artifact.get_parent_relative_path())
    {
        Some(parent_relative_path) => parent_relative_path,
        None => panic!("ClassCastException: {child} cannot be cast to Artifact.TreeFileArtifact"),
    }
}

/// `StaticInputMetadataProvider.empty()`.
// Java: com/google/devtools/build/lib/actions/StaticInputMetadataProvider.java StaticInputMetadataProvider.empty
struct EmptyInputMetadataProvider;

impl InputMetadataProvider for EmptyInputMetadataProvider {
    fn get_input_metadata(&self, _input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        Ok(None)
    }
    fn get_tree_metadata(&self, _input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        None
    }
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        None
    }
    fn get_runfiles_metadata(&self, _input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        None
    }
    fn get_input(&self, _exec_path: &PathFragment) -> Option<ActionInputRef> {
        None
    }
}

fn always_true(_path: &PathFragment) -> bool {
    true
}

fn always_false(_path: &PathFragment) -> bool {
    false
}

/// `Predicates.alwaysTrue()`.
static ALWAYS_TRUE: fn(&PathFragment) -> bool = always_true;

/// `Predicates.alwaysFalse()`.
static ALWAYS_FALSE: fn(&PathFragment) -> bool = always_false;

/// Computes a Merkle tree for a set of inputs, as expected by `Action#getInputRootDigest()`.
///
/// Remote execution should allow a developer to run up to thousands of actions remotely in
/// parallel on a regular machine. As a result, this class is optimized with the following goals
/// in the order of decreasing importance:
///
/// - Above all else, keep peak memory usage as low as possible so that Bazel doesn't OOM when
///   running many remote execution actions in parallel. Allocations with sizes linear in the
///   number of spawn inputs should be avoided if possible and kept as small as possible.
/// - Make incremental builds as fast as possible.
/// - Keep the size of caches kept between builds as small as possible.
// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:116 MerkleTreeComputer
pub struct MerkleTreeComputer {
    digest_util: DigestUtil,
    merkle_tree_uploader: Option<Arc<dyn MerkleTreeUploader>>,
    build_request_id: String,
    command_id: String,
    workspace_name: String,
    empty_digest: Digest,
    empty_tree: MerkleTreeUploadable,
    // PORT: no in-flight deduplication; sub-trees are computed on the calling task
    // (`inFlightComputations` and `inFlightComputationSequence` do not exist).
    //
    // PORT: ArtifactPathResolver is the exec root (missing hook:
    // SpawnExecutionContext.getPathResolver).
    exec_root: Path,
}

/// Specifies which blobs should be retained in the Merkle tree.
///
/// The constants are ordered from the weakest to the strongest policy: the result of a
/// computation performed with a given policy also satisfies every preceding one. Deduplication of
/// ongoing sub-Merkle tree computations relies on this order to decide whether an ongoing
/// computation can be joined.
// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:213 MerkleTreeComputer.BlobPolicy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BlobPolicy {
    /// No blobs are retained and the returned MerkleTree is a `MerkleTree.RootOnly`.
    ///
    /// This is the most lightweight policy. It always suffices when checking for a remote cache
    /// hit and may suffice for remote execution when a cache hit is expected.
    Discard,

    /// Retains all blobs in the tree that aren't contained in subtrees that have already been
    /// uploaded.
    ///
    /// Only blobs that have been uploaded to the remote cache during the lifetime of the Bazel
    /// server are omitted from this tree. This usually suffices for remote execution unless the
    /// remote cache loses entries.
    Keep,

    /// Retains all blobs in the tree and also forces the reupload of all subtrees.
    ///
    /// This is only needed in exceptional cases, such as when the remote cache has lost entries
    /// while the Bazel server is running.
    KeepAndReupload,
}

// PORT: no in-flight deduplication; sub-trees are computed on the calling task. Java's
// `InFlightCacheKey` (MerkleTreeComputer.java:254), `InFlightAttributes` (:269) and `canJoin`
// (:275) only serve the `TaskDeduplicator` and are not ported.

impl MerkleTreeComputer {
    /// `new MerkleTreeComputer(digestUtil, remoteExecutionCache, buildRequestId, commandId,
    /// workspaceName)`.
    ///
    /// PORT: takes the exec root, which stands for the `ArtifactPathResolver` of the spawn
    /// execution contexts (missing hook: SpawnExecutionContext.getPathResolver).
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:187 MerkleTreeComputer.<init>
    pub fn new(
        digest_util: DigestUtil,
        remote_execution_cache: Option<Arc<dyn MerkleTreeUploader>>,
        build_request_id: &str,
        command_id: &str,
        workspace_name: &str,
        exec_root: Path,
    ) -> MerkleTreeComputer {
        let empty_blob: [u8; 0] = [];
        let empty_digest = digest_util.compute(&empty_blob);
        let empty_tree = MerkleTreeUploadable::new(
            MerkleTreeRootOnly::blobs_uploaded(empty_digest.clone(), 0, 0),
            BTreeMap::new(),
            ArtifactPathResolver::ExecRoot(exec_root.clone()),
        );
        MerkleTreeComputer {
            digest_util,
            merkle_tree_uploader: remote_execution_cache,
            build_request_id: build_request_id.to_string(),
            command_id: command_id.to_string(),
            workspace_name: workspace_name.to_string(),
            empty_digest,
            empty_tree,
            exec_root,
        }
    }

    /// Builds a Merkle tree for the inputs of a `Spawn`.
    ///
    /// * `tool_inputs`: the set of paths of inputs that are considered tools. Note that these
    ///   paths are not exec paths, but those returned as keys by
    ///   `SpawnInputExpander#getInputMapping`, i.e., they have already been subject to path
    ///   mapping and runfiles tree as well as tree artifact expansion. Callers have to ensure
    ///   that paths within an aggregate artifact are either all tools or all non-tools.
    /// * `scrubber`: the invocation-global scrubber, or `None` if no scrubbing should be
    ///   performed.
    /// * `blob_policy`: used to decide which blobs should be retained in the returned Merkle
    ///   tree. If `KeepAndReupload` is used, all blobs in the tree are retained and the
    ///   resulting Merkle tree will be a `MerkleTree::Uploadable`.
    ///
    /// Fails with a `LostInputsExecException` if inputs to this spawn that are remote-only have
    /// been discovered to be missing from the remote cache. Action or build rewinding may be able
    /// to recover from this.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:307 MerkleTreeComputer.buildForSpawn
    pub fn build_for_spawn(
        &self,
        spawn: &dyn Spawn,
        tool_inputs: &HashSet<PathFragment>,
        scrubber: Option<&Scrubber>,
        spawn_execution_context: &dyn SpawnExecutionContext,
        remote_path_resolver: &dyn RemotePathResolver,
        blob_policy: BlobPolicy,
    ) -> Result<MerkleTree, SpawnCacheError> {
        // PORT: skipped, profiling ("MerkleTreeComputer.buildForSpawn").
        self.do_build_for_spawn(
            spawn,
            tool_inputs,
            scrubber,
            spawn_execution_context,
            remote_path_resolver,
            blob_policy,
        )
    }

    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:321 MerkleTreeComputer.doBuildForSpawn
    fn do_build_for_spawn(
        &self,
        spawn: &dyn Spawn,
        tool_inputs: &HashSet<PathFragment>,
        scrubber: Option<&Scrubber>,
        spawn_execution_context: &dyn SpawnExecutionContext,
        remote_path_resolver: &dyn RemotePathResolver,
        blob_policy: BlobPolicy,
    ) -> Result<MerkleTree, SpawnCacheError> {
        // The scrubber is a per-invocation setting and invocations do not overlap, so it can be
        // tracked in a static variable.
        {
            let mut last_scrubber = LAST_SCRUBBER.lock();
            if scrubber != last_scrubber.as_ref() {
                PERSISTENT_TOOL_SUB_TREE_CACHE.invalidate_all();
                PERSISTENT_NON_TOOL_SUB_TREE_CACHE.invalidate_all();
                *last_scrubber = scrubber.cloned();
            }
        }
        let spawn_inputs = spawn.get_input_files().to_list();
        // Add output directories to inputs so that they are created as empty directories by the
        // executor. The spec only requires the executor to create the parent directory of an
        // output directory, which differs from the behavior of both local and sandboxed
        // execution.
        let output_directories: Vec<ActionInputRef> = spawn
            .get_output_files()
            .iter()
            .filter(|output| {
                output
                    .as_artifact()
                    .is_some_and(|artifact| artifact.is_tree_artifact())
            })
            .map(|output_dir| ActionInputRef::new(EmptyInputDirectory::new(output_dir.clone())))
            .collect();
        // Reduce peak memory usage by avoiding the allocation of intermediate arrays and sorted
        // map, as well as the prolonged retention of mapped paths. All of these can be
        // reconstructed on-the-fly while iterating over the inputs, only the sorted order has to
        // be retained.
        // PORT: the mapped paths are kept with the inputs (see `SortedInput`). Java's `concat`
        // (MerkleTreeComputer.java:1161) is the iterator chain.
        let mut all_inputs: Vec<SortedInput> = spawn_inputs
            .iter()
            .chain(output_directories.iter())
            .map(|input| {
                (
                    get_output_path(&**input, remote_path_resolver, spawn.get_path_mapper()),
                    Some(input.clone()),
                )
            })
            .collect();
        // `ImmutableList.sortedCopyOf` is a stable sort.
        all_inputs.sort_by(|a, b| PathFragment::hierarchical_compare(&a.0, &b.0));
        let metadata = tracing_metadata_utils::build_metadata(
            &self.build_request_id,
            &self.command_id,
            "subtree",
            Some(spawn.get_resource_owner()),
        );
        let remote_action_execution_context = RemoteActionExecutionContext::create_with_policies(
            spawn,
            spawn_execution_context,
            metadata,
            CachePolicy::RemoteCacheOnly,
            CachePolicy::NoCache,
        );
        let working_directory = remote_path_resolver.get_working_directory();
        let is_tool_input: Box<IsToolInput<'_>> =
            if tool_inputs.is_empty() || working_directory.is_empty() {
                Box::new(move |path: &PathFragment| tool_inputs.contains(path))
            } else {
                Box::new(move |path: &PathFragment| {
                    tool_inputs.contains(&path.relative_to(&working_directory))
                })
            };
        let spawn_scrubber = match scrubber {
            Some(scrubber) => scrubber.for_spawn(spawn),
            None => None,
        };
        let artifact_path_resolver = ArtifactPathResolver::ExecRoot(self.exec_root.clone());
        // PORT: Java's private `getFromFuture` (MerkleTreeComputer.java:1048) unwraps the
        // `WrappedException`s of its executor tasks; errors are `io::Error` values here.
        let result = get_from_future(self.build(
            all_inputs,
            &*is_tool_input,
            spawn_scrubber,
            spawn_execution_context.get_input_metadata_provider(),
            &artifact_path_resolver,
            Some(&remote_action_execution_context),
            Some(remote_path_resolver),
            blob_policy,
        ));
        match result {
            Ok(merkle_tree) => Ok(merkle_tree),
            Err(e) => {
                if let Some(bulk_transfer_exception) =
                    errors::downcast_ref::<BulkTransferException>(&e)
                {
                    bulk_transfer_exception
                        .get_lost_artifacts(&|exec_path: &PathFragment| {
                            spawn_execution_context
                                .get_input_metadata_provider()
                                .get_input(exec_path)
                        })
                        .throw_if_not_empty();
                }
                Err(e.into())
            }
        }
    }

    /// Builds a Merkle tree for a set of files and their logical paths.
    ///
    /// The only use outside testing is by repository rules. Use
    /// [`MerkleTreeComputer::build_for_spawn`] for everything else.
    ///
    /// PORT: `inputs` is Java's `Map<PathFragment, Path>`; any map (or its reference) can be
    /// passed.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:483 MerkleTreeComputer.buildForFiles
    pub fn build_for_files<I, K, V>(&self, inputs: I) -> io::Result<MerkleTreeUploadable>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Borrow<PathFragment>,
        V: Borrow<Path>,
    {
        // PORT: skipped, profiling ("MerkleTreeComputer.buildForFiles").
        let mut sorted_inputs: Vec<SortedInput> = inputs
            .into_iter()
            .map(|(key, value)| {
                let key: &PathFragment = Borrow::<PathFragment>::borrow(&key);
                let value: &Path = Borrow::<Path>::borrow(&value);
                (
                    key.clone(),
                    Some(ActionInputRef::new(PathActionInput::new(value.clone()))),
                )
            })
            .collect();
        sorted_inputs.sort_by(|a, b| PathFragment::hierarchical_compare(&a.0, &b.0));
        let metadata_provider = EmptyInputMetadataProvider;
        let artifact_path_resolver = PATH_ACTION_INPUT_RESOLVER;
        let merkle_tree = get_from_future(self.build(
            sorted_inputs,
            &ALWAYS_FALSE,
            /* spawn_scrubber= */ None,
            &metadata_provider,
            &artifact_path_resolver,
            /* remote_action_execution_context= */ None,
            /* remote_path_resolver= */ None,
            BlobPolicy::KeepAndReupload,
        ))?;
        // BlobPolicy.KEEP_AND_REUPLOAD always results in a MerkleTree.Uploadable.
        match merkle_tree {
            MerkleTree::Uploadable(uploadable) => Ok(uploadable),
            MerkleTree::RootOnly(_) => panic!(
                "ClassCastException: MerkleTree.RootOnly cannot be cast to MerkleTree.Uploadable"
            ),
        }
    }

    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:504 MerkleTreeComputer.build
    fn build<'a>(
        &'a self,
        sorted_inputs: Vec<SortedInput>,
        is_tool_input: &'a IsToolInput<'a>,
        spawn_scrubber: Option<&'a SpawnScrubber>,
        metadata_provider: &'a dyn InputMetadataProvider,
        artifact_path_resolver: &'a ArtifactPathResolver,
        remote_action_execution_context: Option<&'a RemoteActionExecutionContext<'a>>,
        remote_path_resolver: Option<&'a dyn RemotePathResolver>,
        blob_policy: BlobPolicy,
    ) -> LocalBoxFuture<'a, io::Result<MerkleTree>> {
        Box::pin(async move {
            let sub_tree_roots = self
                .precompute_sub_trees(
                    &sorted_inputs,
                    is_tool_input,
                    metadata_provider,
                    artifact_path_resolver,
                    remote_action_execution_context,
                    remote_path_resolver,
                    blob_policy,
                )
                .await?;
            // PORT: Java runs this on an executor (MERKLE_TREE_BUILD_POOL).
            self.build_with_precomputed_sub_trees(
                &sub_tree_roots,
                &sorted_inputs,
                is_tool_input,
                spawn_scrubber,
                metadata_provider,
                artifact_path_resolver,
                blob_policy,
            )
        })
    }

    /// PORT: `sub_tree_roots` is keyed by the position of the entry in `sorted_inputs` (Java
    /// keys the map by the entry itself).
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:542 MerkleTreeComputer.buildWithPrecomputedSubTrees
    fn build_with_precomputed_sub_trees(
        &self,
        sub_tree_roots: &HashMap<usize, MerkleTreeRootOnly>,
        sorted_inputs: &[SortedInput],
        is_tool_input: &IsToolInput<'_>,
        spawn_scrubber: Option<&SpawnScrubber>,
        metadata_provider: &dyn InputMetadataProvider,
        artifact_path_resolver: &ArtifactPathResolver,
        blob_policy: BlobPolicy,
    ) -> io::Result<MerkleTree> {
        if sorted_inputs.is_empty() {
            return Ok(MerkleTree::Uploadable(self.empty_tree.clone()));
        }

        let mut input_files: i64 = 0;
        let mut input_bytes: i64 = 0;
        let mut blobs: BTreeMap<
            /* Digest | FileArtifactValue */ BlobKey,
            /* byte[] | Path | VirtualActionInput */ MerkleTreeBlob,
        > = BTreeMap::new();
        // `Deque<Directory.Builder>` used as a stack: the top is the last element.
        let mut directory_stack: Vec<Directory> = Vec::new();
        directory_stack.push(Directory::default());

        let mut current_parent = PathFragment::empty_fragment();
        let mut last_source_dir_path: Option<PathFragment> = None;
        let mut last_entry: Option<&SortedInput> = None;
        let end_of_inputs_sentinel = end_of_inputs_sentinel();
        for (index, entry) in sorted_inputs
            .iter()
            .chain(std::iter::once(&end_of_inputs_sentinel))
            .enumerate()
        {
            // PORT: no thread interruption (Java: `Thread.interrupted()`).

            let path = &entry.0;
            // The same path may appear multiple times if the inputs are outputs of shared
            // actions, if identical inputs are not deduplicated or if path mapping maps artifacts
            // from different configurations to the same path. Only stage the first one as we
            // assume that the inputs have been validated before.
            if last_entry.is_some_and(|last_entry| *path == last_entry.0) {
                continue;
            }
            last_entry = Some(entry);
            if spawn_scrubber.is_some_and(|spawn_scrubber| spawn_scrubber.should_omit_input(path)) {
                continue;
            }
            if let Some(last_source_dir) = &last_source_dir_path {
                if path.starts_with(last_source_dir) {
                    // The input is part of a source directory that has already been added to the
                    // tree.
                    continue;
                }
                last_source_dir_path = None;
            }
            let input = entry.1.as_ref();
            let new_parent = path.get_parent_directory();
            if Some(&current_parent) != new_parent.as_ref() {
                let common_prefix: Option<PathFragment>;
                let fragment_to_pop: PathFragment;
                match &new_parent {
                    Some(new_parent) => {
                        let prefix = find_common_prefix(&current_parent, new_parent);
                        fragment_to_pop = current_parent.relative_to(&prefix);
                        common_prefix = Some(prefix);
                    }
                    None => {
                        fragment_to_pop = root_fake_path_segment().get_relative(&current_parent);
                        // Unused.
                        common_prefix = None;
                    }
                }
                for dir_to_pop in fragment_to_pop
                    .split_to_list_of_segments()
                    .into_iter()
                    .rev()
                {
                    let directory_blob = match directory_stack.pop() {
                        Some(directory) => directory.to_byte_array(),
                        None => panic!("NoSuchElementException"),
                    };
                    let directory_blob_digest = self.digest_util.compute(&directory_blob);
                    if blob_policy != BlobPolicy::Discard && directory_blob_digest.size_bytes != 0 {
                        blobs
                            .entry(BlobKey::Digest(directory_blob_digest.clone()))
                            .or_insert_with(|| {
                                MerkleTreeBlob::Bytes(bytes::Bytes::from(directory_blob))
                            });
                    }
                    input_bytes += directory_blob_digest.size_bytes;
                    let Some(top_directory) = directory_stack.last_mut() else {
                        if blob_policy == BlobPolicy::Discard {
                            // Make sure that we didn't unnecessarily retain any blobs.
                            blaze_util::check_state!(blobs.is_empty());
                            return Ok(MerkleTree::RootOnly(MerkleTreeRootOnly::blobs_discarded(
                                directory_blob_digest,
                                input_files,
                                input_bytes,
                            )));
                        } else {
                            return Ok(MerkleTree::Uploadable(MerkleTreeUploadable::new(
                                MerkleTreeRootOnly::blobs_uploaded(
                                    directory_blob_digest,
                                    input_files,
                                    input_bytes,
                                ),
                                blobs,
                                ArtifactPathResolver::ExecRoot(self.exec_root.clone()),
                            )));
                        }
                    };
                    top_directory.directories.push(DirectoryNode {
                        name: internal_to_unicode(&dir_to_pop),
                        digest: Some(directory_blob_digest),
                    });
                }
                // Java dereferences `newParent` and `commonPrefix` here; they are only null for
                // the sentinel, for which the loop above returns.
                let (Some(new_parent), Some(common_prefix)) = (new_parent, common_prefix) else {
                    panic!("NullPointerException: newParent");
                };
                for _ in 0..(new_parent.segment_count() - common_prefix.segment_count()) {
                    directory_stack.push(Directory::default());
                }
                current_parent = new_parent;
            }

            let Some(current_directory) = directory_stack.last_mut() else {
                panic!("NullPointerException");
            };
            let name = internal_to_unicode(path.get_base_name());
            let node_properties = if is_tool_input(path) {
                Some(tool_node_properties())
            } else {
                None
            };

            // This is a sentinel value for an empty file. This case only occurs when this method
            // is called from computeForRunfilesTreeIfAbsent.
            let Some(input) = input else {
                add_file(
                    current_directory,
                    name,
                    self.empty_digest.clone(),
                    node_properties,
                );
                input_files += 1;
                continue;
            };

            if let Some(artifact) = input.as_artifact() {
                if artifact.is_tree_artifact() || artifact.is_runfiles_tree() {
                    // Java: `case Artifact.SpecialArtifact specialArtifact when
                    // specialArtifact.isTreeArtifact() || specialArtifact.isRunfilesTree()`.
                    let Some(sub_tree_root) = sub_tree_roots.get(&index) else {
                        panic!("NullPointerException: missing subtree for {input}");
                    };
                    current_directory.directories.push(DirectoryNode {
                        name,
                        digest: Some(sub_tree_root.digest().clone()),
                    });
                    input_files += sub_tree_root.input_files();
                    input_bytes += sub_tree_root.input_bytes();
                } else if input.is_symlink() {
                    // Java: `case Artifact.SpecialArtifact symlink when symlink.isSymlink()`.
                    let Some(metadata) = metadata_provider.get_input_metadata(input)? else {
                        panic!("NullPointerException: missing metadata: {input}");
                    };
                    let Some(target) = metadata.get_unresolved_symlink_target() else {
                        panic!(
                            "NullPointerException: FileArtifactValue.getUnresolvedSymlinkTarget()"
                        );
                    };
                    current_directory.symlinks.push(SymlinkNode {
                        name,
                        target: internal_to_unicode(target),
                        node_properties,
                    });
                    input_files += 1;
                } else {
                    // Java: `case Artifact fileOrSourceDirectory`.
                    let Some(metadata) = metadata_provider.get_input_metadata(input)? else {
                        panic!("NullPointerException: missing metadata: {input}");
                    };
                    if metadata.get_type() == FileStateType::Directory {
                        let Some(sub_tree_root) = sub_tree_roots.get(&index) else {
                            panic!("NullPointerException: missing subtree for {input}");
                        };
                        current_directory.directories.push(DirectoryNode {
                            name,
                            digest: Some(sub_tree_root.digest().clone()),
                        });
                        input_files += sub_tree_root.input_files();
                        input_bytes += sub_tree_root.input_bytes();
                        // The source directory subsumes all children paths, which may be staged
                        // separately as individual files or subdirectories. We rely on the inputs
                        // being sorted such that a path is directly succeeded by all its
                        // children.
                        // Note that this has subtle implications for the distinction between
                        // tool/non-tool inputs:
                        // - If a directory is added as a tool, then all files in it will be
                        //   considered as tools by the worker logic and are included in the
                        //   combined worker hash. This applies even if a file in that directory
                        //   is also added as a non-tool input and thus skipping over that file
                        //   here is correct.
                        // - If a directory is added as a non-tool and a file in it is added as a
                        //   tool, then the file (but not the rest of the directory) will be
                        //   considered as a tool and included in the combined worker hash by the
                        //   worker logic. However, since the file is skipped over in the current
                        //   method, its FileNode is not marked as a tool. Since the worker hash
                        //   still tracks the file, this doesn't cause staleness issues. However,
                        //   if the RE backend relies on the bazel_tool_input NodeProperty, it
                        //   may attempt to replace the file while the worker is still running,
                        //   which can cause issues.
                        //   TODO: Improve test coverage for remote persistent workers to catch
                        //    these edge cases and either fix them or prevent an action from
                        //    having a directory as well as a file in it as separate inputs.
                        last_source_dir_path = Some(path.clone());
                    } else {
                        let Some(metadata_digest) = metadata.get_digest() else {
                            panic!("NullPointerException: FileArtifactValue.getDigest()");
                        };
                        let digest = DigestUtil::build_digest_from_bytes(
                            metadata_digest,
                            metadata.get_size(),
                        );
                        let size_bytes = digest.size_bytes;
                        add_file(current_directory, name, digest, node_properties);
                        if blob_policy != BlobPolicy::Discard && size_bytes != 0 {
                            // If there is both a Digest and a FileArtifactValue key for the same
                            // content, prefer the FileArtifactValue as it is retained anyway.
                            blobs.insert(
                                BlobKey::Metadata(metadata),
                                MerkleTreeBlob::ActionInput(input.clone()),
                            );
                        }
                        input_files += 1;
                        input_bytes += size_bytes;
                    }
                }
            } else if let Some(virtual_action_input) = input.as_virtual_action_input() {
                let digest = self
                    .digest_util
                    .compute_virtual_action_input(virtual_action_input)?;
                let size_bytes = digest.size_bytes;
                add_file(current_directory, name, digest.clone(), node_properties);
                if blob_policy != BlobPolicy::Discard && size_bytes != 0 {
                    blobs
                        .entry(BlobKey::Digest(digest))
                        .or_insert_with(|| MerkleTreeBlob::ActionInput(input.clone()));
                }
                input_files += 1;
                input_bytes += size_bytes;
            } else if input
                .0
                .as_any()
                .downcast_ref::<EmptyInputDirectory>()
                .is_some()
            {
                current_directory.directories.push(DirectoryNode {
                    name,
                    digest: Some(self.empty_digest.clone()),
                });
            } else {
                // The input is not represented by a known subtype of ActionInput. Bare
                // ActionInputs arise from exploded source directories, repository rules or tests.
                let digest = self
                    .digest_util
                    .compute_path(&artifact_path_resolver.to_path(&**input))?;
                let size_bytes = digest.size_bytes;
                add_file(current_directory, name, digest.clone(), node_properties);
                if blob_policy != BlobPolicy::Discard && size_bytes != 0 {
                    blobs
                        .entry(BlobKey::Digest(digest))
                        .or_insert_with(|| MerkleTreeBlob::ActionInput(input.clone()));
                }
                input_files += 1;
                input_bytes += size_bytes;
            }
        }

        panic!("IllegalStateException: not reached");
    }

    /// PORT: the result is keyed by the position of the entry in `sorted_inputs` (Java keys the
    /// map by the entry itself).
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:748 MerkleTreeComputer.precomputeSubTrees
    async fn precompute_sub_trees<'a>(
        &'a self,
        sorted_inputs: &'a [SortedInput],
        is_tool_input: &'a IsToolInput<'a>,
        metadata_provider: &'a dyn InputMetadataProvider,
        artifact_path_resolver: &'a ArtifactPathResolver,
        remote_action_execution_context: Option<&'a RemoteActionExecutionContext<'a>>,
        remote_path_resolver: Option<&'a dyn RemotePathResolver>,
        blob_policy: BlobPolicy,
    ) -> io::Result<HashMap<usize, MerkleTreeRootOnly>> {
        let mut sub_tree_futures = Vec::new();
        for (index, entry) in sorted_inputs.iter().enumerate() {
            let future = self.maybe_cache_subtree(
                entry.1.as_ref(),
                &entry.0,
                is_tool_input,
                metadata_provider,
                artifact_path_resolver,
                remote_action_execution_context,
                remote_path_resolver,
                blob_policy,
            )?;
            if let Some(future) = future {
                sub_tree_futures.push(async move {
                    let sub_tree = future.await?;
                    Ok::<(usize, MerkleTreeRootOnly), io::Error>((index, sub_tree))
                });
            }
        }
        // `allAsList`: fails with the first failure and cancels the other futures.
        Ok(futures::future::try_join_all(sub_tree_futures)
            .await?
            .into_iter()
            .collect())
    }

    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:780 MerkleTreeComputer.maybeCacheSubtree
    fn maybe_cache_subtree<'a>(
        &'a self,
        input: Option<&'a ActionInputRef>,
        mapped_exec_path: &'a PathFragment,
        is_tool_input: &'a IsToolInput<'a>,
        metadata_provider: &'a dyn InputMetadataProvider,
        artifact_path_resolver: &'a ArtifactPathResolver,
        remote_action_execution_context: Option<&'a RemoteActionExecutionContext<'a>>,
        remote_path_resolver: Option<&'a dyn RemotePathResolver>,
        blob_policy: BlobPolicy,
    ) -> io::Result<Option<SubTreeFuture<'a>>> {
        let Some(input) = input else {
            return Ok(None);
        };
        let Some(artifact) = input.as_artifact() else {
            return Ok(None);
        };
        if artifact.is_tree_artifact() {
            let Some(tree_artifact_value) = metadata_provider.get_tree_metadata(input) else {
                panic!("NullPointerException: InputMetadataProvider.getTreeMetadata({input})");
            };
            Ok(Some(self.compute_for_tree_artifact_if_absent(
                artifact.get_exec_path(),
                tree_artifact_value,
                mapped_exec_path,
                is_tool_input,
                metadata_provider,
                artifact_path_resolver,
                remote_action_execution_context,
                remote_path_resolver,
                blob_policy,
            )))
        } else if artifact.is_runfiles_tree() {
            let Some(runfiles_artifact_value) = metadata_provider.get_runfiles_metadata(input)
            else {
                panic!("NullPointerException: InputMetadataProvider.getRunfilesMetadata({input})");
            };
            Ok(Some(self.compute_for_runfiles_tree_if_absent(
                runfiles_artifact_value,
                mapped_exec_path,
                is_tool_input,
                metadata_provider,
                artifact_path_resolver,
                remote_action_execution_context,
                remote_path_resolver,
                blob_policy,
            )))
        } else if artifact.is_source_artifact() {
            let Some(metadata) = metadata_provider.get_input_metadata(input)? else {
                panic!("NullPointerException: missing metadata: {input}");
            };
            if metadata.get_type() != FileStateType::Directory {
                return Ok(None);
            }
            Ok(Some(self.compute_if_absent(
                SubTreeCacheKey::SourceDirectory {
                    metadata,
                    path: artifact_path_resolver.to_path(&**input),
                },
                // Source artifacts are not path mapped.
                /* unmapped_exec_path= */
                None,
                Box::new(move || explode_directory(input, artifact_path_resolver)),
                is_tool_input(mapped_exec_path),
                metadata_provider,
                artifact_path_resolver,
                remote_action_execution_context,
                remote_path_resolver,
                blob_policy,
            )))
        } else {
            Ok(None)
        }
    }

    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:835 MerkleTreeComputer.computeForRunfilesTreeIfAbsent
    fn compute_for_runfiles_tree_if_absent<'a>(
        &'a self,
        runfiles_artifact_value: Arc<RunfilesArtifactValue>,
        mapped_exec_path: &'a PathFragment,
        is_tool_input: &'a IsToolInput<'a>,
        metadata_provider: &'a dyn InputMetadataProvider,
        artifact_path_resolver: &'a ArtifactPathResolver,
        remote_action_execution_context: Option<&'a RemoteActionExecutionContext<'a>>,
        remote_path_resolver: Option<&'a dyn RemotePathResolver>,
        blob_policy: BlobPolicy,
    ) -> SubTreeFuture<'a> {
        // A runfiles tree contains either only tool inputs or only non-tool inputs. It always
        // contains at least one artifact at its canonical location: the executable for which it
        // has been created.
        let Some(artifact_at_canonical_location) = get_one_element(
            &runfiles_artifact_value
                .get_runfiles_tree()
                .get_artifacts_at_canonical_locations_for_logging(),
        ) else {
            panic!(
                "NullPointerException: runfiles tree contains no artifacts at canonical location: {mapped_exec_path}"
            );
        };
        let full_path = mapped_exec_path
            .get_child(&self.workspace_name)
            .get_relative(&artifact_get_runfiles_path(&artifact_at_canonical_location));
        let is_tool = is_tool_input(&full_path);
        // mappedExecPath and isToolInput must not be used below as they aren't part of the cache
        // key - use isTool instead.
        // The metadata is a field of the RunfilesArtifactValue and thus just as suitable as a
        // weak key, but hashes and compares in constant time, whereas RunfilesArtifactValue does
        // so in time linear in the size of the runfiles tree.
        // PORT: the weak key is the `RunfilesArtifactValue` itself (see `SubTreeCacheKey`).
        let cache_key =
            SubTreeCacheKey::RunfilesArtifactValue(Arc::downgrade(&runfiles_artifact_value));
        self.compute_if_absent(
            cache_key,
            // Runfiles metadata already encodes the exec path of the root.
            /* unmapped_exec_path= */
            None,
            Box::new(move || {
                // Values in this entry set may be null, which represents an empty runfile.
                let mut mapping = runfiles_artifact_value.get_runfiles_tree().get_mapping();
                mapping.sort_by(|a, b| PathFragment::hierarchical_compare(&a.0, &b.0));
                Ok(mapping)
            }),
            is_tool,
            metadata_provider,
            artifact_path_resolver,
            remote_action_execution_context,
            remote_path_resolver,
            blob_policy,
        )
    }

    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:882 MerkleTreeComputer.computeForTreeArtifactIfAbsent
    fn compute_for_tree_artifact_if_absent<'a>(
        &'a self,
        unmapped_exec_path: &'a PathFragment,
        tree_artifact_value: Arc<TreeArtifactValue>,
        mapped_exec_path: &'a PathFragment,
        is_tool_input: &'a IsToolInput<'a>,
        metadata_provider: &'a dyn InputMetadataProvider,
        artifact_path_resolver: &'a ArtifactPathResolver,
        remote_action_execution_context: Option<&'a RemoteActionExecutionContext<'a>>,
        remote_path_resolver: Option<&'a dyn RemotePathResolver>,
        blob_policy: BlobPolicy,
    ) -> SubTreeFuture<'a> {
        // A tree artifact contains either only tool inputs or only non-tool inputs.
        let children = tree_artifact_value.get_children();
        let is_tool = !children.is_empty()
            && is_tool_input(
                &mapped_exec_path
                    .get_relative(&tree_file_artifact_get_parent_relative_path(&children[0])),
            );
        // mappedExecPath and isToolInput must not be used below as they aren't part of the cache
        // key - use isTool instead.
        // This must not be replaced by treeArtifactValue.getMetadata() as the latter returns a
        // new instance on every call and is thus unusable as an identity-compared weak key.
        let cache_key = SubTreeCacheKey::TreeArtifactValue(Arc::downgrade(&tree_artifact_value));
        self.compute_if_absent(
            cache_key,
            Some(unmapped_exec_path),
            Box::new(move || {
                let mut sorted_inputs: Vec<SortedInput> = tree_artifact_value
                    .get_children()
                    .into_iter()
                    .map(|child| {
                        (
                            tree_file_artifact_get_parent_relative_path(&child),
                            Some(child),
                        )
                    })
                    .collect();
                sorted_inputs.sort_by(|a, b| PathFragment::hierarchical_compare(&a.0, &b.0));
                Ok(sorted_inputs)
            }),
            is_tool,
            metadata_provider,
            artifact_path_resolver,
            remote_action_execution_context,
            remote_path_resolver,
            blob_policy,
        )
    }

    /// Performs a cached computation of the sub-Merkle tree for the given aggregate input.
    ///
    /// * `cache_key`: a value that fully describes the contents of the aggregate input and is
    ///   retained for as long as those contents are current. The persistent caches use it as a
    ///   weak and thus identity-compared key, which only works if it is a retained instance. The
    ///   in-flight cache compares it by value, so among the eligible values prefer the one with
    ///   the cheapest `hashCode` and `equals`.
    /// * `unmapped_exec_path`: the exec path of the aggregate input before path mapping to be
    ///   added to the cache key, `None` if this aggregate input is not subject to path mapping or
    ///   the metadata already includes the path.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:937 MerkleTreeComputer.computeIfAbsent
    fn compute_if_absent<'a>(
        &'a self,
        cache_key: SubTreeCacheKey,
        // PORT: only part of Java's in-flight cache key.
        _unmapped_exec_path: Option<&'a PathFragment>,
        sorted_inputs_supplier: SortedInputsSupplier<'a>,
        is_tool: bool,
        metadata_provider: &'a dyn InputMetadataProvider,
        artifact_path_resolver: &'a ArtifactPathResolver,
        remote_action_execution_context: Option<&'a RemoteActionExecutionContext<'a>>,
        remote_path_resolver: Option<&'a dyn RemotePathResolver>,
        blob_policy: BlobPolicy,
    ) -> SubTreeFuture<'a> {
        let persistent_cache: &'static SubTreeCache = if is_tool {
            LazyLock::force(&PERSISTENT_TOOL_SUB_TREE_CACHE)
        } else {
            LazyLock::force(&PERSISTENT_NON_TOOL_SUB_TREE_CACHE)
        };
        if blob_policy == BlobPolicy::KeepAndReupload {
            persistent_cache.invalidate(&cache_key);
        } else {
            let cached_root = persistent_cache.get_if_present(&cache_key);
            if let Some(cached_root) = cached_root
                && (blob_policy == BlobPolicy::Discard || cached_root.is_blobs_uploaded())
            {
                return Box::pin(std::future::ready(Ok::<MerkleTreeRootOnly, io::Error>(
                    cached_root,
                )));
            }
        }
        // PORT: no in-flight deduplication; sub-trees are computed on the calling task. Java
        // registers the task below with a `TaskDeduplicator` under the key `(cacheKey, isTool,
        // uploadBlobs, uploadBlobs ? unmappedExecPath : null)` so that concurrent spawns with the
        // same aggregate input share one computation and one upload; here every caller computes
        // (and, without a persistent cache entry, uploads) the sub-tree itself. The uploads
        // themselves are still deduplicated by the cache client.
        Box::pin(async move {
            // A concurrent computation may have completed and populated the persistent cache
            // after this one had already passed the check above. Recheck it to avoid unnecessary
            // work.
            let cached_root = persistent_cache.get_if_present(&cache_key);
            if let Some(cached_root) = cached_root
                && (blob_policy == BlobPolicy::Discard || cached_root.is_blobs_uploaded())
            {
                return Ok(cached_root);
            }
            // Subtrees either consist entirely of tool inputs or don't contain any. The same
            // applies to scrubbed inputs.
            let is_tool_input: &IsToolInput<'_> =
                if is_tool { &ALWAYS_TRUE } else { &ALWAYS_FALSE };
            let merkle_tree = self
                .build(
                    sorted_inputs_supplier()?,
                    is_tool_input,
                    /* spawn_scrubber= */ None,
                    metadata_provider,
                    artifact_path_resolver,
                    remote_action_execution_context,
                    remote_path_resolver,
                    blob_policy,
                )
                .await?;
            if let MerkleTree::Uploadable(uploadable) = &merkle_tree
                && let Some(merkle_tree_uploader) = &self.merkle_tree_uploader
            {
                // PORT: Java passes its (possibly null) context on; a tree with sub-trees is
                // never built without one.
                let Some(remote_action_execution_context) = remote_action_execution_context else {
                    panic!("NullPointerException: remoteActionExecutionContext");
                };
                merkle_tree_uploader
                    .ensure_inputs_present(
                        remote_action_execution_context,
                        uploadable,
                        blob_policy == BlobPolicy::KeepAndReupload,
                        remote_path_resolver,
                    )
                    .await?;
            }
            // Move the computed root to the persistent cache so that it can be reused by later
            // builds.
            persistent_cache.compute(cache_key, |old_root| {
                // Don't downgrade the cached root from one indicating that its blobs have been
                // uploaded.
                match old_root {
                    Some(old_root) if old_root.is_blobs_uploaded() => old_root.clone(),
                    _ => merkle_tree.root().clone(),
                }
            });
            Ok::<MerkleTreeRootOnly, io::Error>(merkle_tree.root().clone())
        })
    }
}

// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:397 MerkleTreeComputer.getOutputPath
fn get_output_path(
    input: &dyn ActionInput,
    remote_path_resolver: &dyn RemotePathResolver,
    path_mapper: &PathMapper,
) -> PathFragment {
    remote_path_resolver
        .get_working_directory()
        .get_relative(&path_mapper.map(input.get_exec_path()))
}

/// `StringEncoding.internalToUnicode(s)`.
// PORT: StringEncoding identity
fn internal_to_unicode(s: &str) -> String {
    s.to_string()
}

/// An `ActionInput` that is a child of another one at a given relative path.
///
/// This is used as a memory optimization as it avoids storing full absolute paths for children
/// of a source directory.
///
/// PORT: `ActionInput::get_exec_path` returns a reference, so the exec path is stored (Java
/// keeps the parent and the relative path and computes the exec path on every call).
// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:410 MerkleTreeComputer.ChildActionInput
#[derive(Debug)]
struct ChildActionInput {
    exec_path: PathFragment,
}

impl ChildActionInput {
    fn new(parent: &ActionInputRef, relative_path: &PathFragment) -> ChildActionInput {
        ChildActionInput {
            exec_path: parent
                .get_exec_path()
                .get_relative_str(relative_path.get_path_string()),
        }
    }
}

impl ActionInput for ChildActionInput {
    fn get_exec_path(&self) -> &PathFragment {
        &self.exec_path
    }
    fn is_directory(&self) -> bool {
        false
    }
    fn is_symlink(&self) -> bool {
        false
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Adapts a `Path` to an `ActionInput`.
///
/// This is only used for remote repository execution and tests and thus its memory usage doesn't
/// matter.
// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:437 MerkleTreeComputer.PathActionInput
#[derive(Debug)]
struct PathActionInput {
    path: Path,
}

impl PathActionInput {
    fn new(path: Path) -> PathActionInput {
        PathActionInput { path }
    }

    fn get_path(&self) -> &Path {
        &self.path
    }
}

impl ActionInput for PathActionInput {
    fn get_exec_path(&self) -> &PathFragment {
        self.path.as_fragment()
    }
    fn is_directory(&self) -> bool {
        false
    }
    fn is_symlink(&self) -> bool {
        false
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:1070 MerkleTreeComputer.addFile
fn add_file(
    directory: &mut Directory,
    name: String,
    digest: Digest,
    node_properties: Option<NodeProperties>,
) {
    directory.files.push(FileNode {
        name,
        digest: Some(digest),
        // We always treat files as executable since Bazel will `chmod 555` on the output
        // files of an action within ActionOutputMetadataStore#getMetadata after action
        // execution if no metadata was injected. We can't use real executable bit of the
        // file until this behavior is changed. See
        // https://github.com/bazelbuild/bazel/issues/13262 for more details.
        is_executable: true,
        node_properties,
    });
}

// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:1091 MerkleTreeComputer.findCommonPrefix
fn find_common_prefix(path1: &PathFragment, path2: &PathFragment) -> PathFragment {
    let mut common_segments = 0;
    let mut segments2 = path2.segments();
    for segment in path1.segments() {
        let Some(segment2) = segments2.next() else {
            break;
        };
        if segment != segment2 {
            break;
        }
        common_segments += 1;
    }
    path1.sub_fragment(0, common_segments)
}

// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:1107 MerkleTreeComputer.explodeDirectory
fn explode_directory(
    dir: &ActionInputRef,
    path_resolver: &ArtifactPathResolver,
) -> io::Result<Vec<SortedInput>> {
    // `ImmutableSortedMap.orderedBy(HIERARCHICAL_COMPARATOR)`.
    let mut inputs: BTreeMap<HierarchicalPath, ActionInputRef> = BTreeMap::new();
    let dir_path = path_resolver.to_path(&**dir);
    explode_directory_recursively(dir, &PathFragment::empty_fragment(), &dir_path, &mut inputs)?;
    Ok(inputs
        .into_iter()
        .map(|(path, input)| (path.0, Some(input)))
        .collect())
}

// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:1115 MerkleTreeComputer.explodeDirectory
fn explode_directory_recursively(
    dir: &ActionInputRef,
    rel_path: &PathFragment,
    dir_path: &Path,
    inputs: &mut BTreeMap<HierarchicalPath, ActionInputRef>,
) -> io::Result<()> {
    // PORT: no thread interruption (Java: `Thread.interrupted()`).
    let entries = dir_path.get_relative(rel_path).readdir(Symlinks::Follow)?;
    for entry in entries {
        let basename = entry.get_name();
        let path = rel_path.get_child(basename);
        match entry.get_type() {
            DirentType::File => {
                let child = ActionInputRef::new(ChildActionInput::new(dir, &path));
                inputs.insert(HierarchicalPath(path), child);
            }
            DirentType::Directory => explode_directory_recursively(dir, &path, dir_path, inputs)?,
            _ => {
                return Err(errors::io_exception(format!(
                    "The file type of '{}' is not supported.",
                    dir_path.get_relative(&path).get_path_string()
                )));
            }
        }
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:1139 MerkleTreeComputer.getOneElement
fn get_one_element<T: Clone + Eq + Hash>(nested_set: &NestedSet<T>) -> Option<T> {
    let leaves = nested_set.get_leaves();
    if let Some(first) = leaves.into_iter().next() {
        return Some(first);
    }
    let non_leaves = nested_set.get_non_leaves();
    for non_leaf in &non_leaves {
        let leaf = get_one_element(non_leaf);
        if leaf.is_some() {
            return leaf;
        }
    }
    None
}

// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeComputer.java:1182 MerkleTreeComputer.EmptyInputDirectory
#[derive(Debug)]
struct EmptyInputDirectory {
    output_dir: ActionInputRef,
}

impl EmptyInputDirectory {
    fn new(output_dir: ActionInputRef) -> EmptyInputDirectory {
        blaze_util::check_argument!(
            output_dir
                .as_artifact()
                .is_some_and(|artifact| artifact.is_tree_artifact())
        );
        EmptyInputDirectory { output_dir }
    }
}

impl ActionInput for EmptyInputDirectory {
    fn get_exec_path_string(&self) -> String {
        self.output_dir.get_exec_path_string()
    }
    fn get_exec_path(&self) -> &PathFragment {
        self.output_dir.get_exec_path()
    }
    fn is_directory(&self) -> bool {
        false
    }
    fn is_symlink(&self) -> bool {
        false
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// PORT: `WrappedException` (MerkleTreeComputer.java:1201) carries checked exceptions through
// Guava's `transform`; errors are `io::Result` values here.
