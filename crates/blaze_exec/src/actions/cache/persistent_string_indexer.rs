//! Port of `lib/actions/cache/PersistentStringIndexer.java` and the `lib/util/StringIndexer.java`
//! interface it implements.

use std::io::{self, Write};
use std::sync::Arc;

use blaze_events::Clock;
use blaze_vfs::Path;
use parking_lot::{Mutex, RwLock};

use super::map_codec::{DataInput, DataOutput, MapCodec};
use super::persistent_map::{PersistentMap, PersistentMapHooks};

/// Provides bidirectional `String` <=> unique integer mapping.
// Java: com/google/devtools/build/lib/util/StringIndexer.java:25 StringIndexer
pub trait StringIndexer: Send + Sync {
    /// Removes all mappings.
    // Java: com/google/devtools/build/lib/util/StringIndexer.java:28 StringIndexer.clear
    fn clear(&self);

    /// Returns the number of strings in the index.
    // Java: com/google/devtools/build/lib/util/StringIndexer.java:31 StringIndexer.size
    fn size(&self) -> i32;

    /// Creates new mapping for the given string if necessary and returns string index.
    // Java: com/google/devtools/build/lib/util/StringIndexer.java:34 StringIndexer.getOrCreateIndex
    fn get_or_create_index(&self, s: &str) -> i32;

    /// Returns the unique index for the given string if one was created via
    /// `get_or_create_index`, or else `None`.
    // Java: com/google/devtools/build/lib/util/StringIndexer.java:41 StringIndexer.getIndex
    fn get_index(&self, s: &str) -> Option<i32>;

    /// Returns the string associated with the given index or `None` if it is not in the index.
    // Java: com/google/devtools/build/lib/util/StringIndexer.java:47 StringIndexer.getStringForIndex
    fn get_string_for_index(&self, i: i32) -> Option<String>;
}

// Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:48 PersistentStringIndexer.INITIAL_CAPACITY
const INITIAL_CAPACITY: usize = 8192;

/// Persistent implementation of [`StringIndexer`].
///
/// This class is backed by a [`PersistentMap`] that holds one direction of the canonicalization
/// mapping. The other direction is handled purely in memory and reconstituted at load-time.
///
/// Thread-safety is ensured by locking on all mutating operations. Read-only operations are not
/// locked.
// Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:46 PersistentStringIndexer
pub struct PersistentStringIndexer {
    lock: Mutex<()>,
    // These two fields act similarly to a (synchronized) BiMap. Mutating operations are performed
    // in synchronized blocks.
    string_to_int: PersistentMap<String, i32>,
    /// Java `AtomicReferenceArray<String>`: its length is the capacity.
    int_to_string: RwLock<Vec<Option<String>>>,
}

impl PersistentStringIndexer {
    /// Instantiates and loads instance of the persistent string indexer.
    // Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:51 PersistentStringIndexer.create
    pub fn create(
        data_path: Path,
        journal_path: Path,
        clock: Arc<dyn Clock>,
    ) -> io::Result<PersistentStringIndexer> {
        let string_to_int = new_persistent_index_map(data_path, journal_path, clock)?;

        // INITIAL_CAPACITY or the next power of two greater than the size.
        let size = string_to_int.size() as i32;
        let highest_one_bit = if size == 0 {
            0
        } else {
            1i32 << (31 - size.leading_zeros())
        };
        let capacity = (INITIAL_CAPACITY as i32).max(highest_one_bit.wrapping_shl(1)) as usize;

        let mut int_to_string: Vec<Option<String>> = vec![None; capacity];
        for (key, index) in string_to_int.entries() {
            if index < 0 || index as usize >= capacity {
                return Err(io::Error::other(format!(
                    "Corrupted filename index {} out of bounds for length {} (map size {})",
                    index,
                    capacity,
                    string_to_int.size()
                )));
            }
            let slot = &mut int_to_string[index as usize];
            if slot.is_some() {
                return Err(io::Error::other(format!(
                    "Corrupted filename index has duplicate entry: {}",
                    key
                )));
            }
            *slot = Some(key);
        }
        Ok(PersistentStringIndexer {
            lock: Mutex::new(()),
            string_to_int,
            int_to_string: RwLock::new(int_to_string),
        })
    }

    /// Saves index data to the file.
    // Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:154 PersistentStringIndexer.save
    pub fn save(&self) -> io::Result<i64> {
        let _guard = self.lock.lock();
        self.string_to_int.save()
    }

    /// Flushes the journal.
    // Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:164 PersistentStringIndexer.flush
    pub fn flush(&self) {
        let _guard = self.lock.lock();
        self.string_to_int.flush_journal();
    }

    // Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:173 PersistentStringIndexer.dump
    pub fn dump(&self, out: &mut dyn Write) {
        let _guard = self.lock.lock();
        // PORT: PrintStream swallows write errors; its charset is ISO-8859-1 (println_latin1!).
        println_latin1!(out, "String indexer ({} records):", self.size());
        for i in 0..self.size() {
            println_latin1!(
                out,
                "  {} <=> {}",
                i,
                self.get_string_for_index(i)
                    .unwrap_or_else(|| "null".to_string())
            );
        }
    }
}

