//! Ports of `runtime/commands/QueryCommand.java` (`doQuery`) and the query-specific part of
//! `runtime/commands/QueryEnvironmentBasedCommand.java` (`execInternal` after package loading is
//! synced, `newQueryEnvironment`, and `QueryEnvironmentFactory.create`), plus
//! `QueryOptionHelper.readQuery`.
//!
//! PORT: blaze_runtime's `CommandEnvironment` does not own a Skyframe executor yet, so the
//! command takes what it reads from the environment as a [`QueryCommandEnvironment`]; the
//! result is the `DetailedExitCode` of Java's `BlazeCommandResult`. The UI-only steps
//! (`PackageProgressReceiver.reset`, `LoadingPhaseStartedEvent`,
//! `switchToAnsiAllowingHandler`) are not ported here: they belong to the UI.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use blaze_cmdline::{RepositoryMapping, TargetPatternParser};
use blaze_events::{Event, EventHandler, ExtendedEventHandler};
use blaze_optdefs::command::query_options::QueryOptions;
use blaze_options::TriState;
use blaze_packages::target::{LabelPrinter, Target};
use blaze_protos::failure_details::{
    FailureDetail, Interrupted, Query, failure_detail::Category, interrupted, query,
};
use blaze_query_engine::callback::IoException;
use blaze_query_engine::query_util::{AggregateAllCallback, AggregateAllOutputFormatterCallback};
use blaze_query_engine::{
    DetailedExitCode, QueryEvalResult, QueryException, QueryExpression, QueryFunction,
    ThreadSafeOutputFormatterCallback,
};
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::{DigestHashFunction, PathFragment};

use crate::common::abstract_blaze_query_environment::{
    AbstractBlazeQueryEnvironment, EvaluateQueryError, LabelFilter,
};
use crate::common::query_util_targets::{
    LexicographicallySortedTargetAggregator, new_lexicographically_sorted_target_aggregator,
};
use crate::pkgcache::{
    CachingPackageLocator, QueryTransitivePackagePreloader, TargetPatternPreloader, TargetProvider,
};
use crate::query::aspectresolvers::aspect_resolver;
use crate::query::graphless_blaze_query_environment::GraphlessBlazeQueryEnvironment;
use crate::query::output::output_formatter::{OutputFormatter, OutputStream};
use crate::query::output::output_formatters;
use crate::query::output::query_options as query_options_util;
use crate::query::output::query_output_utils;

/// `ExitCode.PARTIAL_ANALYSIS_FAILURE`.
pub const PARTIAL_ANALYSIS_FAILURE: i32 = 3;
/// `ExitCode.ANALYSIS_FAILURE`.
pub const ANALYSIS_FAILURE: i32 = 7;
/// `(metadata).exit_code` of `Interrupted.Code.INTERRUPTED`.
const INTERRUPTED_EXIT_CODE: i32 = 8;

/// What the query command reads from Java's `CommandEnvironment` and `BlazeRuntime`.
pub struct QueryCommandEnvironment {
    /// `env.getReporter()`.
    pub reporter: Arc<dyn ExtendedEventHandler>,
    /// `queryRuntimeHelper.getOutputStreamForQueryOutput()` (stdout).
    pub output: OutputStream,
    /// `env.getRuntime().getFileSystem().getDigestFunction()`.
    pub hash_function: DigestHashFunction,
    /// `env.getSkyframeExecutor().getEffectiveStarlarkSemantics(...)`.
    pub starlark_semantics: StarlarkSemantics,
    /// `env.getRelativeWorkingDirectory()`.
    pub relative_working_directory: PathFragment,
    /// `env.getSkyframeExecutor().getMainRepoMapping(...)` (computed by the caller after
    /// `syncPackageLoading`; its failures are reported there).
    pub main_repo_mapping: RepositoryMapping,
    /// `env.getRuntime().getQueryFunctions()`.
    pub query_functions: Vec<Arc<dyn QueryFunction<Target>>>,
    /// `TargetProviderForQueryEnvironment`.
    pub target_provider: Arc<dyn TargetProvider>,
    /// `env.getPackageManager()` as a `CachingPackageLocator`.
    pub caching_package_locator: Arc<dyn CachingPackageLocator>,
    /// `new SkyframeTargetPatternEvaluator(env.getSkyframeExecutor())`.
    pub target_pattern_preloader: Arc<dyn TargetPatternPreloader>,
    /// `env.getSkyframeExecutor().getQueryTransitivePackagePreloader()`.
    pub query_transitive_package_preloader: Option<Arc<dyn QueryTransitivePackagePreloader>>,
}

