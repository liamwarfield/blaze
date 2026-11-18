//! `blaze query`: `runtime/commands/QueryCommand.java` and the command part of
//! `runtime/commands/QueryEnvironmentBasedCommand.java`, over the command's `SkyframeExecutor`
//! (blaze_sky_top) and blaze_query's query implementation.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;
use std::sync::Arc;

use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::top_level::create_build_options;
use blaze_events::{Event, EventHandler, ExtendedEventHandler, OutputStream as EventsOutputStream};
use blaze_optdefs::command::keep_going_option::KeepGoingOption;
use blaze_optdefs::command::loading_phase_threads_option::LoadingPhaseThreadsOption;
use blaze_optdefs::command::query_options::QueryOptions;
use blaze_optdefs::fragment::converters::{FlagAliasConverter, StringToLabelEntry};
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_options::{Converter, PriorityCategory};
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

/// [`create_skyframe_executor`] over the given (re-parsed) command options.
pub(crate) fn create_skyframe_executor_with_options(
    env: &CommandEnvironment,
    options: Arc<OptionsParsingResult>,
    top_level_build_options: Option<BuildOptions>,
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
        reporter: reporter(env),
        client_env: env.get_client_env().clone(),
        keep_going,
        top_level_build_options,
    };
    SkyframeExecutor::create(inputs).map_err(|e| {
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
            // PORT: the oracle repository facts are a Blaze stand-in for repository fetching
            // (see blaze_sky_top::external_repos); a missing file is a local environment error.
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

/// The command's build options with the MODULE.bazel flag aliases injected (Java
/// `BlazeCommandDispatcher.execExclusively`: after computing the main repository mapping it
/// parses `--flag_alias=<name>=<label>` for every `SkyframeExecutor.getFlagAliases` entry at
/// `RC_FILE` priority with source "module resolution"; `createBuildOptions` then reads them).
///
/// PORT: the Blaze dispatcher (blaze_runtime/src/dispatcher.rs) does not inject them yet, and
/// the parsed options are shared and immutable, so the commands that analyze (cquery, aquery)
/// compute the aliases on a loading-only executor and add the converted values to
/// `CoreOptions.commandLineFlagAliases` where the parser would have put them (after the values
/// of priority `RC_FILE` or lower, before the command line's). Remove once the dispatcher does
/// it.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:670 BlazeCommandDispatcher.execExclusively
pub(crate) fn build_options_with_module_flag_aliases(
    env: &CommandEnvironment,
    options: &OptionsParsingResult,
) -> Result<BuildOptions, BlazeCommandResult> {
    // TODO: b/453809359 - Remove this Python logic when Bazel 9+ can read Python flag alias
    // definitions straight from rules_python's MODULE.bazel.
    let option_bool = |name: &str| -> bool {
        match options.get_option_description(name) {
            None => false,
            Some(description) => match options.get_option_value_description(name) {
                Some(cmd_line_value) => cmd_line_value
                    .get_value()
                    .and_then(|v| v.downcast_ref::<bool>().copied())
                    .expect("boolean option"),
                None => description
                    .get_option_definition()
                    .get_default_value(None)
                    .and_then(|v| v.downcast_ref::<bool>().copied())
                    .expect("boolean option default"),
            },
        }
    };
    let remove_py_fragment = option_bool("incompatible_remove_ctx_py_fragment");
    let remove_bazel_py_fragment = option_bool("incompatible_remove_ctx_bazel_py_fragment");

    let loading_executor = create_skyframe_executor(env, None)?;
    let keep_going = options
        .get_options::<KeepGoingOption>()
        .map(|o| o.keep_going)
        .unwrap_or(false);
    let threads = options
        .get_options::<LoadingPhaseThreadsOption>()
        .map(|o| o.threads)
        .unwrap_or(1);
    let interrupted = || {
        let message = "command interrupted while computing main repo mapping";
        env.get_reporter().handle(&Event::error(message));
        BlazeCommandResult::detailed_exit_code(DetailedExitCode::of_exit_code(
            ExitCode::INTERRUPTED,
            FailureDetail {
                message: message.to_owned(),
                category: Some(Category::Interrupted(failure_details::Interrupted {
                    code: failure_details::interrupted::Code::Interrupted as i32,
                })),
            },
        ))
    };
    // Compute the repo mapping of the main repo (its errors end the command).
    match loading_executor.get_main_repo_mapping(keep_going, threads.max(1) as usize, reporter(env))
    {
        Ok(_) => {}
        Err(RepositoryMappingResolutionOrInterrupted::Resolution(e)) => {
            env.get_reporter().handle(&Event::error(e.to_string()));
            return Err(BlazeCommandResult::detailed_exit_code(
                to_runtime_detailed_exit_code(blaze_query::compat::failure_details::from_packages(
                    &e.detailed_exit_code,
                )),
            ));
        }
        Err(RepositoryMappingResolutionOrInterrupted::Interrupted) => return Err(interrupted()),
    }
    let aliases = match loading_executor.get_flag_aliases(
        reporter(env),
        remove_py_fragment,
        remove_bazel_py_fragment,
    ) {
        Ok(Some(aliases)) => aliases,
        Ok(None) => panic!("NullPointerException: BazelDepGraphValue after the main repo mapping"),
        Err(_) => return Err(interrupted()),
    };

    let mut build_options = create_build_options(&BazelRuleClassProvider::create(), options);
    if aliases.is_empty() {
        return Ok(build_options);
    }
    // optionsParser.parse(RC_FILE, "module resolution", ["--flag_alias=k=v", ...]): each value
    // goes through FlagAliasConverter.
    let mut injected: Vec<StringToLabelEntry> = Vec::new();
    for arg in blaze_build_config::flag_aliases::flag_alias_args(&aliases) {
        let value = arg.strip_prefix("--flag_alias=").expect("--flag_alias=");
        let converted = FlagAliasConverter
            .convert(
                Some(value),
                options.get_conversion_context().map(|c| c.as_ref()),
            )
            .unwrap_or_else(|e| panic!("{}", e.get_message()))
            .expect("flag alias value");
        injected.push(
            converted
                .downcast_ref::<StringToLabelEntry>()
                .expect("StringToLabelEntry")
                .clone(),
        );
    }
    // The repeatable option's values are ordered by priority; the injected ones (RC_FILE, parsed
    // last within it) follow every value of priority RC_FILE or lower.
    let lower_or_equal = options
        .get_option_value_description("flag_alias")
        .and_then(|d| d.get_canonical_instances())
        .map(|instances| {
            instances
                .iter()
                .filter(|i| i.get_priority().get_priority_category() <= PriorityCategory::RC_FILE)
                .count()
        })
        .unwrap_or(0);
    let mut core_options = build_options
        .get::<CoreOptions>()
        .expect("CoreOptions")
        .clone();
    let rest = core_options
        .command_line_flag_aliases
        .split_off(lower_or_equal);
    core_options.command_line_flag_aliases.extend(injected);
    core_options.command_line_flag_aliases.extend(rest);
    build_options = build_options
        .to_builder()
        .add_fragment_options(&core_options)
        .build();
    Ok(build_options)
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

    // Java: com/google/devtools/build/lib/runtime/commands/QueryEnvironmentBasedCommand.java:96 QueryEnvironmentBasedCommand.execInternal
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        let skyframe_executor = match create_skyframe_executor(env, None) {
            Ok(e) => e,
            Err(result) => return result,
        };
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

        let query_env = blaze_cquery::commands::query_adapters::query_command_environment(
            &skyframe_executor,
            reporter,
            query_output_stream(env),
            env.get_relative_working_directory().clone(),
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
