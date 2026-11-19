//! The bodies of `runtime/commands/CqueryCommand.java` and `runtime/commands/AqueryCommand.java`
//! (and the parts of `QueryOptionHelper` / `QueryCommandUtils` they use), plus the adapters the
//! `query` command uses to reach the executor's package manager.

pub mod aquery_command;
pub mod cquery_command;
pub mod query_adapters;

use std::sync::Arc;

use blaze_events::{Event, ExtendedEventHandler};
use blaze_optdefs::command::keep_going_option::KeepGoingOption;
use blaze_optdefs::command::loading_phase_threads_option::LoadingPhaseThreadsOption;
use blaze_options::OptionsParser;
use blaze_protos::failure_details::{
    FailureDetail, Interrupted, failure_detail::Category, interrupted, query,
};
use blaze_query::compat::failure_details;
use blaze_query::query::output::output_formatter::OutputStream;
use blaze_query_engine::{DetailedExitCode, QueryException};
use blaze_sky_loading::package_manager::RepositoryMappingResolutionOrInterrupted;
use blaze_sky_top::skyframe_executor::SkyframeExecutor;
use blaze_util::string_encoding::latin1_decode;
use blaze_vfs::{Path, PathFragment};

use crate::cquery::cquery_options::CommonQueryOptionsView;

/// `Interrupted.Code.INTERRUPTED` exit code.
const INTERRUPTED_EXIT_CODE: i32 = 8;

/// What the post-analysis query commands read from Java's `CommandEnvironment`.
pub struct PostAnalysisCommandEnvironment<'a> {
    /// `env.getSkyframeExecutor()` (created with the command's build options as baseline).
    pub skyframe_executor: &'a SkyframeExecutor,
    /// `env.getReporter()`.
    pub reporter: Arc<dyn ExtendedEventHandler>,
    /// `queryRuntimeHelper.getOutputStreamForQueryOutput()` (stdout).
    pub output: OutputStream,
    /// `env.getRelativeWorkingDirectory()`.
    pub relative_working_directory: PathFragment,
    /// `env.getWorkingDirectory()`.
    pub working_directory: Path,
    /// `env.getRuntime().getProductName()`.
    pub product_name: String,
    /// `env.getSkyframeExecutor().tracksStateForIncrementality()`: the `trackIncrementalState`
    /// that `SequencedSkyframeExecutor.decideKeepIncrementalState` set for the command (the
    /// dispatcher calls it, with its warnings, before the command runs).
    pub tracks_state_for_incrementality: bool,
    /// `Runtime.getRuntime().availableProcessors()` (`SkyQueryEnvironment.DEFAULT_THREAD_COUNT`).
    pub available_processors: i32,
    /// `BuildTool.stopRequest(result, crash, detailedExitCode)`, called with the exit code of
    /// `BuildTool.processRequest` where Java's `processRequest` calls it (its `finally` block):
    /// the `BuildCompleteEvent` subscribers print the build summary (`BuildSummaryStatsModule`,
    /// the UI's `Build completed successfully` / `Build did NOT complete successfully`).
    ///
    /// PORT: blaze_sky_top's `BuildTool.process_request` ends before `stopRequest`; the
    /// subscribers live in the command's crate (blaze_runtime), above this one.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:1155 BuildTool.stopRequest
    pub stop_request: &'a dyn Fn(&DetailedExitCode),
    /// The command's options.
    pub options: &'a OptionsParser,
    /// `new ExecutionTool(env, request)`: `Err` with the exit code of the `AbruptExitException`
    /// its constructor throws (an unknown `--spawn_strategy` / `--strategy`, ...). The command
    /// crate builds the executor setup, above this crate.
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:151 ExecutionTool.<init>
    pub execution_tool: &'a dyn Fn() -> Result<(), DetailedExitCode>,
}

/// Java's `BuildTool.buildTargetsWithoutMergedAnalysisExecution` constructs the
/// `ExecutionTool` after the analysis and before `analysisPostProcessor.process` (it sets up the
/// tools' symlinks), for cquery and aquery too; an `AbruptExitException` from its constructor
/// ends the request (`BuildTool.processRequest` reports its message).
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:520 BuildTool.buildTargetsWithoutMergedAnalysisExecution
struct ExecutionToolConstruction<'a> {
    execution_tool: &'a dyn Fn() -> Result<(), DetailedExitCode>,
    post_processor: &'a dyn blaze_sky_top::build_tool::AnalysisPostProcessor,
}

impl blaze_sky_top::build_tool::AnalysisPostProcessor for ExecutionToolConstruction<'_> {
    fn process(
        &self,
        request: &blaze_sky_top::build_tool::BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        analysis_result: &blaze_sky_top::build_view::AnalysisResult,
    ) -> Result<(), blaze_sky_top::build_tool::BuildToolError> {
        // executionTool = new ExecutionTool(env, request);
        // PORT: Java's `catch (AbruptExitException e)` also sets the BuildResult's catastrophe
        // flag; the post-analysis commands keep no BuildResult here (their stopRequest prints
        // the summary from the exit code alone).
        (self.execution_tool)().map_err(blaze_sky_top::build_tool::BuildToolError::Exit)?;
        self.post_processor
            .process(request, skyframe_executor, analysis_result)
    }
}

