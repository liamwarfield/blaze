//! Finding dirty file system values by accessing the file system directly:
//! `SkyValueDirtinessChecker`, `DirtinessCheckerUtils` and the key part of
//! `FilesystemValueChecker` (`getDirtyKeys`, `getNewAndOldValues`).
//!
//! PORT: Java's `FilesystemValueChecker` also checks the outputs of `ACTION_EXECUTION` values
//! (`getDirtyActionValues`); that part needs `ActionExecutionValue` and lives in blaze_sky_exec
//! (`output_checker`), above this crate.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use blaze_cmdline::RepositoryName;
use blaze_skyframe::{FunctionHermeticity, SkyKey, SkyValue, WalkableGraph};
use blaze_util::io::timestamp_granularity_monitor::TimestampGranularityMonitor;
use blaze_vfs::{Root, RootedPath, Symlinks};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use crate::actions::{ContentsComparisonResult, FileStateValue};
use crate::directory_listing::{DirectoryListingStateKey, DirectoryListingStateValue};
use crate::file_state::{ExternalFilesHelper, FileStateKey, FileType};
use crate::io::IOException;
use crate::sky_functions;

/// Given a [`SkyKey`] and the previous [`SkyValue`] it had, returns whether this value is up to
/// date.
///
/// PORT: the `SyscallCache` parameter is not ported (Blaze always asks the file system, Java's
/// `NO_CACHE`); `getMaxTransitiveSourceVersionForNewValue` and the old max transitive source
/// version of `check` are not ported (max transitive source versions are not ported,
/// docs/skyframe-incremental.md section 6).
// Java: com/google/devtools/build/lib/skyframe/SkyValueDirtinessChecker.java:31 SkyValueDirtinessChecker
pub trait SkyValueDirtinessChecker: Send + Sync {
    /// Returns `true` iff the checker can handle `key`. Can only be true if
    /// `key.functionName().getHermeticity() == FunctionHermeticity.NONHERMETIC`.
    // Java: com/google/devtools/build/lib/skyframe/SkyValueDirtinessChecker.java:36 SkyValueDirtinessChecker.applies
    fn applies(&self, key: &SkyKey) -> bool;

    /// If `applies(key)`, returns the new value for `key`.
    // Java: com/google/devtools/build/lib/skyframe/SkyValueDirtinessChecker.java:39 SkyValueDirtinessChecker.createNewValue
    fn create_new_value(
        &self,
        key: &SkyKey,
        tsgm: Option<&TimestampGranularityMonitor>,
    ) -> Result<SkyValue, IOException>;

    /// If `applies(key)`, returns the result of checking whether this key's value is up to date.
    // Java: com/google/devtools/build/lib/skyframe/SkyValueDirtinessChecker.java:56 SkyValueDirtinessChecker.check
    fn check(
        &self,
        key: &SkyKey,
        old_value: Option<&SkyValue>,
        tsgm: Option<&TimestampGranularityMonitor>,
    ) -> Result<DirtyResult, IOException> {
        let new_value = self.create_new_value(key, tsgm)?;
        Ok(if values_equal(key, Some(&new_value), old_value) {
            DirtyResult::not_dirty()
        } else {
            DirtyResult::dirty_with_new_value(new_value)
        })
    }
}

/// Java `newValue.equals(oldValue)` for the values of `key`'s type (the key's
/// [`blaze_skyframe::Key::values_equal`], which is the Java `equals` for the file system values
/// these checkers create).
fn values_equal(key: &SkyKey, new_value: Option<&SkyValue>, old_value: Option<&SkyValue>) -> bool {
    match (new_value, old_value) {
        (Some(new_value), Some(old_value)) => key.argument().dyn_values_equal(old_value, new_value),
        (None, None) => true,
        _ => false,
    }
}

/// An encapsulation of the result of checking to see if a value is up to date.
// Java: com/google/devtools/build/lib/skyframe/SkyValueDirtinessChecker.java:68 SkyValueDirtinessChecker.DirtyResult
#[derive(Clone)]
pub struct DirtyResult {
    is_dirty: bool,
    new_value: Option<SkyValue>,
}

