//! Port of `lib/analysis/Runfiles.java`.
//!
//! PORT: the Starlark `runfiles` value (`RunfilesApi`: the `files`, `symlinks`, `root_symlinks`,
//! `empty_filenames` depsets, `merge`, `merge_all` and `debugPrint`) is left to the analysis
//! lead, who wraps [`Runfiles`] in a Starlark value. The plain methods it needs are here:
//! [`Runfiles::get_artifacts`], [`Runfiles::get_symlinks`], [`Runfiles::get_root_symlinks`],
//! [`Runfiles::get_empty_filenames_for_starlark`], [`Runfiles::merge`] and
//! [`Runfiles::merge_all`] (which take `--nested_set_depth_limit` instead of the thread).

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock, OnceLock};

use blaze_cmdline::EvalException;
use blaze_cmdline::label_constants::EXTERNAL_RUNFILES_PATH_PREFIX;
use blaze_collect::command_line_item::{ExceptionlessMapFn, MapFn, MapFnError};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_util::{Fingerprint, check_argument, check_state, unported};
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};
use parking_lot::RwLock;

use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::artifact::Artifact;
use crate::analysis::runfiles_support::RunfilesSupport;
use crate::analysis::symlink_entry::SymlinkEntry;

/// Interface used for adding empty files to the runfiles at the last minute. Mainly to support
/// python-related rules adding `__init__.py` files.
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:174 Runfiles.EmptyFilesSupplier
pub trait EmptyFilesSupplier: Send + Sync {
    /// Calculate additional empty files to add based on the existing manifest paths.
    ///
    /// `manifest_paths` iterates in the order of the Java set the caller passes.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:176 EmptyFilesSupplier.getExtraPaths
    fn get_extra_paths(&self, manifest_paths: &IndexSet<PathFragment>) -> Vec<PathFragment>;

    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:178 EmptyFilesSupplier.fingerprint
    fn fingerprint(&self, fingerprint: &mut Fingerprint);

    /// Java `getClass().getName()`, printed by `describeFingerprint`.
    fn get_class_name(&self) -> String;

    /// Java `equals(other)` for suppliers that override it. Identity is checked by the caller
    /// first (Java's default `Object.equals`), so the default returns `false`.
    fn equals(&self, _other: &dyn EmptyFilesSupplier) -> bool {
        false
    }
}

/// Shared handle to an [`EmptyFilesSupplier`]; Java compares suppliers by identity
/// (`Arc::ptr_eq`).
pub type EmptyFilesSupplierRef = Arc<dyn EmptyFilesSupplier>;

// Java: com/google/devtools/build/lib/analysis/Runfiles.java:74 Runfiles.DummyEmptyFilesSupplier
struct DummyEmptyFilesSupplier;

// Java: com/google/devtools/build/lib/analysis/Runfiles.java:75 DummyEmptyFilesSupplier.GUID
// UUID.fromString("36437db7-820b-4386-85b4-f7205a2018ae")
const DUMMY_EMPTY_FILES_SUPPLIER_GUID_MOST_SIGNIFICANT: i64 = 0x36437db7820b4386_u64 as i64;
const DUMMY_EMPTY_FILES_SUPPLIER_GUID_LEAST_SIGNIFICANT: i64 = 0x85b4f7205a2018ae_u64 as i64;

impl EmptyFilesSupplier for DummyEmptyFilesSupplier {
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:80 DummyEmptyFilesSupplier.getExtraPaths
    fn get_extra_paths(&self, _manifest_paths: &IndexSet<PathFragment>) -> Vec<PathFragment> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:85 DummyEmptyFilesSupplier.fingerprint
    fn fingerprint(&self, fp: &mut Fingerprint) {
        fp.add_uuid(
            DUMMY_EMPTY_FILES_SUPPLIER_GUID_LEAST_SIGNIFICANT,
            DUMMY_EMPTY_FILES_SUPPLIER_GUID_MOST_SIGNIFICANT,
        );
    }

    fn get_class_name(&self) -> String {
        "com.google.devtools.build.lib.analysis.Runfiles$DummyEmptyFilesSupplier".to_string()
    }
}

// Java: com/google/devtools/build/lib/analysis/Runfiles.java:91 Runfiles.DUMMY_EMPTY_FILES_SUPPLIER
pub static DUMMY_EMPTY_FILES_SUPPLIER: LazyLock<EmptyFilesSupplierRef> =
    LazyLock::new(|| Arc::new(DummyEmptyFilesSupplier));

fn is_dummy_empty_files_supplier(supplier: &EmptyFilesSupplierRef) -> bool {
    Arc::ptr_eq(supplier, &DUMMY_EMPTY_FILES_SUPPLIER)
}

/// `Runfiles.EMPTY`.
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:94 Runfiles.EMPTY
pub static EMPTY: LazyLock<Runfiles> = LazyLock::new(|| RunfilesBuilder::new_empty().build());

// Java: com/google/devtools/build/lib/analysis/Runfiles.java:96 Runfiles.REPO_MAPPING_PATH_FRAGMENT
static REPO_MAPPING_PATH_FRAGMENT: LazyLock<PathFragment> =
    LazyLock::new(|| PathFragment::create("_repo_mapping"));

// ---------------------------------------------------------------------------------------------
// Map fns (one distinct type per Java singleton)
// ---------------------------------------------------------------------------------------------

// Java: com/google/devtools/build/lib/analysis/Runfiles.java:100 Runfiles.SYMLINK_ENTRY_ABSOLUTE_PATH_MAP_FN
pub struct SymlinkEntryAbsolutePathMapFn;
pub static SYMLINK_ENTRY_ABSOLUTE_PATH_MAP_FN: SymlinkEntryAbsolutePathMapFn =
    SymlinkEntryAbsolutePathMapFn;

impl MapFn<SymlinkEntry> for SymlinkEntryAbsolutePathMapFn {
    fn expand_to_command_line(
        &self,
        symlink: &SymlinkEntry,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        args(symlink.get_path_string());
        args(symlink.get_artifact().get_path().get_path_string());
        Ok(())
    }
}

impl ExceptionlessMapFn<SymlinkEntry> for SymlinkEntryAbsolutePathMapFn {}

