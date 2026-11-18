//! Port of `lib/exec/SymlinkTreeStrategy.java`, plus the `OutputService` symlink-tree subset it
//! reads (`lib/vfs/OutputService.java`, `lib/vfs/LocalOutputService.java`).

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_actions::actions::action::Action;
use blaze_actions::actions::artifact::Artifact;
use blaze_actions::analysis::actions::symlink_tree_action::{
    RunfileSymlinksMode, SymlinkTreeAction,
};
use blaze_vfs::PathFragment;

use crate::actions::action_adapters::artifact_input;
use crate::actions::{ActionExecutionException, ExecException};
use crate::analysis::actions::symlink_tree_action::SymlinkTreeActionContext;
use crate::analysis::actions::{get_input_path, post_running_action_event};
use crate::exec::action_execution_context::ActionExecutionContext;
use crate::exec::symlink_tree_helper::SymlinkTreeHelper;

/// The symlink-tree part of `OutputService`.
// Java: com/google/devtools/build/lib/vfs/OutputService.java:56 OutputService
pub trait OutputService: Send + Sync {
    /// Returns true iff `createSymlinkTree` is available.
    // Java: com/google/devtools/build/lib/vfs/OutputService.java:175 OutputService.canCreateSymlinkTree
    fn can_create_symlink_tree(&self) -> bool;

    /// Creates a symlink tree.
    ///
    /// `symlinks`: map from `symlinkTreeRoot`-relative path to symlink target; `None` values
    /// create empty files. `symlink_tree_root`: the symlink tree root, relative to the execroot.
    // Java: com/google/devtools/build/lib/vfs/OutputService.java:189 OutputService.createSymlinkTree
    fn create_symlink_tree(
        &self,
        symlinks: &BTreeMap<PathFragment, Option<PathFragment>>,
        symlink_tree_root: &PathFragment,
    ) -> Result<(), ExecException>;
}

/// Bazel's default output service (no remote output service module).
// Java: com/google/devtools/build/lib/vfs/LocalOutputService.java:35 LocalOutputService
#[derive(Debug, Default)]
pub struct LocalOutputService;

impl OutputService for LocalOutputService {
    // Java: com/google/devtools/build/lib/vfs/LocalOutputService.java:101 LocalOutputService.canCreateSymlinkTree
    fn can_create_symlink_tree(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/vfs/LocalOutputService.java:106 LocalOutputService.createSymlinkTree
    fn create_symlink_tree(
        &self,
        _symlinks: &BTreeMap<PathFragment, Option<PathFragment>>,
        _symlink_tree_root: &PathFragment,
    ) -> Result<(), ExecException> {
        panic!("UnsupportedOperationException")
    }
}

/// Implements SymlinkTreeAction by using the output service or by running an embedded script to
/// create the symlink tree.
// Java: com/google/devtools/build/lib/exec/SymlinkTreeStrategy.java:42 SymlinkTreeStrategy
pub struct SymlinkTreeStrategy {
    output_service: Arc<dyn OutputService>,
    workspace_name: String,
}

/// `SymlinkTreeStrategy.TO_PATH`: `artifact == null ? null : artifact.getPath().asFragment()`.
// Java: com/google/devtools/build/lib/exec/SymlinkTreeStrategy.java:46 SymlinkTreeStrategy.TO_PATH
pub fn to_path(artifact: Option<&Artifact>) -> Option<PathFragment> {
    artifact.map(|a| a.get_path().as_fragment().clone())
}

impl SymlinkTreeStrategy {
    // Java: com/google/devtools/build/lib/exec/SymlinkTreeStrategy.java:52 SymlinkTreeStrategy.<init>
    pub fn new(output_service: Arc<dyn OutputService>, workspace_name: String) -> Self {
        SymlinkTreeStrategy {
            output_service,
            workspace_name,
        }
    }

