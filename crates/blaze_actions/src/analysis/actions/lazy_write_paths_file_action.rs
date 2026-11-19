//! Port of `lib/analysis/actions/LazyWritePathsFileAction.java`.

use std::any::Any;
use std::io::Write;

use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_util::Fingerprint;
use indexmap::{IndexMap, IndexSet};

use crate::actions::action::{AbstractAction, Action, ComputeKeyError, InputMetadataProvider};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;

use super::abstract_file_write_action::{
    self, AbstractFileWriteAction, DeterministicWriter, FnDeterministicWriter,
    NewDeterministicWriterError, new_abstract_file_write_action,
};

// Java: com/google/devtools/build/lib/analysis/actions/LazyWritePathsFileAction.java:40 LazyWritePathsFileAction.GUID
const GUID: &str = "6be94d90-96f3-4bec-8104-1fb08abc2546";

/// Lazily writes the path of the given files separated by newline into a specified output file.
///
/// By default the exec path is written but this behaviour can be customized by providing an
/// alternative converter function.
// Java: com/google/devtools/build/lib/analysis/actions/LazyWritePathsFileAction.java:39 LazyWritePathsFileAction
#[derive(Debug)]
pub struct LazyWritePathsFileAction {
    base: AbstractAction,
    files: NestedSet<Artifact>,
    files_to_ignore: IndexSet<Artifact>,
    include_derived_artifacts: bool,
}

impl LazyWritePathsFileAction {
    // Java: com/google/devtools/build/lib/analysis/actions/LazyWritePathsFileAction.java:46 LazyWritePathsFileAction.<init>
    pub fn new(
        owner: ActionOwner,
        output: Artifact,
        files: NestedSet<Artifact>,
        files_to_ignore: IndexSet<Artifact>,
        include_derived_artifacts: bool,
    ) -> LazyWritePathsFileAction {
        // We don't need to pass the given files as explicit inputs to this action; we don't care
        // about them, we only need their names, which we already know.
        LazyWritePathsFileAction {
            base: new_abstract_file_write_action(
                owner,
                NestedSetBuilder::empty_set(Order::STABLE_ORDER),
                output,
            ),
            files,
            files_to_ignore,
            include_derived_artifacts,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/LazyWritePathsFileAction.java:76 LazyWritePathsFileAction.getContents
    fn get_contents(&self) -> String {
        let mut string_builder = String::new();
        for file in self.files.to_list().iter() {
            if self.files_to_ignore.contains(file) {
                continue;
            }
            if file.is_source_artifact() || self.include_derived_artifacts {
                string_builder.push_str(file.get_exec_path_string());
                string_builder.push('\n');
            }
        }
        string_builder
    }

    // Java: com/google/devtools/build/lib/analysis/actions/LazyWritePathsFileAction.java:90 LazyWritePathsFileAction.getFiles
    pub fn get_files(&self) -> &NestedSet<Artifact> {
        &self.files
    }
}

impl Action for LazyWritePathsFileAction {
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

    /// Computes the Action key for this action by computing the fingerprint for the file
    /// contents.
    // Java: com/google/devtools/build/lib/analysis/actions/LazyWritePathsFileAction.java:67 LazyWritePathsFileAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(GUID);
        fp.add_boolean(self.include_derived_artifacts);
        // Invariant: exec paths are internal strings (see `fingerprint_util`).
        fp.add_string(&self.get_contents());
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

impl AbstractFileWriteAction for LazyWritePathsFileAction {
    // Java: com/google/devtools/build/lib/analysis/actions/LazyWritePathsFileAction.java:61 LazyWritePathsFileAction.newDeterministicWriter
    fn new_deterministic_writer(
        &self,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError> {
        Ok(Box::new(FnDeterministicWriter(|out: &mut dyn Write| {
            // Java: `out.write(StringUnsafe.getInternalStringBytes(getContents()))`.
            out.write_all(&blaze_util::string_encoding::latin1_encode(
                &self.get_contents(),
            ))
        })))
    }
}
