//! `blaze query`: `runtime/commands/QueryCommand.java` and the command part of
//! `runtime/commands/QueryEnvironmentBasedCommand.java`, over the command's `SkyframeExecutor`
//! (blaze_sky_top) and blaze_query's query implementation.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;
use std::sync::Arc;

use blaze_build_config::build_options::BuildOptions;
use blaze_events::{Event, EventHandler, ExtendedEventHandler, OutputStream as EventsOutputStream};
use blaze_optdefs::command::keep_going_option::KeepGoingOption;
use blaze_optdefs::command::loading_phase_threads_option::LoadingPhaseThreadsOption;
use blaze_optdefs::command::query_options::QueryOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_query::query::output::output_formatter::OutputStream;
use blaze_sky_loading::package_manager::RepositoryMappingResolutionOrInterrupted;
use blaze_sky_top::skyframe_executor::{
    SkyframeExecutor, SkyframeExecutorCreationError, SkyframeExecutorInputs,
};

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::command_registry::QUERY_COMMAND;
use crate::exit_code::{DetailedExitCode, ExitCode};
use crate::options_result::OptionsParsingResult;

/// `queryRuntimeHelper.getOutputStreamForQueryOutput()`: the reporter's stdout as a writer.
pub(crate) struct ReporterStdout(pub Arc<dyn EventsOutputStream>);

impl Write for ReporterStdout {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// The query output stream of a command.
pub(crate) fn query_output_stream(env: &CommandEnvironment) -> OutputStream {
    Rc::new(RefCell::new(ReporterStdout(
        env.get_reporter_out_err().get_output_stream().clone(),
    )))
}

/// The reporter as the handler the executor and queries take.
pub(crate) fn reporter(env: &CommandEnvironment) -> Arc<dyn ExtendedEventHandler> {
    env.get_reporter().clone()
}

/// Creates the command's `SkyframeExecutor` (Java: `env.getSkyframeExecutor()` after
/// `syncPackageLoading`; for commands that analyze, with the request's build options as the
/// baseline configuration).
pub(crate) fn create_skyframe_executor(
    env: &CommandEnvironment,
    top_level_build_options: Option<BuildOptions>,
) -> Result<SkyframeExecutor, BlazeCommandResult> {
    create_skyframe_executor_with_options(env, env.get_options().clone(), top_level_build_options)
}

/// Runs the modules' `afterCommand` hooks that read the command's evaluator when dropped (Java
/// `BazelLockFileModule.afterCommand`, called by `BlazeRuntime.afterCommand` on every exit path
/// of a command). Bind it right after creating the command's executor.
pub(crate) struct AfterCommand<'a>(pub &'a SkyframeExecutor);

impl Drop for AfterCommand<'_> {
    fn drop(&mut self) {
        self.0.after_command();
    }
}

/// [`create_skyframe_executor`] over the given (re-parsed) command options.
pub(crate) fn create_skyframe_executor_with_options(
    env: &CommandEnvironment,
    options: Arc<OptionsParsingResult>,
    top_level_build_options: Option<BuildOptions>,
) -> Result<SkyframeExecutor, BlazeCommandResult> {
    create_skyframe_executor_with_execution(env, options, top_level_build_options, None)
}

/// [`create_skyframe_executor_with_options`] for a command that executes actions.
pub(crate) fn create_skyframe_executor_with_execution(
    env: &CommandEnvironment,
    options: Arc<OptionsParsingResult>,
    top_level_build_options: Option<BuildOptions>,
    execution: Option<blaze_sky_top::execution_phase::ExecutionSetup>,
) -> Result<SkyframeExecutor, BlazeCommandResult> {
    create_skyframe_executor_reporting_to(
        env,
        options,
        top_level_build_options,
        execution,
        reporter(env),
    )
}

/// [`create_skyframe_executor_with_execution`] whose evaluations report to `event_handler`
/// (creation errors still go to the command's reporter).
pub(crate) fn create_skyframe_executor_reporting_to(
    env: &CommandEnvironment,
    options: Arc<OptionsParsingResult>,
    top_level_build_options: Option<BuildOptions>,
    execution: Option<blaze_sky_top::execution_phase::ExecutionSetup>,
    event_handler: Arc<dyn ExtendedEventHandler>,
) -> Result<SkyframeExecutor, BlazeCommandResult> {
    create_skyframe_executor_internal(
        env,
        options,
        top_level_build_options,
        execution,
        event_handler,
        Default::default(),
    )
}

/// [`create_skyframe_executor`] with the repository precomputed values a command injects
/// (Java `env.getSkyframeExecutor().injectExtraPrecomputedValues`, e.g. `fetch --force`).
pub(crate) fn create_skyframe_executor_with_repo_precomputed(
    env: &CommandEnvironment,
    repo_precomputed: blaze_repo_rules::registration::RepoPrecomputedValues,
) -> Result<SkyframeExecutor, BlazeCommandResult> {
    create_skyframe_executor_internal(
        env,
        env.get_options().clone(),
        None,
        None,
        reporter(env),
        repo_precomputed,
    )
}

