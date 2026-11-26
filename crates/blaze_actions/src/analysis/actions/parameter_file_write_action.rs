//! Port of `lib/analysis/actions/ParameterFileWriteAction.java`.

use std::any::Any;
use std::io::{self, Write};
use std::sync::Arc;

use blaze_collect::command_line_item::MapFnError;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_util::{Fingerprint, unported};
use indexmap::IndexMap;

use crate::actions::action::{
    AbstractAction, Action, CommandLineError, ComputeKeyError, InputMetadataProvider,
};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;
use crate::actions::command_line::{ArgChunk, CommandLineRef};
use crate::actions::parameter_file::{self, ParameterFileType};
use crate::actions::path_mapper::{OutputPathsMode, PathMapper};

use super::abstract_file_write_action::{
    self, AbstractFileWriteAction, DeterministicWriter, NewDeterministicWriterError,
};
use super::path_mappers;

// Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:60 ParameterFileWriteAction.GUID
const GUID: &str = "45f678d8-e395-401e-8446-e795ccc6361f";

/// Action to write a parameter file for a `CommandLine`.
// Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:58 ParameterFileWriteAction
#[derive(Debug)]
pub struct ParameterFileWriteAction {
    base: AbstractAction,
    command_line: CommandLineRef,
    type_: ParameterFileType,
    make_executable: bool,
    mnemonic: String,
    use_path_stripping: bool,
}

impl ParameterFileWriteAction {
    /// Creates a new instance.
    ///
    /// * `owner`: the action owner
    /// * `output`: the Artifact that will be created by executing this Action
    /// * `command_line`: the contents to be written to the file
    /// * `type_`: the type of the file
    /// * `make_executable`: whether the output file should be made executable
    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:77 ParameterFileWriteAction.<init>
    pub fn new(
        owner: ActionOwner,
        output: Artifact,
        command_line: CommandLineRef,
        type_: ParameterFileType,
        make_executable: bool,
    ) -> ParameterFileWriteAction {
        ParameterFileWriteAction::with_inputs(
            owner,
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            output,
            command_line,
            type_,
            make_executable,
            abstract_file_write_action::MNEMONIC,
            /* execution_info= */ &IndexMap::new(),
            OutputPathsMode::OFF,
        )
    }