    // Java: com/google/devtools/build/lib/exec/SymlinkTreeStrategy.java:100 SymlinkTreeStrategy.getFilesetMap
    fn get_fileset_map(
        action: &SymlinkTreeAction,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> BTreeMap<PathFragment, PathFragment> {
        let fileset = action_execution_context
            .get_input_metadata_provider()
            .get_fileset(&artifact_input(action.get_input_manifest()))
            .unwrap_or_else(|| {
                panic!(
                    "NullPointerException: no fileset for {}",
                    action.get_input_manifest()
                )
            });
        SymlinkTreeHelper::process_fileset_links(
            &fileset.symlinks,
            action.get_workspace_name_for_fileset(),
        )
    }

    // Java: com/google/devtools/build/lib/exec/SymlinkTreeStrategy.java:110 SymlinkTreeStrategy.getRunfilesMap
    fn get_runfiles_map(action: &SymlinkTreeAction) -> BTreeMap<PathFragment, Option<Artifact>> {
        // This call outputs warnings about overlapping symlinks. However, since this has already
        // been called by the SourceManifestAction, we silence the warnings here.
        action
            .get_runfiles()
            .expect("runfiles tree has runfiles")
            .get_runfiles_inputs(action.get_repo_mapping_manifest())
    }

    // Java: com/google/devtools/build/lib/exec/SymlinkTreeStrategy.java:116 SymlinkTreeStrategy.createSymlinkTreeHelper
    fn create_symlink_tree_helper(
        &self,
        action: &SymlinkTreeAction,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> SymlinkTreeHelper {
        let output_manifest =
            get_input_path(action_execution_context, action.get_output_manifest());
        SymlinkTreeHelper::new(
            &get_input_path(action_execution_context, action.get_input_manifest()),
            &output_manifest,
            &output_manifest
                .get_parent_directory()
                .expect("MANIFEST has a parent directory"),
            self.workspace_name.clone(),
        )
    }

    fn create_symlinks_inner(
        &self,
        action: &SymlinkTreeAction,
        action_execution_context: &dyn ActionExecutionContext,
        helper: &SymlinkTreeHelper,
    ) -> Result<(), ExecException> {
        // Note that the output manifest must always be created last, as its presence ascertains
        // that the runfiles tree has been updated (only the output manifest is an action output,
        // so Skyframe cannot invalidate the symlink tree).
        if self.output_service.can_create_symlink_tree() {
            let symlinks: BTreeMap<PathFragment, Option<PathFragment>> = if action.is_fileset_tree()
            {
                Self::get_fileset_map(action, action_execution_context)
                    .into_iter()
                    .map(|(k, v)| (k, Some(v)))
                    .collect()
            } else {
                // TODO(tjgq): This produces an incorrect path for unresolved symlinks, which
                // should be created textually.
                Self::get_runfiles_map(action)
                    .into_iter()
                    .map(|(k, v)| (k, to_path(v.as_ref())))
                    .collect()
            };
            self.output_service.create_symlink_tree(
                &symlinks,
                &action
                    .get_output_manifest()
                    .get_exec_path()
                    .get_parent_directory()
                    .expect("MANIFEST has a parent directory"),
            )?;
            helper.link_manifest()?;
        } else if action.get_runfile_symlinks_mode() == RunfileSymlinksMode::SKIP {
            // Clear the runfiles directory, then create just the output manifest and the
            // workspace subdirectory. This is required because only the output manifest is
            // considered an action output, so if the previous invocation created a symlink tree,
            // Skyframe will not clear it for us.
            helper.create_minimal_runfiles_directory()?;
        } else {
            if action.is_fileset_tree() {
                helper.create_fileset_symlinks(
                    &Self::get_fileset_map(action, action_execution_context)
                        .into_iter()
                        .map(|(k, v)| (k, Some(v)))
                        .collect(),
                )?;
            } else {
                helper.create_runfiles_symlinks(&Self::get_runfiles_map(action))?;
            }
            helper.link_manifest()?;
        }
        Ok(())
    }
}

impl SymlinkTreeActionContext for SymlinkTreeStrategy {
    // Java: com/google/devtools/build/lib/exec/SymlinkTreeStrategy.java:58 SymlinkTreeStrategy.createSymlinks
    fn create_symlinks(
        &self,
        action: &SymlinkTreeAction,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<(), ActionExecutionException> {
        post_running_action_event(action_execution_context, action, "local");
        // PORT: skipped, profiling (GoogleAutoProfilerUtils.logged).
        let helper = self.create_symlink_tree_helper(action, action_execution_context);
        // TODO(tjgq): Respect RunfileSymlinksMode.SKIP even in the presence of an OutputService.
        self.create_symlinks_inner(action, action_execution_context, &helper)
            .map_err(|e| {
                ActionExecutionException::from_exec_exception(&e, None, action as &dyn Action)
            })
    }
}