impl DirtyResult {
    /// Creates a DirtyResult indicating that the external value is the same as the value in the
    /// graph.
    // Java: com/google/devtools/build/lib/skyframe/SkyValueDirtinessChecker.java:80 DirtyResult.notDirty
    pub fn not_dirty() -> DirtyResult {
        DirtyResult {
            is_dirty: false,
            new_value: None,
        }
    }

    /// Creates a DirtyResult indicating that external value is different from the value in the
    /// graph, but this new value is not known.
    // Java: com/google/devtools/build/lib/skyframe/SkyValueDirtinessChecker.java:88 DirtyResult.dirty
    pub fn dirty() -> DirtyResult {
        DirtyResult {
            is_dirty: true,
            new_value: None,
        }
    }

    /// Creates a DirtyResult indicating that the external value is `new_value`, which is
    /// different from the value in the graph,
    // Java: com/google/devtools/build/lib/skyframe/SkyValueDirtinessChecker.java:96 DirtyResult.dirtyWithNewValue
    pub fn dirty_with_new_value(new_value: SkyValue) -> DirtyResult {
        DirtyResult {
            is_dirty: true,
            new_value: Some(new_value),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyValueDirtinessChecker.java:121 DirtyResult.isDirty
    pub fn is_dirty(&self) -> bool {
        self.is_dirty
    }

    /// If `is_dirty()`, then either returns the new value for the value or `None` if the new
    /// value wasn't computed. In the case where the value is dirty and a new value is available,
    /// then the new value can be injected into the skyframe graph. Otherwise, the value should
    /// simply be invalidated.
    // Java: com/google/devtools/build/lib/skyframe/SkyValueDirtinessChecker.java:132 DirtyResult.getNewValue
    pub fn get_new_value(&self) -> Option<&SkyValue> {
        blaze_util::check_state!(self.is_dirty);
        self.new_value.as_ref()
    }
}

/// Checks dirtiness of file keys in the graph.
// Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:44 DirtinessCheckerUtils.FileDirtinessChecker
pub struct FileDirtinessChecker;

impl SkyValueDirtinessChecker for FileDirtinessChecker {
    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:46 FileDirtinessChecker.applies
    fn applies(&self, sky_key: &SkyKey) -> bool {
        sky_key.function_name() == sky_functions::FILE_STATE
    }

    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:52 FileDirtinessChecker.createNewValue
    fn create_new_value(
        &self,
        key: &SkyKey,
        tsgm: Option<&TimestampGranularityMonitor>,
    ) -> Result<SkyValue, IOException> {
        let rooted_path = &key.downcast::<FileStateKey>().expect("FileStateKey").0;
        Ok(Arc::new(FileStateValue::create_with_tsgm(rooted_path, tsgm)?))
    }
}

/// Checks dirtiness of directory keys in the graph.
// Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:61 DirtinessCheckerUtils.DirectoryDirtinessChecker
pub struct DirectoryDirtinessChecker;

impl SkyValueDirtinessChecker for DirectoryDirtinessChecker {
    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:63 DirectoryDirtinessChecker.applies
    fn applies(&self, sky_key: &SkyKey) -> bool {
        sky_key.function_name() == sky_functions::DIRECTORY_LISTING_STATE
    }

    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:68 DirectoryDirtinessChecker.createNewValue
    fn create_new_value(
        &self,
        key: &SkyKey,
        _tsgm: Option<&TimestampGranularityMonitor>,
    ) -> Result<SkyValue, IOException> {
        let rooted_path = &key
            .downcast::<DirectoryListingStateKey>()
            .expect("DirectoryListingStateKey")
            .0;
        // syscallCache.readdir(rootedPath.asPath()): NOFOLLOW, as DirectoryListingStateFunction.
        let dirents = rooted_path
            .as_path()
            .readdir(Symlinks::Nofollow)
            .map_err(|e| IOException::from_io(&e))?;
        Ok(Arc::new(DirectoryListingStateValue::create(dirents)))
    }
}

/// The file and directory checkers, restricted to keys under the package roots without diff
/// information.
// Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:77 DirtinessCheckerUtils.MissingDiffDirtinessChecker
pub struct MissingDiffDirtinessChecker {
    missing_diff_package_roots: HashSet<Root>,
    checker: UnionDirtinessChecker,
}

impl MissingDiffDirtinessChecker {
    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:81 MissingDiffDirtinessChecker.<init>
    pub fn new(missing_diff_package_roots: HashSet<Root>) -> Self {
        MissingDiffDirtinessChecker {
            missing_diff_package_roots,
            checker: create_basic_filesystem_dirtiness_checker(),
        }
    }
}

/// The rooted path a FILE_STATE or DIRECTORY_LISTING_STATE key stands for (Java
/// `(RootedPath) key.argument()`).
fn rooted_path_of(key: &SkyKey) -> &RootedPath {
    if let Some(k) = key.downcast::<FileStateKey>() {
        &k.0
    } else if let Some(k) = key.downcast::<DirectoryListingStateKey>() {
        &k.0
    } else {
        panic!("ClassCastException: {:?} is not a RootedPath", key)
    }
}

impl SkyValueDirtinessChecker for MissingDiffDirtinessChecker {
    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:86 MissingDiffDirtinessChecker.applies
    fn applies(&self, key: &SkyKey) -> bool {
        self.checker.applies(key)
            && self
                .missing_diff_package_roots
                .contains(rooted_path_of(key).get_root())
    }

    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:92 MissingDiffDirtinessChecker.createNewValue
    fn create_new_value(
        &self,
        key: &SkyKey,
        tsgm: Option<&TimestampGranularityMonitor>,
    ) -> Result<SkyValue, IOException> {
        self.checker.create_new_value(key, tsgm)
    }
}

/// Serves for tracking whether there are external and output files (see
/// `ExternalFilesKnowledge`). Filtering of files, for which the new values should not be injected
/// into evaluator, is done in `SequencedSkyframeExecutor.handleChangedFiles()`.
// Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:105 DirtinessCheckerUtils.ExternalDirtinessChecker
pub struct ExternalDirtinessChecker {
    external_files_helper: Arc<ExternalFilesHelper>,
    file_types_to_check: Vec<FileType>,
    checker: UnionDirtinessChecker,
    dirty_external_repos: Mutex<IndexMap<RepositoryName, RootedPath>>,
}

impl ExternalDirtinessChecker {
    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:113 ExternalDirtinessChecker.<init>
    pub fn new(
        external_files_helper: Arc<ExternalFilesHelper>,
        file_types_to_check: Vec<FileType>,
    ) -> Self {
        ExternalDirtinessChecker {
            external_files_helper,
            file_types_to_check,
            checker: create_basic_filesystem_dirtiness_checker(),
            dirty_external_repos: Mutex::new(IndexMap::new()),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:178 ExternalDirtinessChecker.getDirtyExternalRepos
    pub fn get_dirty_external_repos(&self) -> IndexMap<RepositoryName, RootedPath> {
        self.dirty_external_repos.lock().clone()
    }

    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:182 ExternalDirtinessChecker.isCacheableType
    fn is_cacheable_type(file_type: FileType) -> bool {
        match file_type {
            FileType::Internal | FileType::ExternalOther | FileType::Bundled => true,
            FileType::ExternalRepo | FileType::Output => false,
            FileType::RepoContentsCacheDirs => panic!(
                "IllegalStateException: Repo contents cache dirs are not expected to be checked \
                 for dirtiness"
            ),
        }
    }
}

impl SkyValueDirtinessChecker for ExternalDirtinessChecker {
    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:120 ExternalDirtinessChecker.applies
    fn applies(&self, key: &SkyKey) -> bool {
        if !self.checker.applies(key) {
            return false;
        }
        let file_type = self
            .external_files_helper
            .get_and_note_file_type(rooted_path_of(key));
        self.file_types_to_check.contains(&file_type)
    }

    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:129 ExternalDirtinessChecker.createNewValue
    fn create_new_value(
        &self,
        _key: &SkyKey,
        _tsgm: Option<&TimestampGranularityMonitor>,
    ) -> Result<SkyValue, IOException> {
        panic!("UnsupportedOperationException")
    }

    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:135 ExternalDirtinessChecker.check
    fn check(
        &self,
        sky_key: &SkyKey,
        old_value: Option<&SkyValue>,
        tsgm: Option<&TimestampGranularityMonitor>,
    ) -> Result<DirtyResult, IOException> {
        let rooted_path = rooted_path_of(sky_key);
        let file_type = self.external_files_helper.get_and_note_file_type(rooted_path);
        let cacheable = Self::is_cacheable_type(file_type);
        // PORT: `cacheable ? syscallCache : SyscallCache.NO_CACHE`: Blaze has no syscall cache.
        let new_value = self.checker.create_new_value(sky_key, tsgm)?;
        if values_equal(sky_key, Some(&new_value), old_value) {
            return Ok(DirtyResult::not_dirty());
        }
        if cacheable {
            return Ok(DirtyResult::dirty_with_new_value(new_value));
        }
        if file_type == FileType::ExternalRepo
            && let (Some(old_file_state), Some(new_file_state)) = (
                old_value.and_then(|v| v.downcast_ref::<FileStateValue>()),
                new_value.downcast_ref::<FileStateValue>(),
            )
        {
            return Ok(match new_file_state.compare_contents(old_file_state) {
                ContentsComparisonResult::Same => DirtyResult::not_dirty(),
                ContentsComparisonResult::Different => {
                    if let Some(repository_name) =
                        self.external_files_helper.get_external_repo_name(rooted_path)
                    {
                        self.dirty_external_repos
                            .lock()
                            .entry(repository_name)
                            .or_insert_with(|| rooted_path.clone());
                    }
                    DirtyResult::dirty()
                }
                // The hermetic Linux sandbox uses hardlinks to stage inputs, which affects ctimes.
                // Re-evaluate the node to pick up the new ctime, but don't report the file as
                // modified and trigger a refetch of the repo just due to that.
                ContentsComparisonResult::SameExceptChangeTime => DirtyResult::dirty(),
            });
        }
        // Files under output_base/external have a dependency on the WORKSPACE file, so we don't
        // add a new SkyValue to the graph yet because it might change once the WORKSPACE file has
        // been parsed. Similarly, output files might change during execution.
        Ok(DirtyResult::dirty())
    }
}

// Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:194 DirtinessCheckerUtils.createBasicFilesystemDirtinessChecker
pub fn create_basic_filesystem_dirtiness_checker() -> UnionDirtinessChecker {
    UnionDirtinessChecker::new(vec![
        Arc::new(FileDirtinessChecker),
        Arc::new(DirectoryDirtinessChecker),
    ])
}

/// [`SkyValueDirtinessChecker`] that encompasses a union of other dirtiness checkers.
// Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:200 DirtinessCheckerUtils.UnionDirtinessChecker
pub struct UnionDirtinessChecker {
    dirtiness_checkers: Vec<Arc<dyn SkyValueDirtinessChecker>>,
}

impl UnionDirtinessChecker {
    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:203 UnionDirtinessChecker.<init>
    pub fn new(dirtiness_checkers: Vec<Arc<dyn SkyValueDirtinessChecker>>) -> Self {
        UnionDirtinessChecker { dirtiness_checkers }
    }

    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:207 UnionDirtinessChecker.getChecker
    fn get_checker(&self, key: &SkyKey) -> Option<&Arc<dyn SkyValueDirtinessChecker>> {
        self.dirtiness_checkers
            .iter()
            .find(|dirtiness_checker| dirtiness_checker.applies(key))
    }
}

impl SkyValueDirtinessChecker for UnionDirtinessChecker {
    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:217 UnionDirtinessChecker.applies
    fn applies(&self, key: &SkyKey) -> bool {
        self.get_checker(key).is_some()
    }

    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:222 UnionDirtinessChecker.createNewValue
    fn create_new_value(
        &self,
        key: &SkyKey,
        tsgm: Option<&TimestampGranularityMonitor>,
    ) -> Result<SkyValue, IOException> {
        self.get_checker(key)
            .unwrap_or_else(|| panic!("NullPointerException: {:?}", key))
            .create_new_value(key, tsgm)
    }

    // Java: com/google/devtools/build/lib/skyframe/DirtinessCheckerUtils.java:230 UnionDirtinessChecker.check
    fn check(
        &self,
        key: &SkyKey,
        old_value: Option<&SkyValue>,
        tsgm: Option<&TimestampGranularityMonitor>,
    ) -> Result<DirtyResult, IOException> {
        self.get_checker(key)
            .unwrap_or_else(|| panic!("NullPointerException: {:?}", key)).check(key, old_value, tsgm)
    }
}

/// Where [`FilesystemValueChecker`] reads the old values from.
// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:160 FilesystemValueChecker.ValueFetcher
enum ValueFetcher<'a> {
    // Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:179 FilesystemValueChecker.MapBackedValueFetcher
    Map(&'a std::collections::HashMap<SkyKey, SkyValue>),
    // Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:165 FilesystemValueChecker.WalkableGraphBackedValueFetcher
    Walkable(&'a dyn WalkableGraph),
}

impl ValueFetcher<'_> {
    fn get(&self, key: &SkyKey) -> Option<SkyValue> {
        match self {
            ValueFetcher::Map(map) => map.get(key).cloned(),
            ValueFetcher::Walkable(graph) => graph.get_value(key),
        }
    }
}

/// A helper class to find dirty values by accessing the filesystem directly (contrast with
/// `DiffAwareness`).
///
/// PORT: the `SyscallCache` and the `XattrProviderOverrider` are not ported (Blaze asks the file
/// system directly; Bazel's only override is the remote output service's, which Blaze rejects).
/// Java's fixed thread pool of `numThreads` is a scoped pool of at most that many threads; the
/// check is not interruptible (Java's `ExecutorUtil.interruptibleShutdown`).
// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:86 FilesystemValueChecker
pub struct FilesystemValueChecker {
    tsgm: Option<Arc<TimestampGranularityMonitor>>,
    num_threads: usize,
}

impl FilesystemValueChecker {
    // Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:107 FilesystemValueChecker.<init>
    pub fn new(tsgm: Option<Arc<TimestampGranularityMonitor>>, num_threads: usize) -> Self {
        FilesystemValueChecker {
            tsgm,
            num_threads: num_threads.max(1),
        }
    }

    /// Returns a `Differencer.DiffWithDelta` containing keys from the given map that are dirty
    /// according to the passed-in `dirtiness_checker`.
    // Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:123 FilesystemValueChecker.getDirtyKeys
    pub fn get_dirty_keys(
        &self,
        values_map: &std::collections::HashMap<SkyKey, SkyValue>,
        dirtiness_checker: &dyn SkyValueDirtinessChecker,
    ) -> ImmutableBatchDirtyResult {
        let keys: Vec<SkyKey> = values_map.keys().cloned().collect();
        self.get_dirty_values(
            &ValueFetcher::Map(values_map),
            &keys,
            dirtiness_checker,
            /* check_missing_values= */ false,
        )
    }

    /// Returns a `Differencer.DiffWithDelta` containing keys that are dirty according to the
    /// passed-in `dirtiness_checker`.
    // Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:149 FilesystemValueChecker.getNewAndOldValues
    pub fn get_new_and_old_values(
        &self,
        walkable_graph: &dyn WalkableGraph,
        keys: &[SkyKey],
        dirtiness_checker: &dyn SkyValueDirtinessChecker,
    ) -> ImmutableBatchDirtyResult {
        self.get_dirty_values(
            &ValueFetcher::Walkable(walkable_graph),
            keys,
            dirtiness_checker,
            /* check_missing_values= */ true,
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:633 FilesystemValueChecker.getDirtyValues
    fn get_dirty_values(
        &self,
        fetcher: &ValueFetcher<'_>,
        keys: &[SkyKey],
        checker: &dyn SkyValueDirtinessChecker,
        check_missing_values: bool,
    ) -> ImmutableBatchDirtyResult {
        let num_keys_checked = AtomicUsize::new(0);
        let batch_result = MutableBatchDirtyResult::default();
        let mut applicable: Vec<&SkyKey> = Vec::new();
        for key in keys {
            if !checker.applies(key) {
                continue;
            }
            blaze_util::check_state!(
                key.hermeticity() == FunctionHermeticity::Nonhermetic,
                "Only non-hermetic keys can be dirty roots: {:?}",
                key
            );
            applicable.push(key);
        }
        let next = AtomicUsize::new(0);
        let threads = self.num_threads.min(applicable.len().max(1));
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::SeqCst);
                        let Some(key) = applicable.get(i) else {
                            break;
                        };
                        let value = fetcher.get(key);
                        if !check_missing_values && value.is_none() {
                            continue;
                        }
                        num_keys_checked.fetch_add(1, Ordering::SeqCst);
                        // Treat IOException as dirty with an unknown value. If this key is
                        // requested during an evaluation, we'll attempt to evaluate it - the error
                        // may turn out to be permanent or transient.
                        let result = checker
                            .check(key, value.as_ref(), self.tsgm.as_deref())
                            .unwrap_or_else(|_| DirtyResult::dirty());
                        if result.is_dirty() {
                            batch_result.add(
                                (*key).clone(),
                                value,
                                result.get_new_value().cloned(),
                            );
                        }
                    }
                });
            }
        });
        batch_result.to_immutable(num_keys_checked.load(Ordering::SeqCst), keys)
    }
}

