//! The output part of `FilesystemValueChecker`: finding the `ACTION_EXECUTION` values whose
//! outputs were modified outside the build (`getDirtyActionValues`).
//!
//! PORT: Java's `FilesystemValueChecker` is one class; its key part (`getDirtyKeys`) is in
//! blaze_sky_loading (`filesystem_value_checker`), below the crate that knows
//! `ActionExecutionValue`. Java shards the values over a fixed thread pool; here the actions are
//! checked by a scoped pool of at most `num_threads` threads, in no particular order, and the
//! result is sorted by the order of the scanned map's iteration. The check is not interruptible.
//! Bazel's local output service has no `BatchStat` (`getBatchStatter()` returns null), so
//! `batchStatJob` is not ported; `outputStatJob` is.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use blaze_actions::Artifact;
use blaze_exec::actions::FileArtifactValue;
use blaze_exec::actions::action_adapters::as_blaze_artifact;
use blaze_exec::actions::action_cache_checker::OutputChecker;
use blaze_exec::actions::input_metadata_provider::TreeArtifactValue;
use blaze_skyframe::{Key, SkyKey, SkyValue};
use blaze_util::io::timestamp_granularity_monitor::TimestampGranularityMonitor;
use blaze_vfs::{DirentType, ModifiedFileSet, PathFragment, Symlinks};
use indexmap::IndexSet;

use crate::action_execution_value::ActionExecutionValue;
use crate::keys::ActionExecutionKey;

/// Callback for modified output files for logging/metrics.
// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:196 FilesystemValueChecker.ModifiedOutputsReceiver
pub trait ModifiedOutputsReceiver: Sync {
    /// Called on every modified artifact detected by [`get_dirty_action_values`].
    ///
    /// `maybe_modified_time`: best effort modified time, -1 when not available/missing.
    fn report_modified_output_file(&self, maybe_modified_time: i64, artifact: &Artifact);
}

/// Returns the action values which have output files that are not in-sync with the on-disk file
/// value (were modified externally).
// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:210 FilesystemValueChecker.getDirtyActionValues
pub fn get_dirty_action_values(
    values_map: &HashMap<SkyKey, SkyValue>,
    modified_output_files: &ModifiedFileSet,
    output_checker: &dyn OutputChecker,
    modified_outputs_receiver: &dyn ModifiedOutputsReceiver,
    tsgm: Option<&TimestampGranularityMonitor>,
    num_threads: usize,
) -> Vec<SkyKey> {
    if modified_output_files.is_nothing_modified() {
        // Java: logger.atInfo "Not checking for dirty actions since nothing was modified".
        return Vec::new();
    }

    let action_keys: Vec<(&SkyKey, &SkyValue)> = values_map
        .iter()
        .filter(|(k, _)| k.function_name() == ActionExecutionKey::FUNCTION)
        .collect();

    let known_modified_output_files: Option<&IndexSet<PathFragment>> =
        if modified_output_files.treat_everything_as_modified() {
            None
        } else {
            Some(modified_output_files.modified_source_files())
        };
    // Initialized lazily in Java (a memoizing supplier) because it is only used to check
    // modified TreeArtifacts.
    let sorted_known_modified_output_files: Option<Vec<PathFragment>> = known_modified_output_files
        .map(|set| {
            let mut v: Vec<PathFragment> = set.iter().cloned().collect();
            v.sort();
            v
        });

    let dirty_keys: Mutex<HashSet<SkyKey>> = Mutex::new(HashSet::new());
    let next = AtomicUsize::new(0);
    let threads = num_threads.max(1).min(action_keys.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                // Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:443 FilesystemValueChecker.outputStatJob
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some((key, value)) = action_keys.get(i) else {
                        break;
                    };
                    let dirty = match value.downcast_ref::<ActionExecutionValue>() {
                        None => true,
                        Some(value) => action_value_is_dirty_with_direct_system_calls(
                            value,
                            tsgm,
                            output_checker,
                            known_modified_output_files,
                            sorted_known_modified_output_files.as_deref(),
                            modified_outputs_receiver,
                        ),
                    };
                    if dirty {
                        dirty_keys.lock().unwrap().insert((*key).clone());
                    }
                }
            });
        }
    });
    let dirty_keys = dirty_keys.into_inner().unwrap();
    action_keys
        .into_iter()
        .map(|(k, _)| k)
        .filter(|k| dirty_keys.contains(*k))
        .cloned()
        .collect()
}

