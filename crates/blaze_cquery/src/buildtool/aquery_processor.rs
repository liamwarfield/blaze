//! Port of `buildtool/AqueryProcessor.java` (the post-analysis query path and `--skyframe_state`)
//! and `SequencedSkyframeExecutor.dumpSkyframeState`.

use std::sync::Arc;

use blaze_actions::skyframe::actiongraph::v2::action_graph_dump::{
    ActionGraphDump, ActionGraphDumpError, DumpedTarget,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_protos::failure_details::{ActionQuery, FailureDetail, action_query, failure_detail};

use blaze_cmdline::TargetPatternParser;
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::aquery_options::AqueryOptions;
use blaze_query::query::output::output_formatter::OutputStream;
use blaze_query_engine::regex_filter_expression::compile_java_pattern;
use blaze_query_engine::{ArgumentType, FunctionExpression, QueryExpression};
use blaze_sky_top::build_tool::{AnalysisPostProcessor, BuildRequest, BuildToolError};
use blaze_sky_top::build_view::AnalysisResult;
use blaze_sky_top::skyframe_executor::SkyframeExecutor;

use crate::aquery;
use crate::aquery::action_graph_query_environment::{
    ActionGraphQueryEnvironment, aquery_functions, new_action_graph_query_environment,
};
use crate::aquery::aquery_action_filter::AqueryActionFilter;
use crate::buildtool::post_analysis_query_processor::{
    before_post_analysis_query, do_post_analysis_query, get_transitive_configurations,
    handle_post_analysis_query_result,
};
use crate::cquery::cquery_options::CommonQueryOptionsView;
use crate::post_analysis_query_environment::TopLevelConfigurations;

/// Custom exception class for aquery filtering.
// Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:251 AqueryProcessor.AqueryActionFilterException
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct AqueryActionFilterException(pub String);

/// Performs `aquery` processing.
// Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:62 AqueryProcessor
pub struct AqueryProcessor {
    query_expression: Option<QueryExpression>,
    main_repo_target_parser: TargetPatternParser,
    action_filters: AqueryActionFilter,
    aquery_options: AqueryOptions,
    reporter: Arc<dyn ExtendedEventHandler>,
    /// `env.getReporter().getOutErr().getOutputStream()` and `env.getWorkingDirectory()`, for
    /// the `QueryRuntimeHelper`.
    output: OutputStream,
    working_directory: blaze_vfs::Path,
    /// `env.getSkyframeExecutor().tracksStateForIncrementality()`.
    tracks_state_for_incrementality: bool,
}

impl AqueryProcessor {
    // Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:65 AqueryProcessor.<init>
    pub fn new(
        query_expression: Option<QueryExpression>,
        main_repo_target_parser: TargetPatternParser,
        aquery_options: AqueryOptions,
        reporter: Arc<dyn ExtendedEventHandler>,
        output: OutputStream,
        working_directory: blaze_vfs::Path,
        tracks_state_for_incrementality: bool,
    ) -> Result<Self, AqueryActionFilterException> {
        let action_filters = build_action_filters(query_expression.as_ref())?;
        Ok(AqueryProcessor {
            query_expression,
            main_repo_target_parser,
            action_filters,
            aquery_options,
            reporter,
            output,
            working_directory,
            tracks_state_for_incrementality,
        })
    }

    /// Outputs the current action graph from Skyframe.
    // Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:78 AqueryProcessor.dumpActionGraphFromSkyframe
    pub fn dump_action_graph_from_skyframe(
        &self,
        skyframe_executor: &SkyframeExecutor,
    ) -> blaze_query_engine::DetailedExitCode {
        let aquery_options = &self.aquery_options;
        // env.getRuntime().getQueryRuntimeHelperFactory().create(env, aqueryOptions)
        let output = match blaze_query::query::output::query_runtime_helper::create(
            &self.output,
            &self.working_directory,
            &aquery_options.output_file,
        ) {
            Ok(output) => output,
            Err(e) => {
                self.reporter
                    .handle(&blaze_events::Event::error(e.get_message().to_owned()));
                return blaze_query_engine::DetailedExitCode::of(e.get_failure_detail());
            }
        };

        let output_type = match blaze_actions::skyframe::actiongraph::v2::aquery_output_handler::OutputType::from_string(
            &aquery_options.output_format,
        ) {
            Ok(t) => t,
            Err(e) => {
                let message = format!(
                    "--skyframe_state must be used with --output=proto|textproto|jsonproto. {e}"
                );
                self.reporter
                    .handle(&blaze_events::Event::error(message.clone()));
                return get_failure_result(&message, action_query::Code::SkyframeStatePrereqUnmet);
            }
        };
        let mut aquery_output_handler =
            crate::aquery::action_graph_proto_output_formatter_callback::construct_aquery_output_handler(
                output_type,
                output.clone(),
            );
        let result = (|| -> Result<(), ActionGraphDumpError> {
            let mut action_graph_dump = ActionGraphDump::new(
                aquery_options.include_commandline,
                aquery_options.include_artifacts,
                aquery_options.include_pruned_inputs,
                Some(&self.action_filters),
                aquery_options.include_param_files,
                aquery_options.include_file_write_contents,
                &mut *aquery_output_handler,
            );
            dump_action_graph(skyframe_executor, &mut action_graph_dump)?;
            drop(action_graph_dump);
            aquery_output_handler.close()?;
            Ok(())
        })();
        let flushed = output.borrow_mut().flush();
        match result.and(flushed.map_err(ActionGraphDumpError::Io)) {
            Ok(()) => blaze_query_engine::DetailedExitCode::success(),
            Err(ActionGraphDumpError::CommandLineExpansion(m)) => {
                let message = format!("Error while parsing command: {m}");
                self.reporter
                    .handle(&blaze_events::Event::error(message.clone()));
                get_failure_result(&message, action_query::Code::CommandLineExpansionFailure)
            }
            Err(ActionGraphDumpError::TemplateExpansion(m)) => {
                let message = format!("Error while expanding template: {m}");
                self.reporter
                    .handle(&blaze_events::Event::error(message.clone()));
                get_failure_result(&message, action_query::Code::TemplateExpansionFailure)
            }
            Err(ActionGraphDumpError::Io(e)) => {
                let message = format!("Error while emitting output: {e}");
                self.reporter
                    .handle(&blaze_events::Event::error(message.clone()));
                get_failure_result(&message, action_query::Code::OutputFailure)
            }
            Err(ActionGraphDumpError::Interrupted) => blaze_util::unported!(
                "AqueryProcessor.dumpActionGraphFromSkyframe",
                "InterruptedException in SequencedSkyframeExecutor.dumpSkyframeState                  (IllegalStateException: No interruption in sequenced evaluation)"
            ),
        }
    }

    // Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:146 AqueryProcessor.getQueryEnvironment
    fn get_query_environment(
        &self,
        request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        top_level_configurations: TopLevelConfigurations,
        transitive_configurations: indexmap::IndexMap<
            String,
            Arc<blaze_build_config::build_configuration_value::BuildConfigurationValue>,
        >,
        walkable_graph: Arc<dyn blaze_skyframe::WalkableGraph>,
    ) -> ActionGraphQueryEnvironment {
        // PORT: env.getRuntime().getQueryFunctions() is empty for Bazel.
        let extra_functions = aquery_functions();
        let starlark_semantics = skyframe_executor.get_effective_starlark_semantics();
        // queryEnvironment.setActionFilters(actionFilters): the filters are handed to the output
        // callbacks directly (get_default_output_formatters).
        new_action_graph_query_environment(
            request.keep_going,
            self.reporter.clone(),
            extra_functions,
            top_level_configurations,
            transitive_configurations,
            self.main_repo_target_parser.clone(),
            skyframe_executor.get_package_path().clone(),
            walkable_graph,
            &self.aquery_options,
            self.aquery_options.get_label_printer(
                starlark_semantics,
                self.main_repo_target_parser.get_repo_mapping().clone(),
            ),
        )
    }
}

/// Return the action filters in the form { inputs: <pattern>, outputs: <pattern>, ... }.
///
/// Fails if an aquery filter function is preceded by any other function types.
// Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:190 AqueryProcessor.buildActionFilters
fn build_action_filters(
    query_expression: Option<&QueryExpression>,
) -> Result<AqueryActionFilter, AqueryActionFilterException> {
    let mut action_filters_builder = AqueryActionFilter::builder();

    let Some(QueryExpression::Function(function_expression)) = query_expression else {
        return Ok(action_filters_builder.build());
    };

    let mut function_expression_optional: Option<Arc<FunctionExpression>> =
        Some(function_expression.clone());

    let mut non_aquery_filter_function_expression: Option<Arc<FunctionExpression>> = None;

    // Unwrap the function layers
    // Validate that aquery filter functions (inputs, outputs, mnemonics) are not preceded by any
    // other function types
    while let Some(function_expression) = function_expression_optional {
        let function_name = function_expression.get_function().get_name();
        // PORT: Java tests `getFunction() instanceof ActionFilterFunction`; the aquery functions
        // map holds exactly the three ActionFilterFunctions under these names.
        if is_action_filter_function(&function_name) {
            if let Some(non_aquery) = &non_aquery_filter_function_expression {
                return Err(AqueryActionFilterException(format!(
                    "aquery filter functions (inputs, outputs, mnemonic) produce actions, and \
                     therefore can't be the input of other function types: {}",
                    non_aquery.get_function().get_name()
                )));
            }

            let pattern_string = function_expression.get_args()[0].get_word();
            match compile_java_pattern(pattern_string) {
                Ok(pattern) => {
                    action_filters_builder.put(&function_name, pattern);
                }
                Err(e) => {
                    return Err(AqueryActionFilterException(format!(
                        "Wrong query syntax: {}",
                        e
                    )));
                }
            }
        } else {
            non_aquery_filter_function_expression = Some(function_expression.clone());
        }

        function_expression_optional = get_next_function_expression(&function_expression);
    }

    Ok(action_filters_builder.build())
}

fn is_action_filter_function(name: &str) -> bool {
    matches!(name, "inputs" | "outputs" | "mnemonic")
}

/// Unwrap input `function_expression` to get the next FunctionExpression in the query.
// Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:239 AqueryProcessor.getNextFunctionExpression
fn get_next_function_expression(
    function_expression: &FunctionExpression,
) -> Option<Arc<FunctionExpression>> {
    for arg in function_expression.get_args() {
        if arg.get_type() == ArgumentType::Expression {
            if let QueryExpression::Function(f) = arg.get_expression() {
                return Some(f.clone());
            }
        }
    }
    None
}

impl AnalysisPostProcessor for AqueryProcessor {
    // Java: com/google/devtools/build/lib/buildtool/PostAnalysisQueryProcessor.java:72 PostAnalysisQueryProcessor.process
    fn process(
        &self,
        request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        analysis_result: &AnalysisResult,
    ) -> Result<(), BuildToolError> {
        let Some(query_expression) = &self.query_expression else {
            return Ok(());
        };
        before_post_analysis_query(self.tracks_state_for_incrementality)?;

        // doPostAnalysisQuery
        let walkable_graph = skyframe_executor.get_walkable_graph();
        let transitive_configurations = get_transitive_configurations(
            &skyframe_executor.get_transitive_configuration_keys(),
            &*walkable_graph,
        );
        let env = self.get_query_environment(
            request,
            skyframe_executor,
            TopLevelConfigurations::new(analysis_result.get_top_level_targets_with_configs()),
            transitive_configurations,
            walkable_graph,
        );
        // env.getRuntime().getQueryRuntimeHelperFactory().create(env, getQueryOptions(env))
        let query_output = super::post_analysis_query_processor::create_query_runtime_helper(
            &self.output,
            &self.working_directory,
            &self.aquery_options.output_file,
        )?;
        let callbacks = aquery::get_default_output_formatters(
            &env,
            &self.aquery_options,
            self.reporter.clone(),
            query_output,
            &self.action_filters,
        );
        let result = do_post_analysis_query(
            &env,
            &callbacks,
            &self.aquery_options.output_format,
            query_expression,
            &self.reporter,
        );
        handle_post_analysis_query_result(result, request.keep_going, &self.reporter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_expression_has_no_filters() {
        let filters = build_action_filters(None).unwrap();
        assert!(!filters.has_filter_for_function("mnemonic"));
    }
}

// Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:257 AqueryProcessor.getFailureResult
fn get_failure_result(
    message: &str,
    detailed_code: action_query::Code,
) -> blaze_query_engine::DetailedExitCode {
    blaze_query_engine::DetailedExitCode::of(FailureDetail {
        message: message.to_owned(),
        category: Some(failure_detail::Category::ActionQuery(ActionQuery {
            code: detailed_code as i32,
        })),
    })
}

/// Dumps the action graph of every done configured target and aspect of the graph.
///
/// PORT: Java's `aqueryOutputHandler instanceof AqueryConsumingOutputHandler` branch
/// (`dumpSkyframeStateInParallel`) prints the same messages through a consumer thread; Blaze's
/// streamed handlers print on the calling thread (see `StreamedConsumingOutputHandler`), so both
/// branches are `dumpSkyframeState`.
// Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:131 AqueryProcessor.dumpActionGraph
pub fn dump_action_graph(
    skyframe_executor: &SkyframeExecutor,
    action_graph_dump: &mut ActionGraphDump<'_>,
) -> Result<(), ActionGraphDumpError> {
    dump_skyframe_state(skyframe_executor, action_graph_dump)
}

// Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:631 SequencedSkyframeExecutor.dumpSkyframeState
fn dump_skyframe_state(
    skyframe_executor: &SkyframeExecutor,
    action_graph_dump: &mut ActionGraphDump<'_>,
) -> Result<(), ActionGraphDumpError> {
    let evaluator = skyframe_executor.get_evaluator();
    // PORT: java-hash-order, not reproduced. Java iterates memoizingEvaluator.getDoneValues(),
    // the entries of the in-memory graph's ConcurrentHashMap in its table order, which depends
    // on the table's size (every key of the graph, past commands included) and on the hashCode
    // of every key class; Blaze's graph is hashed with RandomState. So the keys that can be
    // dumped (configured targets and aspects) are visited sorted by their Java toString (their
    // Debug form), which makes the order of the messages and their ids the same for every run
    // of the same graph. The order and ids differ from Bazel's when several targets have actions.
    let mut keys: Vec<(String, blaze_skyframe::SkyKey)> = evaluator
        .get_all_keys_for_testing()
        .into_iter()
        .filter(|key| {
            key.downcast::<blaze_analysis::configured_target_key::ConfiguredTargetKey>()
                .is_some()
                || key
                    .downcast::<blaze_analysis::aspect_key_creator::AspectKey>()
                    .is_some()
        })
        .map(|key| (format!("{key:?}"), key))
        .collect();
    keys.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, key) in keys {
        let Some(sky_value) = evaluator.get_existing_value(&key) else {
            // The skyValue may be null in case analysis of the previous build failed.
            continue;
        };
        if let Some(configured_target_value) =
            sky_value
                .downcast_ref::<blaze_analysis::configured_target_value::ConfiguredTargetValue>()
        {
            let configured_target = configured_target_value.get_configured_target();
            // RuleConfiguredTargetValue
            if !matches!(configured_target, ConfiguredTarget::Rule(_)) {
                continue;
            }
            // Only dumps the value for non-delegating keys.
            let lookup_key = configured_target
                .get_lookup_key()
                .as_any()
                .downcast_ref::<blaze_analysis::configured_target_key::ConfiguredTargetKey>(
            );
            if lookup_key.is_some()
                && lookup_key
                    == key.downcast::<blaze_analysis::configured_target_key::ConfiguredTargetKey>()
            {
                let actions: Vec<Arc<dyn blaze_actions::Action>> =
                    configured_target_value.get_actions().to_vec();
                action_graph_dump
                    .dump_configured_target(&dumped_target(configured_target), &actions)?;
            }
        } else if let Some(aspect_key) =
            key.downcast::<blaze_analysis::aspect_key_creator::AspectKey>()
        {
            let aspect_value = sky_value
                .downcast_ref::<blaze_analysis::aspect_value::AspectValue>()
                .expect("ClassCastException: AspectValue");
            let configured_target_value = evaluator
                .get_existing_value(&blaze_skyframe::SkyKey::new(
                    aspect_key.get_base_configured_target_key().clone(),
                ))
                .expect("NullPointerException: the base configured target of an aspect");
            let configured_target_value = configured_target_value
                .downcast_ref::<blaze_analysis::configured_target_value::ConfiguredTargetValue>()
                .expect("ClassCastException: ConfiguredTargetValue");
            let actions: Vec<Arc<dyn blaze_actions::Action>> = aspect_value.get_actions().to_vec();
            action_graph_dump.dump_aspect(
                Some(&actions),
                &dumped_target(configured_target_value.get_configured_target()),
            )?;
        }
    }
    Ok(())
}

/// The target `ActionGraphDump.dumpSingleAction` records for a configured target: its actual
/// target (aliases dereferenced), which must be a rule.
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/ActionGraphDump.java:143 ActionGraphDump.dumpSingleAction
fn dumped_target(configured_target: &ConfiguredTarget) -> DumpedTarget {
    let actual = configured_target.get_actual();
    let ConfiguredTarget::Rule(rule_configured_target) = &actual else {
        panic!("IllegalStateException: not a RuleConfiguredTarget");
    };
    DumpedTarget {
        label: actual.get_label().to_string(),
        rule_class_string: Some(rule_configured_target.get_rule_class_string().to_owned()),
    }
}

/// `BuildTool.dumpSkyframeStateAfterBuild` (the build command's
/// `--experimental_aquery_dump_after_build_format`).
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:774 BuildTool.dumpSkyframeStateAfterBuild
pub struct DumpSkyframeStateAfterBuild {
    pub reporter: Arc<dyn ExtendedEventHandler>,
    /// `env.getOutputBase()`.
    pub output_base: blaze_vfs::Path,
    /// `besOptions.streamingLogFileUploads` (`--experimental_stream_log_file_uploads`).
    pub streaming_log_file_uploads: bool,
}

// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:834 BuildTool.getDefaultOutputFileName
fn get_default_output_file_name(format: &str) -> &'static str {
    match format {
        "proto" => "aquery_dump.proto",
        "streamed_proto" => "aquery_dump.pb",
        "textproto" => "aquery_dump.textproto",
        "jsonproto" => "aquery_dump.json",
        _ => panic!("IllegalArgumentException: Unsupported format type: {format}"),
    }
}

impl blaze_sky_top::build_tool::AqueryDumpAfterBuild for DumpSkyframeStateAfterBuild {
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:774 BuildTool.dumpSkyframeStateAfterBuild
    fn dump_skyframe_state_after_build(
        &self,
        skyframe_executor: &SkyframeExecutor,
        _request: &BuildRequest,
        format: &str,
        output_file_path_fragment: Option<&blaze_vfs::PathFragment>,
    ) -> Result<(), String> {
        let local_output_file_path = match output_file_path_fragment {
            None => {
                let output_file_name = get_default_output_file_name(format);
                if self.streaming_log_file_uploads {
                    blaze_util::unported!(
                        "BuildTool.dumpSkyframeStateAfterBuild",
                        "--experimental_stream_log_file_uploads (BuildEventArtifactUploader \
                         streaming of {})",
                        output_file_name
                    );
                }
                self.output_base.get_relative_str(output_file_name)
            }
            Some(fragment) => self.output_base.get_relative(fragment),
        };

        self.reporter.handle(&blaze_events::Event::info(format!(
            "Writing aquery dump to {local_output_file_path}"
        )));
        // PORT: getReporter().post(new StartingAqueryDumpAfterBuildEvent(localOutputFilePath,
        // outputFileName)) is not posted: no module of Bazel subscribes to it.

        // initOutputStream(streamingContext, localOutputFilePath)
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:844 BuildTool.initOutputStream
        let output_stream = match local_output_file_path.get_output_stream() {
            Ok(s) => s,
            Err(e) => blaze_util::unported!(
                "BuildTool.dumpSkyframeStateAfterBuild",
                "PostExecutionDumpException(IOException) message for {}: {}",
                local_output_file_path,
                e
            ),
        };
        let output: OutputStream = std::rc::Rc::new(std::cell::RefCell::new(
            std::io::BufWriter::new(output_stream),
        ));
        let output_type =
            match blaze_actions::skyframe::actiongraph::v2::aquery_output_handler::OutputType::from_string(
                format,
            ) {
                Ok(t) => t,
                Err(_) => {
                    // catch (InvalidAqueryOutputFormatException e); try-with-resources closes
                    // the output stream it opened.
                    let _ = output.borrow_mut().flush();
                    return Err(
                        "--skyframe_state must be used with --output=proto|streamed_proto|textproto|jsonproto."
                            .to_owned(),
                    );
                }
            };
        let mut aquery_output_handler =
            crate::aquery::action_graph_proto_output_formatter_callback::construct_aquery_output_handler(
                output_type,
                output.clone(),
            );
        let result = (|| -> Result<(), ActionGraphDumpError> {
            // These options are fixed for simplicity. We'll add more configurability if the need
            // arises.
            let mut action_graph_dump = ActionGraphDump::new(
                /* include_action_cmd_line= */ false,
                /* include_artifacts= */ true,
                /* include_pruned_inputs= */ true,
                /* action_filters= */ None,
                /* include_param_files= */ false,
                /* include_file_write_contents= */ false,
                &mut *aquery_output_handler,
            );
            dump_action_graph(skyframe_executor, &mut action_graph_dump)?;
            drop(action_graph_dump);
            aquery_output_handler.close()?;
            Ok(())
        })();
        let flushed = output.borrow_mut().flush();
        match result.and(flushed.map_err(ActionGraphDumpError::Io)) {
            Ok(()) => Ok(()),
            Err(e) => blaze_util::unported!(
                "BuildTool.dumpSkyframeStateAfterBuild",
                "PostExecutionDumpException(e) message (Throwable.toString of the cause): {}",
                e
            ),
        }
    }
}
