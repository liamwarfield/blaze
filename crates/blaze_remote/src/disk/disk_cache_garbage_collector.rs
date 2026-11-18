//! Port of `lib/remote/disk/DiskCacheGarbageCollector.java`.

use std::cmp::Ordering;
use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use blaze_repo_fetch::downloader::download_progress_event::bytes_count_to_display_string;
use blaze_repo_fetch::file_system_lock::{FileSystemLock, LockMode};
use blaze_vfs::{DirentType, Path, Symlinks};

/// `EXCLUDED_DIRS`.
// Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:51 DiskCacheGarbageCollector.EXCLUDED_DIRS
const EXCLUDED_DIRS: [&str; 2] = ["tmp", "gc"];

/// Describes a disk cache entry.
// Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:60 DiskCacheGarbageCollector.Entry
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// Path relative to the root directory of the disk cache.
    path: String,
    /// File size in bytes.
    size: i64,
    /// File modification time.
    mtime: i64,
}

/// Determines which entries should be collected.
// Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:68 DiskCacheGarbageCollector.CollectionPolicy
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CollectionPolicy {
    /// The maximum total size in bytes, or `None` for no size limit.
    max_size_bytes: Option<i64>,
    /// The maximum age of cache entries, or `None` for no age limit.
    max_age: Option<Duration>,
}

impl CollectionPolicy {
    /// `new CollectionPolicy(maxSizeBytes, maxAge)`.
    pub fn new(max_size_bytes: Option<i64>, max_age: Option<Duration>) -> CollectionPolicy {
        CollectionPolicy {
            max_size_bytes,
            max_age,
        }
    }

    /// `maxSizeBytes()`.
    pub fn max_size_bytes(&self) -> Option<i64> {
        self.max_size_bytes
    }

    /// `maxAge()`.
    pub fn max_age(&self) -> Option<Duration> {
        self.max_age
    }

    // Sort older entries before newer ones, tie breaking by path. This causes AC entries to be
    // sorted before CAS entries with the same age, making it less likely for garbage collection
    // to break referential integrity in the event that mtime resolution is insufficient.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:73 CollectionPolicy.COMPARATOR
    fn comparator(x: &Entry, y: &Entry) -> Ordering {
        // PORT: `String.compareTo` orders by UTF-16 code unit; the paths are ASCII (store names
        // and hex digests), for which the byte order is the same.
        x.mtime.cmp(&y.mtime).then_with(|| x.path.cmp(&y.path))
    }

    /// Returns the entries to be deleted: a prefix of `entries`, which this sorts.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:85 CollectionPolicy.getEntriesToDelete
    fn get_entries_to_delete<'a>(&self, entries: &'a mut [Entry]) -> &'a [Entry] {
        entries.sort_by(CollectionPolicy::comparator);

        let mut excess_size_bytes = self.get_excess_size_bytes(entries);
        let time_cutoff = self.get_time_cutoff();

        let mut i = 0;
        while i < entries.len() {
            if excess_size_bytes <= 0 && entries[i].mtime >= time_cutoff {
                break;
            }
            excess_size_bytes = excess_size_bytes.wrapping_sub(entries[i].size);
            i += 1;
        }

        &entries[0..i]
    }

    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:102 CollectionPolicy.getExcessSizeBytes
    fn get_excess_size_bytes(&self, entries: &[Entry]) -> i64 {
        let Some(max_size_bytes) = self.max_size_bytes else {
            return 0;
        };
        let current_size_bytes = sum_sizes(entries);
        current_size_bytes.wrapping_sub(max_size_bytes)
    }

    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:110 CollectionPolicy.getTimeCutoff
    fn get_time_cutoff(&self) -> i64 {
        let Some(max_age) = self.max_age else {
            return 0;
        };
        // `Instant.now().minus(maxAge).toEpochMilli()`.
        let now_nanos: i128 = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(since_epoch) => since_epoch.as_nanos() as i128,
            Err(before_epoch) => -(before_epoch.duration().as_nanos() as i128),
        };
        (now_nanos - max_age.as_nanos() as i128).div_euclid(1_000_000) as i64
    }
}

/// `entries.stream().mapToLong(Entry::size).sum()`.
fn sum_sizes(entries: &[Entry]) -> i64 {
    entries
        .iter()
        .fold(0i64, |sum, entry| sum.wrapping_add(entry.size))
}

// Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:118 DiskCacheGarbageCollector.DeletionStats
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeletionStats {
    deleted_entries: i64,
    deleted_bytes: i64,
    concurrent_update: bool,
}

/// Stats for a garbage collection run.
// Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:121 DiskCacheGarbageCollector.CollectionStats
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CollectionStats {
    /// `totalEntries()`.
    pub total_entries: i64,
    /// `totalBytes()`.
    pub total_bytes: i64,
    /// `deletedEntries()`.
    pub deleted_entries: i64,
    /// `deletedBytes()`.
    pub deleted_bytes: i64,
    /// `concurrentUpdate()`.
    pub concurrent_update: bool,
    /// `elapsedTime()`.
    pub elapsed_time: Duration,
}