// Java: com/google/devtools/build/lib/analysis/Runfiles.java:107 Runfiles.SYMLINK_ENTRY_EXEC_PATH_MAP_FN
pub struct SymlinkEntryExecPathMapFn;
pub static SYMLINK_ENTRY_EXEC_PATH_MAP_FN: SymlinkEntryExecPathMapFn = SymlinkEntryExecPathMapFn;

impl MapFn<SymlinkEntry> for SymlinkEntryExecPathMapFn {
    fn expand_to_command_line(
        &self,
        symlink: &SymlinkEntry,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        args(symlink.get_path_string());
        args(symlink.get_artifact().get_exec_path_string());
        Ok(())
    }
}

impl ExceptionlessMapFn<SymlinkEntry> for SymlinkEntryExecPathMapFn {}

// Java: com/google/devtools/build/lib/analysis/Runfiles.java:114 Runfiles.RUNFILES_AND_ABSOLUTE_PATH_MAP_FN
pub struct RunfilesAndAbsolutePathMapFn;
pub static RUNFILES_AND_ABSOLUTE_PATH_MAP_FN: RunfilesAndAbsolutePathMapFn =
    RunfilesAndAbsolutePathMapFn;

impl MapFn<Artifact> for RunfilesAndAbsolutePathMapFn {
    fn expand_to_command_line(
        &self,
        artifact: &Artifact,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        args(&artifact.get_runfiles_path_string());
        args(artifact.get_path().get_path_string());
        Ok(())
    }
}

impl ExceptionlessMapFn<Artifact> for RunfilesAndAbsolutePathMapFn {}

// Java: com/google/devtools/build/lib/analysis/Runfiles.java:120 Runfiles.RUNFILES_AND_EXEC_PATH_MAP_FN
pub struct RunfilesAndExecPathMapFn;
pub static RUNFILES_AND_EXEC_PATH_MAP_FN: RunfilesAndExecPathMapFn = RunfilesAndExecPathMapFn;

impl MapFn<Artifact> for RunfilesAndExecPathMapFn {
    fn expand_to_command_line(
        &self,
        artifact: &Artifact,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        args(&artifact.get_runfiles_path_string());
        args(artifact.get_exec_path_string());
        Ok(())
    }
}

impl ExceptionlessMapFn<Artifact> for RunfilesAndExecPathMapFn {}

fn symlink_entry_map_fn(
    digest_absolute_paths: bool,
) -> &'static dyn ExceptionlessMapFn<SymlinkEntry> {
    if digest_absolute_paths {
        &SYMLINK_ENTRY_ABSOLUTE_PATH_MAP_FN as &'static dyn ExceptionlessMapFn<SymlinkEntry>
    } else {
        &SYMLINK_ENTRY_EXEC_PATH_MAP_FN as &'static dyn ExceptionlessMapFn<SymlinkEntry>
    }
}

fn artifact_map_fn(digest_absolute_paths: bool) -> &'static dyn ExceptionlessMapFn<Artifact> {
    if digest_absolute_paths {
        &RUNFILES_AND_ABSOLUTE_PATH_MAP_FN as &'static dyn ExceptionlessMapFn<Artifact>
    } else {
        &RUNFILES_AND_EXEC_PATH_MAP_FN as &'static dyn ExceptionlessMapFn<Artifact>
    }
}

// ---------------------------------------------------------------------------------------------
// ConflictPolicy, RunfilesConflictReceiver
// ---------------------------------------------------------------------------------------------

/// Behavior upon finding a conflict between two runfile entries. A conflict means that two
/// different artifacts have the same runfiles path specified. For example, adding artifact
/// "a.foo" at path "bar" when there is already an artifact "b.foo" at path "bar". The policies
/// are ordered from least strict to most strict.
///
/// Note that conflicts are found relatively late, when the manifest file is created, not when
/// the symlinks are added to runfiles.
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:193 Runfiles.ConflictPolicy
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConflictPolicy {
    WARN,
    ERROR,
}

impl ConflictPolicy {
    /// Java `Enum.ordinal`.
    pub fn ordinal(self) -> i32 {
        match self {
            ConflictPolicy::WARN => 0,
            ConflictPolicy::ERROR => 1,
        }
    }
}

// Java: Enum.toString (the constant name).
impl fmt::Display for ConflictPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ConflictPolicy::WARN => "WARN",
            ConflictPolicy::ERROR => "ERROR",
        })
    }
}

/// Informed of conflicts in the runfiles tree.
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:462 Runfiles.RunfilesConflictReceiver
pub trait RunfilesConflictReceiver {
    /// Called when a runfiles tree artifact is detected inside another runfiles tree.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:465 RunfilesConflictReceiver.nestedRunfilesTree
    fn nested_runfiles_tree(&mut self, runfiles_tree: &Artifact);

    /// Called when one runfiles entry is a prefix of another.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:468 RunfilesConflictReceiver.prefixConflict
    fn prefix_conflict(&mut self, message: String);

    /// Java `receiver == RunfilesConflictReceiver.NO_OP`.
    fn is_no_op(&self) -> bool {
        false
    }
}

/// `RunfilesConflictReceiver.NO_OP`.
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:471 RunfilesConflictReceiver.NO_OP
pub struct NoOpRunfilesConflictReceiver;

impl RunfilesConflictReceiver for NoOpRunfilesConflictReceiver {
    fn nested_runfiles_tree(&mut self, _runfiles_tree: &Artifact) {}

    fn prefix_conflict(&mut self, _message: String) {}

