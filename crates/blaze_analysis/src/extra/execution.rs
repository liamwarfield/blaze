//! The execution half of the extra-action classes: `ExtraAction` (`SpawnAction.execute` with
//! `ExtraAction`'s `getSpawn`, `afterExecute` and `discoverInputs`),
//! `ExtraActionInfoFileWriteAction` (`AbstractFileWriteAction.execute`) and `PseudoAction`
//! (`execute` fails).

use blaze_actions::Action;
use blaze_actions::Artifact;
use blaze_actions::analysis::actions::abstract_file_write_action::{
    AbstractFileWriteAction, NewDeterministicWriterError,
};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_exec::actions::{
    ActionExecutionException, ActionResult, EnvironmentalExecException, ExecException,
    ExecutableAction, UserExecException, register_executable_action,
};
use blaze_exec::analysis::actions::abstract_file_write_action::execute_file_write;
use blaze_exec::analysis::actions::get_input_path;
use blaze_exec::analysis::actions::spawn_action::{
    GetSpawnError, get_spawn_with_env, to_action_execution_exception,
};
use blaze_exec::exec::action_execution_context::{ActionExecutionContext, get_context};
use blaze_exec::exec::spawn_strategy_resolver::SpawnStrategyResolver;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use indexmap::IndexSet;

use super::extra_action::{ExtraAction, create_inputs};
use super::extra_action_info_file_write_action::ExtraActionInfoFileWriteAction;
use super::pseudo_action::PseudoAction;

/// Registers the `ExecutableAction` implementations of the extra-action classes. Idempotent.
pub fn register_executable_actions() {
    register_executable_action::<ExtraAction>();
    register_executable_action::<ExtraActionInfoFileWriteAction>();
    register_executable_action::<PseudoAction>();
}

impl ExecutableAction for ExtraAction {
    /// This method returns null when a required SkyValue is missing and a Skyframe restart is
    /// required.
    // Java: com/google/devtools/build/lib/analysis/extra/ExtraAction.java:158 ExtraAction.discoverInputs
    fn discover_inputs(
        &self,
        _action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Option<NestedSet<Artifact>>, ActionExecutionException> {
        blaze_util::check_state!(self.discovers_inputs(), "{}", self.to_string_java());

        // We need to update our inputs to take account of any additional
        // inputs the shadowed action may need to do its work.
        let old_inputs = self.get_inputs();
        let Some(input_files_for_extra_action) = self
            .get_shadowed_action()
            .get_input_files_for_extra_action()
        else {
            return Ok(None);
        };
        self.update_inputs(create_inputs(
            &self.get_shadowed_action().get_inputs(),
            &input_files_for_extra_action,
            self.get_extra_action_inputs(),
        ));
        // Sets.difference(getInputs().toSet(), oldInputs.toSet())
        let old: IndexSet<Artifact> = old_inputs.to_list().iter().cloned().collect();
        let difference: Vec<Artifact> = self
            .get_inputs()
            .to_list()
            .iter()
            .filter(|a| !old.contains(*a))
            .cloned()
            .collect::<IndexSet<Artifact>>()
            .into_iter()
            .collect();
        Ok(Some(NestedSetBuilder::wrap(
            Order::STABLE_ORDER,
            difference,
        )))
    }

    /// `SpawnAction.execute` with `ExtraAction.getSpawn` and `ExtraAction.afterExecute`.
    ///
    /// PORT: `ExtraAction.getCommandLineLimits` is `UNLIMITED` where `SpawnAction`'s is the
    /// configuration's; the spawn is built by `SpawnAction.getSpawn` with the latter. The
    /// limits only decide whether a command line goes to a parameter file, and the extra
    /// action's single command line (`CommandLine.of(argv)`) has no parameter file info, so the
    /// spawn is the same.
    // Java: com/google/devtools/build/lib/analysis/actions/SpawnAction.java:278 SpawnAction.execute
    // Java: com/google/devtools/build/lib/analysis/extra/ExtraAction.java:203 ExtraAction.getSpawn
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        // getSpawn(ctx): super.getSpawn(ctx) (the client environment, reportOutputs) unless
        // createDummyOutput, then getSpawn(ctx, ctx.getClientEnv(), reportOutputs = false).
        let spawn = match get_spawn_with_env(
            self.as_spawn_action(),
            ctx,
            ctx.get_client_env(),
            /* report_outputs= */ !self.creates_dummy_output(),
        ) {
            Ok(s) => s,
            Err(GetSpawnError::CommandLineExpansion(e)) => {
                // SpawnAction.createCommandLineException
                let detail = FailureDetail {
                    message: e.message.clone(),
                    category: Some(Category::Spawn(failure_details::Spawn {
                        code: failure_details::spawn::Code::CommandLineExpansionFailure as i32,
                        ..Default::default()
                    })),
                };
                return Err(ActionExecutionException::with_cause(
                    e.message.clone(),
                    &e.message,
                    Some(self),
                    /* catastrophe= */ false,
                    Some(detail),
                ));
            }
            Err(GetSpawnError::Interrupted) => {
                return Err(ActionExecutionException::from_exec_exception(
                    &ExecException::Interrupted,
                    None,
                    self,
                ));
            }
        };
        let resolver = get_context::<SpawnStrategyResolver>(ctx).unwrap_or_else(|| {
            panic!("NullPointerException: getContext(SpawnStrategyResolver.class) returned null")
        });
        let result = resolver
            .exec(&spawn, ctx)
            .and_then(|result| {
                self.after_execute(ctx)?;
                Ok(result)
            })
            .map_err(|e| match &e {
                ExecException::Spawn(spawn_exec_exception) => {
                    to_action_execution_exception(spawn_exec_exception, self)
                }
                _ => ActionExecutionException::from_exec_exception(&e, None, self),
            })?;
        Ok(ActionResult::create(result))
    }
}

