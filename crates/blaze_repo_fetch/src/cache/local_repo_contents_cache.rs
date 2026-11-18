//! Port of `LocalRepoContentsCache`: the `contents/` area of the repository cache.
//!
//! Layout (shared with Bazel): `<repository_cache>/contents/<predeclared inputs hash>/` holds
//! pairs `<UUID>/` (the repo contents) and `<UUID>.recorded_inputs`; `gc_lock` is the GC lock
//! file and `_trash/` the trash directory.

use std::collections::HashSet;
use std::time::Duration;

use blaze_util::check_state;
use blaze_vfs::{DirentType, NOW_SENTINEL_TIME, Path, Symlinks};
use sha2::Digest as _;

use crate::errors::IoException;
use crate::file_system_lock::{FileSystemLock, LockMode};
use crate::fs_utils;

pub const RECORDED_INPUTS_SUFFIX: &str = ".recorded_inputs";

/// The path to a "lock" file, relative to the root of the repo contents cache. While a shared
/// lock is held, no garbage collection should happen. While an exclusive lock is held, no reads
/// should happen.
pub const LOCK_PATH: &str = "gc_lock";

/// The path to a trash directory relative to the root of the repo contents cache.
pub const TRASH_PATH: &str = "_trash";

/// A candidate repo in the repo contents cache for one predeclared input hash.
// Java: com/google/devtools/build/lib/bazel/repository/cache/LocalRepoContentsCache.java:77 LocalRepoContentsCache.CandidateRepo
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateRepo {
    pub recorded_inputs_file: Path,
    pub contents_dir: Path,
}

impl CandidateRepo {
    // Java: com/google/devtools/build/lib/bazel/repository/cache/LocalRepoContentsCache.java:78 CandidateRepo.fromRecordedInputsFile
    fn from_recorded_inputs_file(recorded_inputs_file: &Path) -> CandidateRepo {
        let base = recorded_inputs_file.get_base_name();
        let contents_dir_base_name = &base[..base.len() - RECORDED_INPUTS_SUFFIX.len()];
        CandidateRepo {
            recorded_inputs_file: recorded_inputs_file.clone(),
            contents_dir: recorded_inputs_file
                .replace_name(contents_dir_base_name)
                .expect("recorded inputs file has a parent"),
        }
    }

    /// Updates the mtime of the recorded inputs file, to delay GC for this entry.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/LocalRepoContentsCache.java:88 CandidateRepo.touch
    pub fn touch(&self) {
        // swallow the exception. it's not a huge deal.
        let _ = self
            .recorded_inputs_file
            .set_last_modified_time(NOW_SENTINEL_TIME);
    }
}

/// A cache directory that stores the contents of fetched repos across different workspaces.
// Java: com/google/devtools/build/lib/bazel/repository/cache/LocalRepoContentsCache.java:52 LocalRepoContentsCache
#[derive(Debug, Default)]
pub struct LocalRepoContentsCache {
    path: Option<Path>,
    shared_lock: Option<FileSystemLock>,
}

fn last_modified_time_or_zero(path: &Path) -> i64 {
    // If we can't read the mtime from the entry, it's broken and treated as outdated.
    path.get_last_modified_time().unwrap_or(0)
}

/// Sorts by descending mtime, stable (Java `sorted(comparingLong(..).reversed())`).
fn sort_by_mtime_desc(mut files: Vec<Path>) -> Vec<Path> {
    let mut keyed: Vec<(i64, Path)> = files
        .drain(..)
        .map(|p| (last_modified_time_or_zero(&p), p))
        .collect();
    keyed.sort_by_key(|k| std::cmp::Reverse(k.0));
    keyed.into_iter().map(|(_, p)| p).collect()
}