    fn is_no_op(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------------------------
// JavaHashMap: java.util.HashMap iteration order for PathFragment keys
// ---------------------------------------------------------------------------------------------

/// A map with the iteration order of a `java.util.HashMap<PathFragment, V>` created by Guava's
/// `Maps.newHashMapWithExpectedSize`.
///
/// PORT: java-hash-order. `Runfiles.filterListForObscuringSymlinks` builds a `HashMap` whose
/// iteration order decides which of two entries mapping to the same final runfiles path wins in
/// `getRunfilesInputs` (an external `../repo/x` and a workspace `x` when the workspace is named
/// `repo`), and the order of the key set handed to `EmptyFilesSupplier.getExtraPaths`. This
/// reproduces `HashMap`'s bucket layout: `PathFragment.hashCode` (`String.hashCode` of the
/// internal Latin-1 string, i.e. of the UTF-8 bytes), spread by `h ^ (h >>> 16)`; the table
/// size `tableSizeFor(Maps.capacity(expectedSize))` (Guava 33.5:
/// `expectedSize < 3 ? expectedSize + 1 : ceil(expectedSize / 0.75)`), doubled when the size
/// exceeds 3/4 of the capacity or a bin of a table smaller than 64 grows past 8 entries; and
/// per-bucket insertion order, which `HashMap.resize` preserves. Treeified bins (9+ colliding
/// entries in a table of 64+ slots) keep insertion order here, while Java moves the tree root
/// to the front of the bin; no realistic runfiles tree reaches that.
pub struct JavaHashMap<V> {
    /// Buckets of (spread hash, key, value). Empty until the first `put` (Java allocates the
    /// table lazily).
    table: Vec<Vec<(u32, PathFragment, V)>>,
    /// The capacity the first allocation uses (`HashMap.threshold` before allocation).
    initial_capacity: usize,
    size: usize,
    threshold: usize,
}

// Java: java/util/HashMap.java TREEIFY_THRESHOLD / MIN_TREEIFY_CAPACITY
const TREEIFY_THRESHOLD: usize = 8;
const MIN_TREEIFY_CAPACITY: usize = 64;

/// Java `PathFragment.hashCode()`: `normalizedPath.hashCode()` over the internal string.
// Java: com/google/devtools/build/lib/vfs/PathFragment.java:501 PathFragment.hashCode
fn java_path_fragment_hash_code(path: &PathFragment) -> i32 {
    let mut h: i32 = 0;
    // PORT: StringEncoding identity; the internal Latin-1 string has one char per UTF-8 byte.
    for b in path.get_path_string().as_bytes() {
        h = h.wrapping_mul(31).wrapping_add(*b as i32);
    }
    h
}

/// Java `HashMap.hash`.
fn java_hash_map_spread(h: i32) -> u32 {
    let h = h as u32;
    h ^ (h >> 16)
}

/// Guava `Maps.capacity` (33.5.0).
fn guava_maps_capacity(expected_size: usize) -> usize {
    if expected_size < 3 {
        return expected_size + 1;
    }
    if expected_size < (1 << 30) {
        return (expected_size as f64 / 0.75).ceil() as usize;
    }
    i32::MAX as usize
}

/// Java `HashMap.tableSizeFor`.
fn java_table_size_for(cap: usize) -> usize {
    cap.max(1).next_power_of_two()
}

impl<V> JavaHashMap<V> {
    /// `Maps.newHashMapWithExpectedSize(expectedSize)`.
    pub fn with_expected_size(expected_size: usize) -> JavaHashMap<V> {
        JavaHashMap {
            table: Vec::new(),
            initial_capacity: java_table_size_for(guava_maps_capacity(expected_size)),
            size: 0,
            threshold: 0,
        }
    }

    /// Java `HashMap.resize`.
    fn resize(&mut self) {
        let new_cap = if self.table.is_empty() {
            self.initial_capacity
        } else {
            self.table.len() * 2
        };
        let mut new_table: Vec<Vec<(u32, PathFragment, V)>> =
            (0..new_cap).map(|_| Vec::new()).collect();
        for bin in self.table.drain(..) {
            for entry in bin {
                let idx = (entry.0 as usize) & (new_cap - 1);
                new_table[idx].push(entry);
            }
        }
        self.table = new_table;
        // (int) (newCap * 0.75f); for capacities >= 16 Java doubles the old threshold, which is
        // the same value.
        self.threshold = new_cap * 3 / 4;
    }

    /// Java `HashMap.put`.
    pub fn put(&mut self, key: PathFragment, value: V) {
        if self.table.is_empty() {
            self.resize();
        }
        let h = java_hash_map_spread(java_path_fragment_hash_code(&key));
        let idx = (h as usize) & (self.table.len() - 1);
        if let Some(entry) = self.table[idx].iter_mut().find(|e| e.0 == h && e.1 == key) {
            entry.2 = value;
            return;
        }
        self.table[idx].push((h, key, value));
        if self.table[idx].len() > TREEIFY_THRESHOLD && self.table.len() < MIN_TREEIFY_CAPACITY {
            // HashMap.treeifyBin resizes instead of treeifying small tables.
            self.resize();
        }
        self.size += 1;
        if self.size > self.threshold {
            self.resize();
        }
    }

    /// Java `HashMap.get`.
    pub fn get(&self, key: &PathFragment) -> Option<&V> {
        if self.table.is_empty() {
            return None;
        }
        let h = java_hash_map_spread(java_path_fragment_hash_code(key));
        let idx = (h as usize) & (self.table.len() - 1);
        self.table[idx]
            .iter()
            .find(|e| e.0 == h && e.1 == *key)
            .map(|e| &e.2)
    }

    pub fn contains_key(&self, key: &PathFragment) -> bool {
        self.get(key).is_some()
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Entries in Java's iteration order.
    pub fn iter(&self) -> impl Iterator<Item = (&PathFragment, &V)> {
        self.table.iter().flatten().map(|e| (&e.1, &e.2))
    }

    /// `keySet()` in Java's iteration order.
    pub fn key_set(&self) -> IndexSet<PathFragment> {
        self.iter().map(|(k, _)| k.clone()).collect()
    }
}

// ---------------------------------------------------------------------------------------------
// Runfiles
// ---------------------------------------------------------------------------------------------

/// An object that encapsulates runfiles. Conceptually, the runfiles are a map of paths to files,
/// forming a symlink tree.
///
/// In order to reduce memory consumption, this map is not explicitly stored here, but instead as
/// a combination of three parts: artifacts placed at their output-dir-relative paths, source
/// tree symlinks and root symlinks (outside of the source tree).
///
/// A shared handle: `Runfiles` has no `equals` in Java, so equality is identity
/// ([`Runfiles::ptr_eq`]).
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:72 Runfiles
#[derive(Clone)]
pub struct Runfiles(Arc<RunfilesInner>);

struct RunfilesInner {
    /// The directory to put all runfiles under.
    ///
    /// Using "foo" will put runfiles under `<target>.runfiles/foo`.
    ///
    /// This is either set to the workspace name, or the empty string.
    prefix: String,

    /// The artifacts that should be present in the runfiles directory.
    ///
    /// This collection may not include any runfiles trees. These artifacts will be placed at a
    /// location that corresponds to the output-dir-relative path of each artifact. It's possible
    /// for several artifacts to have the same output-dir-relative path, in which case the last
    /// one will win.
    artifacts: NestedSet<Artifact>,

    /// A map of symlinks that should be present in the runfiles directory. In general, the
    /// symlink can be determined from the artifact by using the output-dir-relative path, so
    /// this should only be used for cases where that isn't possible.
    ///
    /// This may include runfiles symlinks from the root of the runfiles tree.
    symlinks: NestedSet<SymlinkEntry>,

    /// A map of symlinks that should be present above the runfiles directory. These are useful
    /// for certain rule types like AppEngine apps which have root level config files outside of
    /// the regular source tree.
    root_symlinks: NestedSet<SymlinkEntry>,

    /// A nested set of all artifacts that this Runfiles entry contains symlinks to, including
    /// those at their non-canonical locations which are in `symlinks` and `root_symlinks`.
    all_artifacts: OnceLock<NestedSet<Artifact>>,

    /// Generates extra (empty file) inputs.
    empty_files_supplier: EmptyFilesSupplierRef,

    /// Policy for this Runfiles tree. Mutable in Java (`setConflictPolicy`).
    conflict_policy: RwLock<ConflictPolicy>,
}

impl Runfiles {
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:201 Runfiles.<init>
    fn new(
        prefix: String,
        artifacts: NestedSet<Artifact>,
        symlinks: NestedSet<SymlinkEntry>,
        root_symlinks: NestedSet<SymlinkEntry>,
        empty_files_supplier: EmptyFilesSupplierRef,
        conflict_policy: ConflictPolicy,
    ) -> Runfiles {
        Runfiles(Arc::new(RunfilesInner {
            prefix,
            artifacts,
            symlinks,
            root_symlinks,
            all_artifacts: OnceLock::new(),
            empty_files_supplier,
            conflict_policy: RwLock::new(conflict_policy),
        }))
    }

    /// `Runfiles.EMPTY`.
    pub fn empty() -> Runfiles {
        EMPTY.clone()
    }

    /// Java `==` on `Runfiles` references.
    pub fn ptr_eq(a: &Runfiles, b: &Runfiles) -> bool {
        Arc::ptr_eq(&a.0, &b.0)
    }

    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:217 Runfiles.isImmutable
    pub fn is_immutable(&self) -> bool {
        true // immutable and Starlark-hashable
    }

    /// Returns the runfiles' prefix. This is the same as the workspace name.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:222 Runfiles.getPrefix
    pub fn get_prefix(&self) -> &str {
        &self.0.prefix
    }

    /// Returns the collection of runfiles as artifacts (Starlark `files` wraps this in a depset).
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:232 Runfiles.getArtifacts
    pub fn get_artifacts(&self) -> &NestedSet<Artifact> {
        &self.0.artifacts
    }

    /// Returns the symlinks (Starlark `symlinks` wraps this in a depset).
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:242 Runfiles.getSymlinks
    pub fn get_symlinks(&self) -> &NestedSet<SymlinkEntry> {
        &self.0.symlinks
    }

    /// Starlark `empty_filenames`: the empty file names as a stable-order set of strings.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:247 Runfiles.getEmptyFilenamesForStarlark
    pub fn get_empty_filenames_for_starlark(&self) -> NestedSet<String> {
        NestedSetBuilder::wrap(
            Order::STABLE_ORDER,
            self.get_empty_filenames()
                .iter()
                .map(|p| p.get_path_string().to_string()),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:255 Runfiles.getEmptyFilenames
    pub fn get_empty_filenames(&self) -> Vec<PathFragment> {
        if is_dummy_empty_files_supplier(&self.0.empty_files_supplier) {
            return Vec::new();
        }
        let mut manifest_keys: IndexSet<PathFragment> = IndexSet::new();
        for symlink in self.0.symlinks.to_list().iter() {
            manifest_keys.insert(symlink.get_path().clone());
        }
        for artifact in self.0.artifacts.to_list().iter() {
            manifest_keys.insert(artifact.get_runfiles_path());
        }
        self.0.empty_files_supplier.get_extra_paths(&manifest_keys)
    }

    /// Drops entries obscured by an ancestor entry, reporting obscurers that change the target.
    ///
    /// `working_manifest` iterates in the order of the Java map passed in (the `LinkedHashMap` of
    /// `getRunfilesInputs`). Values are `None` where Java holds null.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:268 Runfiles.filterListForObscuringSymlinks
    pub fn filter_list_for_obscuring_symlinks(
        receiver: &mut dyn RunfilesConflictReceiver,
        working_manifest: &IndexMap<PathFragment, Option<Artifact>>,
    ) -> JavaHashMap<Option<Artifact>> {
        let mut new_manifest = JavaHashMap::with_expected_size(working_manifest.len());
        let mut no_further_obstructions: HashSet<PathFragment> = HashSet::new();

        'outer: for (source, symlink) in working_manifest {
            // drop nested entries; warn if this changes anything
            let n = source.segment_count();
            let mut parents: Vec<PathFragment> = Vec::with_capacity(n);
            for j in 1..n {
                let prefix = source.sub_fragment(0, n - j);
                if no_further_obstructions.contains(&prefix) {
                    break;
                }
                parents.push(prefix.clone());
                if let Some(Some(ancestor)) = working_manifest.get(&prefix) {
                    // This is an obscuring symlink, so just drop it and move on if there's no
                    // reporter.
                    if receiver.is_no_op() {
                        continue 'outer;
                    }
                    let suffix = source.sub_fragment(n - j, n);
                    let via_ancestor = ancestor.get_exec_path().get_relative(&suffix);
                    let expected = symlink
                        .as_ref()
                        .expect("NullPointerException: null runfiles manifest entry")
                        .get_exec_path();
                    if via_ancestor != *expected {
                        receiver.prefix_conflict(format!(
                            "runfiles symlink {} -> {} obscured by {} -> {}",
                            source,
                            expected,
                            prefix,
                            ancestor.get_exec_path()
                        ));
                    }
                    continue 'outer;
                }
            }
            no_further_obstructions.extend(parents);
            new_manifest.put(source.clone(), symlink.clone());
        }
        new_manifest
    }

    /// Returns the symlinks as a map from `PathFragment` to `Artifact`.
    ///
    /// Any errors during the conversion are ignored.
    ///
    /// `repo_mapping_manifest`: repository mapping manifest to add as a root symlink. This
    /// manifest has to be added automatically for every executable and is thus not part of the
    /// Runfiles advertised by a configured target.
    ///
    /// Returns path fragment to artifact, of normal source tree entries and elements that live
    /// outside the source tree. `None` values represent empty input files.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:328 Runfiles.getRunfilesInputs
    pub fn get_runfiles_inputs(
        &self,
        repo_mapping_manifest: Option<&Artifact>,
    ) -> BTreeMap<PathFragment, Option<Artifact>> {
        self.get_runfiles_inputs_with_receiver(
            &mut NoOpRunfilesConflictReceiver,
            repo_mapping_manifest,
        )
    }

    /// Returns the symlinks as a map from PathFragment to Artifact, calling `receiver` for each
    /// conflict.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:343 Runfiles.getRunfilesInputs
    pub fn get_runfiles_inputs_with_receiver(
        &self,
        receiver: &mut dyn RunfilesConflictReceiver,
        repo_mapping_manifest: Option<&Artifact>,
    ) -> BTreeMap<PathFragment, Option<Artifact>> {
        let mut linked_manifest: IndexMap<PathFragment, Option<Artifact>> = IndexMap::new();
        for entry in self.0.symlinks.to_list().iter() {
            check_and_put(
                &mut linked_manifest,
                receiver,
                entry.get_path().clone(),
                Some(entry.get_artifact()),
            );
        }
        for artifact in self.0.artifacts.to_list().iter() {
            check_and_put(
                &mut linked_manifest,
                receiver,
                artifact.get_runfiles_path(),
                Some(artifact),
            );
        }

        let mut manifest = Runfiles::filter_list_for_obscuring_symlinks(receiver, &linked_manifest);

        // TODO(bazel-team): Create /dev/null-like Artifact to avoid nulls?
        let key_set = manifest.key_set();
        for extra_path in self.0.empty_files_supplier.get_extra_paths(&key_set) {
            manifest.put(extra_path, None);
        }

        // Copy manifest map to another manifest map, prepending the workspace name to every
        // path. E.g. for workspace "myworkspace", the runfile entry "mylib.so"->"/path/to/mylib.so"
        // becomes "myworkspace/mylib.so"->"/path/to/mylib.so".
        let workspace_name = PathFragment::create(&self.0.prefix);
        let mut saw_workspace_name = false;
        let mut final_manifest: BTreeMap<PathFragment, Option<Artifact>> = BTreeMap::new();

        for (path, value) in manifest.iter() {
            // Artifacts in the manifest map were already checked for nested runfiles trees, so
            // we can add them directly to finalManifest without calling checkAndAdd again.
            if path.starts_with(&EXTERNAL_RUNFILES_PATH_PREFIX) {
                // Always add the non-legacy .runfiles/repo/whatever path.
                let external_path = path.sub_fragment_from(1);
                saw_workspace_name |= external_path.starts_with(&workspace_name);
                final_manifest.insert(external_path, value.clone());
            } else {
                saw_workspace_name = true;
                // PORT: workspacePathInterner (a memory optimization) is not ported.
                final_manifest.insert(workspace_name.get_relative(path), value.clone());
            }
        }

        for entry in self.0.root_symlinks.to_list().iter() {
            saw_workspace_name |= entry.get_path().starts_with(&workspace_name);
            check_and_put(
                &mut final_manifest,
                receiver,
                entry.get_path().clone(),
                Some(entry.get_artifact()),
            );
        }

        if let Some(repo_mapping_manifest) = repo_mapping_manifest {
            check_and_put(
                &mut final_manifest,
                receiver,
                REPO_MAPPING_PATH_FRAGMENT.clone(),
                Some(repo_mapping_manifest),
            );
        }

        if !saw_workspace_name {
            // If we haven't seen it and we have seen other files, add the workspace name
            // directory. It might not be there if all of the runfiles are from other repos (and
            // then running from x.runfiles/ws will fail, because ws won't exist). We can't tell
            // Runfiles to create a directory, so instead this creates a hidden file inside the
            // desired directory.
            final_manifest.insert(workspace_name.get_relative_str(".runfile"), None);
        }

        final_manifest
    }

    /// Returns the root symlinks (Starlark `root_symlinks` wraps this in a depset).
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:409 Runfiles.getRootSymlinks
    pub fn get_root_symlinks(&self) -> &NestedSet<SymlinkEntry> {
        &self.0.root_symlinks
    }

    /// Returns the manifest expander specified for this runfiles tree.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:416 Runfiles.getEmptyFilesProvider
    pub fn get_empty_files_provider(&self) -> &EmptyFilesSupplierRef {
        &self.0.empty_files_supplier
    }

    /// Returns the unified map of path fragments to artifacts, taking into account artifacts and
    /// symlinks. The returned set is guaranteed to be a (not necessarily strict) superset of the
    /// actual runfiles tree created at execution time.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:425 Runfiles.getAllArtifacts
    pub fn get_all_artifacts(&self) -> NestedSet<Artifact> {
        if self.is_empty() {
            return NestedSetBuilder::empty_set(Order::STABLE_ORDER);
        }

        self.0
            .all_artifacts
            .get_or_init(|| {
                let mut builder: NestedSetBuilder<Artifact> = NestedSetBuilder::stable_order();
                builder.add_transitive(&self.0.artifacts);
                builder.add_all(
                    self.0
                        .symlinks
                        .to_list()
                        .iter()
                        .map(|s| s.get_artifact().clone()),
                );
                builder.add_all(
                    self.0
                        .root_symlinks
                        .to_list()
                        .iter()
                        .map(|s| s.get_artifact().clone()),
                );
                builder.build()
            })
            .clone()
    }

    /// Returns if there are no runfiles.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:445 Runfiles.isEmpty
    pub fn is_empty(&self) -> bool {
        self.0.artifacts.is_empty() && self.0.symlinks.is_empty() && self.0.root_symlinks.is_empty()
    }

    /// Returns currently policy for conflicting symlink entries.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:450 Runfiles.getConflictPolicy
    pub fn get_conflict_policy(&self) -> ConflictPolicy {
        *self.0.conflict_policy.read()
    }

    /// Set whether we should warn about conflicting symlink entries.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:456 Runfiles.setConflictPolicy
    pub fn set_conflict_policy(&self, conflict_policy: ConflictPolicy) -> Runfiles {
        *self.0.conflict_policy.write() = conflict_policy;
        self.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:803 Runfiles.verifyNestedSetDepthLimitHelper
    fn verify_nested_set_depth_limit_helper<T>(
        nested_set: &NestedSet<T>,
        name: &str,
        limit: i32,
    ) -> Result<(), EvalException> {
        if nested_set.get_approx_depth() > limit {
            return Err(EvalException::new(format!(
                "{} depset depth {} exceeds limit ({})",
                name,
                nested_set.get_approx_depth(),
                limit
            )));
        }
        Ok(())
    }

    /// Checks that the depth of a Runfiles object's nested sets (artifacts, symlinks, root
    /// symlinks, etc.) does not exceed Starlark's depset depth limit, as specified by
    /// `--nested_set_depth_limit`.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:821 Runfiles.verifyNestedSetDepthLimit
    // PORT: takes the semantics' `NESTED_SET_DEPTH_LIMIT` value instead of `StarlarkSemantics`.
    fn verify_nested_set_depth_limit(
        self,
        nested_set_depth_limit: i32,
    ) -> Result<Runfiles, EvalException> {
        let limit = nested_set_depth_limit;
        Runfiles::verify_nested_set_depth_limit_helper(&self.0.artifacts, "artifacts", limit)?;
        Runfiles::verify_nested_set_depth_limit_helper(&self.0.symlinks, "symlinks", limit)?;
        Runfiles::verify_nested_set_depth_limit_helper(
            &self.0.root_symlinks,
            "root symlinks",
            limit,
        )?;
        Ok(self)
    }

    /// Starlark `runfiles.merge(other)`.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:835 Runfiles.merge
    // PORT: takes `--nested_set_depth_limit` (`thread.getSemantics()`) instead of the thread.
    pub fn merge(
        &self,
        other: &Runfiles,
        nested_set_depth_limit: i32,
    ) -> Result<Runfiles, EvalException> {
        let o = other;
        if self.is_empty() {
            // This is not just a memory / performance optimization. The Builder requires a
            // valid suffix, but the {@code Runfiles.EMPTY} singleton has an invalid one, which
            // must not be used to construct a Runfiles.Builder.
            return Ok(o.clone());
        } else if o.is_empty() {
            return Ok(self.clone());
        }
        RunfilesBuilder::new(&self.0.prefix)
            .merge(self)
            .merge(o)
            .build()
            .verify_nested_set_depth_limit(nested_set_depth_limit)
    }

    /// Starlark `runfiles.merge_all(other)`.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:848 Runfiles.mergeAll
    // PORT: the Starlark layer performs `Sequence.cast(sequence, Runfiles.class, "param")` and
    // passes the cast elements.
    pub fn merge_all(
        &self,
        sequence: &[Runfiles],
        nested_set_depth_limit: i32,
    ) -> Result<Runfiles, EvalException> {
        // The delayed initialization of the Builder is not just a memory / performance
        // optimization. The Builder requires a valid suffix, but the {@code Runfiles.EMPTY}
        // singleton has an invalid one, which must not be used to construct a Runfiles.Builder.
        let mut builder: Option<RunfilesBuilder> = None;
        // When merging exactly one non-empty Runfiles object, we want to return that object and
        // avoid a Builder. This is a memory optimization and provides identical behavior for
        // `x.merge_all([y])` and `x.merge(y)` in Starlark.
        let mut unique_non_empty_mergee: Option<Runfiles> = None;
        if !self.is_empty() {
            let mut b = RunfilesBuilder::new(&self.0.prefix);
            b.merge(self);
            builder = Some(b);
            unique_non_empty_mergee = Some(self.clone());
        }

        for runfiles in sequence {
            if !runfiles.is_empty() {
                if builder.is_none() {
                    builder = Some(RunfilesBuilder::new(&runfiles.0.prefix));
                    unique_non_empty_mergee = Some(runfiles.clone());
                } else {
                    unique_non_empty_mergee = None;
                }
                builder.as_mut().unwrap().merge(runfiles);
            }
        }

        if let Some(unique) = unique_non_empty_mergee {
            Ok(unique)
        } else if let Some(builder) = builder {
            builder
                .build()
                .verify_nested_set_depth_limit(nested_set_depth_limit)
        } else {
            Ok(Runfiles::empty())
        }
    }

    /// Fingerprint this `Runfiles` tree, including the absolute paths of artifacts.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:893 Runfiles.fingerprint
    pub fn fingerprint(
        &self,
        action_key_context: &ActionKeyContext,
        fp: &mut Fingerprint,
        digest_absolute_paths: bool,
    ) {
        fp.add_int(self.get_conflict_policy().ordinal());
        fp.add_string(&self.0.prefix);

        action_key_context.add_nested_set_to_fingerprint_exceptionless(
            symlink_entry_map_fn(digest_absolute_paths),
            fp,
            &self.0.symlinks,
        );
        action_key_context.add_nested_set_to_fingerprint_exceptionless(
            symlink_entry_map_fn(digest_absolute_paths),
            fp,
            &self.0.root_symlinks,
        );
        action_key_context.add_nested_set_to_fingerprint_exceptionless(
            artifact_map_fn(digest_absolute_paths),
            fp,
            &self.0.artifacts,
        );

        self.0.empty_files_supplier.fingerprint(fp);
    }

    /// Describes the inputs [`Runfiles::fingerprint`] uses to aid `describeKey()` descriptions.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:907 Runfiles.describeFingerprint
    pub fn describe_fingerprint(&self, digest_absolute_paths: bool) -> String {
        format!("conflictPolicy: {}\n", self.get_conflict_policy())
            + &format!("prefix: {}\n", self.0.prefix)
            + &format!(
                "symlinks: {}\n",
                ActionKeyContext::describe_nested_set_fingerprint(
                    symlink_entry_map_fn(digest_absolute_paths),
                    &self.0.symlinks
                )
            )
            + &format!(
                "rootSymlinks: {}\n",
                ActionKeyContext::describe_nested_set_fingerprint(
                    symlink_entry_map_fn(digest_absolute_paths),
                    &self.0.root_symlinks
                )
            )
            + &format!(
                "artifacts: {}\n",
                ActionKeyContext::describe_nested_set_fingerprint(
                    artifact_map_fn(digest_absolute_paths),
                    &self.0.artifacts
                )
            )
            + &format!(
                "emptyFilesSupplier: {}\n",
                self.0.empty_files_supplier.get_class_name()
            )
    }
}

// Java: Object.equals (identity).
impl PartialEq for Runfiles {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Runfiles {}

// Java: Object.hashCode (identity).
impl Hash for Runfiles {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.0) as usize).hash(state);
    }
}

impl fmt::Debug for Runfiles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Runfiles")
            .field("prefix", &self.0.prefix)
            .field("artifacts", &self.0.artifacts.to_list())
            .field("symlinks", &self.0.symlinks.to_list())
            .field("root_symlinks", &self.0.root_symlinks.to_list())
            .finish()
    }
}

/// A map `checkAndPut` writes to (`LinkedHashMap` or `TreeMap` in Java).
trait ManifestPut {
    fn put(&mut self, path: PathFragment, artifact: Option<Artifact>);
}

impl ManifestPut for IndexMap<PathFragment, Option<Artifact>> {
    fn put(&mut self, path: PathFragment, artifact: Option<Artifact>) {
        // LinkedHashMap.put keeps the position of an existing key.
        self.insert(path, artifact);
    }
}

impl ManifestPut for BTreeMap<PathFragment, Option<Artifact>> {
    fn put(&mut self, path: PathFragment, artifact: Option<Artifact>) {
        self.insert(path, artifact);
    }
}

// Java: com/google/devtools/build/lib/analysis/Runfiles.java:480 Runfiles.checkAndPut
fn check_and_put(
    map: &mut dyn ManifestPut,
    receiver: &mut dyn RunfilesConflictReceiver,
    path: PathFragment,
    artifact: Option<&Artifact>,
) {
    match artifact {
        Some(artifact) if artifact.is_runfiles_tree() => {
            receiver.nested_runfiles_tree(artifact);
        }
        _ => map.put(path, artifact.cloned()),
    }
}

// ---------------------------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------------------------

/// Builder for Runfiles objects.
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:493 Runfiles.Builder
pub struct RunfilesBuilder {
    /// This is set to the workspace name
    prefix: String,