impl ExtraAction {
    /// PHASE 3: create dummy output. If the user didn't specify output, we need to create dummy
    /// output to make blaze schedule this action.
    // Java: com/google/devtools/build/lib/analysis/extra/ExtraAction.java:215 ExtraAction.afterExecute
    fn after_execute(&self, ctx: &dyn ActionExecutionContext) -> Result<(), ExecException> {
        if self.creates_dummy_output() {
            for output in self.get_outputs() {
                if let Err(e) = touch_file(&get_input_path(ctx, &output)) {
                    // new EnvironmentalExecException(e, Code.EXTRA_ACTION_OUTPUT_CREATION_FAILURE)
                    // PORT: the cause's message stands for `IOException.toString()`.
                    return Err(ExecException::Environmental(EnvironmentalExecException {
                        message: e.to_string(),
                        failure_detail: FailureDetail {
                            message: String::new(),
                            category: Some(Category::Execution(failure_details::Execution {
                                code: failure_details::execution::Code::ExtraActionOutputCreationFailure
                                    as i32,
                            })),
                        },
                        lost_inputs: None,
                    }));
                }
            }
        }
        Ok(())
    }
}

/// `FileSystemUtils.touchFile`.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:269 FileSystemUtils.touchFile
fn touch_file(path: &blaze_vfs::Path) -> std::io::Result<()> {
    if path.exists() {
        // Path.NOW_SENTINEL_TIME
        path.set_last_modified_time(-1)
    } else {
        // FileSystemUtils.createEmptyFile
        use std::io::Write as _;
        path.get_output_stream()?.flush()
    }
}

impl ExecutableAction for ExtraActionInfoFileWriteAction {
    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:60 AbstractFileWriteAction.execute
    // Java: com/google/devtools/build/lib/analysis/extra/ExtraActionInfoFileWriteAction.java:62 ExtraActionInfoFileWriteAction.newDeterministicWriter
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        execute_file_write(self, ctx, || {
            self.new_deterministic_writer().map_err(|e| match e {
                NewDeterministicWriterError::Interrupted => ExecException::Interrupted,
                // new UserExecException(e, FailureDetail{message: nullToEmpty(e.getMessage()),
                // spawn: {code: COMMAND_LINE_EXPANSION_FAILURE}})
                NewDeterministicWriterError::Exec(message) => {
                    ExecException::User(UserExecException::new(FailureDetail {
                        message,
                        category: Some(Category::Spawn(failure_details::Spawn {
                            code: failure_details::spawn::Code::CommandLineExpansionFailure as i32,
                            ..Default::default()
                        })),
                    }))
                }
            })
        })
    }
}

impl ExecutableAction for PseudoAction {
    // Java: com/google/devtools/build/lib/analysis/PseudoAction.java:68 PseudoAction.execute
    fn execute(
        &self,
        _ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        let message = format!("{}ExtraAction should not be executed.", self.get_mnemonic());
        let detail = FailureDetail {
            message: message.clone(),
            category: Some(Category::Execution(failure_details::Execution {
                code: failure_details::execution::Code::PseudoActionExecutionProhibited as i32,
            })),
        };
        Err(ActionExecutionException::new(
            message,
            Some(self),
            /* catastrophe= */ false,
            Some(detail),
        ))
    }
}
