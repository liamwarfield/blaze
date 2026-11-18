//! Port of `lib/util/PersistentMap.java`.
//!
//! PORT: belongs to `lib/util`; it lives here (its only users are the action cache and its string
//! indexer) until that package is ported.

use std::hash::Hash;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_vfs::Path;
use indexmap::{IndexMap, IndexSet};
use parking_lot::{Mutex, ReentrantMutex, RwLock};

use super::map_codec::{
    MapCodec, MapCodecReader, MapCodecWriter, create_reader, create_writer,
    is_incompatible_format_exception,
};

/// The journal operations a [`PersistentMapHooks`] implementation may call back into
/// (`flushJournal`, `journalSize`, `cacheSize` of the Java subclass's `this`).
pub trait PersistentMapJournal {
    /// `flushJournal()`.
    fn flush_journal(&self);
    /// `journalSize()`.
    fn journal_size(&self) -> io::Result<i64>;
    /// `cacheSize()`.
    fn cache_size(&self) -> io::Result<i64>;
}

/// The overridable policy methods of `PersistentMap` (Java subclasses override them).
pub trait PersistentMapHooks: Send + Sync {
    /// Determines whether the in-memory journal should be flushed to disk.
    ///
    /// Called whenever an update is appended to the in-memory journal. The default is to flush it
    /// immediately, but subclasses are free to override this to implement their own strategy.
    // Java: com/google/devtools/build/lib/util/PersistentMap.java:157 PersistentMap.shouldFlushJournal
    fn should_flush_journal(&self) -> bool {
        true
    }

    /// Whether to keep the journal file on save.
    ///
    /// The default is to always merge the journal file into the main file and delete the former,
    /// but subclasses are free to override this to implement their own strategy.
    // Java: com/google/devtools/build/lib/util/PersistentMap.java:308 PersistentMap.shouldKeepJournal
    fn should_keep_journal(&self, _map: &dyn PersistentMapJournal) -> bool {
        false
    }
}

/// The default `PersistentMap` policy (flush on every update, never keep the journal).
#[derive(Debug, Default)]
pub struct DefaultPersistentMapHooks;

impl PersistentMapHooks for DefaultPersistentMapHooks {}

/// A map that is backed by persistent storage. It uses two files on disk for this: The first file
/// contains all the entries and gets written when invoking the [`PersistentMap::save`] method.
/// The second file contains a journal of all entries that were added to or removed from the map
/// since constructing the instance of the map or the last invocation of `save()` and gets written
/// after each update of the map although sub-classes are free to implement their own journal
/// update strategy.
///
/// Ceci n'est pas un Map. Strictly speaking, the Map interface doesn't permit the possibility of
/// failure. This class uses persistence; persistence means I/O, and I/O means the possibility of
/// failure. Therefore the semantics of this may deviate from the Map contract in failure cases.
/// In particular, updates are not guaranteed to succeed. However, I/O failures are guaranteed to
/// be reported upon the subsequent call to a method that returns an I/O error such as `save`.
///
/// To populate the map entries using the previously persisted entries call
/// [`PersistentMap::load`] prior to invoking any other map operation.
///
/// The constructor allows passing in a version number that gets written to the files on disk and
/// checked before reading from disk. Files with an incompatible version number will be ignored.
/// This allows the client code to change the persistence format without polluting the file system
/// name space.
// Java: com/google/devtools/build/lib/util/PersistentMap.java:63 PersistentMap
pub struct PersistentMap<K, V> {
    version: i32,
    map_file: Path,
    journal_file: Path,
    /// Java `LinkedBlockingQueue<K>`.
    journal: Mutex<Vec<K>>,
    /// If set, contains the message from an I/O error from a previously failed write. This error
    /// is deferred until the next call to a method which is able to return an error.
    deferred_io_failure: Mutex<Option<String>>,
    /// 'loaded' is true when the in-memory representation is at least as recent as the on-disk
    /// representation.
    loaded: AtomicBool,
    // PORT: java-hash-order. Java's delegate is a ConcurrentHashMap, whose iteration order only
    // decides the order of records in a full save (and of dump/trim iteration); the on-disk record
    // order is never read back as meaningful. Insertion order is used instead.
    delegate: RwLock<IndexMap<K, V>>,
    codec: Box<dyn MapCodec<K, V>>,
    hooks: Arc<dyn PersistentMapHooks>,
    /// The Java object monitor (`synchronized` methods), which is reentrant.
    monitor: ReentrantMutex<()>,
}