    /// This must be COMPILE_ORDER because `getRunfilesInputs` overwrites earlier entries with
    /// later ones, so we want a post-order iteration.
    artifacts_builder: NestedSetBuilder<Artifact>,

    symlinks_builder: NestedSetBuilder<SymlinkEntry>,
    root_symlinks_builder: NestedSetBuilder<SymlinkEntry>,
    empty_files_supplier: EmptyFilesSupplierRef,

    /// Build the Runfiles object with this policy
    conflict_policy: ConflictPolicy,
}

impl RunfilesBuilder {
    /// Only used for Runfiles.EMPTY.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:515 Runfiles.Builder.<init>
    fn new_empty() -> RunfilesBuilder {
        RunfilesBuilder::new("")
    }

    /// Creates a builder with the given suffix.
    ///
    /// `workspace` is the string specified in workspace() in the WORKSPACE file.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:524 Runfiles.Builder.<init>
    pub fn new(workspace: &str) -> RunfilesBuilder {
        RunfilesBuilder {
            prefix: workspace.to_string(),
            artifacts_builder: NestedSetBuilder::compile_order(),
            symlinks_builder: NestedSetBuilder::stable_order(),
            root_symlinks_builder: NestedSetBuilder::stable_order(),
            empty_files_supplier: DUMMY_EMPTY_FILES_SUPPLIER.clone(),
            conflict_policy: ConflictPolicy::WARN,
        }
    }

