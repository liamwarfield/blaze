//! Port of `lib/sandbox/SandboxStash.java`.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use blaze_cmdline::Label;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_vfs::{FileSystemExt, FileSystemRef, Path, PathFragment};
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use super::sandbox_helpers::{self, SandboxContents, SandboxOutputs, is_interrupted_error};
use crate::exec::tree_deleter::TreeDeleter;

/// `SandboxStash.SANDBOX_STASH_BASE`.
// Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:58 SandboxStash.SANDBOX_STASH_BASE
pub const SANDBOX_STASH_BASE: &str = "sandbox_stash";

/// `SandboxStash.TEMPORARY_SANDBOX_STASH_BASE`: used while we gather all the contents
/// asynchronously.
// Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:61 SandboxStash.TEMPORARY_SANDBOX_STASH_BASE
pub const TEMPORARY_SANDBOX_STASH_BASE: &str = "tmp_sandbox_stash";
const TEST_RUNNER_MNEMONIC: &str = "TestRunner";
const TEST_SRCDIR: &str = "TEST_SRCDIR";

/// An incrementing count of stashes to avoid filename clashes.
// Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:67 SandboxStash.stash
pub static STASH: AtomicI32 = AtomicI32::new(0);
/// The monitor of `synchronized (stash)`.
static STASH_MONITOR: Mutex<()> = Mutex::new(());

/// Whether to attempt to reuse previously-created sandboxes. Not final because we may turn it off
/// in case of errors.
// Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:76 SandboxStash.reuseSandboxDirectories
pub static REUSE_SANDBOX_DIRECTORIES: AtomicBool = AtomicBool::new(false);

// Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:78 SandboxStash.instance
static INSTANCE: RwLock<Option<Arc<SandboxStash>>> = RwLock::new(None);

fn instance() -> Option<Arc<SandboxStash>> {
    INSTANCE.read().clone()
}

/// Singleton class for the `--reuse_sandbox_directories` flag: Controls a "stash" of old sandbox
/// directories. When a sandboxed runner needs its directory tree, it first tries to grab a stash
/// by just moving it. They are separated by mnemonic because that makes them much more likely to
/// be able to reuse things common for that mnemonic, e.g. standard libraries.
///
/// PORT: Java gathers the in-memory stash contents on a thread pool (`stashFileListingPool`); this
/// port runs that job inline in `stashSandbox`, which is one of the interleavings the Java code
/// allows.
// Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:56 SandboxStash
#[derive(Debug)]
pub struct SandboxStash {
    /// If true, we have already warned about an error causing us to turn off reuse.
    warned_about_turning_off_reuse: AtomicBool,
    workspace_name: String,
    sandbox_base: Path,
    stash_path_to_runfiles_dir: Mutex<HashMap<Path, String>>,
    pub path_to_contents: Mutex<HashMap<Path, SandboxContents>>,
    sandbox_to_target: Mutex<HashMap<Path, Label>>,
    path_to_last_modified: Mutex<HashMap<Path, i64>>,
    in_memory_stashes: AtomicBool,
}

impl SandboxStash {
    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:95 SandboxStash.<init>
    pub fn new(
        workspace_name: String,
        sandbox_base: Path,
        in_memory_stashes: bool,
    ) -> SandboxStash {
        SandboxStash {
            warned_about_turning_off_reuse: AtomicBool::new(false),
            workspace_name,
            sandbox_base,
            stash_path_to_runfiles_dir: Mutex::new(HashMap::new()),
            path_to_contents: Mutex::new(HashMap::new()),
            sandbox_to_target: Mutex::new(HashMap::new()),
            path_to_last_modified: Mutex::new(HashMap::new()),
            in_memory_stashes: AtomicBool::new(in_memory_stashes),
        }
    }

