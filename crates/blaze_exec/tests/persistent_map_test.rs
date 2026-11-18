//! Port of `src/test/java/com/google/devtools/build/lib/util/PersistentMapTest.java`.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_exec::actions::cache::map_codec::{DataInput, DataOutput, MapCodec};
use blaze_exec::actions::cache::persistent_map::{
    PersistentMap, PersistentMapHooks, PersistentMapJournal,
};
use blaze_vfs::{DigestHashFunction, Path, UnixFileSystem};

struct StringCodec;

impl MapCodec<String, String> for StringCodec {
    fn read_key(&self, in_: &mut DataInput) -> io::Result<String> {
        in_.read_utf()
    }
    fn read_value(&self, in_: &mut DataInput) -> io::Result<String> {
        in_.read_utf()
    }
    fn write_key(&self, key: &String, out: &mut DataOutput<'_>) -> io::Result<()> {
        out.write_utf(key)
    }
    fn write_value(&self, value: &String, out: &mut DataOutput<'_>) -> io::Result<()> {
        out.write_utf(value)
    }
}

/// `PersistentStringMap`'s overridable flags.
struct Hooks {
    flush_journal: AtomicBool,
    keep_journal: AtomicBool,
}

impl PersistentMapHooks for Hooks {
    fn should_flush_journal(&self) -> bool {
        self.flush_journal.load(Ordering::SeqCst)
    }
    fn should_keep_journal(&self, _map: &dyn PersistentMapJournal) -> bool {
        self.keep_journal.load(Ordering::SeqCst)
    }
}

struct PersistentStringMap {
    map: PersistentMap<String, String>,
    hooks: Arc<Hooks>,
}

impl PersistentStringMap {
    fn new(map_file: &Path, journal_file: &Path) -> PersistentStringMap {
        let hooks = Arc::new(Hooks {
            flush_journal: AtomicBool::new(true),
            keep_journal: AtomicBool::new(false),
        });
        let map = PersistentMap::new(
            0x0,
            Box::new(StringCodec),
            hooks.clone(),
            map_file.clone(),
            journal_file.clone(),
        );
        map.load().unwrap();
        PersistentStringMap { map, hooks }
    }

    fn set_flush_journal(&self, v: bool) {
        self.hooks.flush_journal.store(v, Ordering::SeqCst);
    }

    fn set_keep_journal(&self, v: bool) {
        self.hooks.keep_journal.store(v, Ordering::SeqCst);
    }

    fn contains_entry(&self, k: &str, v: &str) -> bool {
        self.map.get(&k.to_string()).as_deref() == Some(v)
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    map_file: Path,
    journal_file: Path,
    map: PersistentStringMap,
}

impl Fixture {
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root = Path::create_from_str(tmp.path().to_str().unwrap(), &fs).get_child("tmp");
        root.create_directory_and_parents().unwrap();
        let map_file = root.get_child("map.txt");
        let journal_file = root.get_child("journal.txt");
        let map = PersistentStringMap::new(&map_file, &journal_file);
        Fixture {
            _tmp: tmp,
            map_file,
            journal_file,
            map,
        }
    }

    fn create_map(&mut self) {
        self.map = PersistentStringMap::new(&self.map_file, &self.journal_file);
    }
}

#[test]
fn map() {
    let mut f = Fixture::new();
    f.create_map();
    f.map.map.put("foo".into(), "bar".into());
    f.map.map.put("baz".into(), "bang".into());
    assert!(f.map.contains_entry("foo", "bar"));
    assert!(f.map.contains_entry("baz", "bang"));
    assert_eq!(f.map.map.size(), 2);
    let size = f.map.map.save().unwrap();
    assert_eq!(size, f.map_file.get_file_size().unwrap());
    assert!(f.map.contains_entry("foo", "bar"));
    assert!(f.map.contains_entry("baz", "bang"));
    assert_eq!(f.map.map.size(), 2);

    f.create_map(); // create a new map
    assert!(f.map.contains_entry("foo", "bar"));
    assert!(f.map.contains_entry("baz", "bang"));
    assert_eq!(f.map.map.size(), 2);
}

#[test]
fn put_if_absent() {
    let mut f = Fixture::new();
    f.create_map();
    assert_eq!(f.map.map.put_if_absent("foo".into(), "bar".into()), None);
    assert_eq!(
        f.map.map.put_if_absent("foo".into(), "ignored".into()),
        Some("bar".to_string())
    );
    assert_eq!(f.map.map.put_if_absent("baz".into(), "bang".into()), None);
    assert_eq!(
        f.map.map.put_if_absent("baz".into(), "ignored".into()),
        Some("bang".to_string())
    );
    assert!(f.map.contains_entry("foo", "bar"));
    assert!(f.map.contains_entry("baz", "bang"));
    assert_eq!(f.map.map.size(), 2);
    let size = f.map.map.save().unwrap();
    assert_eq!(size, f.map_file.get_file_size().unwrap());
    assert!(f.map.contains_entry("foo", "bar"));
    assert!(f.map.contains_entry("baz", "bang"));
    assert_eq!(f.map.map.size(), 2);

    f.create_map(); // create a new map
    assert!(f.map.contains_entry("foo", "bar"));
    assert!(f.map.contains_entry("baz", "bang"));
    assert_eq!(f.map.map.size(), 2);
}