fn create_skyframe_executor_internal(
    env: &CommandEnvironment,
    options: Arc<OptionsParsingResult>,
    top_level_build_options: Option<BuildOptions>,
    execution: Option<blaze_sky_top::execution_phase::ExecutionSetup>,
    event_handler: Arc<dyn ExtendedEventHandler>,
    repo_precomputed: blaze_repo_rules::registration::RepoPrecomputedValues,
) -> Result<SkyframeExecutor, BlazeCommandResult> {
    let keep_going = options
        .get_options::<KeepGoingOption>()
        .map(|o| o.keep_going)
        .unwrap_or(false);
    let directories = env.get_directories();
    let inputs = SkyframeExecutorInputs {
        workspace: env
            .get_workspace()
            .cloned()
            .expect("the command runs in a workspace"),
        working_directory: env.get_working_directory().clone(),
        output_base: env.get_output_base().clone(),
        install_base: directories.get_install_base().clone(),
        exec_root_base: directories.get_exec_root_base().clone(),
        product_name: env.get_runtime().get_product_name().to_owned(),
        workspace_name: env.get_workspace_name().to_owned(),
        options,
        reporter: event_handler,
        client_env: env.get_client_env().clone(),
        repo_env: env.get_repo_env().clone(),
        nonstrict_repo_env: env.get_nonstrict_repo_env().clone(),
        repo_precomputed,
        execution,
        keep_going,
        top_level_build_options,
    };
    let created = SkyframeExecutor::create(inputs);
    if let Ok(skyframe_executor) = &created {
        // The command thread's interrupt reaches the evaluations of this executor (Java: the
        // thread that calls `evaluate` is interrupted).
        crate::interrupt::register_interrupt_flag(
            &skyframe_executor.get_evaluator().interrupt_flag(),
        );
    }
    created.map_err(|e| {
        env.get_reporter().handle(&Event::error(e.to_string()));
        let failure_detail = match &e {
            SkyframeExecutorCreationError::Bzlmod { message, code } => FailureDetail {
                message: message.clone(),
                category: Some(Category::ExternalRepository(
                    failure_details::ExternalRepository {
                        code: failure_details::external_repository::Code::from_str_name(code)
                            .unwrap_or(
                                failure_details::external_repository::Code::ExternalRepositoryUnknown,
                            ) as i32,
                    },
                )),
            },
            SkyframeExecutorCreationError::InvalidConfiguration(e) => {
                let detailed =
                    blaze_sky_top::build_tool::invalid_configuration_detailed_exit_code(e);
                return BlazeCommandResult::detailed_exit_code(to_runtime_detailed_exit_code(
                    detailed,
                ));
            }
            // Java: ExecutionTool's AbruptExitExceptions (action cache, strategies).
            SkyframeExecutorCreationError::Execution(message) => {
                if message.starts_with("Couldn't create action cache: ") {
                    FailureDetail {
                        message: message.clone(),
                        category: Some(Category::ActionCache(failure_details::ActionCache {
                            code: failure_details::action_cache::Code::InitializationFailure
                                as i32,
                        })),
                    }
                } else {
                    // ExecutionTool.createExitException(messagePrefix, detailedCode, e): the
                    // prefixes of ExecutionTool.<init> and createActionLogDirectory.
                    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:1015 ExecutionTool.createExitException
                    use failure_details::execution::Code;
                    let code = if message.starts_with("Execroot creation failed: ") {
                        Code::ExecrootCreationFailure
                    } else if message.starts_with("Couldn't delete action output directory: ") {
                        Code::TempActionOutputDirectoryDeletionFailure
                    } else if message.starts_with("Couldn't create action output directory: ") {
                        Code::TempActionOutputDirectoryCreationFailure
                    } else {
                        Code::ExecutionUnknown
                    };
                    FailureDetail {
                        message: message.clone(),
                        category: Some(Category::Execution(failure_details::Execution {
                            code: code as i32,
                        })),
                    }
                }
            }
            // Java: BazelRepositoryModule.beforeCommand / BzlmodConfig errors (caches, overrides).
            SkyframeExecutorCreationError::ExternalRepos(message) => {
                return BlazeCommandResult::detailed_exit_code(DetailedExitCode::of_exit_code(
                    ExitCode::LOCAL_ENVIRONMENTAL_ERROR,
                    FailureDetail {
                        message: message.clone(),
                        category: Some(Category::ExternalRepository(
                            failure_details::ExternalRepository {
                                code: failure_details::external_repository::Code::ExternalRepositoryUnknown
                                    as i32,
                            },
                        )),
                    },
                ));
            }
        };
        BlazeCommandResult::detailed_exit_code(DetailedExitCode::of(failure_detail))
    })
}