impl<K, V> PersistentMap<K, V>
where
    K: Clone + Eq + Hash + Send + Sync,
    V: Clone + Send + Sync,
{
    /// Creates a new PersistentMap instance.
    ///
    /// `version`: the version tag. Changing the version tag allows updating the on disk format.
    /// The map will never read from a file that was written using a different version tag.
    /// `codec`: the codec used to convert between the in-memory and on-disk representations.
    /// `map_file`: the file to save the map entries to. `journal_file`: the journal file to write
    /// entries between invocations of `save()`.
    // Java: com/google/devtools/build/lib/util/PersistentMap.java:103 PersistentMap.<init>
    pub fn new(
        version: i32,
        codec: Box<dyn MapCodec<K, V>>,
        hooks: Arc<dyn PersistentMapHooks>,
        map_file: Path,
        journal_file: Path,
    ) -> PersistentMap<K, V> {
        PersistentMap {
            version,
            map_file,
            journal_file,
            journal: Mutex::new(Vec::new()),
            deferred_io_failure: Mutex::new(None),
            loaded: AtomicBool::new(false),
            delegate: RwLock::new(IndexMap::new()),
            codec,
            hooks,
            monitor: ReentrantMutex::new(()),
        }
    }

    /// The policy hooks.
    pub fn hooks(&self) -> &Arc<dyn PersistentMapHooks> {
        &self.hooks
    }

    /// `get(key)`.
    pub fn get(&self, key: &K) -> Option<V> {
        self.delegate.read().get(key).cloned()
    }

    /// `containsKey(key)`.
    pub fn contains_key(&self, key: &K) -> bool {
        self.delegate.read().contains_key(key)
    }

    /// `size()`.
    pub fn size(&self) -> usize {
        self.delegate.read().len()
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.delegate.read().is_empty()
    }

    /// A snapshot of `entrySet()`.
    pub fn entries(&self) -> Vec<(K, V)> {
        self.delegate
            .read()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// A snapshot of `keySet()`.
    pub fn keys(&self) -> Vec<K> {
        self.delegate.read().keys().cloned().collect()
    }

    // Java: com/google/devtools/build/lib/util/PersistentMap.java:121 PersistentMap.put
    pub fn put(&self, key: K, value: V) -> Option<V> {
        let previous = self.delegate.write().insert(key.clone(), value);
        self.journal.lock().push(key);
        self.maybe_flush_journal();
        previous
    }

    // Java: com/google/devtools/build/lib/util/PersistentMap.java:131 PersistentMap.putIfAbsent
    pub fn put_if_absent(&self, key: K, value: V) -> Option<V> {
        let previous = {
            let mut delegate = self.delegate.write();
            match delegate.get(&key) {
                Some(existing) => Some(existing.clone()),
                None => {
                    delegate.insert(key.clone(), value);
                    None
                }
            }
        };
        if previous.is_none() {
            self.journal.lock().push(key);
            self.maybe_flush_journal();
        }
        previous
    }

    /// Potentially flushes the in-memory journal to disk, as determined by
    /// `shouldFlushJournal()`.
    // Java: com/google/devtools/build/lib/util/PersistentMap.java:145 PersistentMap.maybeFlushJournal
    fn maybe_flush_journal(&self) {
        if self.hooks.should_flush_journal() {
            self.flush_journal_impl();
        }
    }

    // Java: com/google/devtools/build/lib/util/PersistentMap.java:165 PersistentMap.remove
    pub fn remove(&self, key: &K) -> Option<V> {
        let previous = self.delegate.write().swap_remove(key);
        if previous.is_some() {
            // we know that 'object' must be an instance of K, because the
            // remove call succeeded, i.e. 'object' was mapped to 'previous'.
            self.journal.lock().push(key.clone());
            self.maybe_flush_journal();
        }
        previous
    }

    /// Flushes the in-memory journal to disk.
    // Java: com/google/devtools/build/lib/util/PersistentMap.java:187 PersistentMap.flushJournal
    pub fn flush_journal(&self) {
        self.flush_journal_impl();
    }

    fn flush_journal_impl(&self) {
        let _guard = self.monitor.lock();
        // Append to a preexisting journal file, which may have been left around after the last
        // save() because shouldKeepJournal() was true.
        let result = (|| -> io::Result<()> {
            let mut journal_out = create_writer(
                &*self.codec,
                &self.journal_file,
                self.version,
                /* overwrite= */ false,
            )?;
            // Journal may have duplicates, we can ignore them.
            let keys: IndexSet<K> = std::mem::take(&mut *self.journal.lock())
                .into_iter()
                .collect();
            let written = self.write_entries(&mut journal_out, Some(&keys));
            let closed = journal_out.close();
            written.and(closed)
        })();
        if let Err(e) = result {
            *self.deferred_io_failure.lock() = Some(format!("{} during journal append", e));
        }
    }

    /// Loads the previous written map entries from disk.
    ///
    /// If no on-disk state exists, loading is successful and produces an empty map.
    ///
    /// Data corruption is handled differently for each file:
    ///
    /// - Corruption in the map file is treated as an error, as the file is updated atomically.
    /// - Corruption in the journal file is tolerated by ignoring the remaining contents, as the
    ///   file is updated non-atomically.
    ///
    /// Fails with an `IncompatibleFormatException` if the on-disk data is in an incompatible
    /// format, or another I/O error if data corruption is detected and cannot be recovered from,
    /// or some other I/O error occurs.
    // Java: com/google/devtools/build/lib/util/PersistentMap.java:216 PersistentMap.load
    pub fn load(&self) -> io::Result<()> {
        let _guard = self.monitor.lock();
        if !self.loaded.load(Ordering::SeqCst) {
            if self.map_file.exists() {
                self.load_entries(&self.map_file)?;
            }
            if self.journal_file.exists() {
                if let Err(e) = self.load_entries(&self.journal_file)
                    && is_incompatible_format_exception(&e)
                {
                    return Err(e);
                }
                // Merge the journal into the map file and delete the former, ensuring that we
                // don't keep appending to a corrupted journal.
                // TODO(tjgq): Avoid doing this unless journal corruption was detected.
                self.save_with(/* full_save= */ true)?;
            }
            self.loaded.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/util/PersistentMap.java:239 PersistentMap.clear
    pub fn clear(&self) {
        let _guard = self.monitor.lock();
        self.delegate.write().clear();
        // We must do a full save because we're bypassing the journal.
        if let Err(e) = self.save_with(/* full_save= */ true) {
            *self.deferred_io_failure.lock() = Some(format!("{} during map clear", e));
        }
    }

    /// Saves the map to disk. Fails if there was an I/O error during this call, or any previous
    /// call since the last save().
    // Java: com/google/devtools/build/lib/util/PersistentMap.java:255 PersistentMap.save
    pub fn save(&self) -> io::Result<i64> {
        self.save_with(false)
    }

    /// Saves the map to disk.
    ///
    /// `full_save`: if true, the journal file will be merged into the map file and deleted;
    /// otherwise, the decision is made by `shouldKeepJournal()`.
    // Java: com/google/devtools/build/lib/util/PersistentMap.java:267 PersistentMap.save
    fn save_with(&self, full_save: bool) -> io::Result<i64> {
        let _guard = self.monitor.lock();
        /* Report a previously failing I/O operation. */
        if let Some(failure) = self.deferred_io_failure.lock().take() {
            return Err(io::Error::other(failure));
        }
        if !full_save && self.hooks.should_keep_journal(self) {
            self.flush_journal_impl();
        } else {
            let map_temp = self
                .map_file
                .get_parent_directory()
                .expect("map file has a parent directory")
                .get_child(&format!(
                    "{}.tmp",
                    remove_extension(self.map_file.get_base_name())
                ));
            let result = (|| -> io::Result<()> {
                self.save_entries(&map_temp)?;
                self.map_file.delete()?;
                map_temp.rename_to(&self.map_file)
            })();
            // finally
            let deleted = map_temp.delete();
            deleted?;
            result?;
            self.clear_journal();
            self.journal_file.delete()?;
        }
        Ok(self.journal_size_impl()? + self.cache_size_impl()?)
    }

    // Java: com/google/devtools/build/lib/util/PersistentMap.java:294 PersistentMap.journalSize
    fn journal_size_impl(&self) -> io::Result<i64> {
        let _guard = self.monitor.lock();
        if self.journal_file.exists() {
            self.journal_file.get_file_size()
        } else {
            Ok(0)
        }
    }

    // Java: com/google/devtools/build/lib/util/PersistentMap.java:298 PersistentMap.cacheSize
    fn cache_size_impl(&self) -> io::Result<i64> {
        let _guard = self.monitor.lock();
        if self.map_file.exists() {
            self.map_file.get_file_size()
        } else {
            Ok(0)
        }
    }

    // Java: com/google/devtools/build/lib/util/PersistentMap.java:312 PersistentMap.clearJournal
    fn clear_journal(&self) {
        let _guard = self.monitor.lock();
        self.journal.lock().clear();
    }

    /// Loads all entries from the given file into the backing map.
    // Java: com/google/devtools/build/lib/util/PersistentMap.java:322 PersistentMap.loadEntries
    fn load_entries(&self, map_file: &Path) -> io::Result<()> {
        let _guard = self.monitor.lock();
        let mut in_ = create_reader(&*self.codec, map_file, self.version as i64)?;
        self.read_entries(&mut in_)
    }

    /// Saves all backing map entries to the given file, overwriting preexisting contents.
    // Java: com/google/devtools/build/lib/util/PersistentMap.java:329 PersistentMap.saveEntries
    fn save_entries(&self, map_file: &Path) -> io::Result<()> {
        let _guard = self.monitor.lock();
        let mut out = create_writer(
            &*self.codec,
            map_file,
            self.version,
            /* overwrite= */ true,
        )?;
        let written = self.write_entries(&mut out, None);
        let closed = out.close();
        written.and(closed)
    }

    /// Writes backing map entries for a set of keys (`None`: all keys) into a writer.
    // Java: com/google/devtools/build/lib/util/PersistentMap.java:342 PersistentMap.writeEntries
    fn write_entries(
        &self,
        out: &mut MapCodecWriter<'_, K, V>,
        keys: Option<&IndexSet<K>>,
    ) -> io::Result<()> {
        let map = self.delegate.read();
        match keys {
            Some(keys) => {
                for key in keys {
                    out.write_entry(key, map.get(key))?;
                }
            }
            None => {
                for (key, value) in map.iter() {
                    out.write_entry(key, Some(value))?;
                }
            }
        }
        Ok(())
    }

    /// Reads entries from a reader into the backing map.
    // Java: com/google/devtools/build/lib/util/PersistentMap.java:354 PersistentMap.readEntries
    fn read_entries(&self, in_: &mut MapCodecReader<'_, K, V>) -> io::Result<()> {
        while let Some(entry) = in_.read_entry()? {
            let mut map = self.delegate.write();
            match entry.value {
                Some(value) => {
                    map.insert(entry.key, value);
                }
                None => {
                    map.swap_remove(&entry.key);
                }
            }
        }
        Ok(())
    }
}

impl<K, V> PersistentMapJournal for PersistentMap<K, V>
where
    K: Clone + Eq + Hash + Send + Sync,
    V: Clone + Send + Sync,
{
    fn flush_journal(&self) {
        self.flush_journal_impl();
    }

    fn journal_size(&self) -> io::Result<i64> {
        self.journal_size_impl()
    }

    fn cache_size(&self) -> io::Result<i64> {
        self.cache_size_impl()
    }
}

/// `FileSystemUtils.removeExtension(String)`.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:128 FileSystemUtils.removeExtension
fn remove_extension(filename: &str) -> &str {
    let Some(last_dot_index) = filename.rfind('.') else {
        return filename;
    };
    if let Some(last_slash_index) = filename.rfind('/')
        && last_slash_index > last_dot_index
    {
        return filename;
    }
    &filename[..last_dot_index]
}
