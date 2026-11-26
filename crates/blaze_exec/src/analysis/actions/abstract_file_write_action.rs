//! Execution of `lib/analysis/actions/AbstractFileWriteAction.java` and of its subclasses'
//! `newDeterministicWriter(ActionExecutionContext)`: `FileWriteAction`, `BinaryFileWriteAction`,
//! `ParameterFileWriteAction`, `LazyWritePathsFileAction`, `LazyWriteNestedSetOfTupleAction`,
//! `SourceManifestAction` and `RepoMappingManifestAction`.

use std::io::{self, Write};

use blaze_actions::analysis::actions::abstract_file_write_action::{
    AbstractFileWriteAction, DeterministicWriter, NewDeterministicWriterError,
};
use blaze_actions::analysis::actions::binary_file_write_action::BinaryFileWriteAction;
use blaze_actions::analysis::actions::file_write_action::FileWriteAction;
use blaze_actions::analysis::actions::lazy_write_nested_set_of_tuple_action::LazyWriteNestedSetOfTupleAction;
use blaze_actions::analysis::actions::lazy_write_paths_file_action::LazyWritePathsFileAction;
use blaze_actions::analysis::actions::parameter_file_write_action::ParameterFileWriteAction;
use blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingManifestAction;
use blaze_actions::analysis::source_manifest_action::{
    INVALID_RUNFILES_TREE_MESSAGE, SourceManifestAction,
};
use blaze_protos::failure_details::{self, failure_detail::Category};
use blaze_util::profiler::{Profiler, ProfilerTask};

use super::failure_detail;
use super::file_write_action_context::FileWriteActionContext;
use crate::actions::{
    ActionExecutionException, ActionResult, ExecException, ExecutableAction, UserExecException,
};
use crate::exec::action_execution_context::ActionExecutionContext;

/// `AbstractFileWriteAction.execute`, given the result of `newDeterministicWriter(ctx)`.
// Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:60 AbstractFileWriteAction.execute
pub fn execute_file_write<'w, A: AbstractFileWriteAction>(
    action: &A,
    action_execution_context: &dyn ActionExecutionContext,
    new_deterministic_writer: impl FnOnce() -> Result<Box<dyn DeterministicWriter + 'w>, ExecException>,
) -> Result<ActionResult, ActionExecutionException> {
    let run = || -> Result<ActionResult, ExecException> {
        // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:64 AbstractFileWriteAction.execute
        let deterministic_writer = {
            let _c = Profiler::instance().profile(ProfilerTask::INFO, "setupDeterministicWriter");
            new_deterministic_writer()?
        };
        let context = super::get_required_context::<dyn FileWriteActionContext>(
            action_execution_context,
            "FileWriteActionContext",
        );
        let result = context.write_output_to_file(
            action,
            action_execution_context,
            &*deterministic_writer,
            action.make_executable(),
            action.is_remotable(),
        )?;
        // afterWrite(actionExecutionContext): no subclass of the ported actions overrides it.
        Ok(ActionResult::create(result))
    };
    run().map_err(|e| ActionExecutionException::from_exec_exception(&e, None, action))
}

/// `newDeterministicWriter(ctx)` of the subclasses whose writer does not read the context.
fn analysis_writer<A: AbstractFileWriteAction>(
    action: &A,
) -> Result<Box<dyn DeterministicWriter + '_>, ExecException> {
    action.new_deterministic_writer().map_err(|e| match e {
        NewDeterministicWriterError::Interrupted => ExecException::Interrupted,
        NewDeterministicWriterError::Exec(m) => blaze_util::unported!(
            "AbstractFileWriteAction.newDeterministicWriter",
            "unexpected ExecException from {}: {}",
            action.get_mnemonic(),
            m
        ),
    })
}

// Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:271 RegularFileWriteAction.newDeterministicWriter
// Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:341 CompressedFileWriteAction.newDeterministicWriter
impl ExecutableAction for FileWriteAction {
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        execute_file_write(self, ctx, || analysis_writer(self))
    }
}

// Java: com/google/devtools/build/lib/analysis/actions/BinaryFileWriteAction.java:71 BinaryFileWriteAction.newDeterministicWriter
impl ExecutableAction for BinaryFileWriteAction {
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        execute_file_write(self, ctx, || analysis_writer(self))
    }
}