    /// Creates a new instance.
    ///
    /// * `inputs`: the list of TreeArtifacts that must be resolved and expanded before
    ///   evaluating the contents of the command line.
    /// * `mnemonic`: the mnemonic for this action
    /// * `execution_info`: the execution info for this action (only supports-path-mapping is
    ///   used)
    /// * `output_paths_mode`: the output paths mode of the configuration
    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:110 ParameterFileWriteAction.<init>
    pub fn with_inputs(
        owner: ActionOwner,
        inputs: NestedSet<Artifact>,
        output: Artifact,
        command_line: CommandLineRef,
        type_: ParameterFileType,
        make_executable: bool,
        mnemonic: &str,
        execution_info: &IndexMap<String, String>,
        output_paths_mode: OutputPathsMode,
    ) -> ParameterFileWriteAction {
        let base =
            abstract_file_write_action::new_abstract_file_write_action(owner, inputs, output);
        // Save memory by not storing the full execution info, but only what matters for this
        // particular action.
        let use_path_stripping = path_mappers::get_effective_output_paths_mode(
            output_paths_mode,
            mnemonic,
            execution_info,
        ) == OutputPathsMode::STRIP;
        ParameterFileWriteAction {
            base,
            command_line,
            type_,
            make_executable,
            mnemonic: mnemonic.to_string(),
            use_path_stripping,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:149 ParameterFileWriteAction.getOutputPathsMode
    fn get_output_paths_mode(&self) -> OutputPathsMode {
        if self.use_path_stripping {
            OutputPathsMode::STRIP
        } else {
            OutputPathsMode::OFF
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:154 ParameterFileWriteAction.getCommandLine
    pub fn get_command_line(&self) -> &CommandLineRef {
        &self.command_line
    }

    /// The parameter file type (Java's private `type` field).
    pub fn get_type(&self) -> ParameterFileType {
        self.type_
    }

    /// Returns the list of options written to the parameter file. Don't use this method outside
    /// tests - the list is often huge, resulting in significant garbage collection overhead.
    ///
    /// 2019-01-10, @leba: Using this method for aquery since it's not performance-critical and
    /// the includeParamFile option is flag-guarded with warning regarding output size to user.
    ///
    /// TODO(b/161359171): The list of arguments will be incorrect if the arguments contain tree
    /// artifacts or path mapping is used.
    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:168 ParameterFileWriteAction.getArguments
    pub fn get_arguments(&self) -> Result<Vec<String>, CommandLineError> {
        self.command_line.arguments()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:174 ParameterFileWriteAction.getStringContents
    pub fn get_string_contents(&self) -> Result<String, CommandLineError> {
        let mut out: Vec<u8> = Vec::new();
        parameter_file::write_parameter_file(&mut out, self.get_arguments()?, self.type_)
            .expect("writing to a byte array cannot fail");
        // Java: `out.toString(ISO_8859_1)`.
        Ok(blaze_util::string_encoding::latin1_decode(&out).into_owned())
    }

    /// Returns `None` where Java returns null; an error carries the `EvalException` message.
    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:183 ParameterFileWriteAction.getStarlarkContent
    pub fn get_starlark_content(&self) -> Result<Option<String>, ComputeKeyError> {
        if !self.get_inputs().is_empty() {
            // Tree artifact information isn't available at analysis time.
            return Ok(None);
        }
        match self.get_string_contents() {
            Ok(s) => Ok(Some(s)),
            Err(MapFnError::CommandLineExpansion(e)) => Err(ComputeKeyError::Eval(format!(
                "Error expanding command line: {}",
                e.message
            ))),
            Err(MapFnError::Interrupted(e)) => Err(ComputeKeyError::Interrupted(e)),
        }
    }
}

impl Action for ParameterFileWriteAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:138 ParameterFileWriteAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        self.mnemonic.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:232 ParameterFileWriteAction.computeKey
    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(GUID);
        fp.add_string(self.type_.name());
        self.command_line.add_to_fingerprint(
            action_key_context,
            input_metadata_provider,
            path_mappers::get_effective_output_paths_mode(
                self.get_output_paths_mode(),
                &self.get_mnemonic(),
                &self.get_execution_info(),
            ),
            fp,
        )?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:248 ParameterFileWriteAction.describeKey
    fn describe_key(&self) -> Option<String> {
        let mut message = String::new();
        message.push_str("GUID: ");
        message.push_str(GUID);
        message.push_str("\nParam File Type: ");
        message.push_str(self.type_.name());
        message.push_str("\nContent digest (approximate): ");
        // The full contents can be huge, which makes the final error message
        // incomprehensible. Instead, just give a digest, which makes it easy to
        // tell if two contents are equal or not.
        let mut fp = Fingerprint::new();
        let result = self.command_line.add_to_fingerprint(
            &ActionKeyContext::new(),
            None,
            path_mappers::get_effective_output_paths_mode(
                self.get_output_paths_mode(),
                &self.get_mnemonic(),
                &self.get_execution_info(),
            ),
            &mut fp,
        );
        match result {
            Ok(()) => {
                message.push_str(&hex::encode(fp.digest_and_reset()));
                message.push_str(
                    "\nNOTE: Content digest reflects approximate, analysis-time data; it does not \
                     account for data available during execution (e.g. tree artifact expansions)",
                );
            }
            Err(MapFnError::Interrupted(_)) => {
                message.push_str("Interrupted while expanding command line");
            }
            Err(MapFnError::CommandLineExpansion(e)) => {
                message.push_str("Could not expand contents: ");
                // Java: Throwable.toString().
                message.push_str(&format!(
                    "com.google.devtools.build.lib.actions.CommandLineExpansionException: {}",
                    e.message
                ));
            }
        }
        Some(message)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:101 AbstractFileWriteAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        abstract_file_write_action::raw_progress_message(self)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:125 AbstractFileWriteAction.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        abstract_file_write_action::exec_properties()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:143 ParameterFileWriteAction.getExecutionInfo
    fn get_execution_info(&self) -> IndexMap<String, String> {
        let mut m = IndexMap::new();
        if self.use_path_stripping {
            m.insert(
                path_mappers::SUPPORTS_PATH_MAPPING.to_string(),
                String::new(),
            );
        }
        m
    }

    // aquery: `action instanceof ParameterFileWriteAction` → `getArguments()`.
    fn aquery_parameter_file_arguments(&self) -> Option<Result<Vec<String>, CommandLineError>> {
        Some(self.get_arguments())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:183 ParameterFileWriteAction.getStarlarkContent
    fn get_starlark_content(
        &self,
    ) -> Result<Option<String>, crate::actions::action::ActionApiError> {
        ParameterFileWriteAction::get_starlark_content(self).map_err(|e| match e {
            ComputeKeyError::Interrupted(e) => {
                crate::actions::action::ActionApiError::Interrupted(e)
            }
            e => crate::actions::action::ActionApiError::Eval(e.to_string()),
        })
    }
}

impl AbstractFileWriteAction for ParameterFileWriteAction {
    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:133 ParameterFileWriteAction.makeExecutable
    fn make_executable(&self) -> bool {
        self.make_executable
    }

    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:196 ParameterFileWriteAction.newDeterministicWriter
    fn new_deterministic_writer(
        &self,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError> {
        // PORT: the trait method has no `ActionExecutionContext` (see
        // `AbstractFileWriteAction::new_deterministic_writer`). Without tree artifact inputs the
        // provider is never consulted for tree metadata, so the expansion runs without one. With
        // inputs the caller must hand over the context's provider through
        // `new_deterministic_writer_with_input_metadata_provider`.
        if !self.get_inputs().is_empty() {
            unported!("ActionExecutionContext.getInputMetadataProvider");
        }
        self.deterministic_writer(None)
    }
}

impl ParameterFileWriteAction {
    /// `newDeterministicWriter(ctx)` with `ctx.getInputMetadataProvider()`: tree artifacts in the
    /// command line are expanded to the children the provider gives.
    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:196 ParameterFileWriteAction.newDeterministicWriter
    pub fn new_deterministic_writer_with_input_metadata_provider(
        &self,
        input_metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError> {
        self.deterministic_writer(Some(input_metadata_provider))
    }

    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:196 ParameterFileWriteAction.newDeterministicWriter
    fn deterministic_writer(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError> {
        // Other actions consuming this parameter file may have path mapping disabled due to
        // inputs conflicting across configurations, in which case paths written to the file will
        // not match. Since this depends on the consumer but the decision is only made at
        // execution time, it is not clear how to improve that situation. Actions that are prone to
        // such collisions should avoid depending on parameter files.
        let path_mapper = path_mappers::create(
            self,
            self.get_output_paths_mode(),
            /* is_starlark_action= */ false,
            input_metadata_provider,
        );
        let arguments = match self
            .command_line
            .expand(input_metadata_provider, path_mapper.as_ref())
        {
            Ok(a) => a,
            Err(MapFnError::CommandLineExpansion(e)) => {
                // Java: UserExecException with Code.COMMAND_LINE_EXPANSION_FAILURE.
                return Err(NewDeterministicWriterError::Exec(e.message));
            }
            Err(MapFnError::Interrupted(_)) => {
                return Err(NewDeterministicWriterError::Interrupted);
            }
        };
        Ok(Box::new(ParamFileWriter {
            arguments,
            path_mapper,
            type_: self.type_,
        }))
    }
}

// Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:222 ParameterFileWriteAction.ParamFileWriter
struct ParamFileWriter<'a> {
    arguments: Box<dyn ArgChunk + 'a>,
    path_mapper: Arc<dyn PathMapper>,
    type_: ParameterFileType,
}

impl DeterministicWriter for ParamFileWriter<'_> {
    // Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:226 ParamFileWriter.writeTo
    fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        parameter_file::write_parameter_file(
            out,
            self.arguments.arguments(self.path_mapper.as_ref()),
            self.type_,
        )
    }
}
