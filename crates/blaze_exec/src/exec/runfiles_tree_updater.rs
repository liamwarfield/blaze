//! Port of `lib/exec/RunfilesTreeUpdater.java`.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use blaze_actions::actions::artifact::Artifact;
use blaze_actions::analysis::actions::symlink_tree_action::RunfileSymlinksMode;
use blaze_actions::analysis::runfiles_support::RunfilesSupport;
use blaze_vfs::{Path, PathFragment};
use parking_lot::{Condvar, Mutex};

use crate::actions::action_adapters::as_blaze_artifact;
use crate::actions::{ExecException, RunfilesTree};
use crate::exec::exec_command_environment::ExecCommandEnvironment;
use crate::exec::symlink_tree_helper::SymlinkTreeHelper;

/// The exceptions `updateRunfiles` declares: `ExecException`, `IOException` (and
/// `InterruptedException`, carried by [`ExecException::Interrupted`]).
///
/// PORT: both variants are cloneable so that every waiter on a shared attempt rethrows the
/// first attempt's exception, as `CompletableFuture.join` does.
#[derive(Debug, Clone)]
pub enum UpdateRunfilesError {
    Exec(ExecException),
    Io(Arc<io::Error>),
}

impl From<ExecException> for UpdateRunfilesError {
    fn from(e: ExecException) -> Self {
        UpdateRunfilesError::Exec(e)
    }
}

impl From<io::Error> for UpdateRunfilesError {
    fn from(e: io::Error) -> Self {
        UpdateRunfilesError::Io(Arc::new(e))
    }
}

/// The outcome of one attempt (`CompletableFuture<Void>`): `None` while running; a panic of the
/// first attempt is recorded as `Panicked` (Java: an unchecked exception rethrown by `join`).
#[derive(Debug, Clone)]
enum Outcome {
    Done(Result<(), UpdateRunfilesError>),
    Panicked,
}

#[derive(Default)]
struct TreeFuture {
    state: Mutex<Option<Outcome>>,
    done: Condvar,
}

impl TreeFuture {
    fn complete(&self, outcome: Outcome) {
        let mut state = self.state.lock();
        if state.is_none() {
            *state = Some(outcome);
        }
        self.done.notify_all();
    }

    fn join(&self) -> Outcome {
        let mut state = self.state.lock();
        loop {
            if let Some(outcome) = &*state {
                return outcome.clone();
            }
            self.done.wait(&mut state);
        }
    }
}

/// Completes the future as `Panicked` if the first attempt unwinds before completing it.
struct CompleteOnDrop<'a>(&'a TreeFuture);

impl Drop for CompleteOnDrop<'_> {
    fn drop(&mut self) {
        self.0.complete(Outcome::Panicked);
    }
}

/// Utility used in local execution to create a runfiles tree if `--nobuild_runfile_links` has
/// been specified.
///
/// It is safe to call [`RunfilesTreeUpdater::update_runfiles`] concurrently.
///
/// PORT: the `XattrProvider` is the file system's fast digest (`Path::get_fast_digest`), as in
/// `SpawnLogContext.computeDigest`'s port.
// Java: com/google/devtools/build/lib/exec/RunfilesTreeUpdater.java:46 RunfilesTreeUpdater
pub struct RunfilesTreeUpdater {
    exec_root: Path,
    /// Deduplicates multiple attempts to update the same runfiles tree.
    ///
    /// Attempts may occur concurrently, e.g. if multiple local actions have the same input.
    ///
    /// The presence of an entry in the map signifies that an earlier attempt to update the
    /// corresponding runfiles tree was started, and will (have) set the future upon completion.
    updated_trees: Mutex<HashMap<PathFragment, Arc<TreeFuture>>>,
}

impl RunfilesTreeUpdater {
    // Java: com/google/devtools/build/lib/exec/RunfilesTreeUpdater.java:61 RunfilesTreeUpdater.forCommandEnvironment
    pub fn for_command_environment(env: &ExecCommandEnvironment) -> RunfilesTreeUpdater {
        RunfilesTreeUpdater::new(env.exec_root.clone())
    }

    // Java: com/google/devtools/build/lib/exec/RunfilesTreeUpdater.java:65 RunfilesTreeUpdater.<init>
    pub fn new(exec_root: Path) -> RunfilesTreeUpdater {
        RunfilesTreeUpdater {
            exec_root,
            updated_trees: Mutex::new(HashMap::new()),
        }
    }