    /// `takeStashedSandbox(...)`: `None` is Java's `null` (no stash taken); `Some(None)` is
    /// `Optional.empty()` (a stash was taken but its contents are not in memory).
    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:103 SandboxStash.takeStashedSandbox
    pub fn take_stashed_sandbox(
        sandbox_path: &Path,
        mnemonic: &str,
        environment: &IndexMap<String, String>,
        outputs: &SandboxOutputs,
        target: Option<&Label>,
    ) -> Option<Option<SandboxContents>> {
        let instance = instance()?;
        instance.take_stashed_sandbox_internal(sandbox_path, mnemonic, environment, outputs, target)
    }

    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:117 SandboxStash.takeStashedSandboxInternal
    fn take_stashed_sandbox_internal(
        &self,
        sandbox_path: &Path,
        mnemonic: &str,
        environment: &IndexMap<String, String>,
        outputs: &SandboxOutputs,
        target: Option<&Label>,
    ) -> Option<Option<SandboxContents>> {
        let result: io::Result<Option<Option<SandboxContents>>> = (|| {
            let sandboxes = self.get_sandbox_stash_dir(mnemonic, sandbox_path.get_file_system());
            let Some(sandboxes) = sandboxes else {
                return Ok(None);
            };
            if is_test_xml_generation_or_coverage_spawn(mnemonic, outputs) {
                return Ok(None);
            }

            let disk_stashes = sandboxes.get_directory_entries()?;
            if disk_stashes.is_empty() {
                return Ok(None);
            }

            let stashes = self.sort_stashes_by_matching_target_segments(target, disk_stashes);
            // We have to remove the sandbox execroot dir to move a stash there, but it is
            // currently empty and we reinstate it later if we don't get a sandbox. We can't just
            // move the stash dir fully, as we would then lose siblings of the execroot dir, such
            // as hermetic-tmp dirs.
            let sandbox_execroot = sandbox_path.get_child("execroot");
            sandbox_execroot.delete_tree()?;
            for stash in stashes {
                let attempt: io::Result<Option<SandboxContents>> = (|| {
                    let stash_execroot = stash.get_child("execroot");
                    stash_execroot.rename_to(&sandbox_execroot)?;
                    stash.delete_tree()?;
                    if is_test_action(mnemonic) {
                        let relative_stashed_runfiles_dir = self
                            .stash_path_to_runfiles_dir
                            .lock()
                            .get(&stash_execroot)
                            .cloned()
                            .expect("NullPointerException: stashed runfiles dir");
                        let stashed_runfiles_dir =
                            sandbox_execroot.get_relative_str(&relative_stashed_runfiles_dir);
                        let relative_current_runfiles_dir = get_current_runfiles_dir(environment);
                        let current_runfiles =
                            sandbox_execroot.get_relative_str(&relative_current_runfiles_dir);
                        current_runfiles
                            .get_parent_directory()
                            .expect("NullPointerException: runfiles has no parent")
                            .create_directory_and_parents()?;
                        stashed_runfiles_dir.rename_to(&current_runfiles)?;
                        self.stash_path_to_runfiles_dir
                            .lock()
                            .remove(&stash_execroot);
                        if use_in_memory_stashes() {
                            let mut path_to_contents = self.path_to_contents.lock();
                            if let Some(contents) = path_to_contents.get_mut(&stash) {
                                update_stash_contents_after_runfiles_move(
                                    &relative_stashed_runfiles_dir,
                                    &relative_current_runfiles_dir,
                                    contents,
                                );
                            }
                        }
                    }
                    self.sandbox_to_target.lock().remove(&stash);
                    // If we switched the flag experimental_inmemory_sandbox_stashes from false to
                    // true without restarting the Bazel server, we may have a stash but not its
                    // contents in memory.
                    if use_in_memory_stashes() {
                        Ok(self.path_to_contents.lock().remove(&stash))
                    } else {
                        Ok(None)
                    }
                })();
                match attempt {
                    Ok(contents) => return Ok(Some(contents)),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        // Try the next one, somebody else took this one.
                    }
                    Err(e) => {
                        self.turn_off_reuse(format!(
                            "Error renaming sandbox stash {} to {}: {}\n",
                            stash, sandbox_path, e
                        ));
                        return Ok(None);
                    }
                }
            }
            Ok(None)
        })();
        match result {
            Ok(r) => r,
            Err(e) => {
                self.turn_off_reuse(format!(
                    "Failed to prepare for reusing stashed sandbox for {}: {}",
                    sandbox_path, e
                ));
                None
            }
        }
    }

    /// Atomically moves the `path` directory aside for later reuse.
    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:182 SandboxStash.stashSandbox
    pub fn stash_sandbox(
        path: &Path,
        mnemonic: &str,
        environment: &IndexMap<String, String>,
        outputs: &SandboxOutputs,
        tree_deleter: &dyn TreeDeleter,
        target: Option<&Label>,
    ) {
        let Some(instance) = instance() else {
            return;
        };

        let Some(sandboxes) = instance.get_sandbox_stash_dir(mnemonic, path.get_file_system())
        else {
            return;
        };
        if is_test_xml_generation_or_coverage_spawn(mnemonic, outputs) || !path.exists() {
            return;
        }
        let stash_name = (STASH.fetch_add(1, Ordering::SeqCst) + 1).to_string();

        if use_in_memory_stashes() {
            instance.stash_sandbox_internal_with_in_memory_stashes(
                &stash_name,
                &sandboxes,
                path,
                mnemonic,
                environment,
                tree_deleter,
                target,
            );
        } else {
            instance.stash_sandbox_internal(
                &stash_name,
                &sandboxes,
                path,
                mnemonic,
                environment,
                tree_deleter,
                target,
            );
        }
    }

    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:211 SandboxStash.stashSandboxInternalWithInMemoryStashes
    fn stash_sandbox_internal_with_in_memory_stashes(
        &self,
        stash_name: &str,
        sandboxes: &Path,
        path: &Path,
        mnemonic: &str,
        environment: &IndexMap<String, String>,
        tree_deleter: &dyn TreeDeleter,
        target: Option<&Label>,
    ) {
        let temporary_stashes = self.sandbox_base.get_child(TEMPORARY_SANDBOX_STASH_BASE);
        let temporary_stash = temporary_stashes.get_child(stash_name);
        let moved: io::Result<()> = (|| {
            temporary_stashes.create_directory()?;
            path.get_child("execroot").rename_to(&temporary_stash)
        })();
        if let Err(e) = moved {
            self.turn_off_reuse(format!(
                "Error stashing sandbox at {}: {}",
                temporary_stash, e
            ));
        }
        // PORT: Java submits the rest to `stashFileListingPool`; it runs inline here. An unchecked
        // exception in the pool task (null contents) is swallowed by the executor: the task just
        // ends, as the early returns below do.
        let stash_path = sandboxes.get_child(stash_name);
        let job: io::Result<()> = (|| {
            let Some(mut stash_contents) = self.path_to_contents.lock().remove(path) else {
                return Ok(());
            };
            let Some(last_modified) = self.path_to_last_modified.lock().remove(path) else {
                return Ok(());
            };
            sandbox_helpers::update_content_map(
                &temporary_stash,
                last_modified,
                &mut stash_contents,
            )?;
            stash_path.create_directory()?;
            let stash_path_execroot = stash_path.get_child("execroot");
            if is_test_action(mnemonic) {
                let test_tmpdir = environment
                    .get("TEST_TMPDIR")
                    .expect("NullPointerException: TEST_TMPDIR");
                if test_tmpdir.starts_with("_tmp") {
                    let test_workspace = environment
                        .get("TEST_WORKSPACE")
                        .map(String::as_str)
                        .unwrap_or("null");
                    tree_deleter.delete_tree(
                        &temporary_stash.get_relative_str(&format!("{}/_tmp", test_workspace)),
                    )?;
                }
                // We do this before the rename operation to avoid a race condition.
                self.stash_path_to_runfiles_dir.lock().insert(
                    stash_path_execroot.clone(),
                    get_current_runfiles_dir(environment),
                );
            }
            self.path_to_contents
                .lock()
                .insert(stash_path.clone(), stash_contents);
            temporary_stash.rename_to(&stash_path_execroot)?;
            if let Some(target) = target {
                self.sandbox_to_target
                    .lock()
                    .insert(stash_path.clone(), target.clone());
            }
            Ok(())
        })();
        match job {
            Ok(()) => {}
            Err(e) if is_interrupted_error(&e) => {
                // Finish the job without stashing the sandbox
            }
            Err(e) => {
                // TODO(bazel-team): Are we sure we don't want to surface this error?
                self.turn_off_reuse(format!("Error stashing sandbox at {}: {}", stash_path, e));
            }
        }
    }

    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:258 SandboxStash.stashSandboxInternal
    fn stash_sandbox_internal(
        &self,
        stash_name: &str,
        sandboxes: &Path,
        path: &Path,
        mnemonic: &str,
        environment: &IndexMap<String, String>,
        tree_deleter: &dyn TreeDeleter,
        target: Option<&Label>,
    ) {
        let stash_path = sandboxes.get_child(stash_name);
        let result: io::Result<()> = (|| {
            stash_path.create_directory()?;
            let stash_path_execroot = stash_path.get_child("execroot");
            if is_test_action(mnemonic) {
                let test_tmpdir = environment
                    .get("TEST_TMPDIR")
                    .expect("NullPointerException: TEST_TMPDIR");
                if test_tmpdir.starts_with("_tmp") {
                    let test_workspace = environment
                        .get("TEST_WORKSPACE")
                        .map(String::as_str)
                        .unwrap_or("null");
                    tree_deleter.delete_tree(
                        &path.get_relative_str(&format!("execroot/{}/_tmp", test_workspace)),
                    )?;
                }
            }
            if is_test_action(mnemonic) {
                // We do this before the rename operation to avoid a race condition.
                self.stash_path_to_runfiles_dir.lock().insert(
                    stash_path_execroot.clone(),
                    get_current_runfiles_dir(environment),
                );
            }
            path.get_child("execroot").rename_to(&stash_path_execroot)?;
            if let Some(target) = target {
                self.sandbox_to_target
                    .lock()
                    .insert(stash_path.clone(), target.clone());
            }
            Ok(())
        })();
        if let Err(e) = result {
            // Since stash names are unique, this IOException indicates some other problem with
            // stashing, so we turn it off.
            self.turn_off_reuse(format!("Error stashing sandbox at {}: {}", stash_path, e));
        }
    }

    /// Returns the sandbox stashing directory appropriate for this mnemonic. In order to maximize
    /// reuse, we keep stashed sandboxes separated by mnemonic. May return `None` if there are
    /// errors, in which case sandbox reuse also gets turned off.
    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:301 SandboxStash.getSandboxStashDir
    fn get_sandbox_stash_dir(&self, mnemonic: &str, file_system: &FileSystemRef) -> Option<Path> {
        let stash_dir = get_stash_base(&file_system.get_path(self.sandbox_base.get_path_string()));
        match stash_dir.create_directory() {
            Ok(_) => {
                if !self.maybe_clear_existing_stash(&stash_dir) {
                    return None;
                }
            }
            Err(e) => {
                self.turn_off_reuse(format!(
                    "Error creating sandbox stash dir {}, disabling sandbox reuse: {}\n",
                    stash_dir, e
                ));
                return None;
            }
        }
        let mnemonic_stash_dir = stash_dir.get_child(mnemonic);
        match mnemonic_stash_dir.create_directory() {
            Ok(_) => Some(mnemonic_stash_dir),
            Err(e) => {
                self.turn_off_reuse(format!(
                    "Error creating mnemonic stash dir {}: {}\n",
                    mnemonic_stash_dir, e
                ));
                None
            }
        }
    }

    /// Clears away existing stash if this is the first access to the stash in this Blaze server
    /// instance. Returns true unless there was an error deleting sandbox stashes.
    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:335 SandboxStash.maybeClearExistingStash
    fn maybe_clear_existing_stash(&self, stash_path: &Path) -> bool {
        let _monitor = STASH_MONITOR.lock();
        if STASH.fetch_add(1, Ordering::SeqCst) == 0 {
            let cleared: io::Result<()> = (|| {
                for directory_entry in stash_path.get_directory_entries()? {
                    directory_entry.delete_tree()?;
                }
                Ok(())
            })();
            if let Err(e) = cleared {
                self.turn_off_reuse(format!(
                    "Unable to clear old sandbox stash {}: {}\n",
                    stash_path, e
                ));
                return false;
            }
        }
        true
    }

    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:351 SandboxStash.turnOffReuse
    fn turn_off_reuse(&self, _message: String) {
        REUSE_SANDBOX_DIRECTORIES.store(false, Ordering::SeqCst);
        // PORT: logging dropped (once: "Turning off sandbox reuse: " + message).
        let _first_warning = self
            .warned_about_turning_off_reuse
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();
    }

    /// `initialize(workspaceName, sandboxBase, options, treeDeleter)`.
    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:358 SandboxStash.initialize
    pub fn initialize(
        workspace_name: &str,
        sandbox_base: &Path,
        options: &SandboxOptions,
        tree_deleter: &dyn TreeDeleter,
    ) {
        let mut slot = INSTANCE.write();
        if options.reuse_sandbox_directories {
            match slot.as_ref() {
                None => {
                    *slot = Some(Arc::new(SandboxStash::new(
                        workspace_name.to_string(),
                        sandbox_base.clone(),
                        options.experimental_in_memory_sandbox_stashes,
                    )));
                }
                Some(current) => {
                    if workspace_name != current.workspace_name {
                        let stash_base = get_stash_base(&current.sandbox_base);
                        let cleared: io::Result<()> = (|| {
                            for directory_entry in stash_base.get_directory_entries()? {
                                tree_deleter.delete_tree(&directory_entry)?;
                            }
                            Ok(())
                        })();
                        if let Err(e) = cleared {
                            current.turn_off_reuse(format!(
                                "Unable to clear old sandbox stash {}: {}\n",
                                stash_base, e
                            ));
                        }
                        *slot = Some(Arc::new(SandboxStash::new(
                            workspace_name.to_string(),
                            sandbox_base.clone(),
                            options.experimental_in_memory_sandbox_stashes,
                        )));
                    }
                    slot.as_ref().expect("set above").in_memory_stashes.store(
                        options.experimental_in_memory_sandbox_stashes,
                        Ordering::SeqCst,
                    );
                }
            }
        } else {
            *slot = None;
        }
    }

    /// `useInMemoryStashes()`.
    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:387 SandboxStash.useInMemoryStashes
    pub fn use_in_memory_stashes() -> bool {
        use_in_memory_stashes()
    }

    /// `setPathContents(path, contents)`.
    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:392 SandboxStash.setPathContents
    pub fn set_path_contents(path: &Path, contents: SandboxContents) {
        let instance = instance().expect("NullPointerException: SandboxStash instance");
        instance
            .path_to_contents
            .lock()
            .insert(path.clone(), contents);
    }

    /// `setLastModified(path, lastModified)`.
    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:397 SandboxStash.setLastModified
    pub fn set_last_modified(path: &Path, last_modified: i64) {
        if let Some(instance) = instance() {
            instance
                .path_to_last_modified
                .lock()
                .insert(path.clone(), last_modified);
        }
    }

    /// `gotInstance()`.
    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:403 SandboxStash.gotInstance
    pub fn got_instance() -> bool {
        instance().is_some()
    }

    /// `shutdown()`: shuts down the file listing pool (none here; see the type docs).
    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:407 SandboxStash.shutdown
    pub fn shutdown() {}

    /// Cleans up the entire current stash, if any. Cleaning may be asynchronous.
    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:414 SandboxStash.clean
    pub fn clean(tree_deleter: Option<&dyn TreeDeleter>, sandbox_base: &Path) {
        let stash_dir = get_stash_base(sandbox_base);
        if !stash_dir.is_directory() {
            return;
        }
        let mut tree_deleter = tree_deleter;
        let mut stash_trash_dir = stash_dir.get_child("__trash");
        if stash_dir.rename_to(&stash_trash_dir).is_err() {
            // If we couldn't move the stashdir away for deletion, we need to delete it
            // synchronously in place, so we can't use the treeDeleter.
            tree_deleter = None;
            stash_trash_dir = stash_dir.clone();
        }
        let deleted = match tree_deleter {
            Some(d) => d.delete_tree(&stash_trash_dir),
            None => stash_trash_dir.delete_tree(),
        };
        // PORT: logging dropped (on error: "Failed to clean sandbox stash %s").
        let _ = deleted;

        if let Some(instance) = instance() {
            instance.stash_path_to_runfiles_dir.lock().clear();
            instance.path_to_contents.lock().clear();
            instance.sandbox_to_target.lock().clear();
            instance.path_to_last_modified.lock().clear();
        }
    }

    // Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:477 SandboxStash.sortStashesByMatchingTargetSegments
    fn sort_stashes_by_matching_target_segments(
        &self,
        target: Option<&Label>,
        stashes: Vec<Path>,
    ) -> Vec<Path> {
        let mut count_map: HashMap<Path, i32> = HashMap::new();
        let target_str: Option<Vec<String>> =
            target.map(|t| java_split_slash(t.get_package_name()));
        let sandbox_to_target = self.sandbox_to_target.lock();
        for stash in &stashes {
            let stash_target = sandbox_to_target.get(stash);
            let count = match &target_str {
                None => {
                    if stash_target.is_none() {
                        1
                    } else {
                        0
                    }
                }
                Some(target_str) => match stash_target {
                    None => 0,
                    Some(stash_target) => arrays_mismatch(
                        target_str,
                        &java_split_slash(stash_target.get_package_name()),
                    ),
                },
            };
            count_map.insert(stash.clone(), count);
        }
        drop(sandbox_to_target);
        let mut sorted_stashes = stashes;
        // Stable sort, descending (`Comparator.comparingInt(countMap::get).reversed()`).
        sorted_stashes.sort_by(|a, b| count_map[b].cmp(&count_map[a]));
        sorted_stashes
    }
}

