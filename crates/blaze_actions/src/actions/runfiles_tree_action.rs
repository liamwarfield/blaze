//! Port of `lib/actions/RunfilesTreeAction.java` (analysis-time parts; `execute`,
//! `prepare` and `reconstructRichDataOnActionCacheHit` belong to the execution layer).

use std::any::Any;

use blaze_cmdline::Label;
use blaze_collect::NestedSet;
use blaze_util::{Fingerprint, check_argument};
use indexmap::IndexMap;

use crate::actions::action::{
    AbstractAction, Action, ComputeKeyError, InputMetadataProvider, java_list_to_string,
};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;
use crate::actions::platform_info::{PlatformInfoRef, empty_platform_info};
use crate::actions::runfiles_tree::RunfilesTreeRef;

/// `RunfilesTreeAction.MNEMONIC`.
// Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:39 RunfilesTreeAction.MNEMONIC
pub const MNEMONIC: &str = "RunfilesTree";

/// An action that depends on a set of inputs and creates a single output file whenever it runs.
/// This is useful for bundling up a bunch of dependencies that are shared between individual
/// targets in the action graph; for example generated header files.
// Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:38 RunfilesTreeAction
#[derive(Debug)]
pub struct RunfilesTreeAction {
    base: AbstractAction,
    /// The runfiles tree created by this action.
    runfiles_tree: RunfilesTreeRef,
}

impl RunfilesTreeAction {
    // Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:44 RunfilesTreeAction.<init>
    pub fn new(
        owner: ActionOwner,
        runfiles_tree: RunfilesTreeRef,
        inputs: NestedSet<Artifact>,
        outputs: Vec<Artifact>,
    ) -> RunfilesTreeAction {
        let base = AbstractAction::new(owner, inputs, outputs.iter().cloned());
        // Java: Iterables.getOnlyElement(outputs)
        match outputs.len() {
            0 => panic!("NoSuchElementException"),
            1 => {}
            _ => panic!(
                "IllegalArgumentException: expected one element but was: <{}>",
                outputs
                    .iter()
                    .map(|a| a.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
        check_argument!(
            outputs[0].is_runfiles_tree(),
            "{}",
            java_list_to_string(outputs.iter())
        );
        RunfilesTreeAction {
            base,
            runfiles_tree,
        }
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:55 RunfilesTreeAction.getRunfilesTree
    pub fn get_runfiles_tree(&self) -> &RunfilesTreeRef {
        &self.runfiles_tree
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:158 RunfilesTreeAction.mayInsensitivelyPropagateInputs
    pub fn may_insensitively_propagate_inputs(&self) -> bool {
        true
    }
}

impl Action for RunfilesTreeAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:153 RunfilesTreeAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        MNEMONIC.to_string()
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:133 RunfilesTreeAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        _fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        // Only the set of inputs matters, and the dependency checker is
        // responsible for considering those.
        Ok(())
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:143 RunfilesTreeAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        None // this action doesn't actually do anything so let's not report it
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:148 RunfilesTreeAction.prettyPrint
    fn pretty_print(&self) -> String {
        format!(
            "runfiles for {}",
            Label::print(self.get_owner().get_label())
        )
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:163 RunfilesTreeAction.getExecutionPlatform
    fn get_execution_platform(&self) -> Option<PlatformInfoRef> {
        Some(empty_platform_info())
    }

    // Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:168 RunfilesTreeAction.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        // Runfiles tree actions do not execute actual actions, and therefore have no execution
        // platform.
        IndexMap::new()
    }
}