impl CollectionStats {
    /// `new CollectionStats(totalEntries, totalBytes, deletedEntries, deletedBytes,
    /// concurrentUpdate, elapsedTime)`.
    pub fn new(
        total_entries: i64,
        total_bytes: i64,
        deleted_entries: i64,
        deleted_bytes: i64,
        concurrent_update: bool,
        elapsed_time: Duration,
    ) -> CollectionStats {
        CollectionStats {
            total_entries,
            total_bytes,
            deleted_entries,
            deleted_bytes,
            concurrent_update,
            elapsed_time,
        }
    }

    /// Returns a human-readable summary.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:130 CollectionStats.displayString
    pub fn display_string(&self) -> String {
        // `elapsedTime.toSecondsPart() + elapsedTime.toMillisPart() / 1000.0`
        let elapsed_seconds = (self.elapsed_time.as_secs() % 60) as f64
            + self.elapsed_time.subsec_millis() as f64 / 1000.0;
        let files_per_second = math_round(self.deleted_entries as f64 / elapsed_seconds) as i32;
        let mb_per_second =
            math_round((self.deleted_bytes as f64 / (1024.0 * 1024.0)) / elapsed_seconds) as i32;

        format!(
            "Deleted {} of {} files, reclaimed {} of {} in {} seconds ({} files/s, {} MB/s){}",
            self.deleted_entries,
            self.total_entries,
            bytes_count_to_display_string(self.deleted_bytes),
            bytes_count_to_display_string(self.total_bytes),
            format_two_decimals(elapsed_seconds),
            files_per_second,
            mb_per_second,
            if self.concurrent_update {
                " (concurrent update detected)"
            } else {
                ""
            }
        )
    }
}

/// `Math.round(double)`: the closest `long`, ties rounding to positive infinity; NaN is 0 and
/// out-of-range values saturate.
// Java: java/lang/Math.java Math.round
fn math_round(a: f64) -> i64 {
    if a.is_nan() {
        return 0;
    }
    let floor = a.floor();
    // `as` saturates at Long.MIN_VALUE / Long.MAX_VALUE, as Math.round does.
    if a - floor >= 0.5 {
        (floor + 1.0) as i64
    } else {
        floor as i64
    }
}