    /// Builds a new Runfiles object.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:531 Runfiles.Builder.build
    pub fn build(&self) -> Runfiles {
        Runfiles::new(
            self.prefix.clone(),
            self.artifacts_builder.build(),
            self.symlinks_builder.build(),
            self.root_symlinks_builder.build(),
            self.empty_files_supplier.clone(),
            self.conflict_policy,
        )
    }

    /// Adds an artifact to the internal collection of artifacts.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:543 Runfiles.Builder.addArtifact
    pub fn add_artifact(&mut self, artifact: Artifact) -> &mut Self {
        check_argument!(
            !artifact.is_runfiles_tree(),
            "unexpected runfiles tree artifact: {}",
            artifact
        );
        self.artifacts_builder.add(artifact);
        self
    }

    /// Adds several artifacts to the internal collection.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:553 Runfiles.Builder.addArtifacts
    pub fn add_artifacts<I: IntoIterator<Item = Artifact>>(&mut self, artifacts: I) -> &mut Self {
        for artifact in artifacts {
            self.add_artifact(artifact);
        }
        self
    }

    /// Adds a nested set to the internal collection.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:562 Runfiles.Builder.addTransitiveArtifacts
    pub fn add_transitive_artifacts(&mut self, artifacts: &NestedSet<Artifact>) -> &mut Self {
        self.artifacts_builder.add_transitive(artifacts);
        self
    }

