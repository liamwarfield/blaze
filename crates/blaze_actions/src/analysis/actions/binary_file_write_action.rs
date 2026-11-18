//! Port of `lib/analysis/actions/BinaryFileWriteAction.java`.

use std::any::Any;
use std::fmt;
use std::io::{self, Read, Write};
use std::sync::Arc;

use blaze_collect::{NestedSetBuilder, Order};
use blaze_util::Fingerprint;
use indexmap::IndexMap;

use crate::actions::action::{AbstractAction, Action, ComputeKeyError, InputMetadataProvider};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;

use super::abstract_file_write_action::{
    self, AbstractFileWriteAction, DeterministicWriter, FnDeterministicWriter,
    NewDeterministicWriterError, new_abstract_file_write_action,
};

// Java: com/google/devtools/build/lib/analysis/actions/BinaryFileWriteAction.java:40 BinaryFileWriteAction.GUID
const GUID: &str = "eeee07fe-4b40-11e4-82d6-eba0b4f713e2";

/// A readable source of bytes (Guava `ByteSource`).
pub trait ByteSource: Send + Sync + fmt::Debug {
    /// Opens a new stream over the bytes (`ByteSource.openStream`).
    fn open_stream(&self) -> io::Result<Box<dyn Read + '_>>;
}

/// `ByteSource.wrap(byte[])`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrappedByteSource(pub Vec<u8>);

impl ByteSource for WrappedByteSource {
    fn open_stream(&self) -> io::Result<Box<dyn Read + '_>> {
        Ok(Box::new(&self.0[..]))
    }
}

/// `ByteSource.wrap(bytes)`.
pub fn wrap(bytes: Vec<u8>) -> Arc<dyn ByteSource> {
    Arc::new(WrappedByteSource(bytes))
}

/// Action to write a binary file.
// Java: com/google/devtools/build/lib/analysis/actions/BinaryFileWriteAction.java:38 BinaryFileWriteAction
#[derive(Debug)]
pub struct BinaryFileWriteAction {
    base: AbstractAction,
    source: Arc<dyn ByteSource>,
    make_executable: bool,
}

impl BinaryFileWriteAction {
    /// Creates a new BinaryFileWriteAction instance without inputs.
    // Java: com/google/devtools/build/lib/analysis/actions/BinaryFileWriteAction.java:53 BinaryFileWriteAction.<init>
    pub fn new(
        owner: ActionOwner,
        output: Artifact,
        source: Arc<dyn ByteSource>,
        make_executable: bool,
    ) -> BinaryFileWriteAction {
        BinaryFileWriteAction {
            base: new_abstract_file_write_action(
                owner,
                /* inputs= */ NestedSetBuilder::empty_set(Order::STABLE_ORDER),
                output,
            ),
            source,
            make_executable,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/BinaryFileWriteAction.java:66 BinaryFileWriteAction.getSource
    pub fn get_source(&self) -> &Arc<dyn ByteSource> {
        &self.source
    }
}

impl Action for BinaryFileWriteAction {
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

    // Java: com/google/devtools/build/lib/analysis/actions/BinaryFileWriteAction.java:81 BinaryFileWriteAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(GUID);
        fp.add_boolean(self.make_executable);

        let read = |fp: &mut Fingerprint| -> io::Result<()> {
            let mut input = self.source.open_stream()?;
            let mut buffer = [0u8; 512];
            loop {
                let amount_read = input.read(&mut buffer)?;
                if amount_read == 0 {
                    return Ok(());
                }
                fp.add_bytes(&buffer[..amount_read]);
            }
        };
        if let Err(e) = read(fp) {
            panic!("RuntimeException: {}", e);
        }
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

impl AbstractFileWriteAction for BinaryFileWriteAction {
    // Java: com/google/devtools/build/lib/analysis/actions/BinaryFileWriteAction.java:61 BinaryFileWriteAction.makeExecutable
    fn make_executable(&self) -> bool {
        self.make_executable
    }

    // Java: com/google/devtools/build/lib/analysis/actions/BinaryFileWriteAction.java:71 BinaryFileWriteAction.newDeterministicWriter
    fn new_deterministic_writer(
        &self,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError> {
        Ok(Box::new(FnDeterministicWriter(|out: &mut dyn Write| {
            let mut input = self.source.open_stream()?;
            io::copy(&mut input, out)?;
            out.flush()
        })))
    }
}
