//! Port of `lib/analysis/starlark/UnresolvedSymlinkAction.java` (analysis-time parts) and
//! `lib/vfs/SymlinkTargetType.java`.

use std::any::Any;

use blaze_collect::{NestedSetBuilder, Order};
use blaze_util::{Fingerprint, check_argument};
use blaze_vfs::PathFragment;

use crate::actions::action::{AbstractAction, Action, ComputeKeyError, InputMetadataProvider};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;

// Java: com/google/devtools/build/lib/analysis/starlark/UnresolvedSymlinkAction.java:48 UnresolvedSymlinkAction.GUID
const GUID: &str = "0f302651-602c-404b-881c-58913193cfe7";

/// Indicates the file type at the other end of a symlink.
///
/// Required by some filesystems (notably on Windows) to correctly create a symlink when its
/// target does not yet exist, as a different kind of filesystem object might be required
/// depending on the target type.
///
/// PORT: Java's enum lives in `lib/vfs`; blaze_vfs does not port it (it only matters on Windows
/// there), but its `name()` is part of this action's key, so it is ported here.
// PORT: skipped, serialization for remote analysis caching (not ported, ruling 2026-11-19 (1)): SymlinkTargetType.Codec.<init>
// Java: com/google/devtools/build/lib/vfs/SymlinkTargetType.java:26 SymlinkTargetType
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymlinkTargetType {
    /// The target is of unspecified type.
    UNSPECIFIED,
    /// The target is a regular file.
    FILE,
    /// The target is a directory.
    DIRECTORY,
}

impl SymlinkTargetType {
    /// Java `Enum.name()`.
    pub fn name(self) -> &'static str {
        match self {
            SymlinkTargetType::UNSPECIFIED => "UNSPECIFIED",
            SymlinkTargetType::FILE => "FILE",
            SymlinkTargetType::DIRECTORY => "DIRECTORY",
        }
    }
}

/// Action to create a possibly unresolved symbolic link to a raw path.
///
/// To create a symlink to a known-to-exist target with alias semantics similar to a true copy of
/// the input, use `SymlinkAction` instead.
// Java: com/google/devtools/build/lib/analysis/starlark/UnresolvedSymlinkAction.java:47 UnresolvedSymlinkAction
#[derive(Debug)]
pub struct UnresolvedSymlinkAction {
    base: AbstractAction,
    target: String,
    target_type: SymlinkTargetType,
    progress_message: String,
}

impl UnresolvedSymlinkAction {
    // Java: com/google/devtools/build/lib/analysis/starlark/UnresolvedSymlinkAction.java:66 UnresolvedSymlinkAction.create
    pub fn create(
        owner: ActionOwner,
        primary_output: Artifact,
        target: impl Into<String>,
        target_type: SymlinkTargetType,
        progress_message: impl Into<String>,
    ) -> UnresolvedSymlinkAction {
        check_argument!(primary_output.is_symlink());
        // Java: com/google/devtools/build/lib/analysis/starlark/UnresolvedSymlinkAction.java:54 UnresolvedSymlinkAction.<init>
        UnresolvedSymlinkAction {
            base: AbstractAction::new(
                owner,
                NestedSetBuilder::empty_set(Order::STABLE_ORDER),
                [primary_output],
            ),
            target: target.into(),
            target_type,
            progress_message: progress_message.into(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/UnresolvedSymlinkAction.java:120 UnresolvedSymlinkAction.getTarget
    pub fn get_target(&self) -> String {
        self.get_target_path_fragment()
            .get_path_string()
            .to_string()
    }

    /// The `target` field as given (Java's private field, which `execute` prints in its error).
    pub fn get_raw_target(&self) -> &str {
        &self.target
    }

    /// The symlink target type.
    pub fn get_target_type(&self) -> SymlinkTargetType {
        self.target_type
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/UnresolvedSymlinkAction.java:124 UnresolvedSymlinkAction.getTargetPathFragment
    fn get_target_path_fragment(&self) -> PathFragment {
        // TODO: PathFragment#create normalizes the symlink target, which may change how it
        //  resolves when combined with directory symlinks. Ideally, Bazel's file system
        //  abstraction would offer a way to create symlinks without any preprocessing of the
        //  target.
        PathFragment::create(&self.target)
    }
}

impl Action for UnresolvedSymlinkAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/UnresolvedSymlinkAction.java:111 UnresolvedSymlinkAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        "UnresolvedSymlink".to_string()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/UnresolvedSymlinkAction.java:96 UnresolvedSymlinkAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(GUID);
        // Invariant: the target is a Starlark string, in internal form (see `fingerprint_util`).
        fp.add_string(&self.target);
        fp.add_string(self.target_type.name());
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/UnresolvedSymlinkAction.java:106 UnresolvedSymlinkAction.describeKey
    fn describe_key(&self) -> Option<String> {
        Some(format!(
            "GUID: {}\ntarget: {}\ntype: {}\n",
            GUID,
            self.target,
            self.target_type.name()
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/UnresolvedSymlinkAction.java:116 UnresolvedSymlinkAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        Some(self.progress_message.clone())
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/UnresolvedSymlinkAction.java:120 UnresolvedSymlinkAction.getTarget
    fn aquery_unresolved_symlink_target(&self) -> Option<String> {
        Some(self.get_target())
    }
}
