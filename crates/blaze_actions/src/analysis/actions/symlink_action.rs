//! Port of `lib/analysis/actions/SymlinkAction.java` (analysis-time parts).

use std::any::Any;

use blaze_collect::{NestedSetBuilder, Order};
use blaze_util::{Fingerprint, check_state};
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::actions::action::{AbstractAction, Action, ComputeKeyError, InputMetadataProvider};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;
use crate::actions::fingerprint_util::FingerprintPathExt;
use crate::actions::platform_info::{PlatformInfoRef, empty_platform_info};

// Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:61 SymlinkAction.GUID
const GUID: &str = "7f4fab4d-d0a7-4f0f-8649-1d0337a21fee";

// Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:68 SymlinkAction.TargetType
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetType {
    /// The symlink points into a Fileset.
    ///
    /// If this is set, the action also updates the mtime for its target thus forcing actions
    /// depending on it to be re-executed. This would not be necessary in an ideal world, but
    /// dependency checking for Filesets output trees is unsound because they are directories, so
    /// we need to force them to be considered changed this way. Yet Another Reason why Filests
    /// should go away.
    Fileset,
    /// The symlink should point to an executable.
    ///
    /// Blaze will verify that the target is indeed executable.
    Executable,
    /// Just a vanilla symlink. Don't do anything else other than creating the symlink.
    Other,
}

/// Action to create a symlink to a known-to-exist target with alias semantics similar to a true
/// copy of the input (if any).
// Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:60 SymlinkAction
#[derive(Debug)]
pub struct SymlinkAction {
    base: AbstractAction,
    /// `None` when `get_primary_input` is the target of the symlink.
    input_path: Option<PathFragment>,
    progress_message: Option<String>,
    target_type: TargetType,
}

impl SymlinkAction {
    /// `toArtifact(owner, input, output, progressMessage)`.
    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:93 SymlinkAction.toArtifact
    pub fn to_artifact(
        owner: ActionOwner,
        input: Artifact,
        output: Artifact,
        progress_message: Option<String>,
    ) -> SymlinkAction {
        SymlinkAction::to_artifact_with_exec_root(
            owner,
            input,
            output,
            progress_message,
            /* use_exec_root_for_source= */ false,
        )
    }