    /// Adds a symlink.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:569 Runfiles.Builder.addSymlink
    pub fn add_symlink(&mut self, link: PathFragment, target: Artifact) -> &mut Self {
        self.symlinks_builder.add(SymlinkEntry::new(link, target));
        self
    }

    /// Adds several symlinks. Neither keys nor values may be null. (Java overload
    /// `addSymlinks(Map)`.)
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:576 Runfiles.Builder.addSymlinks
    pub fn add_symlinks_map<I: IntoIterator<Item = (PathFragment, Artifact)>>(
        &mut self,
        symlinks: I,
    ) -> &mut Self {
        for (key, value) in symlinks {
            self.symlinks_builder.add(SymlinkEntry::new(key, value));
        }
        self
    }

    /// Adds several symlinks as a NestedSet.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:585 Runfiles.Builder.addSymlinks
    pub fn add_symlinks(&mut self, symlinks: &NestedSet<SymlinkEntry>) -> &mut Self {
        self.symlinks_builder.add_transitive(symlinks);
        self
    }

    /// Adds a root symlink.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:592 Runfiles.Builder.addRootSymlink
    pub fn add_root_symlink(&mut self, link: PathFragment, target: Artifact) -> &mut Self {
        self.root_symlinks_builder
            .add(SymlinkEntry::new(link, target));
        self
    }

