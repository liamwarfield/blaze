//! Port of `src/test/java/com/google/devtools/build/lib/actions/cache/CompactPersistentActionCacheTest.java`.

use std::any::Any;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use blaze_events::{Clock, Event, EventHandler, NullEventHandler};
use blaze_exec::actions::cache::action_cache::{
    ActionCache, ActionCacheEntry, ActionCacheEntryBuilder, OutputPermissions,
    SerializableTreeArtifactValue,
};
use blaze_exec::actions::cache::compact_persistent_action_cache::CompactPersistentActionCache;
use blaze_exec::actions::cache::metadata_digest_utils::get_expiration_time;
use blaze_exec::actions::{
    ActionInput, ActionInputRef, Artifact, FileArtifactValue, TreeArtifactValue,
};
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;
use sha2::{Digest, Sha256};

const DAY_MILLIS: i64 = 24 * 60 * 60 * 1000;

/// Port of `testutil/ManualClock`.
struct ManualClock {
    current_time_millis: AtomicI64,
}

impl ManualClock {
    fn new() -> ManualClock {
        ManualClock {
            current_time_millis: AtomicI64::new(0),
        }
    }

    fn advance(&self, duration: Duration) {
        self.current_time_millis
            .fetch_add(duration.as_millis() as i64, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn current_time_millis(&self) -> i64 {
        self.current_time_millis.load(Ordering::SeqCst)
    }

    /// Nano time should not be confused with wall time. Nano time is only mean to compute time
    /// differences. Because of this, we shift the time returned by 1000s, to test that the users
    /// of this class do not rely on nanoTime == currentTimeMillis.
    fn nano_time(&self) -> i64 {
        self.current_time_millis.load(Ordering::SeqCst) * 1_000_000 + 1000 * 1_000_000_000
    }
}

/// Stands in for Mockito's `spy(EventHandler.class)`.
#[derive(Default)]
struct RecordingEventHandler {
    events: Mutex<Vec<String>>,
}

impl RecordingEventHandler {
    fn count(&self) -> usize {
        self.events.lock().unwrap().len()
    }
}

impl EventHandler for RecordingEventHandler {
    fn handle(&self, event: &Event) {
        self.events
            .lock()
            .unwrap()
            .push(event.get_message().into_owned());
    }
}

/// A test artifact (`SourceArtifact`, `DerivedArtifact`, `SpecialArtifact` or
/// `TreeFileArtifact`).
#[derive(Debug)]
struct FakeArtifact {
    exec_path: PathFragment,
    path: Path,
    tree: bool,
    source: bool,
    parent: Option<ActionInputRef>,
    parent_relative_path: Option<PathFragment>,
}

impl ActionInput for FakeArtifact {
    fn get_exec_path(&self) -> &PathFragment {
        &self.exec_path
    }
    fn is_directory(&self) -> bool {
        self.tree
    }
    fn is_symlink(&self) -> bool {
        false
    }
    fn as_artifact(&self) -> Option<&dyn Artifact> {
        Some(self)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Artifact for FakeArtifact {
    fn is_source_artifact(&self) -> bool {
        self.source
    }
    fn is_tree_artifact(&self) -> bool {
        self.tree
    }
    fn is_runfiles_tree(&self) -> bool {
        false
    }
    fn is_fileset(&self) -> bool {
        false
    }
    fn is_child_of_declared_directory(&self) -> bool {
        self.parent.is_some()
    }
    fn get_parent(&self) -> Option<ActionInputRef> {
        self.parent.clone()
    }
    fn get_parent_relative_path(&self) -> Option<PathFragment> {
        self.parent_relative_path.clone()
    }
    fn get_root_relative_path(&self) -> PathFragment {
        self.exec_path.clone()
    }
    fn get_root_exec_path(&self) -> PathFragment {
        PathFragment::create("")
    }
}

fn artifact_path(input: &ActionInputRef) -> Path {
    input
        .as_any()
        .downcast_ref::<FakeArtifact>()
        .unwrap()
        .path
        .clone()
}

struct Fixture {
    _tmp: tempfile::TempDir,
    exec_root: Path,
    cache_root: Path,
    corrupted_cache_root: Path,
    tmp_dir: Path,
    map_file: Path,
    journal_file: Path,
    index_file: Path,
    index_journal_file: Path,
    timestamp_file: Path,
    timestamp_journal_file: Path,
    clock: Arc<ManualClock>,
    cache: Arc<CompactPersistentActionCache>,
    event_handler: RecordingEventHandler,
}

impl Fixture {
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root = Path::create_from_str(tmp.path().to_str().unwrap(), &fs);
        let exec_root = root.get_child("output");
        let cache_root = root.get_child("cache_root");
        let corrupted_cache_root = root.get_child("corrupted_cache_root");
        let tmp_dir = root.get_child("cache_tmp_dir");
        let clock = Arc::new(ManualClock::new());
        let cache = Arc::new(
            CompactPersistentActionCache::create(
                &cache_root,
                &corrupted_cache_root,
                &tmp_dir,
                clock.clone(),
                NullEventHandler::INSTANCE,
            )
            .unwrap(),
        );
        Fixture {
            _tmp: tmp,
            map_file: CompactPersistentActionCache::cache_file(&cache_root),
            journal_file: CompactPersistentActionCache::journal_file(&cache_root),
            index_file: CompactPersistentActionCache::index_file(&cache_root),
            index_journal_file: CompactPersistentActionCache::index_journal_file(&cache_root),
            timestamp_file: CompactPersistentActionCache::timestamp_file(&cache_root),
            timestamp_journal_file: CompactPersistentActionCache::timestamp_journal_file(
                &cache_root,
            ),
            exec_root,
            cache_root,
            corrupted_cache_root,
            tmp_dir,
            clock,
            cache,
            event_handler: RecordingEventHandler::default(),
        }
    }

    fn create_cache(&self, handler: &dyn EventHandler) -> Arc<CompactPersistentActionCache> {
        Arc::new(
            CompactPersistentActionCache::create(
                &self.cache_root,
                &self.corrupted_cache_root,
                &self.tmp_dir,
                self.clock.clone(),
                handler,
            )
            .unwrap(),
        )
    }

    /// `ActionsTestUtil.DUMMY_ARTIFACT`: a source artifact at `/dummy`.
    fn dummy_artifact(&self) -> ActionInputRef {
        ActionInputRef::new(FakeArtifact {
            exec_path: PathFragment::create("/dummy"),
            path: self.exec_root.get_child("dummy"),
            tree: false,
            source: true,
            parent: None,
            parent_relative_path: None,
        })
    }

    /// `ActionsTestUtil.createArtifact(artifactRoot, path)`.
    fn create_artifact(&self, path: &str) -> ActionInputRef {
        let exec_path = PathFragment::create("bin").get_relative(&PathFragment::create(path));
        ActionInputRef::new(FakeArtifact {
            path: self.exec_root.get_relative(&exec_path),
            exec_path,
            tree: false,
            source: false,
            parent: None,
            parent_relative_path: None,
        })
    }

    /// `ActionsTestUtil.createTreeArtifactWithGeneratingAction(artifactRoot, path)`.
    fn create_tree_artifact(&self, path: &str) -> ActionInputRef {
        let exec_path = PathFragment::create("bin").get_relative(&PathFragment::create(path));
        ActionInputRef::new(FakeArtifact {
            path: self.exec_root.get_relative(&exec_path),
            exec_path,
            tree: true,
            source: false,
            parent: None,
            parent_relative_path: None,
        })
    }
}

/// `TreeFileArtifact.createTreeOutput(parent, parentRelativePath)`.
fn create_tree_output(parent: &ActionInputRef, parent_relative_path: &str) -> ActionInputRef {
    let rel = PathFragment::create(parent_relative_path);
    ActionInputRef::new(FakeArtifact {
        exec_path: parent.get_exec_path().get_relative(&rel),
        path: artifact_path(parent).get_relative(&rel),
        tree: false,
        source: false,
        parent: Some(parent.clone()),
        parent_relative_path: Some(rel),
    })
}

/// `ArchivedTreeArtifact.createForTree(parent)`.
fn create_archived_for_tree(parent: &ActionInputRef) -> ActionInputRef {
    let exec_path = PathFragment::create(&format!(
        ":archived_tree_artifacts/{}.zip",
        parent.get_exec_path_string()
    ));
    ActionInputRef::new(FakeArtifact {
        path: artifact_path(parent)
            .get_parent_directory()
            .unwrap()
            .get_relative(&exec_path),
        exec_path,
        tree: false,
        source: false,
        parent: None,
        parent_relative_path: None,
    })
}

fn write_content(path: &Path, content: &[u8]) {
    let mut out = path.get_output_stream().unwrap();
    out.write_all(content).unwrap();
}

fn read_content(path: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    path.get_input_stream()
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

fn create_local_metadata(artifact: &ActionInputRef, content: &str) -> FileArtifactValue {
    let path = artifact_path(artifact);
    path.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    write_content(&path, content.as_bytes());
    FileArtifactValue::create_for_testing(&path).unwrap()
}

fn create_remote_metadata_full(
    content: &str,
    expiration_time: Option<i64>,
    resolved_path: Option<PathFragment>,
) -> FileArtifactValue {
    let bytes = content.as_bytes();
    let digest = Sha256::digest(bytes).to_vec();
    let mut metadata = FileArtifactValue::create_for_remote_file_with_materialization_data(
        digest,
        bytes.len() as i64,
        1,
        expiration_time,
        /* in_memory_output= */ false,
    );
    if let Some(resolved_path) = resolved_path {
        metadata =
            FileArtifactValue::create_from_existing_with_resolved_path(metadata, resolved_path);
    }
    metadata
}

fn create_remote_metadata(content: &str) -> FileArtifactValue {
    create_remote_metadata_full(content, None, None)
}

fn create_tree_metadata(
    parent: &ActionInputRef,
    children: Vec<(&str, FileArtifactValue)>,
    archived_artifact_value: Option<FileArtifactValue>,
    resolved_path: Option<PathFragment>,
) -> TreeArtifactValue {
    let mut child_values: Vec<(ActionInputRef, FileArtifactValue)> = children
        .into_iter()
        .map(|(name, value)| (create_tree_output(parent, name), value))
        .collect();
    child_values.sort_by(|a, b| a.0.get_exec_path().cmp(b.0.get_exec_path()));
    TreeArtifactValue {
        child_values,
        archived_representation: archived_artifact_value
            .map(|metadata| (create_archived_for_tree(parent), metadata)),
        // PORT: TreeArtifactValue.Builder.build computes the aggregate metadata; this test only
        // asserts the serialized tree, so a fixed aggregate stands in.
        metadata: Some(FileArtifactValue::create_for_directory_with_mtime(0)),
        resolved_path,
    }
}

fn builder(action_key: &str) -> ActionCacheEntryBuilder {
    builder_discovers(action_key, false)
}

fn builder_discovers(action_key: &str, discovers_inputs: bool) -> ActionCacheEntryBuilder {
    ActionCacheEntryBuilder::new(
        action_key,
        discovers_inputs,
        /* client_env= */ IndexMap::new(),
        /* action_execution_salt= */ "",
        OutputPermissions::Readonly,
        /* use_archived_tree_artifacts= */ false,
    )
}

fn put_key_into(key: &str, action_cache: &dyn ActionCache, discovers_inputs: bool) {
    let entry = builder_discovers(key, discovers_inputs).build();
    action_cache.put(key, entry);
}

fn put_key(f: &Fixture, key: &str) {
    put_key_into(key, &*f.cache, false);
}

fn put_key_discovers(f: &Fixture, key: &str, discovers_inputs: bool) {
    put_key_into(key, &*f.cache, discovers_inputs);
}

fn assert_key_equals(cache1: &dyn ActionCache, cache2: &dyn ActionCache, key: &str) {
    let entry = cache1.get(key);
    assert!(entry.is_some());
    assert_eq!(
        cache2.get(key).unwrap().to_string(),
        entry.unwrap().to_string()
    );
}

fn assert_full_save(f: &Fixture) {
    f.cache.save().unwrap();
    assert!(f.map_file.exists());
    assert!(!f.journal_file.exists());
}

fn assert_incremental_save(f: &Fixture, ac: &dyn ActionCache) {
    ac.save().unwrap();
    assert!(f.map_file.exists());
    assert!(f.journal_file.exists());
}

#[test]
fn test_delete_unrecognized_files() {
    let mut f = Fixture::new();
    let unrecognized_file = f.cache_root.get_child("unrecognized");
    write_content(&unrecognized_file, b"");

    f.cache = f.create_cache(NullEventHandler::INSTANCE);

    assert!(!unrecognized_file.exists());
}

#[test]
fn test_get_invalid_key() {
    let f = Fixture::new();
    assert!(f.cache.get("key").is_none());
}

#[test]
fn test_put_and_get() {
    let f = Fixture::new();
    let key = "key";
    put_key(&f, key);
    let readentry = f.cache.get(key);
    assert!(readentry.is_some());
    assert_eq!(
        readentry.unwrap().to_string(),
        f.cache.get(key).unwrap().to_string()
    );
    assert!(!f.map_file.exists());
}

#[test]
fn test_put_and_remove() {
    let f = Fixture::new();
    let key = "key";
    put_key(&f, key);
    f.cache.remove(key);
    assert!(f.cache.get(key).is_none());
    assert!(!f.map_file.exists());
}

#[test]
fn test_get_size() {
    let f = Fixture::new();
    // initial state.
    assert_eq!(f.cache.size(), 0);

    let key = "key";
    put_key(&f, key);
    // the inserted key, and the validation key
    assert_eq!(f.cache.size(), 2);

    f.cache.remove(key);
    // the validation key
    assert_eq!(f.cache.size(), 1);

    f.cache.clear();
    assert_eq!(f.cache.size(), 0);
}

fn assert_save(discover_inputs: bool) {
    let f = Fixture::new();
    let key = "key";
    put_key_discovers(&f, key, discover_inputs);
    f.cache.save().unwrap();
    assert!(f.map_file.exists());
    assert!(!f.journal_file.exists());

    let new_cache = f.create_cache(&f.event_handler);
    assert_eq!(f.event_handler.count(), 0);
    let readentry = new_cache.get(key);
    assert!(readentry.is_some());
    assert_eq!(
        readentry.unwrap().to_string(),
        f.cache.get(key).unwrap().to_string()
    );
}

#[test]
fn test_save_discover_inputs() {
    assert_save(true);
}

#[test]
fn test_save_no_discover_inputs() {
    assert_save(false);
}

#[test]
fn test_incremental_save() {
    let f = Fixture::new();
    for i in 0..300 {
        put_key(&f, &i.to_string());
    }
    assert_full_save(&f);

    // Add 2 entries to 300. Might as well just leave them in the journal.
    put_key(&f, "abc");
    put_key(&f, "123");
    assert_incremental_save(&f, &*f.cache);

    // Make sure we have all the entries, including those in the journal,
    // after deserializing into a new cache.
    let newcache = f.create_cache(&f.event_handler);
    assert_eq!(f.event_handler.count(), 0);
    for i in 0..100 {
        assert_key_equals(&*f.cache, &*newcache, &i.to_string());
    }
    assert_key_equals(&*f.cache, &*newcache, "abc");
    assert_key_equals(&*f.cache, &*newcache, "123");
    put_key_into("xyz", &*newcache, true);
    assert_incremental_save(&f, &*newcache);

    // Make sure we can see previous journal values after a second incremental save.
    let newer_cache = f.create_cache(&f.event_handler);
    assert_eq!(f.event_handler.count(), 0);
    for i in 0..100 {
        assert_key_equals(&*f.cache, &*newer_cache, &i.to_string());
    }
    assert_key_equals(&*f.cache, &*newer_cache, "abc");
    assert_key_equals(&*f.cache, &*newer_cache, "123");
    assert!(newer_cache.get("xyz").is_some());
    assert!(newer_cache.get("not_a_key").is_none());

    // Add another 10 entries. This should not be incremental.
    for i in 300..310 {
        put_key(&f, &i.to_string());
    }
    assert_full_save(&f);
}

#[test]
fn test_remove_if() {
    let f = Fixture::new();
    // Add 100 entries, 5 of which discover inputs, and do a full save.
    for i in 0..100 {
        put_key_discovers(&f, &i.to_string(), i % 20 == 0);
    }
    assert_full_save(&f);

    // Remove entries that discover inputs and flush the journal.
    f.cache.remove_if(&ActionCacheEntry::discovers_inputs);
    assert_full_save(&f);

    // Check that the entries that discover inputs are gone, and the rest are still there.
    for i in 0..100 {
        let entry = f.cache.get(&i.to_string());
        if i % 20 == 0 {
            assert!(entry.is_none());
        } else {
            assert!(entry.is_some());
        }
    }

    // Make sure we get the same result after deserializing into a new cache.
    let newer_cache = f.create_cache(&f.event_handler);
    assert_eq!(f.event_handler.count(), 0);
    for i in 0..100 {
        let entry = newer_cache.get(&i.to_string());
        if i % 20 == 0 {
            assert!(entry.is_none());
        } else {
            assert!(entry.is_some());
        }
    }
}

#[test]
fn test_clear() {
    let f = Fixture::new();
    // Add 100 entries and do a full save.
    for i in 0..100 {
        put_key(&f, &i.to_string());
    }
    assert_full_save(&f);

    // Clear the cache (which implicitly saves it).
    f.cache.clear();

    // Check that the cache is empty.
    assert_eq!(f.cache.size(), 0);

    // Make sure we get the same result after deserializing into a new cache.
    let newer_cache = f.create_cache(&f.event_handler);
    assert_eq!(f.event_handler.count(), 0);
    assert_eq!(newer_cache.size(), 0);
}

fn days(n: u64) -> Duration {
    Duration::from_secs(n * 24 * 60 * 60)
}

#[test]
fn test_timestamps() {
    let f = Fixture::new();
    f.clock.advance(days(100));
    put_key(&f, "abc");
    f.clock.advance(days(100));
    put_key(&f, "def");
    f.clock.advance(days(100));
    put_key(&f, "ghi");
    f.clock.advance(days(100));
    put_key(&f, "jkl");
    f.clock.advance(days(100));
    put_key_discovers(&f, "mno", /* discovers_inputs= */ true);

    // Getting an entry should update its timestamp.
    f.clock.advance(days(100));
    let _unused = f.cache.get("abc");

    // Overwriting an entry should update its timestamp.
    f.clock.advance(days(100));
    put_key(&f, "def");

    // Remove an entry should remove its timestamp.
    f.clock.advance(days(100));
    f.cache.remove("ghi");

    // Removing entries matching a predicate should not affect the timestamp of other entries.
    f.clock.advance(days(100));
    f.cache.remove_if(&ActionCacheEntry::discovers_inputs);

    assert_full_save(&f);

    let expected: IndexMap<String, i64> = IndexMap::from([
        ("abc".to_string(), 600 * DAY_MILLIS),
        ("def".to_string(), 700 * DAY_MILLIS),
        ("jkl".to_string(), 400 * DAY_MILLIS),
    ]);
    let mut actual: Vec<(String, i64)> = f.cache.get_action_timestamp_map().into_iter().collect();
    actual.sort();
    let mut expected_sorted: Vec<(String, i64)> = expected.clone().into_iter().collect();
    expected_sorted.sort();
    assert_eq!(actual, expected_sorted);

    // Make sure we get the same result after deserializing into a new cache.
    let newer_cache = f.create_cache(&f.event_handler);
    assert_eq!(f.event_handler.count(), 0);
    let mut actual: Vec<(String, i64)> =
        newer_cache.get_action_timestamp_map().into_iter().collect();
    actual.sort();
    assert_eq!(actual, expected_sorted);
}

fn put_four_keys_and_save(f: &Fixture) {
    f.clock.advance(days(100));
    put_key(f, "abc");
    f.clock.advance(days(100));
    put_key(f, "def");
    f.clock.advance(days(100));
    put_key(f, "ghi");
    f.clock.advance(days(100));
    put_key(f, "jkl");
    f.clock.advance(days(100));
    assert_full_save(f);
}

#[test]
fn test_trim_no_threshold() {
    let mut f = Fixture::new();
    put_four_keys_and_save(&f);

    f.cache = f.cache.clone().trim(0.0, days(250)).unwrap();

    // Check that the cache was trimmed correctly.
    assert!(f.cache.get("abc").is_none());
    assert!(f.cache.get("def").is_none());
    assert!(f.cache.get("ghi").is_some());
    assert!(f.cache.get("jkl").is_some());

    // Make sure we get the same result after deserializing into a new cache.
    let newer_cache = f.create_cache(&f.event_handler);
    assert_eq!(f.event_handler.count(), 0);
    assert!(newer_cache.get("abc").is_none());
    assert!(newer_cache.get("def").is_none());
    assert!(newer_cache.get("ghi").is_some());
    assert!(newer_cache.get("jkl").is_some());
}

#[test]
fn test_trim_below_threshold() {
    let mut f = Fixture::new();
    put_four_keys_and_save(&f);

    // 1 of 4 entries is stale, below 30% threshold.
    f.cache = f.cache.clone().trim(0.3, days(350)).unwrap();

    assert!(f.cache.get("abc").is_some());
    assert!(f.cache.get("def").is_some());
    assert!(f.cache.get("ghi").is_some());
    assert!(f.cache.get("jkl").is_some());
}

#[test]
fn test_trim_above_threshold() {
    let mut f = Fixture::new();
    put_four_keys_and_save(&f);

    // 1 of 4 entries is stale, above 20% threshold.
    f.cache = f.cache.clone().trim(0.2, days(350)).unwrap();

    assert!(f.cache.get("abc").is_none());
    assert!(f.cache.get("def").is_some());
    assert!(f.cache.get("ghi").is_some());
    assert!(f.cache.get("jkl").is_some());
}

#[derive(Clone, Copy, Debug)]
#[allow(clippy::enum_variant_names)]
enum IncompatibleFile {
    MapFile,
    JournalFile,
    IndexFile,
    IndexJournalFile,
    TimestampFile,
    TimestampJournalFile,
}

fn do_test_incompatible_format(param: IncompatibleFile) {
    let mut f = Fixture::new();
    let incompatible_file = match param {
        IncompatibleFile::MapFile => f.map_file.clone(),
        IncompatibleFile::JournalFile => f.journal_file.clone(),
        IncompatibleFile::IndexFile => f.index_file.clone(),
        IncompatibleFile::IndexJournalFile => f.index_journal_file.clone(),
        IncompatibleFile::TimestampFile => f.timestamp_file.clone(),
        IncompatibleFile::TimestampJournalFile => f.timestamp_journal_file.clone(),
    };

    write_content(&incompatible_file, b"incompatible");

    f.cache = f.create_cache(&f.event_handler);

    assert_eq!(f.event_handler.count(), 0, "{:?}", param);
    assert!(!f.corrupted_cache_root.exists(), "{:?}", param);
    assert_eq!(f.cache.size(), 0, "{:?}", param);
}

#[test]
fn test_incompatible_format() {
    for param in [
        IncompatibleFile::MapFile,
        IncompatibleFile::JournalFile,
        IncompatibleFile::IndexFile,
        IncompatibleFile::IndexJournalFile,
        IncompatibleFile::TimestampFile,
        IncompatibleFile::TimestampJournalFile,
    ] {
        do_test_incompatible_format(param);
    }
}

#[test]
fn test_truncated_map_file() {
    let mut f = Fixture::new();
    for i in 0..300 {
        put_key(&f, &i.to_string());
    }
    assert_full_save(&f);

    let contents = read_content(&f.map_file);
    write_content(&f.map_file, &contents[..contents.len() - 1]);

    f.cache = f.create_cache(&f.event_handler);

    assert_eq!(f.event_handler.count(), 1);
    assert!(f.corrupted_cache_root.exists());
    assert_eq!(f.cache.size(), 0);
}

#[test]
fn test_truncated_journal_file() {
    let mut f = Fixture::new();
    for i in 0..300 {
        put_key(&f, &i.to_string());
    }
    assert_full_save(&f);

    put_key(&f, "abc");
    assert_incremental_save(&f, &*f.cache);

    assert_eq!(f.cache.size(), 302); // 301 entries + validation record

    let contents = read_content(&f.journal_file);
    write_content(&f.journal_file, &contents[..contents.len() - 1]);

    f.cache = f.create_cache(&f.event_handler);

    assert_eq!(f.event_handler.count(), 0);
    assert!(!f.corrupted_cache_root.exists());
    assert_eq!(f.cache.size(), 301);
}

#[test]
fn put_and_get_saves_remote_file_metadata() {
    let f = Fixture::new();
    let artifact = f.dummy_artifact();
    let metadata = create_remote_metadata("content");
    let entry = builder("key")
        .add_output_file_save_metadata(&*artifact, metadata.clone(), true)
        .build();
    f.cache.put("key", entry);

    let entry = f.cache.get("key").unwrap();

    assert_eq!(entry.get_output_file(&*artifact), Some(&metadata));
}

#[test]
fn put_and_get_saves_remote_file_metadata_with_expiration_time() {
    let f = Fixture::new();
    let artifact = f.dummy_artifact();
    let expiration_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let metadata = create_remote_metadata_full("content", Some(expiration_time), None);
    let entry = builder("key")
        .add_output_file_save_metadata(&*artifact, metadata, true)
        .build();
    f.cache.put("key", entry);

    let entry = f.cache.get("key").unwrap();

    assert_eq!(
        get_expiration_time(entry.get_output_file(&*artifact).unwrap()),
        Some(expiration_time)
    );
}

#[test]
fn put_and_get_saves_remote_file_metadata_with_resolved_path() {
    let f = Fixture::new();
    let artifact = f.dummy_artifact();
    let metadata = create_remote_metadata_full(
        "content",
        None,
        Some(
            f.exec_root
                .get_relative_str("some/path")
                .as_fragment()
                .clone(),
        ),
    );
    let entry = builder("key")
        .add_output_file_save_metadata(&*artifact, metadata.clone(), true)
        .build();
    f.cache.put("key", entry);

    let entry = f.cache.get("key").unwrap();

    assert_eq!(entry.get_output_file(&*artifact), Some(&metadata));
}

#[test]
fn put_and_get_saves_proxy_outputs() {
    let f = Fixture::new();
    let artifact = f.dummy_artifact();
    let metadata = FileArtifactValue::Proxy {
        delegate: Box::new(create_local_metadata(&artifact, "content")),
        path: artifact_path(&artifact),
    };
    let entry = builder("key")
        .add_output_file_save_metadata(&*artifact, metadata, true)
        .build();
    f.cache.put("key", entry);

    let entry = f.cache.get("key").unwrap();

    assert_eq!(
        entry.get_proxy_outputs(),
        &[artifact.get_exec_path_string()][..]
    );
}

#[test]
fn put_and_get_ignores_local_file_metadata() {
    let f = Fixture::new();
    let artifact = f.dummy_artifact();
    let metadata = create_local_metadata(&artifact, "content");
    let entry = builder("key")
        .add_output_file_save_metadata(&*artifact, metadata, true)
        .build();
    f.cache.put("key", entry);

    let entry = f.cache.get("key").unwrap();

    assert_eq!(entry.get_output_file(&*artifact), None);
}

#[test]
fn put_and_get_tree_metadata_only_saves_remote_file_metadata() {
    let f = Fixture::new();
    let artifact = f.create_tree_artifact("bin/dummy");
    let metadata = create_tree_metadata(
        &artifact,
        vec![
            ("file1", create_remote_metadata("content1")),
            (
                "file2",
                create_local_metadata(&create_tree_output(&artifact, "file2"), "content2"),
            ),
        ],
        /* archived_artifact_value= */ None,
        /* resolved_path= */ None,
    );
    let entry = builder("key")
        .add_output_tree_save_metadata(&*artifact, &metadata, true)
        .build();
    f.cache.put("key", entry);

    let entry = f.cache.get("key").unwrap();

    assert_eq!(
        entry.get_output_tree(&*artifact),
        Some(&SerializableTreeArtifactValue::new(
            IndexMap::from([("file1".to_string(), create_remote_metadata("content1"))]),
            /* archived_file_value= */ None,
            /* resolved_path= */ None,
        ))
    );
}

#[test]
fn put_and_get_tree_metadata_saves_remote_archived_artifact() {
    let f = Fixture::new();
    let artifact = f.create_tree_artifact("bin/dummy");
    let metadata = create_tree_metadata(
        &artifact,
        vec![],
        Some(create_remote_metadata("content")),
        /* resolved_path= */ None,
    );
    let entry = builder("key")
        .add_output_tree_save_metadata(&*artifact, &metadata, true)
        .build();
    f.cache.put("key", entry);

    let entry = f.cache.get("key").unwrap();

    assert_eq!(
        entry.get_output_tree(&*artifact),
        Some(&SerializableTreeArtifactValue::new(
            /* child_values= */ IndexMap::new(),
            /* archived_file_value= */ Some(create_remote_metadata("content")),
            /* resolved_path= */ None,
        ))
    );
}

#[test]
fn put_and_get_tree_metadata_ignores_local_archived_artifact() {
    let f = Fixture::new();
    let artifact = f.create_tree_artifact("bin/dummy");
    let metadata = create_tree_metadata(
        &artifact,
        /* children= */ vec![],
        Some(create_local_metadata(
            &f.create_artifact("bin/archive"),
            "content",
        )),
        /* resolved_path= */ None,
    );
    let entry = builder("key")
        .add_output_tree_save_metadata(&*artifact, &metadata, true)
        .build();
    f.cache.put("key", entry);

    let entry = f.cache.get("key").unwrap();

    assert_eq!(
        entry
            .get_output_tree(&*artifact)
            .unwrap()
            .archived_file_value(),
        None
    );
}

#[test]
fn put_and_get_tree_metadata_saves_resolved_path() {
    let f = Fixture::new();
    let resolved_path = f
        .exec_root
        .get_relative_str("some/path")
        .as_fragment()
        .clone();
    let artifact = f.create_tree_artifact("bin/dummy");
    let metadata = create_tree_metadata(
        &artifact,
        vec![],
        /* archived_artifact_value= */ None,
        Some(resolved_path.clone()),
    );
    let entry = builder("key")
        .add_output_tree_save_metadata(&*artifact, &metadata, true)
        .build();

    f.cache.put("key", entry);

    let entry = f.cache.get("key").unwrap();

    assert_eq!(
        entry.get_output_tree(&*artifact),
        Some(&SerializableTreeArtifactValue::new(
            /* child_values= */ IndexMap::new(),
            /* archived_file_value= */ None,
            Some(resolved_path),
        ))
    );
}
