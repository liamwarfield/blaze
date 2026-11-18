//! Port of `lib/actions/cache/CompactPersistentActionCache.java`: the on-disk action cache.

use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use blaze_events::{Clock, Event, EventHandler, NullEventHandler};
use blaze_shell::InterruptedException;
use blaze_shell::thread_interrupt;
use blaze_util::{check_argument, check_state};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;
use parking_lot::Mutex;

use super::action_cache::{
    ActionCache, ActionCacheEntry, ActionCacheStatistics, MissDetail, MissReason,
    SerializableTreeArtifactValue,
};
use super::digest_utils::DigestUtils;
use super::map_codec::{DataInput, DataOutput, MapCodec, is_incompatible_format_exception};
use super::metadata_digest_utils::{
    MetadataDigestUtils, get_expiration_time, instant_to_string, is_in_memory_output,
};
use super::output_metadata_store::IoOrInterruptedException;
use super::persistent_map::{PersistentMap, PersistentMapHooks, PersistentMapJournal};
use super::persistent_string_indexer::{PersistentStringIndexer, StringIndexer};
use super::var_int::{ByteBuffer, VarInt, is_buffer_underflow};
use crate::actions::file_artifact_value::FileArtifactValue;

// Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:71 CompactPersistentActionCache.SAVE_INTERVAL
const SAVE_INTERVAL_NANOS: i64 = 3 * 1000 * 1000 * 1000;

// Key of the action cache record that holds information used to verify referential integrity
// between action cache and string indexer. Must be < 0 to avoid conflict with real action
// cache records.
// Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:76 CompactPersistentActionCache.VALIDATION_KEY
const VALIDATION_KEY: i32 = -10;

// Padded with 900 to distinguish the Bazel 9.x release line from future versions at HEAD.
// Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:79 CompactPersistentActionCache.VERSION
const VERSION: i32 = 90024;

/// The miss reasons in `MissReason` declaration order (Java `MissReason.values()`, without the
/// protobuf `UNRECOGNIZED`).
const MISS_REASONS: [MissReason; 8] = [
    MissReason::DifferentActionKey,
    MissReason::DifferentDeps,
    MissReason::DifferentEnvironment,
    MissReason::DifferentFiles,
    MissReason::CorruptedCacheEntry,
    MissReason::NotCached,
    MissReason::UnconditionalExecution,
    MissReason::DigestMismatch,
];

/// A timestamp, represented as the number of minutes since the Unix epoch.
///
/// This provides adequate accuracy for garbage collection purposes while reducing storage
/// requirements.
// PORT: Java interns instances (BlazeInterners.newWeakInterner); a Copy i32 needs no interning.
// Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:87 CompactPersistentActionCache.Timestamp
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Timestamp {
    epoch_minutes: i32,
}

// Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:91 CompactPersistentActionCache.Timestamp.MINUTE_IN_MILLIS
const MINUTE_IN_MILLIS: i64 = 60 * 1000;

impl Timestamp {
    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:99 CompactPersistentActionCache.Timestamp.fromEpochMinutes
    fn from_epoch_minutes(epoch_minutes: i32) -> Timestamp {
        Timestamp { epoch_minutes }
    }