/// Java `QueryRuntimeHelper` writing to stdout: nothing to do after the output.
fn after_query_output_is_written() {}

fn failure(message: impl Into<String>, code: query::Code) -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        message: message.into(),
        category: Some(Category::Query(Query { code: code as i32 })),
    })
}

/// Java `InterruptedFailureDetails.detailedExitCode(message)`.
// Java: com/google/devtools/build/lib/util/InterruptedFailureDetails.java:30 InterruptedFailureDetails.detailedExitCode
fn interrupted_detailed_exit_code(message: &str) -> DetailedExitCode {
    DetailedExitCode::of_exit_code(
        INTERRUPTED_EXIT_CODE,
        FailureDetail {
            message: message.to_owned(),
            category: Some(Category::Interrupted(Interrupted {
                code: interrupted::Code::Interrupted as i32,
            })),
        },
    )
}

/// Reads the query expression from the residue or `--query_file`.
// Java: com/google/devtools/build/lib/query2/common/QueryOptionHelper.java:33 QueryOptionHelper.readQuery
pub fn read_query(
    query_options: &QueryOptions,
    residue: &[String],
    allow_empty_query: bool,
) -> Result<String, QueryException> {
    let mut query = String::new();
    if !residue.is_empty() {
        if !query_options.query_file.is_empty() {
            return Err(QueryException::new(
                None,
                "Command-line query and --query_file cannot both be specified",
                query::Code::QueryFileWithCommandLineExpression,
            ));
        }
        query = residue.join(" ");
    } else if !query_options.query_file.is_empty() {
        blaze_util::unported!("QueryOptionHelper.readQuery", "--query_file");
    } else if !allow_empty_query {
        return Err(QueryException::new(
            None,
            // env.getRuntime().getProductName()
            format!(
                "missing query expression. Type '{}' for syntax and help",
                "bazel help query"
            ),
            query::Code::CommandLineExpressionMissing,
        ));
    }
    Ok(query)
}

