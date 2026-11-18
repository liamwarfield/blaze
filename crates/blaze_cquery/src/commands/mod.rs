//! The bodies of `runtime/commands/CqueryCommand.java` and `runtime/commands/AqueryCommand.java`
//! (and the parts of `QueryOptionHelper` / `QueryCommandUtils` they use), plus the adapters the
//! `query` command uses to reach the executor's package manager.

pub mod aquery_command;
pub mod cquery_command;
pub mod query_adapters;

use std::sync::Arc;

use blaze_events::{Event, ExtendedEventHandler};
use blaze_optdefs::command::analysis_options::AnalysisOptions;
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
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
    /// `runtime.getStartupOptionsProvider().getOptions(BlazeServerStartupOptions.class).batch`.
    pub batch: bool,
    /// The command's options.
    pub options: &'a OptionsParser,
}

/// `env.getSkyframeExecutor().tracksStateForIncrementality()`: the `trackIncrementalState`
/// `SequencedSkyframeExecutor.decideKeepIncrementalState` sets for the command.
///
/// PORT: blaze_sky_top's `SkyframeExecutor` does not keep this state (`decideKeepIncrementalState`
/// and its warnings belong to the dispatcher); the value is computed from the same options here.
// Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:399 SequencedSkyframeExecutor.decideKeepIncrementalState
pub(crate) fn tracks_state_for_incrementality(env: &PostAnalysisCommandEnvironment<'_>) -> bool {
    let should_track_incremental_state = env
        .options
        .get_options::<CommonCommandOptions>()
        .expect("CommonCommandOptions")
        .track_incremental_state;
    let discard_analysis_cache = env
        .options
        .get_options::<AnalysisOptions>()
        .expect("AnalysisOptions")
        .discard_analysis_cache;
    // First check if the incrementality state should be kept around during the build.
    let explicitly_requested_no_incremental_data = !should_track_incremental_state;
    let implicitly_requested_no_incremental_data = env.batch && discard_analysis_cache;
    !explicitly_requested_no_incremental_data && !implicitly_requested_no_incremental_data
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
        match std::fs::read(residue_path.get_path_string()) {
            // new String(bytes, ISO_8859_1): the internal string form (Latin-1 characters holding
            // the file's bytes). PORT: StringEncoding identity for UTF-8 content. A Rust String
            // cannot hold other bytes, and replacing them would change the query silently.
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(content) => query = content,
                Err(e) => blaze_util::unported!(
                    "QueryOptionHelper.readQuery",
                    "--query_file {} is not valid UTF-8 ({})",
                    residue_path.get_path_string(),
                    e.utf8_error()
                ),
            },
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