/// `new BuildTool(env, processor).processRequest(request, null, options).getDetailedExitCode()`:
/// `BuildTool.processRequest` with the `stopRequest` of its `finally` block.
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:892 BuildTool.processRequest
pub(crate) fn process_request(
    env: &PostAnalysisCommandEnvironment<'_>,
    request: &blaze_sky_top::build_tool::BuildRequest,
    post_processor: &dyn blaze_sky_top::build_tool::AnalysisPostProcessor,
) -> DetailedExitCode {
    let post_processor = ExecutionToolConstruction {
        execution_tool: env.execution_tool,
        post_processor,
    };
    let detailed_exit_code = blaze_sky_top::build_tool::BuildTool::new(
        env.skyframe_executor,
        env.reporter.clone(),
        env.relative_working_directory.clone(),
        &post_processor,
    )
    .process_request(request);
    (env.stop_request)(&detailed_exit_code);
    detailed_exit_code
}

/// The main repository mapping as the post-analysis commands compute it, or the command's
/// result.
// Java: com/google/devtools/build/lib/runtime/commands/CqueryCommand.java:120 CqueryCommand.exec
pub(crate) fn main_repo_target_parser(
    env: &PostAnalysisCommandEnvironment<'_>,
) -> Result<blaze_cmdline::TargetPatternParser, DetailedExitCode> {
    let keep_going = env
        .options
        .get_options::<KeepGoingOption>()
        .expect("KeepGoingOption")
        .keep_going;
    let threads = env
        .options
        .get_options::<LoadingPhaseThreadsOption>()
        .expect("LoadingPhaseThreadsOption")
        .threads;
    match env.skyframe_executor.get_main_repo_mapping(
        keep_going,
        threads.max(1) as usize,
        env.reporter.clone(),
    ) {
        Ok(repo_mapping) => Ok(blaze_cmdline::TargetPatternParser::new(
            env.relative_working_directory.clone(),
            blaze_cmdline::repository_name::MAIN.clone(),
            repo_mapping,
        )),
        Err(RepositoryMappingResolutionOrInterrupted::Resolution(e)) => {
            env.reporter.handle(&Event::error(e.to_string()));
            Err(failure_details::from_packages(&e.detailed_exit_code))
        }
        Err(RepositoryMappingResolutionOrInterrupted::Interrupted) => {
            // InterruptedException.getMessage() is null here.
            let error_message = "Fetch interrupted: null";
            env.reporter.handle(&Event::error(error_message));
            Err(DetailedExitCode::of_exit_code(
                INTERRUPTED_EXIT_CODE,
                FailureDetail {
                    message: error_message.to_owned(),
                    category: Some(Category::Interrupted(Interrupted {
                        code: interrupted::Code::Interrupted as i32,
                    })),
                },
            ))
        }
    }
}

/// Reads the query for query, cquery and aquery using the `--query_file` option or from the
/// residue of the command line.
// Java: com/google/devtools/build/lib/runtime/commands/QueryOptionHelper.java:35 QueryOptionHelper.readQuery
pub(crate) fn read_query(
    query_options: &dyn CommonQueryOptionsView,
    env: &PostAnalysisCommandEnvironment<'_>,
    allow_empty_query: bool,
) -> Result<String, QueryException> {
    let mut query = String::new();
    let residue = env.options.get_residue();
    if !residue.is_empty() {
        if !query_options.query_file().is_empty() {
            return Err(QueryException::new(
                None,
                "Command-line query and --query_file cannot both be specified",
                query::Code::QueryFileWithCommandLineExpression,
            ));
        }
        query = residue.join(" ");
    } else if !query_options.query_file().is_empty() {
        // Works for absolute or relative query file.
        let residue_path = env
            .working_directory
            .get_relative(&PathFragment::create(query_options.query_file()));
        // PORT: skipped, Google-internal: InputFileEvent ("query_file", size) has no subscriber
        // in Bazel.
        // FileSystemUtils.readContent(residuePath): the whole file through the VFS.
        let content = residue_path.get_input_stream().and_then(|mut stream| {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut stream, &mut bytes).map(|_| bytes)
        });
        match content {
            // new String(bytes, ISO_8859_1): the internal string, one char per byte.
            Ok(bytes) => query = latin1_decode(&bytes).into_owned(),
            Err(_) => {
                return Err(QueryException::new(
                    None,
                    format!("I/O error reading from {}", residue_path.get_path_string()),
                    query::Code::QueryFileReadFailure,
                ));
            }
        }
    } else {
        // When querying for the state of Skyframe, it's possible to omit the query expression.
        if !allow_empty_query {
            return Err(QueryException::new(
                None,
                format!(
                    "missing query expression. Type '{} help query' for syntax and help",
                    env.product_name
                ),
                query::Code::CommandLineExpressionMissing,
            ));
        }
    }
    Ok(query)
}