    /// Creates an action that creates a symlink pointing to an artifact.
    ///
    /// `use_exec_root_for_source`: whether to link source artifacts to exec root as opposed to
    /// the artifact itself. This indirection makes sure that the symlink is always in sync with
    /// exec root, which could go out of sync with it when re-planting the symlink tree and the
    /// header was unchanged.
    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:110 SymlinkAction.toArtifact
    pub fn to_artifact_with_exec_root(
        owner: ActionOwner,
        input: Artifact,
        output: Artifact,
        progress_message: Option<String>,
        use_exec_root_for_source: bool,
    ) -> SymlinkAction {
        let input_path = if use_exec_root_for_source && input.is_source_artifact() {
            Some(input.get_exec_path().clone())
        } else {
            None
        };
        SymlinkAction::new(
            owner,
            input_path,
            Some(input),
            output,
            progress_message,
            TargetType::Other,
        )
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:125 SymlinkAction.toExecutable
    pub fn to_executable(
        owner: ActionOwner,
        input: Artifact,
        output: Artifact,
        progress_message: Option<String>,
    ) -> SymlinkAction {
        SymlinkAction::new(
            owner,
            None,
            Some(input),
            output,
            progress_message,
            TargetType::Executable,
        )
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:130 SymlinkAction.<init>
    fn new(
        owner: ActionOwner,
        input_path: Option<PathFragment>,
        primary_input: Option<Artifact>,
        primary_output: Artifact,
        progress_message: Option<String>,
        target_type: TargetType,
    ) -> SymlinkAction {
        let inputs = match primary_input {
            Some(i) => NestedSetBuilder::create(Order::STABLE_ORDER, [i]),
            None => NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        };
        SymlinkAction {
            base: AbstractAction::new(owner, inputs, [primary_output]),
            input_path,
            progress_message,
            target_type,
        }
    }

    /// Creates a symlink to a Fileset.
    ///
    /// This is different from a regular `SymlinkAction` in that the target is in the output tree
    /// but not an artifact and that when running this action, the mtime of its target is updated
    /// (necessary because dependency checking of Filesets is unsound).
    ///
    /// WARNING: Do not use this for anything else other than Filesets. If you do, your
    /// correctness will depend on a subtle interaction between various parts of Blaze.
    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:165 SymlinkAction.toFileset
    pub fn to_fileset(
        owner: ActionOwner,
        exec_path: PathFragment,
        primary_input: Artifact,
        primary_output: Artifact,
        progress_message: Option<String>,
    ) -> SymlinkAction {
        check_state!(!exec_path.is_absolute());
        SymlinkAction::new(
            owner,
            Some(exec_path),
            Some(primary_input),
            primary_output,
            progress_message,
            TargetType::Fileset,
        )
    }

    /// Creates a new SymlinkAction instance, where an input artifact is not present. This is
    /// useful when dealing with special cases where input paths that are outside the exec root
    /// directory tree. Currently, the only instance where this happens is for FDO builds where the
    /// profile file is outside the exec root structure.
    ///
    /// Deprecated: this action is not hermetic.
    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:194 SymlinkAction.toAbsolutePath
    pub fn to_absolute_path(
        owner: ActionOwner,
        absolute_path: PathFragment,
        output: Artifact,
        progress_message: Option<String>,
    ) -> SymlinkAction {
        check_state!(absolute_path.is_absolute());
        SymlinkAction::new(
            owner,
            Some(absolute_path),
            None,
            output,
            progress_message,
            TargetType::Other,
        )
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:200 SymlinkAction.getInputPath
    pub fn get_input_path(&self) -> PathFragment {
        match &self.input_path {
            Some(p) => p.clone(),
            None => self
                .get_primary_input()
                .expect("SymlinkAction without input path has a primary input")
                .get_exec_path()
                .clone(),
        }
    }

    /// Whether this symlink points into a Fileset (`targetType == TargetType.FILESET`).
    pub fn is_fileset(&self) -> bool {
        self.target_type == TargetType::Fileset
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:471 SymlinkAction.mayInsensitivelyPropagateInputs
    pub fn may_insensitively_propagate_inputs(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:417 SymlinkAction.printInputs
    #[allow(dead_code)]
    fn print_inputs(&self) -> String {
        let inputs = self.get_inputs();
        if inputs.is_empty() {
            self.input_path
                .as_ref()
                .map(|p| p.get_path_string().to_string())
                .unwrap_or_else(|| "null".to_string())
        } else if inputs.is_singleton() {
            self.get_primary_input()
                .map(|i| i.get_exec_path_string().to_string())
                .unwrap_or_default()
        } else {
            panic!(
                "IllegalStateException: Inputs unexpectedly contains more than 1 element: {}",
                inputs
            )
        }
    }
}

impl Action for SymlinkAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:447 SymlinkAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        if self.target_type == TargetType::Executable {
            "ExecutableSymlink".to_string()
        } else {
            "Symlink".to_string()
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:429 SymlinkAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(GUID);
        // We don't normally need to add inputs to the key. In this case, however, the inputPath
        // can be different from the actual input artifact.
        if let Some(input_path) = &self.input_path {
            fp.add_path_fragment(input_path);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:442 SymlinkAction.describeKey
    fn describe_key(&self) -> Option<String> {
        Some(format!(
            "GUID: {}\ninputPath: {}\n",
            GUID,
            match &self.input_path {
                Some(p) => p.get_path_string().to_string(),
                None => "null".to_string(),
            }
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:452 SymlinkAction.isVolatile
    fn is_volatile(&self) -> bool {
        self.input_path.as_ref().is_some_and(|p| p.is_absolute())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:457 SymlinkAction.executeUnconditionally
    fn execute_unconditionally(&self) -> bool {
        // If the SymlinkAction points to an absolute path, we can't verify that its output
        // artifact did not change purely by looking at the output tree. Thus, we re-execute the
        // action just to be safe. Change pruning will take care of not re-running dependent
        // actions and this is used only in very rare cases (only C++ FDO and even then, only twice
        // per build at most) anyway.
        self.input_path.as_ref().is_some_and(|p| p.is_absolute())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:466 SymlinkAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        self.progress_message.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:484 SymlinkAction.getExecutionPlatform
    fn get_execution_platform(&self) -> Option<PlatformInfoRef> {
        Some(empty_platform_info())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:489 SymlinkAction.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        // SymlinkAction is platform agnostic.
        IndexMap::new()
    }
}