/// An immutable `Differencer.DiffWithDelta`.
///
/// PORT: Java's `Delta` (old value, new value, max transitive source version) is the new value
/// only: the old value is not read by the differencer and max transitive source versions are not
/// ported.
// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:705 FilesystemValueChecker.ImmutableBatchDirtyResult
pub struct ImmutableBatchDirtyResult {
    dirty_keys_without_new_values: Vec<SkyKey>,
    dirty_keys_with_new_and_old_values: IndexMap<SkyKey, SkyValue>,
    num_keys_checked: usize,
}

impl ImmutableBatchDirtyResult {
    // Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:721 ImmutableBatchDirtyResult.changedKeysWithoutNewValues
    pub fn changed_keys_without_new_values(&self) -> &[SkyKey] {
        &self.dirty_keys_without_new_values
    }

    // Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:726 ImmutableBatchDirtyResult.changedKeysWithNewValues
    pub fn changed_keys_with_new_values(&self) -> &IndexMap<SkyKey, SkyValue> {
        &self.dirty_keys_with_new_and_old_values
    }

    // Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:730 ImmutableBatchDirtyResult.getNumKeysChecked
    pub fn get_num_keys_checked(&self) -> usize {
        self.num_keys_checked
    }

    /// Java `Diff.isEmpty`.
    pub fn is_empty(&self) -> bool {
        self.dirty_keys_without_new_values.is_empty()
            && self.dirty_keys_with_new_and_old_values.is_empty()
    }
}

