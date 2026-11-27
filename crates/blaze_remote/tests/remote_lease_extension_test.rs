//! Tests of `RemoteLeaseExtension` (lib/remote).
//!
//! Java has no unit test of the class; its one test is the integration test
//! `BuildWithoutTheBytesIntegrationTest.leaseExtension` (a build whose remote outputs' blobs get
//! a newer mtime at the remote worker while the build runs), which `scripts/remote/lease-probe`
//! runs against the oracle and Blaze. These tests check the class's own steps against the Java:
//! the remote outputs with a TTL are found, `FindMissingBlobs` is called per action, the
//! expiration of the outputs that are still in the cache moves to now + TTL in the Skyframe
//! value (shared, as Java's one object) and in the action cache entry, which is stored again.

mod util;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use blaze_actions::actions::action::{AbstractAction, Action};
use blaze_actions::analysis::workspace_status_action::system_action_owner;
use blaze_actions::{Artifact, ArtifactRoot, RootType, SpecialArtifactType};
use blaze_collect::NestedSetBuilder;
use blaze_collect::Order;
use blaze_events::{EventHandler, JavaClock};
use blaze_exec::actions::cache::compact_persistent_action_cache::CompactPersistentActionCache;
use blaze_exec::actions::cache::{ActionCache, ActionCacheEntryBuilder};
use blaze_exec::actions::{ActionInputRef, FileArtifactValue, TreeArtifactValue};
use blaze_remote::combined_cache::CombinedCache;
use blaze_remote::common::RemoteCacheClient;
use blaze_remote::lease_service::LeaseExtension;
use blaze_remote::reapi::Digest;
use blaze_remote::remote_lease_extension::{LeaseExtensionGraph, RemoteLeaseExtension};
use blaze_remote::util::DigestUtil;
use blaze_vfs::{DigestHashFunction, FileSystemExt, PathFragment, UnixFileSystem};
use indexmap::IndexMap;
use util::in_memory_cache_client::InMemoryCacheClient;

/// `ActionsTestUtil.NULL_ARTIFACT_OWNER`.
#[derive(Debug)]
struct NullArtifactOwner;

impl std::fmt::Display for NullArtifactOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NULL_ARTIFACT_OWNER")
    }
}

impl blaze_actions::actions::action_lookup::ActionLookupKey for NullArtifactOwner {
    fn get_label(&self) -> Option<blaze_cmdline::Label> {
        Some(blaze_cmdline::Label::parse_canonical("//null/action:owner").unwrap())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "NULL_ARTIFACT_OWNER"
    }
    fn key_equals(
        &self,
        other: &dyn blaze_actions::actions::action_lookup::ActionLookupKey,
    ) -> bool {
        other.as_any().is::<NullArtifactOwner>()
    }
    fn key_hash(&self) -> u64 {
        1
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn null_artifact_owner() -> blaze_actions::ActionLookupKeyRef {
    blaze_actions::ActionLookupKeyRef::new(NullArtifactOwner)
}

#[derive(Debug)]
struct TestAction(AbstractAction);

impl Action for TestAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.0
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn get_mnemonic(&self) -> String {
        "Test".to_string()
    }
    fn compute_key(
        &self,
        _action_key_context: &blaze_actions::actions::action_key_context::ActionKeyContext,
        _input_metadata_provider: Option<
            &dyn blaze_actions::actions::action::InputMetadataProvider,
        >,
        _fp: &mut blaze_util::Fingerprint,
    ) -> Result<(), blaze_actions::actions::action::ComputeKeyError> {
        unreachable!("the lease extension does not compute keys")
    }
}

/// One `ACTION_EXECUTION` value.
struct Graph {
    action: TestAction,
    files: IndexMap<Artifact, FileArtifactValue>,
    trees: IndexMap<Artifact, Arc<TreeArtifactValue>>,
}

impl LeaseExtensionGraph for Graph {
    fn for_each_action_execution_value(
        &self,
        visit: &mut dyn FnMut(
            &dyn Action,
            &IndexMap<Artifact, FileArtifactValue>,
            &IndexMap<Artifact, Arc<TreeArtifactValue>>,
        ) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        visit(&self.action, &self.files, &self.trees)
    }
}

struct NullHandler;
impl EventHandler for NullHandler {
    fn handle(&self, _event: &blaze_events::Event) {}
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: ArtifactRoot,
    digest_util: DigestUtil,
    client: Arc<InMemoryCacheClient>,
    combined_cache: Arc<CombinedCache>,
    action_cache: Arc<dyn ActionCache>,
}

impl Fixture {
    /// A cache that has `present`.
    fn new(present: &[&[u8]]) -> Fixture {
        blaze_remote::util::tracing_metadata_utils::set_tool_version("test");
        let dir = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let base = fs.get_path(dir.path().to_str().unwrap());
        let digest_util = DigestUtil::new(DigestHashFunction::Sha256);
        let mut cas: HashMap<Digest, Vec<u8>> = HashMap::new();
        for data in present {
            cas.insert(digest_util.compute(data), data.to_vec());
        }
        let client = Arc::new(InMemoryCacheClient::new_with_cas_entries(cas));
        let combined_cache = Arc::new(CombinedCache::new(
            Some(client.clone() as Arc<dyn RemoteCacheClient>),
            None,
            None,
            digest_util.clone(),
            None,
        ));
        let action_cache: Arc<dyn ActionCache> = Arc::new(
            CompactPersistentActionCache::create(
                &base.get_child("action_cache"),
                &base.get_child("action_cache.bad"),
                &base.get_child("action_cache.tmp"),
                Arc::new(JavaClock),
                &NullHandler,
            )
            .unwrap(),
        );
        Fixture {
            root: ArtifactRoot::as_derived_root(
                &base.get_child("execroot"),
                RootType::OUTPUT,
                &["out"],
            ),
            _dir: dir,
            digest_util,
            client,
            combined_cache,
            action_cache,
        }
    }