/// The query part of `QueryEnvironmentBasedCommand.execInternal`, after `syncPackageLoading` and
/// the main repository mapping.
// Java: com/google/devtools/build/lib/runtime/commands/QueryEnvironmentBasedCommand.java:96 QueryEnvironmentBasedCommand.execInternal
pub fn exec_internal(
    env: &QueryCommandEnvironment,
    query_options: &QueryOptions,
    keep_going: bool,
    loading_phase_threads: i32,
    residue: &[String],
) -> DetailedExitCode {
    let main_repo_target_parser = TargetPatternParser::new(
        env.relative_working_directory.clone(),
        blaze_cmdline::repository_name::MAIN.clone(),
        env.main_repo_mapping.clone(),
    );

    let query = match read_query(query_options, residue, /* allow_empty_query= */ false) {
        Ok(query) => query,
        Err(e) => {
            return DetailedExitCode::of(e.get_failure_detail().clone());
        }
    };

    let formatters = output_formatters::get_default_formatters();
    let formatter =
        match output_formatters::get_formatter(&formatters, &query_options.output_format) {
            Some(formatter) => formatter,
            None => {
                let message = format!(
                    "Invalid output format '{}'. Valid values are: {}",
                    query_options.output_format,
                    output_formatters::formatter_names(&formatters)
                );
                env.reporter.handle(&Event::error(message.clone()));
                return failure(message, query::Code::OutputFormatInvalid);
            }
        };
    let settings = query_options_util::to_settings(query_options);
    let stream_results = query_output_utils::should_stream_results(query_options, formatter);
    let use_graphless_query = query_options.use_graphless_query == TriState::YES
        || (query_options.use_graphless_query == TriState::AUTO && stream_results);
    if use_graphless_query && !stream_results {
        let message = format!(
            "--experimental_graphless_query requires --order_output=no or --order_output=auto and \
             an --output option that supports streaming; valid values are: {}",
            output_formatters::streaming_formatter_names(&formatters)
        );
        env.reporter.handle(&Event::error(message.clone()));
        return failure(message, query::Code::GraphlessPrereqUnmet);
    }

    let label_printer = query_options_util::get_label_printer(
        query_options,
        &env.starlark_semantics,
        main_repo_target_parser.get_repo_mapping().clone(),
    );

    // PORT: env.getSkyframeExecutor().maybeGetHardcodedUniverseScope() is empty for Bazel.
    let universe_scope_is_empty =
        query_options.universe_scope.is_empty() && !query_options.infer_universe_scope;
    let query_env = new_query_environment(
        env,
        keep_going,
        /* ordered_results= */ !stream_results,
        universe_scope_is_empty,
        loading_phase_threads,
        settings,
        use_graphless_query,
        main_repo_target_parser,
        label_printer,
    );
    let result = do_query(
        &query,
        env,
        query_options,
        stream_results,
        formatter,
        &*query_env,
    );
    query_env.close();
    match result {
        Err(detailed_exit_code) => detailed_exit_code,
        Ok(query_eval_result) => {
            if query_eval_result.is_empty() {
                env.reporter.handle(&Event::info("Empty results"));
            }
            after_query_output_is_written();
            if query_eval_result.get_success() {
                return DetailedExitCode::success();
            }
            DetailedExitCode::of_exit_code(
                PARTIAL_ANALYSIS_FAILURE,
                query_eval_result
                    .get_detailed_exit_code()
                    .get_failure_detail()
                    .cloned()
                    .expect("failed query result without FailureDetail"),
            )
        }
    }
}

/// Java `QueryEnvironmentBasedCommand.newQueryEnvironment` with
/// `QueryEnvironmentFactory.create`.
// Java: com/google/devtools/build/lib/runtime/commands/QueryEnvironmentBasedCommand.java:232 QueryEnvironmentBasedCommand.newQueryEnvironment
fn new_query_environment(
    env: &QueryCommandEnvironment,
    keep_going: bool,
    ordered_results: bool,
    universe_scope_is_empty: bool,
    loading_phase_threads: i32,
    settings: std::collections::HashSet<blaze_query_engine::Setting>,
    use_graphless_query: bool,
    main_repo_target_parser: TargetPatternParser,
    label_printer: LabelPrinter,
) -> Box<dyn AbstractBlazeQueryEnvironment> {
    // Java: com/google/devtools/build/lib/query2/QueryEnvironmentFactory.java:41 QueryEnvironmentFactory.create
    // canUseSkyQuery(orderedResults, universeScope, packagePath, strictScope=true, ALL_LABELS)
    if !ordered_results && !universe_scope_is_empty {
        blaze_util::unported!("SkyQueryEnvironment");
    } else if use_graphless_query {
        Box::new(GraphlessBlazeQueryEnvironment::new(
            env.query_transitive_package_preloader.clone(),
            env.target_provider.clone(),
            env.caching_package_locator.clone(),
            env.target_pattern_preloader.clone(),
            main_repo_target_parser,
            keep_going,
            /* strict_scope= */ true,
            loading_phase_threads,
            /* label_filter= */ LabelFilter::AllLabels,
            env.reporter.clone(),
            settings,
            env.query_functions.clone(),
            label_printer,
        ))
    } else {
        blaze_util::unported!("BlazeQueryEnvironment", "needs lib/graph Digraph")
    }
}