#[test]
fn remove() {
    let mut f = Fixture::new();
    f.create_map();
    f.map.map.put("foo".into(), "bar".into());
    f.map.map.put("baz".into(), "bang".into());
    let size = f.map.map.save().unwrap();
    assert_eq!(size, f.map_file.get_file_size().unwrap());
    assert!(!f.journal_file.exists());
    f.map.map.remove(&"foo".to_string());
    assert_eq!(f.map.map.size(), 1);
    assert!(f.journal_file.exists());
    f.create_map(); // create a new map
    assert_eq!(f.map.map.size(), 1);
}

#[test]
fn clear() {
    let mut f = Fixture::new();
    f.create_map();
    f.map.map.put("foo".into(), "bar".into());
    f.map.map.put("baz".into(), "bang".into());
    f.map.map.save().unwrap();
    assert!(f.map_file.exists());
    assert!(!f.journal_file.exists());
    f.map.map.clear();
    assert!(f.map.map.is_empty());
    assert!(f.map_file.exists());
    assert!(!f.journal_file.exists());
    f.create_map(); // create a new map
    assert!(f.map.map.is_empty());
}

#[test]
fn no_flush_journal() {
    let mut f = Fixture::new();
    f.create_map();
    f.map.map.put("foo".into(), "bar".into());
    f.map.map.put("baz".into(), "bang".into());
    f.map.map.save().unwrap();
    assert!(!f.journal_file.exists());
    // prevent flushing the journal
    f.map.set_flush_journal(false);
    // remove an entry
    f.map.map.remove(&"foo".to_string());
    assert_eq!(f.map.map.size(), 1);
    // no journal file written
    assert!(!f.journal_file.exists());
    f.create_map(); // create a new map
    // both entries are still in the map on disk
    assert_eq!(f.map.map.size(), 2);
}

#[test]
fn keep_journal() {
    let mut f = Fixture::new();
    f.create_map();
    f.map.map.put("foo".into(), "bar".into());
    f.map.map.put("baz".into(), "bang".into());
    f.map.map.save().unwrap();
    assert!(!f.journal_file.exists());

    // Keep the journal through the save.
    f.map.set_flush_journal(false);
    f.map.set_keep_journal(true);

    // remove an entry
    f.map.map.remove(&"foo".to_string());
    assert_eq!(f.map.map.size(), 1);
    // no journal file written
    assert!(!f.journal_file.exists());

    let size = f.map.map.save().unwrap();
    assert_eq!(f.map.map.size(), 1);
    // The journal must be serialized on save(), even if !flushJournal.
    assert!(f.journal_file.exists());
    assert_eq!(
        size,
        f.journal_file.get_file_size().unwrap() + f.map_file.get_file_size().unwrap()
    );

    f.map.map.load().unwrap();
    assert_eq!(f.map.map.size(), 1);
    assert!(f.journal_file.exists());

    f.create_map(); // create a new map
    assert_eq!(f.map.map.size(), 1);

    f.map.set_keep_journal(false);
    f.map.map.save().unwrap();
    assert_eq!(f.map.map.size(), 1);
    assert!(!f.journal_file.exists());
}

#[test]
fn keep_journal_with_multiple_saves() {
    let mut f = Fixture::new();
    f.create_map();
    f.map.map.put("foo".into(), "bar".into());
    f.map.map.put("baz".into(), "bang".into());
    f.map.map.save().unwrap();
    f.map.set_flush_journal(false);
    f.map.set_keep_journal(true);
    f.map.map.remove(&"foo".to_string());
    assert_eq!(f.map.map.size(), 1);
    f.map.map.save().unwrap();
    f.map.map.remove(&"baz".to_string());
    f.map.map.save().unwrap();
    assert!(f.map.map.is_empty());
    // Ensure recreating the map loads the correct state.
    f.create_map();
    assert!(f.map.map.is_empty());
    assert!(!f.journal_file.exists());
}

#[test]
fn multiple_journal_updates() {
    let mut f = Fixture::new();
    f.create_map();
    f.map.map.put("foo".into(), "bar".into());
    f.map.map.save().unwrap();
    assert!(!f.journal_file.exists());
    // add an entry
    f.map.map.put("baz".into(), "bang".into());
    assert_eq!(f.map.map.size(), 2);
    // journal file written
    assert!(f.journal_file.exists());
    f.create_map(); // create a new map
    // both entries are still in the map on disk
    assert_eq!(f.map.map.size(), 2);
    // add another entry
    f.map.map.put("baz2".into(), "bang2".into());
    assert_eq!(f.map.map.size(), 3);
    // journal file written
    assert!(f.journal_file.exists());
    f.create_map(); // create a new map
    // all three entries are still in the map on disk
    assert_eq!(f.map.map.size(), 3);
}

#[test]
fn concurrent_operations() {
    let mut f = Fixture::new();
    f.create_map();
    f.map.set_keep_journal(true);
    let num_iters = 1000;
    std::thread::scope(|s| {
        let map = &f.map.map;
        let foo_putter = s.spawn(move || {
            for i in 0..num_iters {
                map.put("foo".into(), format!("bar{}", i));
                map.remove(&"baz".to_string());
            }
        });
        let baz_putter = s.spawn(move || {
            for i in 0..num_iters {
                map.put("baz".into(), format!("bar{}", i));
                map.remove(&"noexist".to_string());
            }
        });
        foo_putter.join().unwrap();
        baz_putter.join().unwrap();
    });
    f.map.map.save().unwrap();
    assert!(f.journal_file.exists());
    f.create_map();
    assert!(
        f.map
            .contains_entry("foo", &format!("bar{}", num_iters - 1))
    );
    if let Some(baz_value) = f.map.map.get(&"baz".to_string()) {
        assert_eq!(baz_value, format!("bar{}", num_iters - 1));
    }
}