impl StringIndexer for PersistentStringIndexer {
    // Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:88 PersistentStringIndexer.clear
    fn clear(&self) {
        let _guard = self.lock.lock();
        self.string_to_int.clear();
        *self.int_to_string.write() = vec![None; INITIAL_CAPACITY];
    }

    // Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:99 PersistentStringIndexer.size
    fn size(&self) -> i32 {
        self.string_to_int.size() as i32
    }

    // Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:104 PersistentStringIndexer.getOrCreateIndex
    fn get_or_create_index(&self, s: &str) -> i32 {
        if let Some(i) = self.string_to_int.get(&s.to_string()) {
            return i;
        }
        // PORT: String.intern() dropped (identity only).
        let _guard = self.lock.lock();
        let i = self.string_to_int.size() as i32;
        if let Some(existing) = self.string_to_int.put_if_absent(s.to_string(), i) {
            return existing; // Another thread won the race.
        }
        let mut int_to_string = self.int_to_string.write();
        let capacity = int_to_string.len();
        if i as usize == capacity {
            // Java: copyOf(intToString, capacity * 2).
            int_to_string.resize(capacity * 2, None);
        }
        int_to_string[i as usize] = Some(s.to_string());
        i
    }

    // Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:139 PersistentStringIndexer.getIndex
    fn get_index(&self, s: &str) -> Option<i32> {
        self.string_to_int.get(&s.to_string())
    }

    // Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:145 PersistentStringIndexer.getStringForIndex
    fn get_string_for_index(&self, i: i32) -> Option<String> {
        if i < 0 {
            return None;
        }
        let snapshot = self.int_to_string.read();
        if (i as usize) < snapshot.len() {
            snapshot[i as usize].clone()
        } else {
            None
        }
    }
}

// Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:185 PersistentStringIndexer.CODEC
struct StringIndexCodec;

impl MapCodec<String, i32> for StringIndexCodec {
    fn read_key(&self, in_: &mut DataInput) -> io::Result<String> {
        let length = in_.read_int()?;
        if length < 0 {
            return Err(io::Error::other(format!("corrupt key length: {}", length)));
        }
        let mut content = vec![0u8; length as usize];
        in_.read_fully(&mut content)?;
        // Java `new String(content, UTF_8)`.
        Ok(blaze_util::string_encoding::java_new_string_utf8(&content))
    }

    fn read_value(&self, in_: &mut DataInput) -> io::Result<i32> {
        in_.read_int()
    }

    fn write_key(&self, key: &String, out: &mut DataOutput<'_>) -> io::Result<()> {
        let content = key.as_bytes();
        out.write_int(content.len() as i32)?;
        out.write(content)
    }

    fn write_value(&self, value: &i32, out: &mut DataOutput<'_>) -> io::Result<()> {
        out.write_int(*value)
    }
}

// Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:221 PersistentStringIndexer.PersistentIndexMap.VERSION
const PERSISTENT_INDEX_MAP_VERSION: i32 = 0x01;
// Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:222 PersistentStringIndexer.PersistentIndexMap.SAVE_INTERVAL_NS
const SAVE_INTERVAL_NS: i64 = 3 * 1000 * 1000 * 1000;

/// The `shouldFlushJournal` policy of `PersistentIndexMap`: flush at most every 3 seconds.
// Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:220 PersistentStringIndexer.PersistentIndexMap
struct PersistentIndexMapHooks {
    clock: Arc<dyn Clock>,
    next_update: Mutex<i64>,
}

impl PersistentMapHooks for PersistentIndexMapHooks {
    // Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:235 PersistentStringIndexer.PersistentIndexMap.shouldFlushJournal
    fn should_flush_journal(&self) -> bool {
        let time = self.clock.nano_time();
        let mut next_update = self.next_update.lock();
        if SAVE_INTERVAL_NS == 0 || time > *next_update {
            *next_update = time + SAVE_INTERVAL_NS;
            return true;
        }
        false
    }
}

/// Persistent metadata map. Used as a backing map to provide a persistent implementation of the
/// metadata cache.
// Java: com/google/devtools/build/lib/actions/cache/PersistentStringIndexer.java:227 PersistentStringIndexer.PersistentIndexMap.<init>
fn new_persistent_index_map(
    map_file: Path,
    journal_file: Path,
    clock: Arc<dyn Clock>,
) -> io::Result<PersistentMap<String, i32>> {
    let next_update = clock.nano_time();
    let map = PersistentMap::new(
        PERSISTENT_INDEX_MAP_VERSION,
        Box::new(StringIndexCodec),
        Arc::new(PersistentIndexMapHooks {
            clock,
            next_update: Mutex::new(next_update),
        }),
        map_file,
        journal_file,
    );
    map.load()?;
    Ok(map)
}