/// The query command, which evaluates a query and outputs its result.
// Java: com/google/devtools/build/lib/runtime/commands/QueryCommand.java:89 QueryCommand.doQuery
pub fn do_query(
    query: &str,
    env: &QueryCommandEnvironment,
    query_options: &QueryOptions,
    stream_results: bool,
    formatter: &dyn OutputFormatter,
    query_env: &dyn AbstractBlazeQueryEnvironment,
) -> Result<QueryEvalResult, DetailedExitCode> {
    let mut expr = match QueryExpression::parse(query, query_env.as_query_environment()) {
        Ok(expr) => expr,
        Err(e) => {
            let message = format!(
                "Error while parsing '{}': {}",
                QueryExpression::truncate(query),
                e.get_message()
            );
            env.reporter.handle(&Event::error_at(None, message));
            return Err(failure(e.get_message(), query::Code::SyntaxError));
        }
    };

    if let Err(e) = formatter.verify_compatible(query_env, &expr) {
        env.reporter
            .handle(&Event::error(e.get_message().to_owned()));
        return Err(DetailedExitCode::of(e.get_failure_detail().clone()));
    }

    expr = query_env.transform_parsed_query(expr);

    let lexicographically_sort_output =
        query_output_utils::lexicographically_sort_output(query_options, formatter)
            && !expr.is_top_level_some_path_function();

    // PORT: formatter.canBeBuffered() wraps the stream in a 16 KiB BufferedOutputStream; the
    // bytes written are the same.
    let out: OutputStream = env.output.clone();

    let callback: QueryCallback = if stream_results {
        // disableAnsiCharactersFiltering(env): PORT: UI, see the module documentation.
        let streamed_formatter = formatter
            .as_streamed_formatter()
            .expect("StreamedFormatter");
        streamed_formatter.set_options(
            query_options,
            aspect_resolver::create_resolver(query_options.aspect_deps),
            env.hash_function,
        );
        streamed_formatter.set_event_handler(Some(event_handler_arc(env)));
        if lexicographically_sort_output {
            QueryCallback::Lexicographic(new_lexicographically_sorted_target_aggregator())
        } else {
            QueryCallback::Streamed(streamed_formatter.create_stream_callback(
                out.clone(),
                query_options,
                query_env,
            ))
        }
    } else {
        QueryCallback::Ordered(
            blaze_query_engine::query_util::new_ordered_aggregate_all_output_formatter_callback(
                query_env.as_query_environment(),
            ),
        )
    };

    let result = match query_env.evaluate_query(&expr, callback.as_callback()) {
        Ok(result) => result,
        Err(EvaluateQueryError::Query(e)) => {
            env.reporter
                .handle(&Event::error(e.get_message().to_owned()));
            return Err(finalize_blaze_command_result(ANALYSIS_FAILURE, &e));
        }
        Err(EvaluateQueryError::Interrupted(_)) => {
            let io_exception = callback.as_callback().get_io_exception();
            return Err(match io_exception {
                // PORT: ClosedByInterruptException cannot occur here.
                None => report_and_create_interrupted_result(env),
                Some(io_exception) => {
                    report_and_create_io_exception_result(env, &io_exception.to_string())
                }
            });
        }
        Err(EvaluateQueryError::Io(e)) => {
            return Err(report_and_create_io_exception_result(env, &e.to_string()));
        }
    };
    if let Err(e) = out.borrow_mut().flush() {
        return Err(report_and_create_flush_failure_result(env, &e.to_string()));
    }

    if !stream_results || lexicographically_sort_output {
        // ((AggregateAllOutputFormatterCallback<Target, ?>) callback).getResult()
        let targets = callback.get_result();
        let output_result = query_output_utils::output(
            query_options,
            &targets,
            formatter,
            out.clone(),
            aspect_resolver::create_resolver(query_options.aspect_deps),
            Some(event_handler_arc(env)),
            env.hash_function,
            query_env.get_label_printer(),
        );
        let flush_result = out.borrow_mut().flush();
        match output_result {
            Ok(()) => {}
            Err(blaze_query_engine::OutputFormatterError::Interrupted(_)) => {
                return Err(report_and_create_interrupted_result(env));
            }
            Err(blaze_query_engine::OutputFormatterError::Io(e)) => {
                return Err(report_and_create_io_exception_result(env, &io_message(&e)));
            }
        }
        if let Err(e) = flush_result {
            return Err(report_and_create_flush_failure_result(env, &e.to_string()));
        }
    }

    Ok(result)
}

