//! Port of `lib/analysis/actions/SymlinkTreeAction.java`, plus
//! `BuildConfigurationValue.RunfileSymlinksMode`.

use std::any::Any;
use std::sync::Arc;

use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_util::{Fingerprint, check_argument};

use crate::actions::action::{AbstractAction, Action, ComputeKeyError, InputMetadataProvider};
use crate::actions::action_environment::ActionEnvironment;
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;
use crate::actions::fingerprint_util::FingerprintPathExt;
use crate::analysis::runfiles::Runfiles;

/// Whether runfile symlinks are created.
// Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:861 BuildConfigurationValue.RunfileSymlinksMode
// PORT: blaze_build_config has its own copy of this enum (`build_configuration_value.rs`);
// blaze_actions cannot depend on it, so the analysis lead converts between the two.
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RunfileSymlinksMode {
    SKIP,
    CREATE,
}

impl RunfileSymlinksMode {
    /// All values in Java declaration order (`values()`).
    pub const VALUES: [RunfileSymlinksMode; 2] =
        [RunfileSymlinksMode::SKIP, RunfileSymlinksMode::CREATE];

    /// Java `Enum.ordinal`.
    pub fn ordinal(self) -> i32 {
        match self {
            RunfileSymlinksMode::SKIP => 0,
            RunfileSymlinksMode::CREATE => 1,
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:50 SymlinkTreeAction.GUID
const GUID: &str = "7a16371c-cd4a-494d-b622-963cd89f5212";

/// Action responsible for the symlink tree creation. Used to generate runfiles and fileset
/// symlink trees.
// Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:48 SymlinkTreeAction
#[derive(Debug)]
pub struct SymlinkTreeAction {
    base: AbstractAction,
    input_manifest: Artifact,
    output_manifest: Artifact,
    env: Arc<ActionEnvironment>,
    runfile_symlinks_mode: RunfileSymlinksMode,
    repo_mapping_manifest: Option<Artifact>,

    // Exactly one of these two fields is non-null.
    runfiles: Option<Runfiles>,
    workspace_name_for_fileset: Option<String>,
}

impl SymlinkTreeAction {
    /// Creates SymlinkTreeAction instance.
    ///
    /// - `owner`: action owner
    /// - `input_manifest`: the input runfiles manifest
    /// - `runfiles`: the input runfiles
    /// - `output_manifest`: the generated symlink tree manifest (must have "MANIFEST" base name).
    ///   Symlink tree root will be set to the artifact's parent directory.
    /// - `repo_mapping_manifest`: the repository mapping manifest
    /// - `env`, `runfile_symlinks_mode`, `workspace_name`: Java's other constructor reads these
    ///   from the `BuildConfigurationValue` (`getActionEnvironment()`,
    ///   `getRunfileSymlinksMode()`, `getWorkspaceName()`); callers pass them here.
    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:73 SymlinkTreeAction.<init>
    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:107 SymlinkTreeAction.<init>
    pub fn new(
        owner: ActionOwner,
        input_manifest: Artifact,
        runfiles: Option<Runfiles>,
        output_manifest: Artifact,
        repo_mapping_manifest: Option<Artifact>,
        env: ActionEnvironment,
        runfile_symlinks_mode: RunfileSymlinksMode,
        workspace_name: String,
    ) -> SymlinkTreeAction {
        let base = AbstractAction::new(
            owner,
            SymlinkTreeAction::compute_inputs(
                runfile_symlinks_mode,
                runfiles.as_ref(),
                &input_manifest,
                repo_mapping_manifest.as_ref(),
            ),
            [output_manifest.clone()],
        );
        check_argument!(
            output_manifest.get_exec_path().get_base_name() == "MANIFEST",
            "{}",
            output_manifest
        );
        let (runfiles, workspace_name_for_fileset) = if input_manifest.is_fileset() {
            check_argument!(
                runfiles.is_none(),
                "Runfiles present for fileset {}",
                input_manifest
            );
            (None, Some(workspace_name))
        } else {
            match runfiles {
                Some(runfiles) => (Some(runfiles), None),
                None => panic!("NullPointerException"),
            }
        };
        SymlinkTreeAction {
            base,
            input_manifest,
            output_manifest,
            env: Arc::new(env),
            runfile_symlinks_mode,
            repo_mapping_manifest,
            runfiles,
            workspace_name_for_fileset,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:135 SymlinkTreeAction.computeInputs
    fn compute_inputs(
        runfile_symlinks_mode: RunfileSymlinksMode,
        runfiles: Option<&Runfiles>,
        input_manifest: &Artifact,
        repo_mapping_manifest: Option<&Artifact>,
    ) -> NestedSet<Artifact> {
        let mut inputs: NestedSetBuilder<Artifact> = NestedSetBuilder::stable_order();
        inputs.add(input_manifest.clone());
        // On Windows, we need to know whether the target artifact is a file or a directory in
        // order to correctly create a symlink or junction to it.
        // PORT: skipped, Windows only (adds `runfiles.getAllArtifacts()` and the repo mapping
        // manifest when the mode is CREATE and the OS is Windows).
        let _ = (runfile_symlinks_mode, runfiles, repo_mapping_manifest);
        inputs.build()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:159 SymlinkTreeAction.getInputManifest
    pub fn get_input_manifest(&self) -> &Artifact {
        &self.input_manifest
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:164 SymlinkTreeAction.getRunfiles
    pub fn get_runfiles(&self) -> Option<&Runfiles> {
        self.runfiles.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:168 SymlinkTreeAction.getOutputManifest
    pub fn get_output_manifest(&self) -> &Artifact {
        &self.output_manifest
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:173 SymlinkTreeAction.getRepoMappingManifest
    pub fn get_repo_mapping_manifest(&self) -> Option<&Artifact> {
        self.repo_mapping_manifest.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:177 SymlinkTreeAction.isFilesetTree
    pub fn is_fileset_tree(&self) -> bool {
        self.workspace_name_for_fileset.is_some()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:181 SymlinkTreeAction.getWorkspaceNameForFileset
    pub fn get_workspace_name_for_fileset(&self) -> &str {
        match &self.workspace_name_for_fileset {
            Some(name) => name,
            None => panic!(
                "NullPointerException: Not a fileset tree: {}",
                self.output_manifest
            ),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:185 SymlinkTreeAction.getRunfileSymlinksMode
    pub fn get_runfile_symlinks_mode(&self) -> RunfileSymlinksMode {
        self.runfile_symlinks_mode
    }
}

impl Action for SymlinkTreeAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:251 SymlinkTreeAction.mayInsensitivelyPropagateInputs
    fn may_insensitively_propagate_inputs(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:190 SymlinkTreeAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        "SymlinkTree".to_string()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:155 SymlinkTreeAction.getEnvironment
    fn get_environment(&self) -> Arc<ActionEnvironment> {
        self.env.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:195 SymlinkTreeAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        Some(format!(
            "{}{}",
            if self.is_fileset_tree() {
                "Creating Fileset tree "
            } else {
                "Creating runfiles tree "
            },
            self.output_manifest
                .get_exec_path()
                .get_parent_directory()
                .expect("NullPointerException")
                .get_path_string()
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:201 SymlinkTreeAction.computeKey
    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(GUID);
        fp.add_nullable_string(self.workspace_name_for_fileset.as_deref());
        fp.add_int(self.runfile_symlinks_mode.ordinal());
        self.env.add_to(fp);
        // We need to ensure that the fingerprints for two different instances of this action
        // are different. Consider the hypothetical scenario where we add a second runfiles
        // object to this class, which could also be null: the sequence
        //    if (r1 != null) r1.fingerprint(fp);
        //    if (r2 != null) r2.fingerprint(fp);
        // would *not* be safe; we'd get a collision between an action that has only r1 set, and
        // another that has only r2 set. Prefixing with a boolean indicating the presence of
        // runfiles makes it safe to add more fields in the future.
        fp.add_boolean(self.runfiles.is_some());
        if let Some(runfiles) = &self.runfiles {
            runfiles.fingerprint(
                action_key_context,
                fp,
                /* digest_absolute_paths= */ true,
            );
        }
        fp.add_boolean(self.repo_mapping_manifest.is_some());
        if let Some(repo_mapping_manifest) = &self.repo_mapping_manifest {
            fp.add_path_fragment(repo_mapping_manifest.get_exec_path());
        }
        Ok(())
    }
}
