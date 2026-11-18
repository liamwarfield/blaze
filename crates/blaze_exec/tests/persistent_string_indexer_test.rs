//! Port of `src/test/java/com/google/devtools/build/lib/actions/cache/PersistentStringIndexerTest.java`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Barrier, Mutex};

use blaze_events::Clock;
use blaze_exec::actions::cache::persistent_string_indexer::{
    PersistentStringIndexer, StringIndexer,
};
use blaze_vfs::{DigestHashFunction, Path, UnixFileSystem};

struct ManualClock {
    current_time: AtomicI64,
}

impl ManualClock {
    fn advance(&self, time: i64) {
        self.current_time.fetch_add(time, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn current_time_millis(&self) -> i64 {
        panic!("unexpected method call");
    }
    fn nano_time(&self) -> i64 {
        self.current_time.load(Ordering::SeqCst)
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    mappings: Mutex<HashMap<i32, String>>,
    clock: Arc<ManualClock>,
    data_path: Path,
    journal_path: Path,
    indexer: PersistentStringIndexer,
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

impl Fixture {
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let cache_root =
            Path::create_from_str(tmp.path().to_str().unwrap(), &fs).get_child("cache");
        cache_root.create_directory_and_parents().unwrap();
        let data_path = cache_root.get_child("test.dat");
        let journal_path = cache_root.get_child("test.journal");
        let clock = Arc::new(ManualClock {
            current_time: AtomicI64::new(0),
        });
        let indexer =
            PersistentStringIndexer::create(data_path.clone(), journal_path.clone(), clock.clone())
                .unwrap();
        Fixture {
            _tmp: tmp,
            mappings: Mutex::new(HashMap::new()),
            clock,
            data_path,
            journal_path,
            indexer,
        }
    }

    fn create(&self) -> std::io::Result<PersistentStringIndexer> {
        PersistentStringIndexer::create(
            self.data_path.clone(),
            self.journal_path.clone(),
            self.clock.clone(),
        )
    }

    fn assert_size(&self, expected: i32) {
        assert_eq!(self.indexer.size(), expected);
    }

    fn assert_index(&self, expected: i32, s: &str) {
        let index = self.indexer.get_or_create_index(s);
        assert_eq!(index, expected);
        self.mappings
            .lock()
            .unwrap()
            .insert(expected, s.to_string());
    }

    fn assert_content(&self) {
        let mappings = self.mappings.lock().unwrap();
        for i in 0..self.indexer.size() {
            if let Some(s) = mappings.get(&i) {
                assert_eq!(Some(s.clone()), self.indexer.get_string_for_index(i));
            }
        }
    }

    fn setup_test_content(&self) {
        self.assert_size(0);
        self.assert_index(0, "abcdefghi"); // Create leafs
        self.assert_index(1, "abcdefjkl");
        self.assert_index(2, "abcdefmno");
        self.assert_index(3, "abcdefjklpr");
        self.assert_index(3, "abcdefjklpr");
        self.assert_index(4, "abcdstr");
        self.assert_index(5, "012345");
        self.assert_size(6);
        self.assert_index(6, "abcdef"); // Validate inner nodes
        self.assert_index(7, "abcd");
        self.assert_index(8, "");
        self.assert_size(9);
        self.assert_content();
    }

    /// Writes lots of entries with labels "fooconcurrent[int]" at the same time. The set of
    /// labels written is deterministic, but the label:index mapping is not.
    fn write_lots_of_entries_concurrently(&self, num_to_write: i32) {
        let num_threads = 10;
        let synchronizer = Barrier::new(num_threads);
        std::thread::scope(|s| {
            for _ in 0..num_threads {
                s.spawn(|| {
                    // Java: a CountDownLatch that only holds the threads back on the first
                    // iteration.
                    synchronizer.wait();
                    for i in 0..num_to_write {
                        let value = format!("fooconcurrent{}", i);
                        let index = self.indexer.get_or_create_index(&value);
                        self.mappings.lock().unwrap().insert(index, value);
                    }
                });
            }
        });
    }
}

#[test]
fn returns_same_integer_instance() {
    let f = Fixture::new();
    let n = 1000; // Greater than the default java.lang.Integer.IntegerCache.high of 127.
    for i in 0..n {
        let s = "a".repeat(i);
        let index = f.indexer.get_or_create_index(&s);
        // PORT: Java checks instance identity of the boxed Integer; values are compared here.
        assert_eq!(f.indexer.get_index(&s), Some(index));
    }
}

#[test]
fn unindexed_string_returns_null() {
    let f = Fixture::new();
    assert_eq!(f.indexer.get_index("absent"), None);
}

#[test]
fn test_normal_operation() {
    let mut f = Fixture::new();
    assert!(!f.data_path.exists());
    assert!(!f.journal_path.exists());
    f.setup_test_content();
    assert!(!f.data_path.exists());
    assert!(!f.journal_path.exists());

    f.clock.advance(4);
    f.assert_index(9, "xyzqwerty"); // This should flush journal to disk.
    assert!(!f.data_path.exists());
    assert!(f.journal_path.exists());

    f.indexer.save().unwrap(); // Successful save will remove journal file.
    assert!(f.data_path.exists());
    assert!(!f.journal_path.exists());

    // Now restore data from file and verify it.
    f.indexer = f.create().unwrap();
    assert!(!f.journal_path.exists());
    f.clock.advance(4);
    f.assert_size(10);
    f.assert_content();
    assert!(!f.journal_path.exists());
}

#[test]
fn test_journal_recovery_without_main_data_file() {
    let mut f = Fixture::new();
    assert!(!f.data_path.exists());
    assert!(!f.journal_path.exists());
    f.setup_test_content();
    assert!(!f.data_path.exists());
    assert!(!f.journal_path.exists());

    f.clock.advance(4);
    f.assert_index(9, "abc1234"); // This should flush journal to disk.
    assert!(!f.data_path.exists());
    assert!(f.journal_path.exists());

    // Now restore data from file and verify it. All data should be restored from journal;
    f.indexer = f.create().unwrap();
    assert!(f.data_path.exists());
    assert!(!f.journal_path.exists());
    f.clock.advance(4);
    f.assert_size(10);
    f.assert_content();
    assert!(!f.journal_path.exists());
}

#[test]
fn test_journal_recovery() {
    let mut f = Fixture::new();
    assert!(!f.data_path.exists());
    assert!(!f.journal_path.exists());
    f.setup_test_content();
    f.indexer.save().unwrap();
    assert!(f.data_path.exists());
    assert!(!f.journal_path.exists());
    let old_data_file_len = f.data_path.get_file_size().unwrap();

    f.clock.advance(4);
    f.assert_index(9, "another record"); // This should flush journal to disk.
    f.assert_size(10);
    assert!(f.data_path.exists());
    assert!(f.journal_path.exists());

    // Now restore data from file and verify it. All data should be restored from journal;
    f.indexer = f.create().unwrap();
    assert!(f.data_path.exists());
    assert!(!f.journal_path.exists());
    assert!(f.data_path.get_file_size().unwrap() > old_data_file_len); // data file should have been updated
    f.clock.advance(4);
    f.assert_size(10);
    f.assert_content();
    assert!(!f.journal_path.exists());
}

#[test]
fn test_concurrent_writes_journal_recovery() {
    let mut f = Fixture::new();
    assert!(!f.data_path.exists());
    assert!(!f.journal_path.exists());
    f.setup_test_content();
    f.indexer.save().unwrap();
    assert!(f.data_path.exists());
    assert!(!f.journal_path.exists());
    let old_data_file_len = f.data_path.get_file_size().unwrap();

    let size = f.indexer.size();
    let num_to_write = 50000;
    f.write_lots_of_entries_concurrently(num_to_write);
    assert!(!f.journal_path.exists());
    f.clock.advance(4);
    f.assert_index(size + num_to_write, "another record"); // This should flush journal to disk.
    f.assert_size(size + num_to_write + 1);
    assert!(f.data_path.exists());
    assert!(f.journal_path.exists());

    // Now restore data from file and verify it. All data should be restored from journal;
    f.indexer = f.create().unwrap();
    assert!(f.data_path.exists());
    assert!(!f.journal_path.exists());
    assert!(f.data_path.get_file_size().unwrap() > old_data_file_len); // data file should have been updated
    f.clock.advance(4);
    f.assert_size(size + num_to_write + 1);
    f.assert_content();
    assert!(!f.journal_path.exists());
}

#[test]
fn test_corrupted_journal() {
    let mut f = Fixture::new();
    f.journal_path
        .get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    write_content(&f.journal_path, b"bogus content");
    let e = f.create().err().expect("create must fail");
    assert!(
        e.to_string().contains("too short: 13 bytes"),
        "unexpected message: {}",
        e
    );

    f.journal_path.delete().unwrap();
    f.setup_test_content();
    assert!(!f.data_path.exists());
    assert!(!f.journal_path.exists());

    f.clock.advance(4);
    f.assert_index(9, "abc1234"); // This should flush journal to disk.
    assert!(!f.data_path.exists());
    assert!(f.journal_path.exists());

    let journal_content = read_content(&f.journal_path);

    // Restore data from file and verify it.
    f.indexer = f.create().unwrap();
    assert_eq!(f.indexer.size(), 10);
    assert!(f.data_path.exists());
    assert!(!f.journal_path.exists());

    // Replace journal with a truncated copy. We should tolerate it and drop the incomplete record.
    assert!(f.data_path.delete().unwrap());
    write_content(
        &f.journal_path,
        &journal_content[..journal_content.len() - 1],
    );
    f.indexer = f.create().unwrap();
    assert_eq!(f.indexer.size(), 9);

    // Replace journal with a corrupted copy. We should tolerate it and drop remaining records.
    let mut journal_copy = journal_content.clone();
    journal_copy[95] = -2i8 as u8; // make the key size negative
    write_content(&f.journal_path, &journal_copy);
    f.indexer = f.create().unwrap();
    assert_eq!(f.indexer.size(), 9);
}

#[test]
fn test_dupe_index_corruption() {
    let f = Fixture::new();
    f.setup_test_content();
    assert!(!f.data_path.exists());
    assert!(!f.journal_path.exists());

    f.assert_index(9, "abc1234"); // This should flush journal to disk.
    f.indexer.save().unwrap();
    assert!(f.data_path.exists());
    assert!(!f.journal_path.exists());

    let mut content = read_content(&f.data_path);

    // We remove the data file, and instead create a corrupt journal.
    //
    // The journal has a header followed by a sequence of (String, int) pairs, where each int is
    // a unique value. The String is encoded by the length (as an int), and the int is simply
    // encoded as an int. Note that the DataOutputStream class uses big endian by default, so the
    // low-order bits are at the end.
    //
    // For the purpose of this test, we want to make the journal contain two entries with the
    // same index (which is illegal). The PersistentStringIndexer assigns int values in the usual
    // order, starting with zero, and it now contains 9 entries. We simply change the last entry
    // to an index that is guaranteed to already exist. If it is the index 1, we change it to 2,
    // otherwise we change it to 1 - in both cases, the code currently guarantees that the
    // duplicate comes earlier in the stream.
    assert!(f.data_path.delete().unwrap());
    let last = content.len() - 1;
    content[last] = if content[last] == 1 { 2 } else { 1 };
    write_content(&f.journal_path, &content);

    let e = f.create().err().expect("create must fail");
    assert!(
        e.to_string()
            .contains("Corrupted filename index has duplicate entry"),
        "unexpected message: {}",
        e
    );
}

#[test]
fn test_deferred_io_failure() {
    let f = Fixture::new();
    assert!(!f.data_path.exists());
    assert!(!f.journal_path.exists());
    f.setup_test_content();
    assert!(!f.data_path.exists());
    assert!(!f.journal_path.exists());

    // Ensure that journal cannot be saved.
    f.journal_path.create_directory_and_parents().unwrap();

    f.clock.advance(4);
    f.assert_index(9, "abc1234"); // This should flush journal to disk (and fail at that).
    assert!(!f.data_path.exists());

    // Subsequent updates should succeed even though journaling is disabled at this point.
    f.clock.advance(4);
    f.assert_index(10, "another record");
    let e = f.indexer.save().expect_err("save must fail");
    let expected = format!("{} (Is a directory)", f.journal_path.get_path_string());
    assert!(
        e.to_string().contains(&expected),
        "unexpected message: {}",
        e
    );
}