impl LocalRepoContentsCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_path(&mut self, path: Option<Path>) {
        self.path = path;
    }

    pub fn get_path(&self) -> Option<&Path> {
        self.path.as_ref()
    }

    pub fn is_enabled(&self) -> bool {
        self.path.is_some()
    }

    fn root(&self) -> &Path {
        check_state!(self.path.is_some());
        self.path.as_ref().unwrap()
    }

    /// Returns the list of candidate repos for the given predeclared input hash, most recently
    /// used first.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/LocalRepoContentsCache.java:99 LocalRepoContentsCache.getCandidateRepos
    pub fn get_candidate_repos(&self, predeclared_input_hash: &str) -> Vec<CandidateRepo> {
        let entry_dir = self.root().get_relative_str(predeclared_input_hash);
        match entry_dir.get_directory_entries() {
            Ok(entries) => {
                let files: Vec<Path> = entries
                    .into_iter()
                    .filter(|p| p.get_base_name().ends_with(RECORDED_INPUTS_SUFFIX))
                    .collect();
                sort_by_mtime_desc(files)
                    .iter()
                    .map(CandidateRepo::from_recorded_inputs_file)
                    .collect()
            }
            // This should only happen if `entryDir` doesn't exist yet or is not a directory.
            // Either way, don't outright fail; just treat it as if the cache is empty.
            Err(_) => Vec::new(),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/LocalRepoContentsCache.java:125 LocalRepoContentsCache.ensureTrashDir
    fn ensure_trash_dir(&self) -> Result<Path, IoException> {
        let trash_dir = self.root().get_child(TRASH_PATH);
        trash_dir.create_directory_and_parents()?;
        Ok(trash_dir)
    }

    /// Moves a freshly fetched repo into the contents cache and leaves a symlink behind.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/LocalRepoContentsCache.java:137 LocalRepoContentsCache.moveToCache
    pub fn move_to_cache(
        &self,
        fetched_repo_dir: &Path,
        fetched_repo_marker_file: &Path,
        predeclared_input_hash: &str,
    ) -> Result<CandidateRepo, IoException> {
        let entry_dir = self.root().get_relative_str(predeclared_input_hash);
        let unique_entry_name = uuid::Uuid::new_v4().to_string();
        let cache_recorded_inputs_file =
            entry_dir.get_child(&format!("{}{}", unique_entry_name, RECORDED_INPUTS_SUFFIX));
        let cache_repo_dir = entry_dir.get_child(&unique_entry_name);

        cache_repo_dir.delete_tree()?;
        if let Some(parent) = cache_repo_dir.get_parent_directory() {
            parent.create_directory_and_parents()?;
        }
        // Move the fetched marker file to a temp location, so that if following operations
        // fail, both the fetched repo and the cache locations are considered out-of-date.
        let temporary_marker = self
            .ensure_trash_dir()?
            .get_child(&uuid::Uuid::new_v4().to_string());
        fs_utils::move_file(fetched_repo_marker_file, &temporary_marker)?;
        // Now perform the move, and afterwards, restore the marker file.
        if fetched_repo_dir.rename_to(&cache_repo_dir).is_err() {
            cache_repo_dir.create_directory_and_parents()?;
            fs_utils::move_trees_below(fetched_repo_dir, &cache_repo_dir)?;
        }
        temporary_marker.rename_to(&cache_recorded_inputs_file)?;
        // Set up a symlink at the original fetched repo dir path.
        fetched_repo_dir.delete_tree()?;
        fs_utils::ensure_symbolic_link(fetched_repo_dir, cache_repo_dir.as_fragment())?;
        Ok(CandidateRepo {
            recorded_inputs_file: cache_recorded_inputs_file,
            contents_dir: cache_repo_dir,
        })
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/LocalRepoContentsCache.java:166 LocalRepoContentsCache.acquireSharedLock
    pub fn acquire_shared_lock(&mut self) -> Result<(), IoException> {
        let root = self.root().clone();
        check_state!(
            self.shared_lock.is_none(),
            "this process already has the shared lock"
        );
        self.shared_lock = Some(FileSystemLock::get(
            &root.get_relative_str(LOCK_PATH),
            LockMode::SHARED,
        )?);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/LocalRepoContentsCache.java:172 LocalRepoContentsCache.releaseSharedLock
    pub fn release_shared_lock(&mut self) -> Result<(), IoException> {
        check_state!(self.shared_lock.is_some());
        self.shared_lock.take().unwrap().close();
        Ok(())
    }

    /// The body of the GC idle task (`createGcIdleTask(maxAge, idleDelay).run()`): deletes
    /// cached repos last accessed more than `max_age` ago (never, if zero) and duplicates.
    ///
    /// PORT: the `IdleTask` wrapper (display name "Repo contents cache garbage collection",
    /// delay) belongs to the server's idle-task scheduler, which is not ported.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/LocalRepoContentsCache.java:202 LocalRepoContentsCache.createGcIdleTask
    pub fn run_gc_idle_task(&self, max_age: Duration) -> Result<(), IoException> {
        let root = self.root().clone();
        // If we can't grab the lock, abort GC. Someone will come along later.
        {
            let lock =
                FileSystemLock::try_get(&root.get_relative_str(LOCK_PATH), LockMode::EXCLUSIVE)?;
            self.run_gc(max_age)?;
            lock.close();
        }
        // Empty the trash dir outside the lock.
        root.get_child(TRASH_PATH).delete_trees_below()?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/LocalRepoContentsCache.java:236 LocalRepoContentsCache.runGc
    fn run_gc(&self, max_age: Duration) -> Result<(), IoException> {
        let path = self.root();
        path.set_last_modified_time(NOW_SENTINEL_TIME)?;
        let cutoff: i64 = if max_age.is_zero() {
            i64::MIN
        } else {
            path.get_last_modified_time()? - max_age.as_millis() as i64
        };
        let trash_dir = self.ensure_trash_dir()?;
        for dirent in path.readdir(Symlinks::Nofollow)? {
            if dirent.get_type() != DirentType::Directory || dirent.get_name() == TRASH_PATH {
                continue;
            }
            // Sort all recorded input files by descending mtime, so that deduplication keeps
            // around the most recent entry.
            let files: Vec<Path> = path
                .get_child(dirent.get_name())
                .get_directory_entries()?
                .into_iter()
                .filter(|p| p.get_base_name().ends_with(RECORDED_INPUTS_SUFFIX))
                .collect();
            let recorded_inputs_files = sort_by_mtime_desc(files);
            let mut seen: HashSet<Vec<u8>> = HashSet::new();
            for recorded_inputs_file in recorded_inputs_files {
                // In addition to deleting old entries, also remove identical entries.
                let old = recorded_inputs_file.get_last_modified_time()? < cutoff;
                let delete = old || {
                    let content = fs_utils::read_content(&recorded_inputs_file)?;
                    !seen.insert(sha2::Sha256::digest(&content).to_vec())
                };
                if delete {
                    recorded_inputs_file.delete()?;
                    let repo_dir = CandidateRepo::from_recorded_inputs_file(&recorded_inputs_file)
                        .contents_dir;
                    // Use a UUID to avoid clashes.
                    repo_dir.rename_to(&trash_dir.get_child(&uuid::Uuid::new_v4().to_string()))?;
                }
            }
        }
        Ok(())
    }
}