/// `SandboxStash.useInMemoryStashes()` (`checkNotNull(instance)`).
fn use_in_memory_stashes() -> bool {
    instance()
        .expect("NullPointerException: SandboxStash instance")
        .in_memory_stashes
        .load(Ordering::SeqCst)
}

// Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:324 SandboxStash.getStashBase
fn get_stash_base(sandbox_base: &Path) -> Path {
    sandbox_base.get_child(SANDBOX_STASH_BASE)
}

/// Test actions are guaranteed to have a runfiles directory with the test name as part of the
/// name. The path to the directory is unique between tests. If two tests (foo and bar) have the
/// directory <source-root>/pkg/my_runfiles as part of their runfiles and this directory contains
/// 1000 files, we would be symlinking the 1000 files for each test since the paths do not
/// coincide. To make sure we can reuse the runfiles directory we must rename the old runfiles
/// directory for the action that was stashed to the path that is expected by the current test.
// Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:454 SandboxStash.isTestAction
fn is_test_action(mnemonic: &str) -> bool {
    mnemonic == TEST_RUNNER_MNEMONIC
}

/// Test actions are split in two spawns. The first one runs the test and the second generates the
/// XML output from the test log. We do not want the second spawn to reuse the stash because it
/// doesn't contain the inputs needed to run the test; if it reused it, it would be expensive in
/// two ways: it would have to clean up all the inputs, and it would destroy a valid stash that a
/// different test could potentially use. If we are running coverage, there might be a third spawn
/// for coverage where we apply the same reasoning.
///
/// We identify the second and third spawn because they have a single output.
// Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:468 SandboxStash.isTestXmlGenerationOrCoverageSpawn
fn is_test_xml_generation_or_coverage_spawn(mnemonic: &str, outputs: &SandboxOutputs) -> bool {
    is_test_action(mnemonic) && outputs.files.len() == 1
}

// Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:473 SandboxStash.getCurrentRunfilesDir
fn get_current_runfiles_dir(environment: &IndexMap<String, String>) -> String {
    format!(
        "{}/{}",
        environment
            .get("TEST_WORKSPACE")
            .map(String::as_str)
            .unwrap_or("null"),
        environment
            .get(TEST_SRCDIR)
            .map(String::as_str)
            .unwrap_or("null")
    )
}

/// Java `String.split("/")`: trailing empty strings are removed.
fn java_split_slash(s: &str) -> Vec<String> {
    let mut parts: Vec<String> = s.split('/').map(str::to_string).collect();
    if s.is_empty() {
        return parts;
    }
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

/// `Arrays.mismatch(a, b)`.
fn arrays_mismatch(a: &[String], b: &[String]) -> i32 {
    let n = a.len().min(b.len());
    for i in 0..n {
        if a[i] != b[i] {
            return i as i32;
        }
    }
    if a.len() == b.len() { -1 } else { n as i32 }
}

// Java: com/google/devtools/build/lib/sandbox/SandboxStash.java:501 SandboxStash.updateStashContentsAfterRunfilesMove
fn update_stash_contents_after_runfiles_move(
    stashed_runfiles: &str,
    current_runfiles: &str,
    stash_contents: &mut SandboxContents,
) {
    let stashed_runfiles_segments: Vec<String> = PathFragment::create(stashed_runfiles)
        .segments()
        .map(str::to_string)
        .collect();
    let mut runfiles_stash_contents: &mut SandboxContents = &mut *stash_contents;
    for segment in &stashed_runfiles_segments[..stashed_runfiles_segments.len() - 1] {
        runfiles_stash_contents = runfiles_stash_contents
            .dir_map
            .get_mut(segment)
            .expect("NullPointerException: stashed runfiles contents");
    }
    let removed = runfiles_stash_contents.dir_map.remove(
        stashed_runfiles_segments
            .last()
            .expect("NoSuchElementException"),
    );

    let current_runfiles_segments: Vec<String> = PathFragment::create(current_runfiles)
        .segments()
        .map(str::to_string)
        .collect();
    let mut current_stash_contents: &mut SandboxContents = &mut *stash_contents;
    for segment in &current_runfiles_segments[..current_runfiles_segments.len() - 1] {
        current_stash_contents = current_stash_contents
            .dir_map
            .entry(segment.clone())
            .or_default();
    }
    // PORT: Java puts a null value when the stashed runfiles directory was not in the map; the
    // entry is left out here.
    if let Some(removed) = removed {
        current_stash_contents.dir_map.insert(
            current_runfiles_segments
                .last()
                .expect("NoSuchElementException")
                .clone(),
            removed,
        );
    }
}
