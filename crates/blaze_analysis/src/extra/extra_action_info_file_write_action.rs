//! Port of `analysis/extra/ExtraActionInfoFileWriteAction.java`.

use std::any::Any;
use std::io::Write;
use std::sync::Arc;

use blaze_actions::actions::action::{
    AbstractAction, Action, ComputeKeyError, InputMetadataProvider,
};
use blaze_actions::analysis::actions::abstract_file_write_action::{
    self, AbstractFileWriteAction, DeterministicWriter, FnDeterministicWriter,
    NewDeterministicWriterError, new_abstract_file_write_action,
};
use blaze_actions::{ActionKeyContext, ActionOwner, ActionRef, Artifact};
use blaze_collect::command_line_item::MapFnError;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_protos::JavaMessage;
use blaze_util::Fingerprint;
use indexmap::IndexMap;

// Java: com/google/devtools/build/lib/analysis/extra/ExtraActionInfoFileWriteAction.java:46 ExtraActionInfoFileWriteAction.UUID
const UUID: &str = "1759f81d-e72e-477d-b182-c4532bdbaeeb";

/// Requests extra action info from shadowed action and writes it, in protocol buffer format, to
/// an .xa file for use by an extra action. This can only be done at execution time because
/// actions may store information only known at execution time into the protocol buffer.
// Java: com/google/devtools/build/lib/analysis/extra/ExtraActionInfoFileWriteAction.java:45 ExtraActionInfoFileWriteAction
///
/// PORT: Java reads the `ActionKeyContext` from the execution context
/// (`ctx.getActionKeyContext()`, the skyframe executor's, the same object the analysis uses);
/// Blaze's execution context has none, so the action keeps the analysis environment's.
pub struct ExtraActionInfoFileWriteAction {
    base: AbstractAction,
    shadowed_action: ActionRef,
    action_key_context: Arc<ActionKeyContext>,
}

impl std::fmt::Debug for ExtraActionInfoFileWriteAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string_java())
    }
}

impl ExtraActionInfoFileWriteAction {
    // Java: com/google/devtools/build/lib/analysis/extra/ExtraActionInfoFileWriteAction.java:50 ExtraActionInfoFileWriteAction.<init>
    pub fn new(
        owner: ActionOwner,
        primary_output: Artifact,
        shadowed_action: ActionRef,
        action_key_context: Arc<ActionKeyContext>,
    ) -> ExtraActionInfoFileWriteAction {
        let inputs = if shadowed_action.discovers_inputs() {
            let mut b = NestedSetBuilder::stable_order();
            b.add_all(shadowed_action.get_outputs());
            b.build()
        } else {
            NestedSetBuilder::empty_set(Order::STABLE_ORDER)
        };
        ExtraActionInfoFileWriteAction {
            base: new_abstract_file_write_action(owner, inputs, primary_output),
            shadowed_action,
            action_key_context,
        }
    }

    /// The shadowed action.
    pub fn get_shadowed_action(&self) -> &ActionRef {
        &self.shadowed_action
    }
}

impl Action for ExtraActionInfoFileWriteAction {
    fn java_class_name(&self) -> Option<&'static str> {
        Some("com.google.devtools.build.lib.analysis.extra.ExtraActionInfoFileWriteAction")
    }

    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:96 AbstractFileWriteAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        abstract_file_write_action::MNEMONIC.to_string()
    }

    // Java: com/google/devtools/build/lib/analysis/extra/ExtraActionInfoFileWriteAction.java:78 ExtraActionInfoFileWriteAction.computeKey
    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(UUID);
        fp.add_string(
            &self
                .shadowed_action
                .get_key(action_key_context, input_metadata_provider)?,
        );
        fp.add_bytes(
            &self
                .shadowed_action
                .get_extra_action_info(action_key_context)?
                .to_byte_array(),
        );
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:101 AbstractFileWriteAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        abstract_file_write_action::raw_progress_message(self)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:125 AbstractFileWriteAction.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        abstract_file_write_action::exec_properties()
    }
}

impl AbstractFileWriteAction for ExtraActionInfoFileWriteAction {
    /// `new ProtoDeterministicWriter(shadowedAction.getExtraActionInfo(ctx.getActionKeyContext())
    /// .build())`. Java turns a `CommandLineExpansionException` into a `UserExecException` with
    /// the spawn code `COMMAND_LINE_EXPANSION_FAILURE`; that is
    /// [`NewDeterministicWriterError::Exec`] with the message here, the code is set by
    /// [`super::execution`].
    // Java: com/google/devtools/build/lib/analysis/extra/ExtraActionInfoFileWriteAction.java:62 ExtraActionInfoFileWriteAction.newDeterministicWriter
    fn new_deterministic_writer(
        &self,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError> {
        let bytes = match self
            .shadowed_action
            .get_extra_action_info(&self.action_key_context)
        {
            Ok(info) => info.to_byte_array(),
            Err(MapFnError::Interrupted(_)) => {
                return Err(NewDeterministicWriterError::Interrupted);
            }
            // Strings.nullToEmpty(e.getMessage())
            Err(MapFnError::CommandLineExpansion(e)) => {
                return Err(NewDeterministicWriterError::Exec(e.message));
            }
        };
        // ProtoDeterministicWriter.writeTo: message.writeTo(out).
        Ok(Box::new(FnDeterministicWriter(
            move |out: &mut dyn Write| out.write_all(&bytes),
        )))
    }
}