    /// `fromInstant(Instant)`, with the instant as epoch milliseconds.
    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:103 CompactPersistentActionCache.Timestamp.fromInstant
    fn from_instant(epoch_millis: i64) -> Timestamp {
        Timestamp::from_epoch_minutes((epoch_millis / MINUTE_IN_MILLIS) as i32)
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:107 CompactPersistentActionCache.Timestamp.toEpochMinutes
    fn to_epoch_minutes(self) -> i32 {
        self.epoch_minutes
    }

    /// `toInstant()`, as epoch milliseconds.
    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:111 CompactPersistentActionCache.Timestamp.toInstant
    fn to_instant(self) -> i64 {
        self.epoch_minutes as i64 * MINUTE_IN_MILLIS
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:129 CompactPersistentActionCache.Timestamp.toString
    fn to_java_string(self) -> String {
        instant_to_string(self.to_instant())
    }
}

// Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:134 CompactPersistentActionCache.TIMESTAMP_CODEC
struct TimestampCodec;

impl MapCodec<i32, Timestamp> for TimestampCodec {
    fn read_key(&self, in_: &mut DataInput) -> io::Result<i32> {
        in_.read_int()
    }

    fn read_value(&self, in_: &mut DataInput) -> io::Result<Timestamp> {
        Ok(Timestamp::from_epoch_minutes(in_.read_int()?))
    }

    fn write_key(&self, key: &i32, out: &mut DataOutput<'_>) -> io::Result<()> {
        out.write_int(*key)
    }

    fn write_value(&self, value: &Timestamp, out: &mut DataOutput<'_>) -> io::Result<()> {
        out.write_int(value.to_epoch_minutes())
    }
}

/// `shouldKeepJournal` shared by `TimestampMap` and `ActionMap`.
fn should_keep_journal_if_small(map: &dyn PersistentMapJournal) -> bool {
    // We must first flush the journal to get an accurate measure of its size.
    map.flush_journal();
    match (map.journal_size(), map.cache_size()) {
        (Ok(journal_size), Ok(cache_size)) => journal_size * 100 < cache_size,
        _ => false,
    }
}

/// The policy of `TimestampMap`, a [`PersistentMap`] mapping the string index of the action's
/// primary output path to that entry's last access time.
// Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:161 CompactPersistentActionCache.TimestampMap
struct TimestampMapHooks {
    clock: Arc<dyn Clock>,
    next_update_nanos: Mutex<i64>,
}

impl PersistentMapHooks for TimestampMapHooks {
    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:174 CompactPersistentActionCache.TimestampMap.shouldFlushJournal
    fn should_flush_journal(&self) -> bool {
        // Use nanoTime() instead of currentTimeMillis() to get monotonic time, not wall time.
        let current_time_nanos = self.clock.nano_time();
        let mut next_update_nanos = self.next_update_nanos.lock();
        if current_time_nanos > *next_update_nanos {
            *next_update_nanos = current_time_nanos + SAVE_INTERVAL_NANOS;
            return true;
        }
        false
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:185 CompactPersistentActionCache.TimestampMap.shouldKeepJournal
    fn should_keep_journal(&self, map: &dyn PersistentMapJournal) -> bool {
        should_keep_journal_if_small(map)
    }
}

// Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:165 CompactPersistentActionCache.TimestampMap.<init>
fn new_timestamp_map(
    clock: Arc<dyn Clock>,
    timestamp_file: Path,
    timestamp_journal_file: Path,
) -> io::Result<PersistentMap<i32, Timestamp>> {
    let next_update_nanos = clock.nano_time() + SAVE_INTERVAL_NANOS;
    let map = PersistentMap::new(
        VERSION,
        Box::new(TimestampCodec),
        Arc::new(TimestampMapHooks {
            clock,
            next_update_nanos: Mutex::new(next_update_nanos),
        }),
        timestamp_file,
        timestamp_journal_file,
    );
    map.load()?;
    Ok(map)
}

// Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:200 CompactPersistentActionCache.ACTION_CODEC
struct ActionCodec;

impl MapCodec<i32, Vec<u8>> for ActionCodec {
    fn read_key(&self, in_: &mut DataInput) -> io::Result<i32> {
        in_.read_int()
    }

    fn read_value(&self, in_: &mut DataInput) -> io::Result<Vec<u8>> {
        let size = in_.read_int()?;
        if size < 0 {
            return Err(io::Error::other(format!(
                "found negative array size: {}",
                size
            )));
        }
        let mut data = vec![0u8; size as usize];
        in_.read_fully(&mut data)?;
        Ok(data)
    }

    fn write_key(&self, key: &i32, out: &mut DataOutput<'_>) -> io::Result<()> {
        out.write_int(*key)
    }

    fn write_value(&self, value: &Vec<u8>, out: &mut DataOutput<'_>) -> io::Result<()> {
        out.write_int(value.len() as i32)?;
        out.write(value)
    }
}

/// The policy of `ActionMap`, a [`PersistentMap`] mapping the string index of the action's
/// primary output path to the serialized [`ActionCacheEntry`].
// Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:234 CompactPersistentActionCache.ActionMap
struct ActionMapHooks {
    clock: Arc<dyn Clock>,
    indexer: Arc<PersistentStringIndexer>,
    timestamp_map: Arc<PersistentMap<i32, Timestamp>>,
    next_update_nanos: Mutex<i64>,
}

impl PersistentMapHooks for ActionMapHooks {
    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:257 CompactPersistentActionCache.ActionMap.shouldFlushJournal
    fn should_flush_journal(&self) -> bool {
        // Use nanoTime() instead of currentTimeMillis() to get monotonic time, not wall time.
        let current_time_nanos = self.clock.nano_time();
        let flush = {
            let mut next_update_nanos = self.next_update_nanos.lock();
            if current_time_nanos > *next_update_nanos {
                *next_update_nanos = current_time_nanos + SAVE_INTERVAL_NANOS;
                true
            } else {
                false
            }
        };
        if flush {
            // Flush the PersistentStringIndexer and TimestampMap.
            // This ensures an action isn't saved to disk before its timestamp or referenced
            // strings.
            self.indexer.flush();
            self.timestamp_map.flush_journal();
            return true;
        }
        false
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:272 CompactPersistentActionCache.ActionMap.shouldKeepJournal
    fn should_keep_journal(&self, map: &dyn PersistentMapJournal) -> bool {
        should_keep_journal_if_small(map)
    }
}

// Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:240 CompactPersistentActionCache.ActionMap.<init>
fn new_action_map(
    indexer: Arc<PersistentStringIndexer>,
    timestamp_map: Arc<PersistentMap<i32, Timestamp>>,
    clock: Arc<dyn Clock>,
    map_file: Path,
    journal_file: Path,
) -> io::Result<PersistentMap<i32, Vec<u8>>> {
    // Use nanoTime() instead of currentTimeMillis() to get monotonic time, not wall time.
    let next_update_nanos = clock.nano_time() + SAVE_INTERVAL_NANOS;
    let map = PersistentMap::new(
        VERSION,
        Box::new(ActionCodec),
        Arc::new(ActionMapHooks {
            clock,
            indexer,
            timestamp_map,
            next_update_nanos: Mutex::new(next_update_nanos),
        }),
        map_file,
        journal_file,
    );
    map.load()?;
    Ok(map)
}

/// Java `IOException.getMessage()` for an [`io::Error`] (`EOFException` has a null message).
fn java_message(e: &io::Error) -> String {
    if e.kind() == io::ErrorKind::UnexpectedEof && e.get_ref().is_none() {
        "null".to_string()
    } else {
        e.to_string()
    }
}

/// An implementation of the [`ActionCache`] interface that uses a [`StringIndexer`] to reduce
/// memory footprint and saves cached actions using the [`PersistentMap`].
// Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:68 CompactPersistentActionCache
pub struct CompactPersistentActionCache {
    cache_root: Path,
    corrupted_cache_root: Path,
    tmp_dir: Path,
    clock: Arc<dyn Clock>,
    indexer: Arc<PersistentStringIndexer>,
    action_map: PersistentMap<i32, Vec<u8>>,
    timestamp_map: Arc<PersistentMap<i32, Timestamp>>,
    misses: Vec<(MissReason, AtomicI32)>,
    hits: AtomicI32,
    load_time: Mutex<Option<Duration>>,
}

impl CompactPersistentActionCache {
    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:313 CompactPersistentActionCache.create
    pub fn create(
        cache_root: &Path,
        corrupted_cache_root: &Path,
        tmp_dir: &Path,
        clock: Arc<dyn Clock>,
        reporter_for_initialization_errors: &dyn EventHandler,
    ) -> io::Result<CompactPersistentActionCache> {
        let before = clock.current_time_millis();
        let compact_persistent_action_cache = CompactPersistentActionCache::create_retrying(
            cache_root,
            corrupted_cache_root,
            tmp_dir,
            clock.clone(),
            reporter_for_initialization_errors,
            /* retrying= */ false,
        )?;
        let after = clock.current_time_millis();
        // PORT: java.time.Duration may be negative; std Duration cannot, so it is clamped at 0.
        *compact_persistent_action_cache.load_time.lock() =
            Some(Duration::from_millis((after - before).max(0) as u64));

        Ok(compact_persistent_action_cache)
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:335 CompactPersistentActionCache.create
    fn create_retrying(
        cache_root: &Path,
        corrupted_cache_root: &Path,
        tmp_dir: &Path,
        clock: Arc<dyn Clock>,
        reporter_for_initialization_errors: &dyn EventHandler,
        retrying: bool,
    ) -> io::Result<CompactPersistentActionCache> {
        cache_root.create_directory_and_parents()?;

        let cache_file = CompactPersistentActionCache::cache_file(cache_root);
        let journal_file = CompactPersistentActionCache::journal_file(cache_root);
        let index_file = CompactPersistentActionCache::index_file(cache_root);
        let index_journal_file = CompactPersistentActionCache::index_journal_file(cache_root);
        let timestamp_file = CompactPersistentActionCache::timestamp_file(cache_root);
        let timestamp_journal_file =
            CompactPersistentActionCache::timestamp_journal_file(cache_root);

        let indexer =
            match PersistentStringIndexer::create(index_file, index_journal_file, clock.clone()) {
                Ok(indexer) => Arc::new(indexer),
                Err(e) => {
                    return CompactPersistentActionCache::log_and_throw_or_recurse(
                        cache_root,
                        corrupted_cache_root,
                        tmp_dir,
                        clock,
                        "Failed to load action cache index data",
                        e,
                        reporter_for_initialization_errors,
                        retrying,
                    );
                }
            };

        let timestamp_map =
            match new_timestamp_map(clock.clone(), timestamp_file, timestamp_journal_file) {
                Ok(map) => Arc::new(map),
                Err(e) => {
                    return CompactPersistentActionCache::log_and_throw_or_recurse(
                        cache_root,
                        corrupted_cache_root,
                        tmp_dir,
                        clock,
                        "Failed to load action cache timestamp data",
                        e,
                        reporter_for_initialization_errors,
                        retrying,
                    );
                }
            };

        let action_map = match new_action_map(
            indexer.clone(),
            timestamp_map.clone(),
            clock.clone(),
            cache_file,
            journal_file,
        ) {
            Ok(map) => map,
            Err(e) => {
                return CompactPersistentActionCache::log_and_throw_or_recurse(
                    cache_root,
                    corrupted_cache_root,
                    tmp_dir,
                    clock,
                    "Failed to load action cache data",
                    e,
                    reporter_for_initialization_errors,
                    retrying,
                );
            }
        };

        // Validate referential integrity between action map and indexer.
        if !action_map.is_empty()
            && let Err(e) = CompactPersistentActionCache::validate_integrity(
                indexer.size(),
                action_map.get(&VALIDATION_KEY).as_deref(),
            )
        {
            return CompactPersistentActionCache::log_and_throw_or_recurse(
                cache_root,
                corrupted_cache_root,
                tmp_dir,
                clock,
                "Failed action cache referential integrity check",
                e,
                reporter_for_initialization_errors,
                retrying,
            );
        }

        // Delete unrecognized on-disk files left around by previous Bazel versions.
        // This is required to fix an incrementality bug that occurs when building a runfiles tree
        // while alternating between two Bazel versions (as might occur during a migration).
        // See https://github.com/bazelbuild/bazel/issues/26818 for details.
        // Note that such files can only originate from Bazel 8 and earlier, which embedded the
        // version number in filenames; Bazel 9 and later use fixed filenames. Thus this can be
        // safely removed once backcompatibility of the output tree with Bazel 8 and earlier is no
        // longer a concern.
        CompactPersistentActionCache::delete_unrecognized_files(cache_root)?;

        // Populate the map now, so that concurrent updates to the values can happen safely.
        let misses = MISS_REASONS
            .iter()
            .map(|reason| (*reason, AtomicI32::new(0)))
            .collect();

        Ok(CompactPersistentActionCache {
            cache_root: cache_root.clone(),
            corrupted_cache_root: corrupted_cache_root.clone(),
            tmp_dir: tmp_dir.clone(),
            clock,
            indexer,
            action_map,
            timestamp_map,
            misses,
            hits: AtomicI32::new(0),
            load_time: Mutex::new(None),
        })
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:445 CompactPersistentActionCache.deleteUnrecognizedFiles
    fn delete_unrecognized_files(cache_root: &Path) -> io::Result<()> {
        let known_files = [
            CompactPersistentActionCache::cache_file(cache_root),
            CompactPersistentActionCache::journal_file(cache_root),
            CompactPersistentActionCache::index_file(cache_root),
            CompactPersistentActionCache::index_journal_file(cache_root),
            CompactPersistentActionCache::timestamp_file(cache_root),
            CompactPersistentActionCache::timestamp_journal_file(cache_root),
        ];
        for child in cache_root.get_directory_entries()? {
            if !known_files.contains(&child) {
                child.delete()?;
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:461 CompactPersistentActionCache.logAndThrowOrRecurse
    fn log_and_throw_or_recurse(
        cache_root: &Path,
        corrupted_cache_root: &Path,
        tmp_dir: &Path,
        clock: Arc<dyn Clock>,
        message: &str,
        e: io::Error,
        reporter_for_initialization_errors: &dyn EventHandler,
        retrying: bool,
    ) -> io::Result<CompactPersistentActionCache> {
        if retrying {
            // Prevent a retry loop.
            return Err(io::Error::other(
                "Action cache initialization is stuck in a retry loop",
            ));
        }

        if is_incompatible_format_exception(&e) {
            // Format incompatibility is expected when switching between Bazel versions, so we
            // don't treat it as corruption; we simply delete the cache directory and start fresh.
            cache_root.delete_tree()?;
        } else {
            // Move the corrupted cache to a separate location so it can be analyzed later.
            // This also ensures that the next initialization attempt will create an empty cache.
            // To avoid using too much disk space, only keep the most recent corrupted cache
            // around.
            corrupted_cache_root.delete_tree()?;
            if cache_root.rename_to(corrupted_cache_root).is_err() {
                // If renaming fails, delete instead.
                cache_root.delete_tree()?;
            }

            let e = io::Error::other(format!("{}: {}", message, java_message(&e)));

            // PORT: logging dropped.

            reporter_for_initialization_errors.handle(&Event::warn(format!(
                "Error during action cache initialization: {}. Data may be incomplete, \
                 potentially causing rebuilds",
                e
            )));
        }

        CompactPersistentActionCache::create_retrying(
            cache_root,
            corrupted_cache_root,
            tmp_dir,
            clock,
            reporter_for_initialization_errors,
            /* retrying= */ true,
        )
    }

    /// Fails if indexer contains no data or integrity check has failed.
    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:523 CompactPersistentActionCache.validateIntegrity
    fn validate_integrity(indexer_size: i32, validation_record: Option<&[u8]>) -> io::Result<()> {
        if indexer_size == 0 {
            return Err(io::Error::other("empty index"));
        }
        let Some(validation_record) = validation_record else {
            return Err(io::Error::other("missing validation record"));
        };
        match ByteBuffer::wrap(validation_record).get_int() {
            Ok(validation_size) => {
                if validation_size > indexer_size {
                    return Err(io::Error::other(format!(
                        "validation record {} is too large compared to index size {}",
                        validation_size, indexer_size
                    )));
                }
                Ok(())
            }
            Err(_) => Err(io::Error::other("validation record is incomplete")),
        }
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:544 CompactPersistentActionCache.cacheFile
    pub fn cache_file(cache_root: &Path) -> Path {
        cache_root.get_child("action_cache.blaze")
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:548 CompactPersistentActionCache.journalFile
    pub fn journal_file(cache_root: &Path) -> Path {
        cache_root.get_child("action_journal.blaze")
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:552 CompactPersistentActionCache.indexFile
    pub fn index_file(cache_root: &Path) -> Path {
        cache_root.get_child("filename_index.blaze")
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:556 CompactPersistentActionCache.indexJournalFile
    pub fn index_journal_file(cache_root: &Path) -> Path {
        cache_root.get_child("filename_index_journal.blaze")
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:560 CompactPersistentActionCache.timestampFile
    pub fn timestamp_file(cache_root: &Path) -> Path {
        cache_root.get_child("timestamp.blaze")
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:564 CompactPersistentActionCache.timestampJournalFile
    pub fn timestamp_journal_file(cache_root: &Path) -> Path {
        cache_root.get_child("timestamp_journal.blaze")
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:591 CompactPersistentActionCache.put
    fn put_with_timestamp(&self, key: &str, entry: &ActionCacheEntry, timestamp_millis: i64) {
        // Encode record. Note that both methods may create new mappings in the indexer.
        let index = self.indexer.get_or_create_index(key);
        let content = match self.encode(entry) {
            Ok(content) => content,
            Err(_) => {
                // PORT: logging dropped ("Failed to save cache entry %s with key %s").
                return;
            }
        };

        // Update validation record.
        // Note the benign race condition in which two threads might race on updating the
        // validation record: if the most recent update loses the race, a value lower than the
        // indexer size will remain in the validation record, which will still pass the integrity
        // check.
        let index_size = self.indexer.size();
        self.action_map
            .put(VALIDATION_KEY, index_size.to_be_bytes().to_vec());

        // Update the timestamp map.
        self.timestamp_map
            .put(index, Timestamp::from_instant(timestamp_millis));

        // Update the action map.
        // This is last so that, if a flush occurs, the index and timestamp also make it to disk.
        self.action_map.put(index, content);
    }

    /// Returns a map from action key to last access time (epoch millis).
    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:676 CompactPersistentActionCache.getActionTimestampMap
    pub fn get_action_timestamp_map(&self) -> IndexMap<String, i64> {
        // Iterate the timestamp map, not the action map, so that the result may be used for
        // testing that an entry is removed from the timestamp map when removed from the action
        // map. Note that the indexer does not support removing entries.
        let mut builder = IndexMap::with_capacity(self.timestamp_map.size());
        for (key, value) in self.timestamp_map.entries() {
            if let Some(action_key) = self.indexer.get_string_for_index(key) {
                // buildKeepingLast
                builder.insert(action_key, value.to_instant());
            }
        }
        builder
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:693 CompactPersistentActionCache.trim
    pub fn trim(
        self: Arc<Self>,
        threshold: f32,
        max_age: Duration,
    ) -> Result<Arc<CompactPersistentActionCache>, IoOrInterruptedException> {
        let cutoff_time = self.clock.current_time_millis() - max_age.as_millis() as i64;

        let access_time_map = self.get_action_timestamp_map();

        // Count the number of stale entries.
        let mut num_stale: i32 = 0;
        for (_, access_time) in &access_time_map {
            if thread_interrupt::interrupted() {
                // If interrupted, return promptly.
                return Err(InterruptedException.into());
            }
            if *access_time < cutoff_time {
                num_stale += 1;
            }
        }

        // Skip garbage collection if below the threshold.
        if num_stale == 0 || (num_stale as f32) < threshold * (self.action_map.size() as f32) {
            return Ok(self);
        }

        // Clear preexisting temporary directory contents.
        self.tmp_dir.delete_tree()?;

        let new_root = self.tmp_dir.get_child("new");
        let old_root = self.tmp_dir.get_child("old");

        // Create a new cache backed by a temporary directory.
        let new_cache = CompactPersistentActionCache::create(
            &new_root,
            &self.corrupted_cache_root,
            &self.tmp_dir,
            self.clock.clone(),
            NullEventHandler::INSTANCE,
        )?;

        // Copy sufficiently recent entries into the new cache.
        for (key, value) in self.action_map.entries() {
            if thread_interrupt::interrupted() {
                // If interrupted, return promptly but avoid leaving the temporary directory
                // behind.
                self.tmp_dir.delete_tree()?;
                return Err(InterruptedException.into());
            }
            if key == VALIDATION_KEY {
                // Skip the validation record.
                continue;
            }
            let action_key = self
                .indexer
                .get_string_for_index(key)
                .unwrap_or_else(|| panic!("NullPointerException: {}", key));
            // If the timestamp is missing, assume the entry was recently added but its timestamp
            // update was lost.
            let timestamp = access_time_map
                .get(&action_key)
                .copied()
                .unwrap_or_else(|| self.clock.current_time_millis());
            if timestamp < cutoff_time {
                continue;
            }
            // The entry must be reencoded so that strings it references are inserted into the
            // indexer.
            new_cache.put_with_timestamp(&action_key, &self.decode(&value), timestamp);
        }

        // Save the new cache to disk.
        new_cache.save()?;

        // Replace the on-disk representation.
        self.cache_root.rename_to(&old_root)?;
        new_root.rename_to(&self.cache_root)?;

        // Delete the temporary directory.
        self.tmp_dir.delete_tree()?;

        // Reload the cache from disk and return it.
        Ok(Arc::new(CompactPersistentActionCache::create(
            &self.cache_root,
            &self.corrupted_cache_root,
            &self.tmp_dir,
            self.clock.clone(),
            NullEventHandler::INSTANCE,
        )?))
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:794 CompactPersistentActionCache.encodeRemoteMetadata
    fn encode_remote_metadata(
        &self,
        value: &FileArtifactValue,
        sink: &mut Vec<u8>,
    ) -> io::Result<()> {
        check_argument!(value.is_remote(), "metadata is not remote: {}", value);

        MetadataDigestUtils::write(value.get_digest().unwrap_or_default(), sink)?;

        VarInt::put_var_long(value.get_size(), sink)?;

        VarInt::put_var_int(value.get_location_index(), sink)?;

        VarInt::put_var_long(get_expiration_time(value).unwrap_or(-1), sink)?;

        VarInt::put_var_int(if is_in_memory_output(value) { 1 } else { 0 }, sink)?;

        match value.get_resolved_path() {
            Some(resolved_path) => {
                VarInt::put_var_int(1, sink)?;
                VarInt::put_var_int(
                    self.indexer
                        .get_or_create_index(resolved_path.get_path_string()),
                    sink,
                )?;
            }
            None => {
                VarInt::put_var_int(0, sink)?;
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:818 CompactPersistentActionCache.MAX_REMOTE_METADATA_SIZE
    const MAX_REMOTE_METADATA_SIZE: i32 = (1 + DigestUtils::ESTIMATED_SIZE) // digest length + digest
        + VarInt::MAX_VARLONG_SIZE // size
        + VarInt::MAX_VARINT_SIZE // locationIndex
        + VarInt::MAX_VARLONG_SIZE // expirationTime
        + VarInt::MAX_VARINT_SIZE // inMemoryOutput
        + (1 + VarInt::MAX_VARINT_SIZE); // resolvedPath

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:826 CompactPersistentActionCache.decodeRemoteMetadata
    fn decode_remote_metadata(&self, source: &mut ByteBuffer<'_>) -> io::Result<FileArtifactValue> {
        let digest = MetadataDigestUtils::read(source)?;

        let size = VarInt::get_var_long(source)?;

        let location_index = VarInt::get_var_int(source)?;

        let expiration_time_epoch_milli = VarInt::get_var_long(source)?;

        let in_memory_output = VarInt::get_var_int(source)? != 0;

        let mut resolved_path = None;
        let num_resolved_path = VarInt::get_var_int(source)?;
        if num_resolved_path > 0 {
            if num_resolved_path != 1 {
                return Err(io::Error::other(
                    "Invalid presence marker for resolved path",
                ));
            }
            resolved_path = Some(PathFragment::create(&get_string_for_index(
                &*self.indexer,
                VarInt::get_var_int(source)?,
            )?));
        }

        let mut metadata = if expiration_time_epoch_milli >= 0 || in_memory_output {
            let expiration_time = if expiration_time_epoch_milli >= 0 {
                Some(expiration_time_epoch_milli)
            } else {
                None
            };
            FileArtifactValue::create_for_remote_file_with_materialization_data(
                digest,
                size,
                location_index,
                expiration_time,
                in_memory_output,
            )
        } else {
            FileArtifactValue::create_for_remote_file(digest, size, location_index)
        };

        if let Some(resolved_path) = resolved_path {
            metadata =
                FileArtifactValue::create_from_existing_with_resolved_path(metadata, resolved_path);
        }

        Ok(metadata)
    }

    /// Returns the action data encoded as a byte array.
    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:867 CompactPersistentActionCache.encode
    fn encode(&self, entry: &ActionCacheEntry) -> io::Result<Vec<u8>> {
        check_state!(!entry.is_corrupted());

        let mut max_discovered_inputs_size: i32 = 1; // presence marker
        if entry.discovers_inputs() {
            max_discovered_inputs_size += VarInt::MAX_VARINT_SIZE // length
                + (VarInt::MAX_VARINT_SIZE // execPath
                    * entry.get_discovered_input_paths().unwrap().len() as i32);
        }

        let mut max_output_metadata_size: i32 = 1; // presence marker
        if entry.has_output_metadata() {
            let max_output_files_size = VarInt::MAX_VARINT_SIZE // entry.getOutputFiles().size()
                + (VarInt::MAX_VARINT_SIZE // execPath
                    + Self::MAX_REMOTE_METADATA_SIZE)
                    * entry.get_output_files().len() as i32;

            let mut max_output_trees_size = VarInt::MAX_VARINT_SIZE; // entry.getOutputTrees().size()
            for (_, value) in entry.get_output_trees() {
                max_output_trees_size += VarInt::MAX_VARINT_SIZE; // execPath

                max_output_trees_size += VarInt::MAX_VARINT_SIZE; // value.childValues().size()
                max_output_trees_size += (VarInt::MAX_VARINT_SIZE // parentRelativePath
                    + Self::MAX_REMOTE_METADATA_SIZE)
                    * value.child_values().len() as i32;

                // value.archivedFileValue() optional
                max_output_trees_size += 1 + value
                    .archived_file_value()
                    .map_or(0, |_| Self::MAX_REMOTE_METADATA_SIZE);
                // value.resolvedPath() optional
                max_output_trees_size +=
                    1 + value.resolved_path().map_or(0, |_| VarInt::MAX_VARINT_SIZE);
            }

            let max_proxy_outputs_size =
                VarInt::MAX_VARINT_SIZE * (entry.get_proxy_outputs().len() as i32 + 1); // +1 for the size itself.

            max_output_metadata_size +=
                max_output_files_size + max_output_trees_size + max_proxy_outputs_size;
        }

        // Estimate the size of the buffer.
        let max_size = (1 + DigestUtils::ESTIMATED_SIZE) // digest length + digest
            + max_discovered_inputs_size
            + max_output_metadata_size;
        let mut sink: Vec<u8> = Vec::with_capacity(max_size.max(0) as usize);

        MetadataDigestUtils::write(entry.get_digest(), &mut sink)?;

        VarInt::put_var_int(if entry.discovers_inputs() { 1 } else { 0 }, &mut sink)?;
        if let Some(discovered_input_paths) = entry.get_discovered_input_paths() {
            VarInt::put_var_int(discovered_input_paths.len() as i32, &mut sink)?;
            for discovered_input_path in discovered_input_paths {
                VarInt::put_var_int(
                    self.indexer.get_or_create_index(discovered_input_path),
                    &mut sink,
                )?;
            }
        }

        VarInt::put_var_int(if entry.has_output_metadata() { 1 } else { 0 }, &mut sink)?;
        if entry.has_output_metadata() {
            VarInt::put_var_int(entry.get_output_files().len() as i32, &mut sink)?;
            for (exec_path, value) in entry.get_output_files() {
                VarInt::put_var_int(self.indexer.get_or_create_index(exec_path), &mut sink)?;
                self.encode_remote_metadata(value, &mut sink)?;
            }

            VarInt::put_var_int(entry.get_output_trees().len() as i32, &mut sink)?;
            for (tree_key, serializable_tree_artifact_value) in entry.get_output_trees() {
                VarInt::put_var_int(self.indexer.get_or_create_index(tree_key), &mut sink)?;

                VarInt::put_var_int(
                    serializable_tree_artifact_value.child_values().len() as i32,
                    &mut sink,
                )?;
                for (child_key, child_value) in serializable_tree_artifact_value.child_values() {
                    VarInt::put_var_int(self.indexer.get_or_create_index(child_key), &mut sink)?;
                    self.encode_remote_metadata(child_value, &mut sink)?;
                }

                match serializable_tree_artifact_value.archived_file_value() {
                    Some(archived_file_value) => {
                        VarInt::put_var_int(1, &mut sink)?;
                        self.encode_remote_metadata(archived_file_value, &mut sink)?;
                    }
                    None => {
                        VarInt::put_var_int(0, &mut sink)?;
                    }
                }

                match serializable_tree_artifact_value.resolved_path() {
                    Some(resolved_path) => {
                        VarInt::put_var_int(1, &mut sink)?;
                        VarInt::put_var_int(
                            self.indexer
                                .get_or_create_index(resolved_path.get_path_string()),
                            &mut sink,
                        )?;
                    }
                    None => {
                        VarInt::put_var_int(0, &mut sink)?;
                    }
                }
            }

            VarInt::put_var_int(entry.get_proxy_outputs().len() as i32, &mut sink)?;
            for exec_path in entry.get_proxy_outputs() {
                VarInt::put_var_int(self.indexer.get_or_create_index(exec_path), &mut sink)?;
            }
        }

        Ok(sink)
    }

    /// Creates an [`ActionCacheEntry`] from the given compressed data; fails if the compressed
    /// data is corrupted.
    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:993 CompactPersistentActionCache.decodeInternal
    fn decode_internal(&self, data: &[u8]) -> io::Result<ActionCacheEntry> {
        self.decode_internal_unchecked(data).map_err(|e| {
            if is_buffer_underflow(&e) {
                io::Error::other("encoded entry data is incomplete")
            } else {
                e
            }
        })
    }

    fn decode_internal_unchecked(&self, data: &[u8]) -> io::Result<ActionCacheEntry> {
        let mut source = ByteBuffer::wrap(data);
        let source = &mut source;

        let digest = MetadataDigestUtils::read(source)?;

        let mut discovered_input_paths = None;
        let discovered_inputs_presence_marker = VarInt::get_var_int(source)?;
        if discovered_inputs_presence_marker != 0 {
            if discovered_inputs_presence_marker != 1 {
                return Err(io::Error::other(format!(
                    "Invalid presence marker for discovered inputs: {}",
                    discovered_inputs_presence_marker
                )));
            }
            let num_discovered_inputs = VarInt::get_var_int(source)?;
            if num_discovered_inputs < 0 {
                return Err(io::Error::other(format!(
                    "Invalid discovered input count: {}",
                    num_discovered_inputs
                )));
            }
            let mut builder = Vec::with_capacity(num_discovered_inputs as usize);
            for _ in 0..num_discovered_inputs {
                let id = VarInt::get_var_int(source)?;
                let filename = get_string_for_index(&*self.indexer, id)?;
                builder.push(filename);
            }
            discovered_input_paths = Some(builder);
        }

        let output_metadata_presence_marker = VarInt::get_var_int(source)?;
        if output_metadata_presence_marker == 0 {
            if source.remaining() > 0 {
                return Err(io::Error::other(
                    "serialized entry data has not been fully decoded",
                ));
            }
            return Ok(ActionCacheEntry::new(
                Some(digest),
                discovered_input_paths,
                /* output_file_metadata= */ IndexMap::new(),
                /* output_tree_metadata= */ IndexMap::new(),
                /* proxy_outputs= */ Vec::new(),
            ));
        }

        let num_output_files = VarInt::get_var_int(source)?;
        if num_output_files < 0 {
            return Err(io::Error::other(format!(
                "Invalid output file count: {}",
                num_output_files
            )));
        }
        let mut output_files = IndexMap::with_capacity(num_output_files as usize);
        for _ in 0..num_output_files {
            let exec_path = get_string_for_index(&*self.indexer, VarInt::get_var_int(source)?)?;
            let value = self.decode_remote_metadata(source)?;
            put_or_throw(&mut output_files, exec_path, value);
        }

        let num_output_trees = VarInt::get_var_int(source)?;
        if num_output_trees < 0 {
            return Err(io::Error::other(format!(
                "invalid output tree count: {}",
                num_output_trees
            )));
        }
        let mut output_trees = IndexMap::with_capacity(num_output_trees as usize);
        for _ in 0..num_output_trees {
            let tree_key = get_string_for_index(&*self.indexer, VarInt::get_var_int(source)?)?;

            let mut child_values = IndexMap::new();
            let num_child_values = VarInt::get_var_int(source)?;
            for _ in 0..num_child_values {
                let child_key = get_string_for_index(&*self.indexer, VarInt::get_var_int(source)?)?;
                let value = self.decode_remote_metadata(source)?;
                put_or_throw(&mut child_values, child_key, value);
            }

            let mut archived_file_value = None;
            let archived_file_value_presence_marker = VarInt::get_var_int(source)?;
            if archived_file_value_presence_marker != 0 {
                if archived_file_value_presence_marker != 1 {
                    return Err(io::Error::other(format!(
                        "Invalid presence marker for archived representation: {}",
                        archived_file_value_presence_marker
                    )));
                }
                archived_file_value = Some(self.decode_remote_metadata(source)?);
            }

            let mut resolved_path = None;
            let resolved_path_presence_marker = VarInt::get_var_int(source)?;
            if resolved_path_presence_marker != 0 {
                if resolved_path_presence_marker != 1 {
                    return Err(io::Error::other(format!(
                        "Invalid presence marker for resolved path: {}",
                        resolved_path_presence_marker
                    )));
                }
                resolved_path = Some(PathFragment::create(&get_string_for_index(
                    &*self.indexer,
                    VarInt::get_var_int(source)?,
                )?));
            }

            let value = SerializableTreeArtifactValue::new(
                child_values,
                archived_file_value,
                resolved_path,
            );
            put_or_throw(&mut output_trees, tree_key, value);
        }

        let num_proxy_artifacts = VarInt::get_var_int(source)?;
        if num_proxy_artifacts < 0 {
            return Err(io::Error::other(format!(
                "Invalid proxy artifact count: {}",
                num_proxy_artifacts
            )));
        }
        let mut proxy_artifacts = Vec::with_capacity(num_proxy_artifacts as usize);
        for _ in 0..num_proxy_artifacts {
            let exec_path = get_string_for_index(&*self.indexer, VarInt::get_var_int(source)?)?;
            proxy_artifacts.push(exec_path);
        }

        if source.remaining() > 0 {
            return Err(io::Error::other(
                "serialized entry data has not been fully decoded",
            ));
        }
        Ok(ActionCacheEntry::new(
            Some(digest),
            discovered_input_paths,
            output_files,
            output_trees,
            proxy_artifacts,
        ))
    }

    /// Creates an [`ActionCacheEntry`] from the given compressed data, returning
    /// [`ActionCacheEntry::corrupted`] if the compressed data is corrupted.
    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:1120 CompactPersistentActionCache.decode
    fn decode(&self, data: &[u8]) -> ActionCacheEntry {
        match self.decode_internal(data) {
            Ok(entry) => entry,
            Err(_) => ActionCacheEntry::corrupted(),
        }
    }
}

/// Guava `ImmutableMap.Builder.put` + `buildOrThrow`: duplicate keys are an
/// `IllegalArgumentException`.
fn put_or_throw<V>(map: &mut IndexMap<String, V>, key: String, value: V) {
    if map.contains_key(&key) {
        panic!(
            "IllegalArgumentException: Multiple entries with same key: {}",
            key
        );
    }
    map.insert(key, value);
}

// Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:980 CompactPersistentActionCache.getStringForIndex
fn get_string_for_index(indexer: &dyn StringIndexer, index: i32) -> io::Result<String> {
    let path = if index >= 0 {
        indexer.get_string_for_index(index)
    } else {
        None
    };
    path.ok_or_else(|| io::Error::other("Corrupted string index"))
}

impl ActionCache for CompactPersistentActionCache {
    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:570 CompactPersistentActionCache.get
    fn get(&self, key: &str) -> Option<ActionCacheEntry> {
        let index = self.indexer.get_index(key)?;
        let data = self.action_map.get(&index)?;
        let entry = self.decode(&data);
        if !entry.is_corrupted() {
            self.timestamp_map.put(
                index,
                Timestamp::from_instant(self.clock.current_time_millis()),
            );
        }
        Some(entry)
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:587 CompactPersistentActionCache.put
    fn put(&self, key: &str, entry: ActionCacheEntry) {
        self.put_with_timestamp(key, &entry, self.clock.current_time_millis());
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:620 CompactPersistentActionCache.remove
    fn remove(&self, key: &str) {
        if let Some(index) = self.indexer.get_index(key) {
            self.action_map.remove(&index);
            self.timestamp_map.remove(&index);
        }
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:629 CompactPersistentActionCache.removeIf
    fn remove_if(&self, predicate: &dyn Fn(&ActionCacheEntry) -> bool) {
        // Be careful not to cause the timestamp to be updated on kept entries (i.e., don't use
        // get()).
        for (key, value) in self.action_map.entries() {
            if key == VALIDATION_KEY {
                // Skip the validation record.
                continue;
            }
            let decoded_entry = self.decode(&value);
            if decoded_entry.is_corrupted() {
                // Skip corrupted entries.
                continue;
            }
            if predicate(&decoded_entry) {
                // Although this is racy (the key might be concurrently set to a different value),
                // we don't care because it's a very small window and it only impacts performance,
                // not correctness.
                self.action_map.remove(&key);
                self.timestamp_map.remove(&key);
            }
        }
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:652 CompactPersistentActionCache.save
    fn save(&self) -> io::Result<i64> {
        // TODO(b/314086729): Remove after we understand the bug.
        // PORT: logging dropped ("Integrity check failed on the inmemory objects right before
        // save"); the check has no other effect.
        let _ = CompactPersistentActionCache::validate_integrity(
            self.indexer.size(),
            self.action_map.get(&VALIDATION_KEY).as_deref(),
        );

        let index_size = self.indexer.save()?;
        let action_map_size = self.action_map.save()?;
        let timestamp_map_size = self.timestamp_map.save()?;
        Ok(index_size + action_map_size + timestamp_map_size)
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:669 CompactPersistentActionCache.clear
    fn clear(&self) {
        self.indexer.clear();
        self.action_map.clear();
        self.timestamp_map.clear();
    }

    fn trim(
        self: Arc<Self>,
        threshold: f32,
        max_age: Duration,
    ) -> Result<Arc<dyn ActionCache>, IoOrInterruptedException> {
        let cache: Arc<dyn ActionCache> =
            CompactPersistentActionCache::trim(self, threshold, max_age)?;
        Ok(cache)
    }

    /// Dumps the action cache into a human-readable format.
    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:766 CompactPersistentActionCache.dump
    fn dump(&self, out: &mut dyn Write) {
        // PORT: PrintStream swallows write errors.
        let mut sorted_keys: Vec<i32> = self
            .action_map
            .keys()
            .into_iter()
            .filter(|k| *k != VALIDATION_KEY)
            .collect();
        sorted_keys.sort();
        let _ = writeln!(out, "Action cache ({} records):", sorted_keys.len());
        for key in sorted_keys {
            let Some(encoded_entry) = self.action_map.get(&key) else {
                panic!("NullPointerException");
            };
            let decoded_entry = self.decode(&encoded_entry);
            let timestamp = self.timestamp_map.get(&key);
            let _ = writeln!(
                out,
                "  {} -> {}",
                key,
                self.indexer
                    .get_string_for_index(key)
                    .unwrap_or_else(|| "null".to_string())
            );
            let _ = writeln!(out, "  packed_len = {}", encoded_entry.len());
            let _ = writeln!(
                out,
                "  timestamp = {}",
                match timestamp {
                    Some(timestamp) => timestamp.to_java_string(),
                    None => "unknown".to_string(),
                }
            );
            decoded_entry.dump(out);
        }
        self.indexer.dump(out);
    }

    /// Returns the number of entries in the action map. If non-zero, it means that the map has
    /// been initialized and contains the validation record.
    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:790 CompactPersistentActionCache.size
    fn size(&self) -> i32 {
        self.action_map.size() as i32
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:1129 CompactPersistentActionCache.accountHit
    fn account_hit(&self) {
        self.hits.fetch_add(1, Ordering::SeqCst);
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:1134 CompactPersistentActionCache.accountMiss
    fn account_miss(&self, reason: MissReason) {
        let Some((_, counter)) = self.misses.iter().find(|(r, _)| *r == reason) else {
            panic!(
                "NullPointerException: Miss reason {} was not registered in the misses map during cache construction",
                reason.as_str_name()
            );
        };
        counter.fetch_add(1, Ordering::SeqCst);
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:1144 CompactPersistentActionCache.mergeIntoActionCacheStatistics
    fn merge_into_action_cache_statistics(&self, builder: &mut ActionCacheStatistics) {
        builder.hits = self.hits.load(Ordering::SeqCst);

        let mut total_misses = 0;
        for (reason, counter) in &self.misses {
            let count = counter.load(Ordering::SeqCst);
            builder.miss_details.push(MissDetail {
                reason: *reason as i32,
                count,
            });
            total_misses += count;
        }
        builder.misses = total_misses;
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:1157 CompactPersistentActionCache.resetStatistics
    fn reset_statistics(&self) {
        self.hits.store(0, Ordering::SeqCst);
        for (_, counter) in &self.misses {
            counter.store(0, Ordering::SeqCst);
        }
    }

    // Java: com/google/devtools/build/lib/actions/cache/CompactPersistentActionCache.java:1166 CompactPersistentActionCache.getLoadTime
    fn get_load_time(&self) -> Option<Duration> {
        // As a side effect, reset the load time, so it is only reported for the actual
        // invocation that loaded the action cache.
        self.load_time.lock().take()
    }
}