    fn artifact(&self, path: &str) -> Artifact {
        Artifact::create_derived_artifact(
            self.root.clone(),
            self.root
                .get_exec_path()
                .get_relative(&PathFragment::create(path)),
            null_artifact_owner(),
        )
    }

    /// `RemoteFileArtifactValueWithMaterializationData` of `data` expiring at `expiration`.
    fn remote(&self, data: &[u8], expiration: Option<i64>) -> FileArtifactValue {
        let digest = self.digest_util.compute(data);
        FileArtifactValue::create_for_remote_file_with_materialization_data(
            hex::decode(&digest.hash).unwrap(),
            digest.size_bytes,
            1,
            expiration,
            false,
        )
    }

    fn extension(&self, graph: Graph, ttl: Duration) -> RemoteLeaseExtension {
        RemoteLeaseExtension::new(
            Arc::new(graph),
            Some(self.action_cache.clone()),
            "build-request-id",
            "command-id",
            self.combined_cache.clone(),
            ttl,
        )
    }

    /// Starts the extension and waits until it has called `FindMissingBlobs` for `digest`.
    fn run_once(&self, extension: &RemoteLeaseExtension, digest: &Digest) {
        extension.start();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while self
            .client
            .get_num_find_missing_digests()
            .get(digest)
            .copied()
            .unwrap_or(0)
            == 0
        {
            assert!(
                std::time::Instant::now() < deadline,
                "no FindMissingBlobs call"
            );
            std::thread::yield_now();
        }
        extension.stop();
    }
}

fn action(outputs: Vec<Artifact>) -> TestAction {
    TestAction(AbstractAction::new(
        system_action_owner(),
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        outputs,
    ))
}

#[test]
fn extends_the_expiration_of_remote_outputs_still_in_the_cache() {
    let f = Fixture::new(&[b"alive"]);
    let alive = f.artifact("alive");
    let lost = f.artifact("lost");
    let old = now_millis() + 1000;
    let alive_value = f.remote(b"alive", Some(old));
    let lost_value = f.remote(b"lost", Some(old));
    // The action cache entry of the action, under its first output.
    let mut builder = ActionCacheEntryBuilder::new(
        "key",
        false,
        IndexMap::new(),
        "",
        blaze_exec::actions::cache::OutputPermissions::Readonly,
        false,
    );
    builder.add_output_file_save_metadata(&alive, f.remote(b"alive", Some(old)), true);
    builder.add_output_file_save_metadata(&lost, f.remote(b"lost", Some(old)), true);
    f.action_cache
        .put(&alive.get_exec_path_string(), builder.build());

    let mut files = IndexMap::new();
    files.insert(alive.clone(), alive_value.clone());
    files.insert(lost.clone(), lost_value.clone());
    let ttl = Duration::from_secs(3600);
    let extension = f.extension(
        Graph {
            action: action(vec![alive.clone(), lost.clone()]),
            files,
            trees: IndexMap::new(),
        },
        ttl,
    );
    let before = now_millis();
    f.run_once(&extension, &f.digest_util.compute(b"alive"));

    // The Skyframe value (the clone given to the graph shares the expiration time).
    let extended = alive_value.get_expiration_time().unwrap();
    assert!(extended >= before + ttl.as_millis() as i64, "{extended}");
    assert_eq!(lost_value.get_expiration_time(), Some(old));
    // The action cache entry was stored again with the new time.
    let entry = f.action_cache.get(&alive.get_exec_path_string()).unwrap();
    assert_eq!(
        entry.get_output_file(&alive).unwrap().get_expiration_time(),
        Some(extended)
    );
    assert_eq!(
        entry.get_output_file(&lost).unwrap().get_expiration_time(),
        Some(old)
    );
}

#[test]
fn outputs_without_ttl_are_not_looked_up() {
    let f = Fixture::new(&[b"a", b"b"]);
    let a = f.artifact("a");
    let b = f.artifact("b");
    let mut files = IndexMap::new();
    // A plain RemoteFileArtifactValue (no expiration time) and one with a TTL.
    let digest = f.digest_util.compute(b"a");
    files.insert(
        a.clone(),
        FileArtifactValue::create_for_remote_file(
            hex::decode(&digest.hash).unwrap(),
            digest.size_bytes,
            1,
        ),
    );
    files.insert(b.clone(), f.remote(b"b", Some(now_millis())));
    let extension = f.extension(
        Graph {
            action: action(vec![a, b]),
            files,
            trees: IndexMap::new(),
        },
        Duration::from_secs(3600),
    );
    f.run_once(&extension, &f.digest_util.compute(b"b"));
    assert_eq!(f.client.get_num_find_missing_digests().get(&digest), None);
}

#[test]
fn extends_tree_files_in_the_action_cache_entry() {
    let f = Fixture::new(&[b"child"]);
    let tree = Artifact::create_special_artifact(
        f.root.clone(),
        f.root
            .get_exec_path()
            .get_relative(&PathFragment::create("tree")),
        null_artifact_owner(),
        SpecialArtifactType::TREE,
    );
    tree.set_generating_action_key(blaze_actions::ActionLookupData::create(
        null_artifact_owner(),
        0,
    ));
    let child = Artifact::create_tree_output_str(&tree, "dir/child");
    let old = now_millis() + 1000;
    let child_value = f.remote(b"child", Some(old));
    let tree_value = TreeArtifactValue::build(
        vec![(ActionInputRef::new(child.clone()), child_value.clone())],
        None,
        None,
    );
    let mut builder = ActionCacheEntryBuilder::new(
        "key",
        false,
        IndexMap::new(),
        "",
        blaze_exec::actions::cache::OutputPermissions::Readonly,
        false,
    );
    let entry_tree_value = TreeArtifactValue::build(
        vec![(
            ActionInputRef::new(child.clone()),
            f.remote(b"child", Some(old)),
        )],
        None,
        None,
    );
    builder.add_output_tree_save_metadata(&tree, &entry_tree_value, true);
    f.action_cache
        .put(&tree.get_exec_path_string(), builder.build());

    let mut trees = IndexMap::new();
    trees.insert(tree.clone(), Arc::new(tree_value));
    let extension = f.extension(
        Graph {
            action: action(vec![tree.clone()]),
            files: IndexMap::new(),
            trees,
        },
        Duration::from_secs(3600),
    );
    f.run_once(&extension, &f.digest_util.compute(b"child"));

    let extended = child_value.get_expiration_time().unwrap();
    assert!(extended > old);
    let entry = f.action_cache.get(&tree.get_exec_path_string()).unwrap();
    let entry_tree = entry.get_output_tree(&tree).unwrap();
    assert_eq!(
        entry_tree.child_values["dir/child"].get_expiration_time(),
        Some(extended)
    );
}

/// `setExpirationTime` changes the value for every holder (Java's one object); a plain
/// `RemoteFileArtifactValue` ignores it.
#[test]
fn set_expiration_time_is_seen_by_every_clone() {
    let f = Fixture::new(&[]);
    let value = f.remote(b"x", Some(5));
    let clone = value.clone();
    value.set_expiration_time(Some(7));
    assert_eq!(clone.get_expiration_time(), Some(7));
    // A value equal but for the time is still equal (the time is not compared).
    assert_eq!(value, f.remote(b"x", Some(1)));
    let digest = f.digest_util.compute(b"x");
    let plain = FileArtifactValue::create_for_remote_file(
        hex::decode(&digest.hash).unwrap(),
        digest.size_bytes,
        1,
    );
    plain.set_expiration_time(Some(7));
    assert_eq!(plain.get_expiration_time(), None);
}
