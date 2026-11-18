//! Ports of `lib/analysis/actions/AbstractFileWriteAction.java` and `lib/util/DeterministicWriter.java`.

use std::io::{self, Write};

use crate::actions::action::Action;

/// Like `OutputStream`-writing `Writer`, but writes the same bytes each time it is called.
// Java: com/google/devtools/build/lib/util/DeterministicWriter.java:25 DeterministicWriter
pub trait DeterministicWriter {
    // Java: com/google/devtools/build/lib/util/DeterministicWriter.java:43 DeterministicWriter.writeTo
    fn write_to(&self, out: &mut dyn Write) -> io::Result<()>;

    /// Returns the contents that would be written, as a byte string.
    // Java: com/google/devtools/build/lib/util/DeterministicWriter.java:54 DeterministicWriter.getBytes
    fn get_bytes(&self) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        self.write_to(&mut out)?;
        Ok(out)
    }
}

/// A `DeterministicWriter` over a closure.
pub struct FnDeterministicWriter<F: Fn(&mut dyn Write) -> io::Result<()>>(pub F);

impl<F: Fn(&mut dyn Write) -> io::Result<()>> DeterministicWriter for FnDeterministicWriter<F> {
    fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        (self.0)(out)
    }
}

/// Errors of `newDeterministicWriter`: `InterruptedException`, `ExecException`.
#[derive(Debug, thiserror::Error)]
pub enum NewDeterministicWriterError {
    #[error("InterruptedException")]
    Interrupted,
    /// `ExecException` (its message).
    #[error("{0}")]
    Exec(String),
}

/// `MNEMONIC` of file write actions.
// Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:41 AbstractFileWriteAction.MNEMONIC
pub const MNEMONIC: &str = "FileWrite";

/// Abstract Action to write to a file.
///
/// Implementors embed an `AbstractAction` built with [`new_abstract_file_write_action`] and, in
/// their [`Action`] impl, delegate `get_mnemonic`, `get_raw_progress_message` and
/// `get_exec_properties` to the helpers below unless their Java class overrides them.
// Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:39 AbstractFileWriteAction
pub trait AbstractFileWriteAction: Action {
    /// Returns true if the output should be marked as executable.
    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:55 AbstractFileWriteAction.makeExecutable
    fn make_executable(&self) -> bool {
        false
    }

    /// Produce a DeterministicWriter that can write the file to an OutputStream deterministically.
    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:84 AbstractFileWriteAction.newDeterministicWriter
    // PORT: the `ActionExecutionContext` parameter is not ported yet (execution is wave W6);
    // writers that need it call `unported!`.
    fn new_deterministic_writer(
        &self,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError>;

    /// Whether the file write can run remotely.
    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:110 AbstractFileWriteAction.isRemotable
    fn is_remotable(&self) -> bool {
        true
    }
}

/// `AbstractFileWriteAction(owner, inputs, output)`: the `AbstractAction` with the single output.
// Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:50 AbstractFileWriteAction.<init>
pub fn new_abstract_file_write_action(
    owner: crate::actions::action_owner::ActionOwner,
    inputs: blaze_collect::NestedSet<crate::actions::artifact::Artifact>,
    output: crate::actions::artifact::Artifact,
) -> crate::actions::action::AbstractAction {
    crate::actions::action::AbstractAction::new(owner, inputs, [output])
}

/// `AbstractFileWriteAction.getRawProgressMessage`.
// Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:101 AbstractFileWriteAction.getRawProgressMessage
pub fn raw_progress_message<A: AbstractFileWriteAction + ?Sized>(action: &A) -> Option<String> {
    let outputs = action.get_outputs();
    blaze_util::check_argument!(
        outputs.len() == 1,
        "expected one element but was: {:?}",
        outputs
    );
    Some(format!(
        "{}{}",
        if action.make_executable() {
            "Writing script "
        } else {
            "Writing file "
        },
        outputs[0].pretty_print()
    ))
}

/// `AbstractFileWriteAction.getExecProperties`: always empty.
// Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:125 AbstractFileWriteAction.getExecProperties
pub fn exec_properties() -> indexmap::IndexMap<String, String> {
    indexmap::IndexMap::new()
}
