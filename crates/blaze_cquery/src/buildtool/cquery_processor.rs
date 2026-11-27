//! Port of `buildtool/CqueryProcessor.java`.

use std::sync::Arc;

use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_value::AspectValue;
use blaze_cmdline::TargetPatternParser;
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::cquery_options::CqueryOptions;
use blaze_query::query::output::output_formatter::OutputStream;
use blaze_query_engine::QueryExpression;
use blaze_sky_top::build_tool::{AnalysisPostProcessor, BuildRequest, BuildToolError};
use blaze_sky_top::build_view::AnalysisResult;
use blaze_sky_top::skyframe_executor::SkyframeExecutor;

use crate::buildtool::post_analysis_query_processor::{
    before_post_analysis_query, do_post_analysis_query, get_transitive_configurations,
    handle_post_analysis_query_result,
};
use crate::cquery;
use crate::cquery::config_function::ConfigFunctionSlot;
use crate::cquery::configured_target_query_environment::{
    ConfiguredTargetQueryEnvironment, cquery_functions, new_configured_target_query_environment,
};
use crate::cquery::cquery_options::CommonQueryOptionsView;
use crate::post_analysis_query_environment::TopLevelConfigurations;

/// Performs `cquery` processing.
// Java: com/google/devtools/build/lib/buildtool/CqueryProcessor.java:35 CqueryProcessor
pub struct CqueryProcessor {
    query_expression: Option<QueryExpression>,
    main_repo_target_parser: TargetPatternParser,
    cquery_options: CqueryOptions,
    config_function_slot: ConfigFunctionSlot,
    reporter: Arc<dyn ExtendedEventHandler>,
    /// `env.getReporter().getOutErr().getOutputStream()` and `env.getWorkingDirectory()`, for
    /// the `QueryRuntimeHelper`.
    output: OutputStream,
    working_directory: blaze_vfs::Path,
    /// The workspace root: the server process's working directory in Bazel, which
    /// `--starlark:file` is read relative to.
    workspace: blaze_vfs::Path,
    /// `env.getSkyframeExecutor().tracksStateForIncrementality()`.
    tracks_state_for_incrementality: bool,
}

impl CqueryProcessor {
    // Java: com/google/devtools/build/lib/buildtool/CqueryProcessor.java:37 CqueryProcessor.<init>
    pub fn new(
        query_expression: Option<QueryExpression>,
        main_repo_target_parser: TargetPatternParser,
        cquery_options: CqueryOptions,
        config_function_slot: ConfigFunctionSlot,
        reporter: Arc<dyn ExtendedEventHandler>,
        output: OutputStream,
        working_directory: blaze_vfs::Path,
        workspace: blaze_vfs::Path,
        tracks_state_for_incrementality: bool,
    ) -> Self {
        CqueryProcessor {
            query_expression,
            main_repo_target_parser,
            cquery_options,
            config_function_slot,
            reporter,
            output,
            working_directory,
            workspace,
            tracks_state_for_incrementality,
        }
    }

    // Java: com/google/devtools/build/lib/buildtool/CqueryProcessor.java:48 CqueryProcessor.getQueryEnvironment
    fn get_query_environment(
        &self,
        request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        configurations: TopLevelConfigurations,
        transitive_configurations: indexmap::IndexMap<
            String,
            Arc<blaze_build_config::build_configuration_value::BuildConfigurationValue>,
        >,
        top_level_aspects: &indexmap::IndexMap<AspectKey, Arc<AspectValue>>,
        walkable_graph: Arc<dyn blaze_skyframe::WalkableGraph>,
    ) -> ConfiguredTargetQueryEnvironment {
        // PORT: env.getRuntime().getQueryFunctions() is empty for Bazel.
        let extra_functions = cquery_functions(&self.config_function_slot);
        let starlark_semantics = skyframe_executor.get_effective_starlark_semantics();
        new_configured_target_query_environment(
            request.keep_going,
            self.reporter.clone(),
            extra_functions,
            configurations,
            transitive_configurations,
            top_level_aspects,
            self.main_repo_target_parser.clone(),
            skyframe_executor.get_package_path().clone(),
            walkable_graph,
            &self.cquery_options,
            self.cquery_options.get_label_printer(
                starlark_semantics,
                self.main_repo_target_parser.get_repo_mapping().clone(),
            ),
        )
    }
}

impl AnalysisPostProcessor for CqueryProcessor {
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
            analysis_result.get_aspects_map(),
            walkable_graph,
        );
        // env.getRuntime().getQueryRuntimeHelperFactory().create(env, getQueryOptions(env))
        let query_output = super::post_analysis_query_processor::create_query_runtime_helper(
            &self.output,
            &self.working_directory,
            &self.cquery_options.output_file,
        )?;
        // PORT: ConfigFunction reaches the environment through this slot (see
        // config_function.rs).
        *self.config_function_slot.write() = Some(env.graph_lookup());
        let result = match cquery::get_default_output_formatters(
            &env,
            &self.cquery_options,
            self.reporter.clone(),
            query_output,
            skyframe_executor,
            request.get_top_level_artifact_context_output_groups(),
            &self.workspace,
        ) {
            Ok(callbacks) => {
                let result = do_post_analysis_query(
                    &env,
                    &callbacks,
                    &self.cquery_options.output_format,
                    query_expression,
                    &self.reporter,
                );
                drop(callbacks);
                result
            }
            // getDefaultOutputFormatters throws QueryException inside doPostAnalysisQuery.
            Err(e) => Err(super::post_analysis_query_processor::PostAnalysisQueryError::Query(e)),
        };
        *self.config_function_slot.write() = None;
        handle_post_analysis_query_result(result, request.keep_going, &self.reporter)
    }
}