fn io_message(e: &IoException) -> String {
    e.to_string()
}

fn event_handler_arc(env: &QueryCommandEnvironment) -> Arc<dyn EventHandler> {
    struct Forward(Arc<dyn ExtendedEventHandler>);
    impl EventHandler for Forward {
        fn handle(&self, event: &Event) {
            self.0.handle(event);
        }
    }
    Arc::new(Forward(env.reporter.clone()))
}

/// The callback `doQuery` evaluates into.
enum QueryCallback {
    Streamed(Box<dyn ThreadSafeOutputFormatterCallback<Target>>),
    Lexicographic(LexicographicallySortedTargetAggregator),
    Ordered(Box<dyn AggregateAllOutputFormatterCallback<Target, Vec<Target>>>),
}

impl QueryCallback {
    fn as_callback(&self) -> &dyn ThreadSafeOutputFormatterCallback<Target> {
        match self {
            QueryCallback::Streamed(c) => &**c,
            QueryCallback::Lexicographic(c) => c,
            QueryCallback::Ordered(c) => &**c,
        }
    }

    fn get_result(&self) -> Vec<Target> {
        match self {
            QueryCallback::Streamed(_) => {
                panic!("ClassCastException: not an AggregateAllOutputFormatterCallback")
            }
            QueryCallback::Lexicographic(c) => c.get_result(),
            QueryCallback::Ordered(c) => c.get_result(),
        }
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/QueryCommand.java:201 QueryCommand.reportAndCreateFlushFailureResult
fn report_and_create_flush_failure_result(
    env: &QueryCommandEnvironment,
    e: &str,
) -> DetailedExitCode {
    let message = format!("Failed to flush query results: {e}");
    env.reporter.handle(&Event::error(message.clone()));
    failure(message, query::Code::QueryResultsFlushFailure)
}

// Java: com/google/devtools/build/lib/runtime/commands/QueryCommand.java:213 QueryCommand.reportAndCreateInterruptedResult
fn report_and_create_interrupted_result(env: &QueryCommandEnvironment) -> DetailedExitCode {
    let message = "query interrupted";
    env.reporter.handle(&Event::error(message));
    interrupted_detailed_exit_code(message)
}

// Java: com/google/devtools/build/lib/runtime/commands/QueryCommand.java:221 QueryCommand.reportAndCreateIOExceptionResult
fn report_and_create_io_exception_result(
    env: &QueryCommandEnvironment,
    message: &str,
) -> DetailedExitCode {
    let prefixed_message = format!("I/O error: {message}");
    env.reporter.handle(&Event::error(prefixed_message.clone()));
    failure(prefixed_message, query::Code::OutputFormatterIoException)
}

// Java: com/google/devtools/build/lib/runtime/commands/QueryCommand.java:233 QueryCommand.finalizeBlazeCommandResult
fn finalize_blaze_command_result(exit_code: i32, e: &QueryException) -> DetailedExitCode {
    DetailedExitCode::of_exit_code(exit_code, e.get_failure_detail().clone())
}

/// A shared, in-memory output stream (for tests and the dev binary).
pub fn new_output_stream<W: std::io::Write + 'static>(w: W) -> OutputStream {
    Rc::new(RefCell::new(w))
}
