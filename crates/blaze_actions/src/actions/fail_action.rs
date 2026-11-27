//! Port of `lib/actions/FailAction.java` (analysis-time parts; `execute` belongs to the execution
//! layer, `blaze_exec::actions::fail_action`).

use std::any::Any;

use blaze_collect::{NestedSetBuilder, Order};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_util::Fingerprint;

use crate::actions::action::{AbstractAction, Action, ComputeKeyError, InputMetadataProvider};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;

// Java: com/google/devtools/build/lib/actions/FailAction.java:34 FailAction.GUID
const GUID: &str = "626cb78a-810f-4af3-979c-ee194955f04c";

/// FailAction is an Action that always fails to execute. (Used as scaffolding for rules we haven't
/// yet implemented. Also useful for testing.)
// Java: com/google/devtools/build/lib/actions/FailAction.java:32 FailAction
#[derive(Debug)]
pub struct FailAction {
    base: AbstractAction,
    failure_detail: FailureDetail,
}

impl FailAction {
    // Java: com/google/devtools/build/lib/actions/FailAction.java:38 FailAction.<init>
    pub fn new(
        owner: ActionOwner,
        outputs: impl IntoIterator<Item = Artifact>,
        error_message: &str,
        fail_action_code: failure_details::fail_action::Code,
    ) -> FailAction {
        let base = AbstractAction::new(
            owner,
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            outputs,
        );
        // Java string concatenation of a null label prints "null".
        let label = match base.owner().get_label() {
            Some(label) => label.to_string(),
            None => "null".to_string(),
        };
        let failure_detail = FailureDetail {
            message: format!("{error_message} caused by {label}"),
            category: Some(failure_detail::Category::FailAction(
                failure_details::FailAction {
                    code: fail_action_code as i32,
                },
            )),
        };
        FailAction {
            base,
            failure_detail,
        }
    }

    // Java: com/google/devtools/build/lib/actions/FailAction.java:54 FailAction.getErrorMessage
    pub fn get_error_message(&self) -> &str {
        &self.failure_detail.message
    }

    /// The `FailureDetail` that `execute` throws with.
    pub fn get_failure_detail(&self) -> &FailureDetail {
        &self.failure_detail
    }
}

impl Action for FailAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn java_class_name(&self) -> Option<&'static str> {
        Some("com.google.devtools.build.lib.actions.FailAction")
    }

    // Java: com/google/devtools/build/lib/actions/FailAction.java:50 FailAction.getPrimaryInput
    fn get_primary_input(&self) -> Option<Artifact> {
        None
    }

    // Java: com/google/devtools/build/lib/actions/FailAction.java:66 FailAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(GUID);
        // Should never be cached, but just be safe.
        fp.add_string(self.get_error_message());
        Ok(())
    }

    // Java: com/google/devtools/build/lib/actions/FailAction.java:76 FailAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        let label = match self.get_owner().get_label() {
            Some(label) => label.to_string(),
            None => "null".to_string(),
        };
        Some(format!(
            "Reporting failed target {} located at {}",
            label,
            self.get_owner().get_location()
        ))
    }

    // Java: com/google/devtools/build/lib/actions/FailAction.java:84 FailAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        "Fail".to_string()
    }
}