    /// Adds several root symlinks. Neither keys nor values may be null. (Java overload
    /// `addRootSymlinks(Map)`.)
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:599 Runfiles.Builder.addRootSymlinks
    pub fn add_root_symlinks_map<I: IntoIterator<Item = (PathFragment, Artifact)>>(
        &mut self,
        symlinks: I,
    ) -> &mut Self {
        for (key, value) in symlinks {
            self.root_symlinks_builder
                .add(SymlinkEntry::new(key, value));
        }
        self
    }

    /// Adds several root symlinks as a NestedSet.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:608 Runfiles.Builder.addRootSymlinks
    pub fn add_root_symlinks(&mut self, symlinks: &NestedSet<SymlinkEntry>) -> &mut Self {
        self.root_symlinks_builder.add_transitive(symlinks);
        self
    }

    /// Specify a function that can create additional manifest entries based on the input
    /// entries, see [`EmptyFilesSupplier`] for more details.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:617 Runfiles.Builder.setEmptyFilesSupplier
    pub fn set_empty_files_supplier(&mut self, supplier: EmptyFilesSupplierRef) -> &mut Self {
        self.empty_files_supplier = supplier;
        self
    }

    /// Merges runfiles from a given runfiles support.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:628 Runfiles.Builder.merge
    pub fn merge_runfiles_support(
        &mut self,
        runfiles_support: Option<&RunfilesSupport>,
    ) -> &mut Self {
        let Some(runfiles_support) = runfiles_support else {
            return self;
        };
        self.merge(runfiles_support.get_runfiles());
        self
    }