    /// Creates or updates input runfiles trees for a spawn.
    // Java: com/google/devtools/build/lib/exec/RunfilesTreeUpdater.java:71 RunfilesTreeUpdater.updateRunfiles
    pub fn update_runfiles(
        &self,
        runfiles_trees: &[Arc<dyn RunfilesTree>],
    ) -> Result<(), UpdateRunfilesError> {
        for tree in runfiles_trees {
            let runfiles_dir = tree.get_exec_path();
            if tree.is_build_runfile_links() {
                continue;
            }

            let fresh_future = Arc::new(TreeFuture::default());
            let prior_future = {
                let mut updated_trees = self.updated_trees.lock();
                match updated_trees.get(&runfiles_dir) {
                    Some(prior) => Some(prior.clone()),
                    None => {
                        updated_trees.insert(runfiles_dir.clone(), fresh_future.clone());
                        None
                    }
                }
            };

            match prior_future {
                None => {
                    // We are the first attempt; update the runfiles tree and mark the future
                    // complete.
                    let guard = CompleteOnDrop(&fresh_future);
                    let result = self.update_runfiles_tree(tree.as_ref());
                    fresh_future.complete(Outcome::Done(result.clone()));
                    drop(guard);
                    result?;
                }
                Some(prior_future) => {
                    // There was a previous attempt; wait for it to complete.
                    match prior_future.join() {
                        Outcome::Done(result) => result?,
                        Outcome::Panicked => panic!(
                            "CompletionException: the first attempt to update runfiles tree {} \
                             failed with an unchecked exception",
                            runfiles_dir
                        ),
                    }
                }
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/exec/RunfilesTreeUpdater.java:109 RunfilesTreeUpdater.updateRunfilesTree
    fn update_runfiles_tree(&self, tree: &dyn RunfilesTree) -> Result<(), UpdateRunfilesError> {
        let runfiles_dir = self.exec_root.get_relative(&tree.get_exec_path());
        let input_manifest =
            self.exec_root
                .get_relative(&RunfilesSupport::input_manifest_exec_path(
                    &tree.get_exec_path(),
                ));
        if !input_manifest.exists() {
            return Ok(());
        }
        let output_manifest =
            self.exec_root
                .get_relative(&RunfilesSupport::output_manifest_exec_path(
                    &tree.get_exec_path(),
                ));
        // Avoid rebuilding the runfiles directory if the manifest in it matches the input
        // manifest, implying the symlinks exist and are already up to date. If the output
        // manifest is a symbolic link, it is likely a symbolic link to the input manifest, so we
        // cannot trust it as an up-to-date check.
        // PORT: skipped, windows only (RunfilesTreeUpdater.isRunfilesDirectoryPopulated: on Linux the condition is
        // `OS.getCurrent() != OS.WINDOWS`, always true).
        let up_to_date = || -> io::Result<bool> {
            Ok(tree.get_symlinks_mode() == RunfileSymlinksMode::CREATE
                && !output_manifest.is_symbolic_link()
                && get_digest_with_manual_fallback(&output_manifest)?
                    == get_digest_with_manual_fallback(&input_manifest)?)
        };
        match up_to_date() {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            // Ignore it - we will just try to create runfiles directory.
            Err(_) => {}
        }

        if !runfiles_dir.exists() {
            runfiles_dir.create_directory_and_parents()?;
        }

        let helper = SymlinkTreeHelper::new(
            &input_manifest,
            &output_manifest,
            &runfiles_dir,
            tree.get_workspace_name(),
        );

        match tree.get_symlinks_mode() {
            RunfileSymlinksMode::CREATE => {
                helper.create_runfiles_symlinks(&to_artifact_mapping(tree))?;
                helper.link_manifest()?;
            }
            RunfileSymlinksMode::SKIP => helper.create_minimal_runfiles_directory()?,
        }
        Ok(())
    }
}

/// `tree.getMapping()` as the `SortedMap<PathFragment, Artifact>` `SymlinkTreeHelper` takes.
///
/// PORT: the execution layer's mapping holds `ActionInputRef`s; every runfiles tree of a build
/// holds `blaze_actions` artifacts.
fn to_artifact_mapping(tree: &dyn RunfilesTree) -> BTreeMap<PathFragment, Option<Artifact>> {
    tree.get_mapping()
        .into_iter()
        .map(|(path, input)| {
            let artifact = input.map(|input| match as_blaze_artifact(&input) {
                Some(artifact) => artifact.clone(),
                None => blaze_util::unported!(
                    "RunfilesTreeUpdater.updateRunfilesTree",
                    "runfiles entry {} is not a blaze_actions Artifact",
                    path
                ),
            });
            (path, artifact)
        })
        .collect()
}

/// `DigestUtils.getDigestWithManualFallback(path, xattrProvider)`, through the digest cache as
/// in Java.
// Java: com/google/devtools/build/lib/vfs/DigestUtils.java:127 DigestUtils.getDigestWithManualFallback
fn get_digest_with_manual_fallback(path: &Path) -> io::Result<Vec<u8>> {
    crate::actions::cache::digest_utils::DigestUtils::get_digest_with_manual_fallback(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_vfs::{DigestHashFunction, FileSystemExt, UnixFileSystem};

    // No JUnit test of RunfilesTreeUpdater exists at the pin (LocalSpawnRunnerTest and
    // StandaloneSpawnStrategyTest mock it). These check the behaviour of the Java code above.

    #[derive(Debug)]
    struct Tree {
        exec_path: PathFragment,
        build_runfile_links: bool,
        mode: RunfileSymlinksMode,
    }

    impl RunfilesTree for Tree {
        fn get_exec_path(&self) -> PathFragment {
            self.exec_path.clone()
        }
        fn get_mapping(&self) -> Vec<(PathFragment, Option<crate::actions::ActionInputRef>)> {
            vec![(PathFragment::create("_main/empty"), None)]
        }
        fn get_symlinks_mode(&self) -> RunfileSymlinksMode {
            self.mode
        }
        fn is_build_runfile_links(&self) -> bool {
            self.build_runfile_links
        }
        fn get_workspace_name(&self) -> String {
            "_main".to_string()
        }
        fn get_artifacts_at_canonical_locations_for_logging(
            &self,
        ) -> blaze_collect::NestedSet<crate::actions::ActionInputRef> {
            unreachable!()
        }
        fn get_symlinks_for_logging(
            &self,
        ) -> blaze_collect::NestedSet<crate::actions::SymlinkEntry> {
            unreachable!()
        }
        fn get_root_symlinks_for_logging(
            &self,
        ) -> blaze_collect::NestedSet<crate::actions::SymlinkEntry> {
            unreachable!()
        }
        fn get_empty_filenames_for_logging(&self) -> Vec<PathFragment> {
            unreachable!()
        }
        fn get_repo_mapping_manifest_for_logging(&self) -> Option<crate::actions::ActionInputRef> {
            unreachable!()
        }
    }

    fn exec_root() -> (tempfile::TempDir, Path) {
        let dir = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root = fs.get_path(dir.path().to_str().unwrap());
        (dir, root)
    }

    fn tree(build_runfile_links: bool, mode: RunfileSymlinksMode) -> Arc<dyn RunfilesTree> {
        Arc::new(Tree {
            exec_path: PathFragment::create("bin/tool.runfiles"),
            build_runfile_links,
            mode,
        })
    }

    #[test]
    fn build_runfile_links_tree_is_left_alone() {
        let (_dir, root) = exec_root();
        root.get_relative_str("bin").create_directory().unwrap();
        std::fs::write(
            (root.get_relative_str("bin/tool.runfiles_manifest")).get_path_string(),
            b"_main/empty \n",
        )
        .unwrap();
        let updater = RunfilesTreeUpdater::new(root.clone());
        updater
            .update_runfiles(&[tree(true, RunfileSymlinksMode::CREATE)])
            .unwrap();
        assert!(!root.get_relative_str("bin/tool.runfiles").exists());
    }

    #[test]
    fn missing_input_manifest_creates_nothing() {
        let (_dir, root) = exec_root();
        let updater = RunfilesTreeUpdater::new(root.clone());
        updater
            .update_runfiles(&[tree(false, RunfileSymlinksMode::CREATE)])
            .unwrap();
        assert!(!root.get_relative_str("bin/tool.runfiles").exists());
    }

    #[test]
    fn create_mode_builds_tree_and_links_manifest() {
        let (_dir, root) = exec_root();
        root.get_relative_str("bin").create_directory().unwrap();
        let input_manifest = root.get_relative_str("bin/tool.runfiles_manifest");
        std::fs::write((input_manifest).get_path_string(), b"_main/empty \n").unwrap();
        let updater = RunfilesTreeUpdater::new(root.clone());
        updater
            .update_runfiles(&[tree(false, RunfileSymlinksMode::CREATE)])
            .unwrap();
        let runfiles = root.get_relative_str("bin/tool.runfiles");
        assert!(runfiles.get_relative_str("_main/empty").is_file());
        let manifest = runfiles.get_relative_str("MANIFEST");
        assert!(manifest.is_symbolic_link());
        assert_eq!(
            manifest.read_symbolic_link().unwrap(),
            *input_manifest.as_fragment()
        );
    }

    #[test]
    fn skip_mode_creates_minimal_directory() {
        let (_dir, root) = exec_root();
        root.get_relative_str("bin").create_directory().unwrap();
        std::fs::write(
            (root.get_relative_str("bin/tool.runfiles_manifest")).get_path_string(),
            b"_main/empty \n",
        )
        .unwrap();
        let updater = RunfilesTreeUpdater::new(root.clone());
        updater
            .update_runfiles(&[tree(false, RunfileSymlinksMode::SKIP)])
            .unwrap();
        let runfiles = root.get_relative_str("bin/tool.runfiles");
        assert!(runfiles.get_relative_str("_main").is_directory());
        assert!(!runfiles.get_relative_str("_main/empty").exists());
        assert!(runfiles.get_relative_str("MANIFEST").is_symbolic_link());
    }
}