/// A query-engine `DetailedExitCode` as the runtime's.
pub(crate) fn to_runtime_detailed_exit_code(
    detailed_exit_code: blaze_query_engine::DetailedExitCode,
) -> DetailedExitCode {
    match detailed_exit_code.get_failure_detail() {
        None if detailed_exit_code.is_success() => DetailedExitCode::success(),
        None => panic!(
            "IllegalStateException: exit code {} without FailureDetail",
            detailed_exit_code.get_exit_code()
        ),
        Some(failure_detail) => DetailedExitCode::of_exit_code(
            ExitCode::for_code(detailed_exit_code.get_exit_code()).unwrap_or_else(|| {
                panic!(
                    "No ExitCode for numericExitCode {}",
                    detailed_exit_code.get_exit_code()
                )
            }),
            failure_detail.clone(),
        ),
    }
}

/// Command line wrapper for executing a query with blaze.
// Java: com/google/devtools/build/lib/runtime/commands/QueryCommand.java:73 QueryCommand
pub struct QueryCommand;

impl BlazeCommand for QueryCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &QUERY_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/QueryEnvironmentBasedCommand.java:80 QueryEnvironmentBasedCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        crate::commands::bep::exec_with_no_build_events(env, |env| self.exec_internal(env, options))
    }
}

impl QueryCommand {
    // Java: com/google/devtools/build/lib/runtime/commands/QueryEnvironmentBasedCommand.java:96 QueryEnvironmentBasedCommand.execInternal
    fn exec_internal(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        let skyframe_executor = match create_skyframe_executor(env, None) {
            Ok(e) => e,
            Err(result) => return result,
        };
        let _after_command = AfterCommand(&skyframe_executor);
        let reporter = reporter(env);
        let query_options: QueryOptions = options.get_options().expect("QueryOptions");
        let keep_going = options
            .get_options::<KeepGoingOption>()
            .expect("KeepGoingOption")
            .keep_going;
        let loading_phase_threads = options
            .get_options::<LoadingPhaseThreadsOption>()
            .expect("LoadingPhaseThreadsOption")
            .threads;
        // PORT: syncPackageLoading happens when the executor is created; PackageProgressReceiver
        // and LoadingPhaseStartedEvent belong to the UI.

        // env.getSkyframeExecutor().getMainRepoMapping(keepGoing, threads, reporter)
        let main_repo_mapping = match skyframe_executor.get_main_repo_mapping(
            keep_going,
            loading_phase_threads.max(1) as usize,
            reporter.clone(),
        ) {
            Ok(mapping) => mapping,
            Err(RepositoryMappingResolutionOrInterrupted::Resolution(e)) => {
                env.get_reporter().handle(&Event::error(e.to_string()));
                return BlazeCommandResult::detailed_exit_code(to_runtime_detailed_exit_code(
                    blaze_query::compat::failure_details::from_packages(&e.detailed_exit_code),
                ));
            }
            Err(RepositoryMappingResolutionOrInterrupted::Interrupted) => {
                let message = "query interrupted";
                env.get_reporter().handle(&Event::error(message));
                return BlazeCommandResult::detailed_exit_code(DetailedExitCode::of_exit_code(
                    ExitCode::INTERRUPTED,
                    FailureDetail {
                        message: message.to_owned(),
                        category: Some(Category::Interrupted(failure_details::Interrupted {
                            code: failure_details::interrupted::Code::Interrupted as i32,
                        })),
                    },
                ));
            }
        };

        // env.getRuntime().getQueryRuntimeHelperFactory().create(env, queryOptions)
        // Java: com/google/devtools/build/lib/runtime/commands/QueryEnvironmentBasedCommand.java:169 QueryEnvironmentBasedCommand.execInternal
        let query_output = match blaze_query::query::output::query_runtime_helper::create(
            &query_output_stream(env),
            env.get_working_directory(),
            &query_options.output_file,
        ) {
            Ok(output) => output,
            Err(e) => {
                env.get_reporter().handle(&Event::error(e.get_message()));
                return BlazeCommandResult::detailed_exit_code(DetailedExitCode::of(
                    e.get_failure_detail(),
                ));
            }
        };
        let query_env = blaze_cquery::commands::query_adapters::query_command_environment(
            &skyframe_executor,
            reporter,
            query_output,
            env.get_relative_working_directory().clone(),
            env.get_working_directory().clone(),
            main_repo_mapping,
        );
        let residue = options.get_residue();
        let detailed_exit_code = blaze_query::commands::query_command::exec_internal(
            &query_env,
            &query_options,
            keep_going,
            loading_phase_threads,
            &residue,
        );
        let _ = env.get_reporter_out_err().get_output_stream().flush();
        BlazeCommandResult::detailed_exit_code(to_runtime_detailed_exit_code(detailed_exit_code))
    }
}