/// The exec paths of the files present now under a tree artifact's directory, against the local
/// children of its value.
// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:476 FilesystemValueChecker.treeArtifactIsDirty
fn tree_artifact_is_dirty(artifact: &Artifact, value: &TreeArtifactValue) -> bool {
    let path = artifact.get_path();
    if path.is_symbolic_link() {
        return true; // TreeArtifacts may not be symbolic links.
    }

    // This could be improved by short-circuiting as soon as we see a child that is not present
    // in the TreeArtifactValue, but it doesn't seem to be a major source of overhead.
    let mut current_local_children: HashSet<PathFragment> = HashSet::new();
    if TreeArtifactValue::visit_tree(&path, &mut |child, type_, _traversed_symlink| {
        if type_ != DirentType::Directory {
            current_local_children.insert(child.clone());
        }
        Ok(())
    })
    .is_err()
    {
        return true;
    }

    if current_local_children.is_empty() && value.is_entirely_remote() {
        return false;
    }

    let tree_exec_path = artifact.get_exec_path();
    let last_known_local_children: HashSet<PathFragment> = value
        .child_values
        .iter()
        .filter(|(_, metadata)| {
            // For remote metadata, a non-null contents proxy indicates that the file has been
            // materialized in the local filesystem.
            !metadata.is_remote() || metadata.get_contents_proxy().is_some()
        })
        .map(|(child, _)| child.get_exec_path().relative_to(tree_exec_path))
        .collect();

    current_local_children != last_known_local_children
}

// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:527 FilesystemValueChecker.artifactIsDirtyWithDirectSystemCalls
fn artifact_is_dirty_with_direct_system_calls(
    tsgm: Option<&TimestampGranularityMonitor>,
    output_checker: &dyn OutputChecker,
    known_modified_output_files: Option<&IndexSet<PathFragment>>,
    file: &Artifact,
    last_known_data: &FileArtifactValue,
    modified_outputs_receiver: &dyn ModifiedOutputsReceiver,
) -> bool {
    if file.is_runfiles_tree() || !should_check_file(known_modified_output_files, file) {
        return false;
    }
    match crate::skyframe_action_executor::file_artifact_value_for_output_check(file, tsgm) {
        Ok(file_metadata) => {
            // Remote metadata may record that the file has been materialized in the local
            // filesystem as a requested top-level output without its generating action having
            // been reexecuted (e.g. by the completion function after an action cache hit). If the
            // file is missing now, the action has to be invalidated so that its reevaluation can
            // restore the file in case the current invocation wants it locally.
            let deleted_after_materialization = file_metadata.get_type()
                == blaze_exec::actions::file_artifact_value::FileStateType::Nonexistent
                && last_known_data.is_remote()
                && last_known_data.was_materialized_as_toplevel_output();
            let is_trusted_remote_value = !deleted_after_materialization
                && file_metadata.get_type()
                    == blaze_exec::actions::file_artifact_value::FileStateType::Nonexistent
                && last_known_data.is_remote()
                && output_checker.should_trust_metadata(
                    &*blaze_exec::actions::action_adapters::artifact_input(file).0,
                    last_known_data,
                );
            if !is_trusted_remote_value && file_metadata.could_be_modified_since(last_known_data) {
                if deleted_after_materialization {
                    // Clear the record so that a reevaluation that chooses not to rematerialize
                    // the file doesn't invalidate the action again on every subsequent
                    // invocation.
                    last_known_data.set_materialized_as_toplevel_output(false);
                }
                let maybe_modified_time = if file_metadata.get_type()
                    != blaze_exec::actions::file_artifact_value::FileStateType::Nonexistent
                {
                    // file.getPath().getLastModifiedTime(Symlinks.FOLLOW); its IOException is
                    // caught below in Java (the catch reports -1 and dirty).
                    match file
                        .get_path()
                        .get_last_modified_time_with(Symlinks::Follow)
                    {
                        Ok(t) => t,
                        Err(_) => {
                            modified_outputs_receiver.report_modified_output_file(-1, file);
                            return true;
                        }
                    }
                } else {
                    -1
                };
                modified_outputs_receiver.report_modified_output_file(maybe_modified_time, file);
                return true;
            }
            false
        }
        Err(_) => {
            // This is an unexpected failure getting a digest or symlink target.
            modified_outputs_receiver.report_modified_output_file(-1, file);
            true
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:582 FilesystemValueChecker.actionValueIsDirtyWithDirectSystemCalls
fn action_value_is_dirty_with_direct_system_calls(
    action_value: &ActionExecutionValue,
    tsgm: Option<&TimestampGranularityMonitor>,
    output_checker: &dyn OutputChecker,
    known_modified_output_files: Option<&IndexSet<PathFragment>>,
    sorted_known_modified_output_files: Option<&[PathFragment]>,
    modified_outputs_receiver: &dyn ModifiedOutputsReceiver,
) -> bool {
    let mut is_dirty = false;
    for (artifact, value) in action_value.get_all_file_values() {
        if artifact_is_dirty_with_direct_system_calls(
            tsgm,
            output_checker,
            known_modified_output_files,
            artifact,
            value,
            modified_outputs_receiver,
        ) {
            is_dirty = true;
        }
    }

    for (tree_artifact, tree) in action_value.get_all_tree_artifact_values() {
        for (child, child_value) in &tree.child_values {
            // A tree's children are TreeFileArtifacts (blaze_actions artifacts).
            let child = as_blaze_artifact(child).expect("TreeFileArtifact");
            if artifact_is_dirty_with_direct_system_calls(
                tsgm,
                output_checker,
                known_modified_output_files,
                child,
                child_value,
                modified_outputs_receiver,
            ) {
                is_dirty = true;
            }
        }
        is_dirty = is_dirty
            || tree
                .archived_representation
                .as_ref()
                .is_some_and(|(archived, archived_value)| {
                    artifact_is_dirty_with_direct_system_calls(
                        tsgm,
                        output_checker,
                        known_modified_output_files,
                        as_blaze_artifact(archived).expect("ArchivedTreeArtifact"),
                        archived_value,
                        modified_outputs_receiver,
                    )
                });

        if should_check_tree_artifact(sorted_known_modified_output_files, tree_artifact)
            && tree_artifact_is_dirty(tree_artifact, tree)
        {
            // Count the changed directory as one "file".
            modified_outputs_receiver.report_modified_output_file(
                get_best_effort_modified_time(tree_artifact),
                tree_artifact,
            );
            is_dirty = true;
        }
    }

    is_dirty
}

// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:637 FilesystemValueChecker.getBestEffortModifiedTime
fn get_best_effort_modified_time(artifact: &Artifact) -> i64 {
    let path = artifact.get_path();
    if !path.exists() {
        return -1;
    }
    // path.exists() ? path.getLastModifiedTime() : -1; an IOException logs and gives -1.
    path.get_last_modified_time().unwrap_or(-1)
}

// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:647 FilesystemValueChecker.shouldCheckFile
fn should_check_file(
    known_modified_output_files: Option<&IndexSet<PathFragment>>,
    artifact: &Artifact,
) -> bool {
    known_modified_output_files.is_none_or(|known| known.contains(artifact.get_exec_path()))
}

// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:653 FilesystemValueChecker.shouldCheckTreeArtifact
fn should_check_tree_artifact(
    known_modified_output_files: Option<&[PathFragment]>,
    tree_artifact: &Artifact,
) -> bool {
    // If null, everything needs to be checked.
    let Some(known_modified_output_files) = known_modified_output_files else {
        return true;
    };

    // Here we do the following to see whether a TreeArtifact is modified:
    // 1. Sort the set of modified file paths in lexicographical order using TreeSet.
    // 2. Get the first modified output file path that is greater than or equal to the exec path
    //    of the TreeArtifact to check.
    // 3. Check whether the returned file path contains the exec path of the TreeArtifact as a
    //    prefix path.
    let artifact_exec_path = tree_artifact.get_exec_path();
    let i = known_modified_output_files.partition_point(|p| p < artifact_exec_path);
    known_modified_output_files
        .get(i)
        .is_some_and(|head_path| head_path.starts_with(artifact_exec_path))
}

#[cfg(test)]
mod tests {
    //! The `getDirtyActionValues` tests of
    //! src/test/java/com/google/devtools/build/lib/skyframe/FilesystemValueCheckerTest.java (the
    //! output part of `FilesystemValueChecker`; the key part's tests are in blaze_sky_loading).
    //! Java injects the values into an evaluator and reads `evaluator.getValues()`; here the map
    //! is built directly (the evaluation has no function to run).
    //!
    //! Not ported yet: the tree artifact tests (`getDirtyActionValues_*TreeDirectory*`,
    //! `*TreeFile*`, `*ArchivedFile*`, `*everythingModified*`, `*ModifiedFileSet*`,
    //! `*nothingModified*`) and the remote metadata tests other than the materialization one.

    use std::any::Any;
    use std::fmt;
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::sync::Arc;

    use blaze_actions::actions::action_lookup::ActionLookupKey;
    use blaze_actions::{ActionLookupData, ActionLookupKeyRef, ArtifactRoot, RootType};
    use blaze_cmdline::Label;
    use blaze_exec::actions::action_cache_checker::{TrustAll, TrustLocalOnly};
    use blaze_exec::actions::file_artifact_value::FileContentsProxy;
    use blaze_util::profiler::Clock;
    use blaze_vfs::{DigestHashFunction, FileSystemExt, Path, UnixFileSystem};
    use indexmap::IndexMap;

    use super::*;

    const FSVC_THREADS_FOR_TEST: usize = 200;

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct Key(String);

    impl fmt::Display for Key {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "ConfiguredTargetKey{{label={}}}", self.0)
        }
    }

    impl ActionLookupKey for Key {
        fn get_label(&self) -> Option<Label> {
            Some(Label::parse_canonical(&self.0).unwrap())
        }
        fn get_configuration_checksum(&self) -> Option<String> {
            None
        }
        fn function_name(&self) -> &'static str {
            "CONFIGURED_TARGET"
        }
        fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
            other.as_any().downcast_ref::<Key>() == Some(self)
        }
        fn key_hash(&self) -> u64 {
            let mut h = DefaultHasher::new();
            self.hash(&mut h);
            h.finish()
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    /// `BlazeClock.instance()`.
    struct SystemClock;

    impl Clock for SystemClock {
        fn current_time_millis(&self) -> i64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
        }
        fn nano_time(&self) -> i64 {
            self.current_time_millis() * 1_000_000
        }
    }

    struct NoReceiver;

    impl ModifiedOutputsReceiver for NoReceiver {
        fn report_modified_output_file(&self, _: i64, _: &Artifact) {}
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        root: Path,
        action_lookup_key: ActionLookupKeyRef,
    }

    impl Fixture {
        fn new() -> Fixture {
            let dir = tempfile::tempdir().unwrap();
            let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
            let root = fs.get_path(dir.path().to_str().unwrap());
            Fixture {
                _dir: dir,
                root,
                action_lookup_key: ActionLookupKeyRef(Arc::new(Key("//foo:foo".to_string()))),
            }
        }

        // Java: src/test/java/com/google/devtools/build/lib/skyframe/FilesystemValueCheckerTest.java:1270 FilesystemValueCheckerTest.createDerivedArtifact
        fn create_derived_artifact(&self, rel_path: &str) -> Artifact {
            let output_path = self.root.get_child("bin");
            let _ = output_path.create_directory();
            let root = ArtifactRoot::as_derived_root(&self.root, RootType::OUTPUT, &["bin"]);
            let exec_path = root.get_exec_path().get_relative_str(rel_path);
            Artifact::create_derived_artifact(root, exec_path, self.action_lookup_key.clone())
        }

        fn action_key(&self, index: i32) -> SkyKey {
            SkyKey::new(ActionExecutionKey(ActionLookupData::create(
                self.action_lookup_key.clone(),
                index,
            )))
        }
    }

    fn write(path: &Path, content: &str) {
        std::fs::write(path.get_path_string(), content).unwrap();
    }

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/FilesystemValueCheckerTest.java:1347 FilesystemValueCheckerTest.actionValue
    fn action_value(outputs: &[&Artifact]) -> SkyValue {
        let mut artifact_data = IndexMap::new();
        for output in outputs {
            // fileArtifactValueFromArtifact(output, stat, NO_CACHE, null), then
            // createFromInjectedDigest(noDigest, path.getDigest()).
            let no_digest =
                crate::skyframe_action_executor::file_artifact_value_for_output_check(output, None)
                    .unwrap();
            let path = output.get_path();
            let with_digest = FileArtifactValue::create_for_normal_file(
                Some(path.get_digest().unwrap()),
                no_digest.get_contents_proxy(),
                no_digest.get_size(),
            );
            artifact_data.insert((*output).clone(), with_digest);
        }
        Arc::new(ActionExecutionValue::create(
            artifact_data,
            IndexMap::new(),
            None,
        ))
    }

    fn dirty(
        values: &HashMap<SkyKey, SkyValue>,
        modified: &ModifiedFileSet,
        output_checker: &dyn OutputChecker,
        tsgm: Option<&TimestampGranularityMonitor>,
    ) -> Vec<SkyKey> {
        get_dirty_action_values(
            values,
            modified,
            output_checker,
            &NoReceiver,
            tsgm,
            FSVC_THREADS_FOR_TEST,
        )
    }

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/FilesystemValueCheckerTest.java:734 FilesystemValueCheckerTest.checkDirtyActions
    fn check_dirty_actions() {
        let t = Fixture::new();
        let out1 = t.create_derived_artifact("fiz");
        let out2 = t.create_derived_artifact("pop");

        write(&out1.get_path(), "hello");
        write(&out2.get_path(), "fizzlepop");

        let tsgm = TimestampGranularityMonitor::new(Arc::new(SystemClock));
        let action_key1 = t.action_key(0);
        let action_key2 = t.action_key(1);

        let mut values = HashMap::new();
        pretend_build_two_artifacts(&mut values, &out1, &action_key1, &out2, &action_key2, &tsgm);

        // Change the file but not its size
        write(&out1.get_path(), "hallo");
        check_action_dirtied_by_file(&values, &out1, &action_key1, &tsgm);
        pretend_build_two_artifacts(&mut values, &out1, &action_key1, &out2, &action_key2, &tsgm);

        // Now try with a different size
        write(&out1.get_path(), "hallo2");
        check_action_dirtied_by_file(&values, &out1, &action_key1, &tsgm);
    }

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/FilesystemValueCheckerTest.java:757 FilesystemValueCheckerTest.pretendBuildTwoArtifacts
    fn pretend_build_two_artifacts(
        values: &mut HashMap<SkyKey, SkyValue>,
        out1: &Artifact,
        action_key1: &SkyKey,
        out2: &Artifact,
        action_key2: &SkyKey,
        tsgm: &TimestampGranularityMonitor,
    ) {
        tsgm.set_command_start_time();
        values.insert(action_key1.clone(), action_value(&[out1]));
        values.insert(action_key2.clone(), action_value(&[out2]));
        assert!(
            dirty(
                values,
                &ModifiedFileSet::EVERYTHING_MODIFIED,
                &TrustLocalOnly,
                None
            )
            .is_empty()
        );
        tsgm.wait_for_timestamp_granularity();
    }

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/FilesystemValueCheckerTest.java:804 FilesystemValueCheckerTest.checkActionDirtiedByFile
    fn check_action_dirtied_by_file(
        values: &HashMap<SkyKey, SkyValue>,
        file: &Artifact,
        action_key: &SkyKey,
        tsgm: &TimestampGranularityMonitor,
    ) {
        let tsgm = Some(tsgm);
        assert_eq!(
            dirty(
                values,
                &ModifiedFileSet::EVERYTHING_MODIFIED,
                &TrustLocalOnly,
                tsgm
            ),
            vec![action_key.clone()]
        );
        assert_eq!(
            dirty(
                values,
                &ModifiedFileSet::EVERYTHING_DELETED,
                &TrustLocalOnly,
                tsgm
            ),
            vec![action_key.clone()]
        );
        let single = ModifiedFileSet::builder()
            .modify(file.get_exec_path().clone())
            .build();
        assert_eq!(
            dirty(values, &single, &TrustLocalOnly, tsgm),
            vec![action_key.clone()]
        );
        let parent = ModifiedFileSet::builder()
            .modify(file.get_exec_path().get_parent_directory().unwrap())
            .build();
        assert!(dirty(values, &parent, &TrustLocalOnly, tsgm).is_empty());
        assert!(
            dirty(
                values,
                &ModifiedFileSet::nothing_modified(),
                &TrustLocalOnly,
                tsgm
            )
            .is_empty()
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/FilesystemValueCheckerTest.java:1287 FilesystemValueCheckerTest.testDirtyActions
    #[test]
    fn test_dirty_actions() {
        check_dirty_actions();
    }

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/FilesystemValueCheckerTest.java:1294 FilesystemValueCheckerTest.testDirtyActionsBatchStat
    #[test]
    #[ignore = "@Ignore(\"b/154337187\") in Java; BatchStat is not ported (Bazel's local output service has none)"]
    fn test_dirty_actions_batch_stat() {}

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/FilesystemValueCheckerTest.java:1314 FilesystemValueCheckerTest.testDirtyActionsBatchStatWithDigest
    #[test]
    #[ignore = "@Ignore(\"b/154337187\") in Java; BatchStat is not ported (Bazel's local output service has none)"]
    fn test_dirty_actions_batch_stat_with_digest() {}

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/FilesystemValueCheckerTest.java:1333 FilesystemValueCheckerTest.testDirtyActionsBatchStatFallback
    #[test]
    #[ignore = "@Ignore(\"b/154337187\") in Java; BatchStat is not ported (Bazel's local output service has none)"]
    fn test_dirty_actions_batch_stat_fallback() {}

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/FilesystemValueCheckerTest.java:1525 FilesystemValueCheckerTest.testRemoteArtifactMaterializedAsToplevelOutputThenDeleted
    #[test]
    fn test_remote_artifact_materialized_as_toplevel_output_then_deleted() {
        // Test that a remote artifact recorded as materialized in the local filesystem as a
        // top-level output invalidates the generating action when the local file is deleted, even
        // if remote metadata is otherwise trusted - but only once: the record is cleared so that a
        // reevaluation that doesn't rematerialize the file isn't invalidated again.
        let t = Fixture::new();
        let action_key = t.action_key(0);

        let out = t.create_derived_artifact("foo");
        // createRemoteMetadata("foo-content")
        let data = b"foo-content";
        let out_metadata = FileArtifactValue::create_for_remote_file_with_materialization_data(
            sha2_digest(data),
            data.len() as i64,
            -1,
            None,
            false,
        );
        let mut artifact_data = IndexMap::new();
        artifact_data.insert(out.clone(), out_metadata.clone());
        let mut values: HashMap<SkyKey, SkyValue> = HashMap::new();
        values.insert(
            action_key.clone(),
            Arc::new(ActionExecutionValue::create(
                artifact_data,
                IndexMap::new(),
                None,
            )),
        );

        // Materialize the file without reexecuting the action, then delete it.
        write(&out.get_path(), "foo-content");
        out_metadata.set_contents_proxy(FileContentsProxy::create(&out.get_path().stat().unwrap()));
        out_metadata.set_materialized_as_toplevel_output(true);
        out.get_path().delete().unwrap();

        assert_eq!(
            dirty(
                &values,
                &ModifiedFileSet::EVERYTHING_MODIFIED,
                &TrustAll,
                None
            ),
            vec![action_key]
        );

        // The materialization record has been cleared, so the still-missing file is trusted
        // again.
        assert!(
            dirty(
                &values,
                &ModifiedFileSet::EVERYTHING_MODIFIED,
                &TrustAll,
                None
            )
            .is_empty()
        );
    }

    fn sha2_digest(data: &[u8]) -> Vec<u8> {
        use sha2::Digest;
        sha2::Sha256::digest(data).to_vec()
    }
}