/// Result of a batch call to [`SkyValueDirtinessChecker::check`]. Partitions the dirty values
/// based on whether we have a new value available for them or not.
// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:739 FilesystemValueChecker.MutableBatchDirtyResult
#[derive(Default)]
struct MutableBatchDirtyResult {
    concurrent_dirty_keys_without_new_values: Mutex<IndexSet<SkyKey>>,
    concurrent_dirty_keys_with_new_and_old_values: Mutex<IndexMap<SkyKey, SkyValue>>,
}

impl MutableBatchDirtyResult {
    // Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:750 MutableBatchDirtyResult.add
    fn add(&self, key: SkyKey, _old_value: Option<SkyValue>, new_value: Option<SkyValue>) {
        match new_value {
            None => {
                self.concurrent_dirty_keys_without_new_values
                    .lock()
                    .insert(key);
            }
            Some(new_value) => {
                self.concurrent_dirty_keys_with_new_and_old_values
                    .lock()
                    .insert(key, new_value);
            }
        }
    }

    /// PORT: Java's concurrent collections have no order; the result is in the order of the
    /// scanned keys so that the invalidation it leads to (and the progress receiver's calls) is
    /// the same from run to run.
    // Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:769 MutableBatchDirtyResult.toImmutable
    fn to_immutable(self, num_checked: usize, order: &[SkyKey]) -> ImmutableBatchDirtyResult {
        let without = self.concurrent_dirty_keys_without_new_values.into_inner();
        let mut with = self.concurrent_dirty_keys_with_new_and_old_values.into_inner();
        let mut dirty_keys_without_new_values = Vec::with_capacity(without.len());
        let mut dirty_keys_with_new_and_old_values = IndexMap::with_capacity(with.len());
        for key in order {
            if without.contains(key) {
                dirty_keys_without_new_values.push(key.clone());
            }
            if let Some(v) = with.swap_remove(key) {
                dirty_keys_with_new_and_old_values.insert(key.clone(), v);
            }
        }
        ImmutableBatchDirtyResult {
            dirty_keys_without_new_values,
            dirty_keys_with_new_and_old_values,
            num_keys_checked: num_checked,
        }
    }
}