// Java: com/google/devtools/build/lib/analysis/actions/LazyWritePathsFileAction.java:61 LazyWritePathsFileAction.newDeterministicWriter
impl ExecutableAction for LazyWritePathsFileAction {
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        execute_file_write(self, ctx, || analysis_writer(self))
    }
}

// Java: com/google/devtools/build/lib/analysis/actions/LazyWriteNestedSetOfTupleAction.java:52 LazyWriteNestedSetOfTupleAction.newDeterministicWriter
impl ExecutableAction for LazyWriteNestedSetOfTupleAction {
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        execute_file_write(self, ctx, || analysis_writer(self))
    }
}

// Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:187 RepoMappingManifestAction.newDeterministicWriter
impl ExecutableAction for RepoMappingManifestAction {
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        execute_file_write(self, ctx, || analysis_writer(self))
    }
}

impl ExecutableAction for ParameterFileWriteAction {
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        execute_file_write(self, ctx, || parameter_file_writer(self, ctx))
    }
}

/// `ParameterFileWriteAction.newDeterministicWriter(ctx)`: the command line is expanded with
/// `ctx.getInputMetadataProvider()` (tree artifacts become their children) and the path mapper of
/// `getOutputPathsMode()` (`blaze_actions` creates it).
// Java: com/google/devtools/build/lib/analysis/actions/ParameterFileWriteAction.java:196 ParameterFileWriteAction.newDeterministicWriter
fn parameter_file_writer<'a>(
    action: &'a ParameterFileWriteAction,
    ctx: &dyn ActionExecutionContext,
) -> Result<Box<dyn DeterministicWriter + 'a>, ExecException> {
    let input_metadata_provider = super::KeyMetadataAdapter(ctx.get_input_metadata_provider());
    action
        .new_deterministic_writer_with_input_metadata_provider(&input_metadata_provider)
        .map_err(|e| match e {
            NewDeterministicWriterError::Interrupted => ExecException::Interrupted,
            NewDeterministicWriterError::Exec(message) => {
                // new UserExecException(e, FailureDetail(message, Spawn.COMMAND_LINE_EXPANSION_FAILURE))
                UserExecException::with_cause(
                    &message,
                    failure_detail(
                        &message,
                        Category::Spawn(failure_details::Spawn {
                            code: failure_details::spawn::Code::CommandLineExpansionFailure as i32,
                            ..Default::default()
                        }),
                    ),
                )
                .into()
            }
        })
}

impl ExecutableAction for SourceManifestAction {
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        execute_file_write(self, ctx, || source_manifest_writer(self, ctx))
    }
}

/// `SourceManifestAction.newDeterministicWriter(ctx)`.
// Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:233 SourceManifestAction.newDeterministicWriter
fn source_manifest_writer<'a>(
    action: &'a SourceManifestAction,
    ctx: &dyn ActionExecutionContext,
) -> Result<Box<dyn DeterministicWriter + 'a>, ExecException> {
    let inputs = action.compute_execution_runfiles_inputs();
    // eventHandler.replayOn(ctx.getEventHandler())
    for event in &inputs.events {
        ctx.get_event_handler().handle(event);
    }
    if inputs.seen_nested_runfiles_tree {
        let failure_detail = failure_detail(
            INVALID_RUNFILES_TREE_MESSAGE,
            Category::Analysis(failure_details::Analysis {
                code: failure_details::analysis::Code::InvalidRunfilesTree as i32,
            }),
        );
        return Err(UserExecException::new(failure_detail).into());
    }
    Ok(Box::new(SourceManifestWriter {
        action,
        runfiles_inputs: inputs.runfiles_inputs,
    }))
}

struct SourceManifestWriter<'a> {
    action: &'a SourceManifestAction,
    runfiles_inputs: blaze_actions::actions::runfiles_tree::RunfilesMapping,
}

impl DeterministicWriter for SourceManifestWriter<'_> {
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:284 SourceManifestAction.newDeterministicWriter (lambda)
    // PORT: Java's `writeFile(out, runfilesInputs, ctx.getInputMetadataProvider())` reads the
    // targets of unresolved symlink artifacts from their metadata; `blaze_actions`' `write_file`
    // reads them from the file system, which for local execution holds the target the metadata
    // was computed from.
    fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        self.action.write_file(out, &self.runfiles_inputs)
    }
}