    /// Adds the other [`Runfiles`] object transitively.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:638 Runfiles.Builder.merge
    pub fn merge(&mut self, runfiles: &Runfiles) -> &mut Self {
        // Propagate the most strict conflict checking from merged-in runfiles.
        if self.conflict_policy == ConflictPolicy::WARN {
            self.conflict_policy = runfiles.get_conflict_policy();
        }
        if runfiles.is_empty() {
            return self;
        }
        // The prefix should be the same within any blaze build, except for the EMPTY runfiles,
        // which may have an empty prefix, but that is covered above.
        check_argument!(
            self.prefix == runfiles.0.prefix,
            "{} != {}",
            self.prefix,
            runfiles.0.prefix
        );
        self.artifacts_builder
            .add_transitive(runfiles.get_artifacts());
        self.symlinks_builder
            .add_transitive(runfiles.get_symlinks());
        self.root_symlinks_builder
            .add_transitive(runfiles.get_root_symlinks());
        if is_dummy_empty_files_supplier(&self.empty_files_supplier) {
            self.empty_files_supplier = runfiles.get_empty_files_provider().clone();
        } else {
            let other_supplier = runfiles.get_empty_files_provider();
            check_state!(
                is_dummy_empty_files_supplier(other_supplier)
                    || Arc::ptr_eq(&self.empty_files_supplier, other_supplier)
                    || self.empty_files_supplier.equals(other_supplier.as_ref())
            );
        }
        self
    }

    /// Adds the runfiles for a particular target and visits the transitive closure of "srcs",
    /// "deps" and "data", collecting all of their respective runfiles.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:669 Runfiles.Builder.addRunfiles
    // PORT: needs RuleContext and TransitiveInfoCollection (analysis lead); parameters omitted.
    pub fn add_runfiles(&mut self) -> &mut Self {
        unported!("Runfiles.Builder.addRunfiles")
    }

    /// Adds the files specified by a mapping from the transitive info collection to the
    /// runfiles. Dependencies in `srcs` and `deps` are considered.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:684 Runfiles.Builder.add
    // PORT: needs RuleContext and TransitiveInfoCollection (analysis lead); parameters omitted.
    pub fn add(&mut self) -> &mut Self {
        unported!("Runfiles.Builder.add")
    }

    /// Collects runfiles from data dependencies of a target.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:699 Runfiles.Builder.addDataDeps
    // PORT: needs RuleContext (analysis lead); parameters omitted.
    pub fn add_data_deps(&mut self) -> &mut Self {
        unported!("Runfiles.Builder.addDataDeps")
    }

    /// Collects runfiles from "srcs" and "deps" of a target.
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:709 Runfiles.Builder.addNonDataDeps
    // PORT: needs RuleContext (analysis lead); parameters omitted.
    pub fn add_non_data_deps(&mut self) -> &mut Self {
        unported!("Runfiles.Builder.addNonDataDeps")
    }

    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:718 Runfiles.Builder.addTargets
    // PORT: needs TransitiveInfoCollection and RunfilesProvider (analysis lead); parameters
    // omitted.
    pub fn add_targets(&mut self) -> &mut Self {
        unported!("Runfiles.Builder.addTargets")
    }

    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:724 Runfiles.Builder.addTarget
    // PORT: needs TransitiveInfoCollection and RunfilesProvider (analysis lead); parameters
    // omitted. `addTargetIncludingFileTargets`, `addTargetExceptFileTargets`, `getNonDataDeps`
    // and `getPrerequisites` belong with it.
    pub fn add_target(&mut self) -> &mut Self {
        unported!("Runfiles.Builder.addTarget")
    }
}