/// `String.format("%.2f", value)` for a finite, non-negative value: `java.util.Formatter` rounds
/// the shortest decimal representation of the double (`Double.toString`'s digits) half up.
// Java: java/util/Formatter.java Formatter.FormatSpecifier.print(double)
fn format_two_decimals(value: f64) -> String {
    // Rust's `{}` prints the shortest representation that round-trips, without an exponent.
    let shortest = format!("{value}");
    let (integer_part, fraction_part) = match shortest.split_once('.') {
        Some((integer_part, fraction_part)) => (integer_part, fraction_part),
        None => (shortest.as_str(), ""),
    };
    let mut digits: Vec<u8> = integer_part.bytes().map(|b| b - b'0').collect();
    let fraction: Vec<u8> = fraction_part.bytes().map(|b| b - b'0').collect();
    for i in 0..2 {
        digits.push(fraction.get(i).copied().unwrap_or(0));
    }
    if fraction.get(2).copied().unwrap_or(0) >= 5 {
        // Round half up.
        let mut i = digits.len();
        loop {
            if i == 0 {
                digits.insert(0, 1);
                break;
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    let integer_len = digits.len() - 2;
    let mut out = String::new();
    for (i, digit) in digits.iter().enumerate() {
        if i == integer_len {
            out.push('.');
        }
        out.push((b'0' + digit) as char);
    }
    out
}

/// A garbage collector for the disk cache.
///
/// Garbage collection works by enumerating the entire contents of the disk cache, identifying
/// candidates for deletion according to a [`CollectionPolicy`], and deleting them. This process
/// may take a significant amount of time on large disk caches and slow filesystems, and may be
/// interrupted at any time.
///
/// PORT: Java scans and deletes in parallel on an `ExecutorService` (two `AbstractQueueVisitor`s
/// in fail-fast mode). This port is sequential, on the calling thread: the entries found, the
/// entries deleted and the stats are the same, and the first I/O error ends the run as it does
/// in fail-fast mode. The constructor therefore has no executor parameter.
// Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:50 DiskCacheGarbageCollector
#[derive(Debug, Clone)]
pub struct DiskCacheGarbageCollector {
    root: Path,
    policy: CollectionPolicy,
    excluded_dirs: Vec<Path>,
}

impl DiskCacheGarbageCollector {
    /// Creates a new garbage collector.
    ///
    /// * `root`: the root directory of the disk cache
    /// * `policy`: the garbage collection policy to use
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:160 DiskCacheGarbageCollector.<init>
    pub fn new(root: Path, policy: CollectionPolicy) -> DiskCacheGarbageCollector {
        let excluded_dirs = EXCLUDED_DIRS
            .iter()
            .map(|dir| root.get_child(dir))
            .collect();
        DiskCacheGarbageCollector {
            root,
            policy,
            excluded_dirs,
        }
    }

    /// `getRoot()`.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:169 DiskCacheGarbageCollector.getRoot
    pub fn get_root(&self) -> &Path {
        &self.root
    }

    /// `getPolicy()`.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:174 DiskCacheGarbageCollector.getPolicy
    pub fn get_policy(&self) -> &CollectionPolicy {
        &self.policy
    }

    /// Runs garbage collection.
    ///
    /// Fails if an I/O error occurred.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:184 DiskCacheGarbageCollector.run
    pub fn run(&self) -> io::Result<CollectionStats> {
        // Acquire an exclusive lock to prevent two Bazel processes from simultaneously running
        // garbage collection, which can waste resources and lead to incorrect results.
        let lock =
            FileSystemLock::try_get(&self.root.get_relative_str("gc/lock"), LockMode::EXCLUSIVE)
                .map_err(|e| e.into_io_error())?;
        let result = self.run_under_lock();
        lock.close();
        result
    }

    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:192 DiskCacheGarbageCollector.runUnderLock
    fn run_under_lock(&self) -> io::Result<CollectionStats> {
        let start_time = Instant::now();
        let scanner = EntryScanner { collector: self };
        let mut deleter = EntryDeleter {
            collector: self,
            deleted_entries: 0,
            deleted_bytes: 0,
            concurrent_update: false,
        };

        let mut all_entries = scanner.scan()?;
        let entries_to_delete = self.policy.get_entries_to_delete(&mut all_entries);

        for entry in entries_to_delete {
            deleter.delete(entry)?;
        }

        let deletion_stats = deleter.await_();
        let elapsed_time = start_time.elapsed();

        Ok(CollectionStats::new(
            all_entries.len() as i64,
            sum_sizes(&all_entries),
            deletion_stats.deleted_entries,
            deletion_stats.deleted_bytes,
            deletion_stats.concurrent_update,
            elapsed_time,
        ))
    }
}

/// Lists all disk cache entries.
// Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:217 DiskCacheGarbageCollector.EntryScanner
struct EntryScanner<'a> {
    collector: &'a DiskCacheGarbageCollector,
}

impl EntryScanner<'_> {
    /// Lists all disk cache entries.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:229 EntryScanner.scan
    fn scan(&self) -> io::Result<Vec<Entry>> {
        let mut entries: Vec<Entry> = Vec::new();
        // The queue of `execute(() -> visitDirectory(path))` tasks.
        let mut queue: VecDeque<Path> = VecDeque::new();
        queue.push_back(self.collector.root.clone());
        while let Some(path) = queue.pop_front() {
            self.visit_directory(&path, &mut entries, &mut queue)?;
        }
        Ok(entries)
    }

    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:239 EntryScanner.visitDirectory
    fn visit_directory(
        &self,
        path: &Path,
        entries: &mut Vec<Entry>,
        queue: &mut VecDeque<Path>,
    ) -> io::Result<()> {
        for dirent in path.readdir(Symlinks::Nofollow)? {
            let child_path = path.get_child(dirent.get_name());
            if dirent.get_type() == DirentType::File {
                // The file may be gone by the time we stat it.
                let status = child_path.stat_if_found(Symlinks::Follow)?;
                if let Some(status) = status {
                    let entry = Entry {
                        path: child_path
                            .relative_to(&self.collector.root)
                            .get_path_string()
                            .to_string(),
                        size: status.get_size(),
                        mtime: status.get_last_modified_time(),
                    };
                    entries.push(entry);
                }
            } else if dirent.get_type() == DirentType::Directory
                && !self.collector.excluded_dirs.contains(&child_path)
            {
                queue.push_back(child_path);
            }
            // Deliberately ignore other file types, which should never occur in a well-formed
            // cache.
        }
        Ok(())
    }
}

/// Deletes disk cache entries.
// Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:269 DiskCacheGarbageCollector.EntryDeleter
struct EntryDeleter<'a> {
    collector: &'a DiskCacheGarbageCollector,
    deleted_entries: i64,
    deleted_bytes: i64,
    concurrent_update: bool,
}

impl EntryDeleter<'_> {
    /// Deletes an entry (Java: enqueues it to be deleted).
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:283 EntryDeleter.delete
    fn delete(&mut self, entry: &Entry) -> io::Result<()> {
        let path = self.collector.root.get_relative_str(&entry.path);
        let Some(status) = path.stat_if_found(Symlinks::Follow)? else {
            // The entry is already gone.
            self.concurrent_update = true;
            return Ok(());
        };
        if status.get_last_modified_time() != entry.mtime {
            // The entry was likely accessed by a build since we statted it.
            self.concurrent_update = true;
            return Ok(());
        }
        if path.delete()? {
            self.deleted_entries += 1;
            self.deleted_bytes = self.deleted_bytes.wrapping_add(entry.size);
        } else {
            // The entry is already gone.
            self.concurrent_update = true;
        }
        Ok(())
    }

    /// The stats of the deletions (Java: waits for all enqueued deletions to complete).
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheGarbageCollector.java:313 EntryDeleter.await
    fn await_(&self) -> DeletionStats {
        DeletionStats {
            deleted_entries: self.deleted_entries,
            deleted_bytes: self.deleted_bytes,
            concurrent_update: self.concurrent_update,
        }
    }
}
