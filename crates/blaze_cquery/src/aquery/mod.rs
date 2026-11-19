//! Port of `com.google.devtools.build.lib.query2.aquery`: the action graph query environment,
//! its accessor, action filters and output formatter callbacks.
//!
//! `AqueryUtils` (`getActionInputs`, `matchesAqueryFilters`) is ported in blaze_actions next to
//! `ActionGraphDump`, its only user.

pub mod action_graph_proto_output_formatter_callback;
pub mod action_graph_query_environment;
pub mod action_graph_summary_output_formatter_callback;
pub mod action_graph_text_output_formatter_callback;
pub mod aquery_action_filter;

use std::sync::Arc;

use blaze_actions::skyframe::actiongraph::v2::aquery_output_handler::OutputType;
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::aquery_options::AqueryOptions;
use blaze_query::query::output::output_formatter::OutputStream;

use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;
use action_graph_proto_output_formatter_callback::ActionGraphProtoOutputFormatterCallback;
use action_graph_query_environment::{ActionGraphQueryEnvironment, KeyedConfiguredTargetValue};
use action_graph_summary_output_formatter_callback::ActionGraphSummaryOutputFormatterCallback;
use action_graph_text_output_formatter_callback::ActionGraphTextOutputFormatterCallback;
use aquery_action_filter::AqueryActionFilter;

/// The output formatter callbacks of aquery, in Java's order.
// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphQueryEnvironment.java:149 ActionGraphQueryEnvironment.getDefaultOutputFormatters
pub fn get_default_output_formatters<'a>(
    env: &'a ActionGraphQueryEnvironment,
    aquery_options: &'a AqueryOptions,
    event_handler: Arc<dyn ExtendedEventHandler>,
    out: OutputStream,
    action_filters: &'a AqueryActionFilter,
) -> Vec<Box<dyn NamedThreadSafeOutputFormatterCallback<KeyedConfiguredTargetValue> + 'a>> {
    let proto = |output_type: OutputType| -> Box<
        dyn NamedThreadSafeOutputFormatterCallback<KeyedConfiguredTargetValue> + 'a,
    > {
        Box::new(ActionGraphProtoOutputFormatterCallback::new(
            event_handler.clone(),
            aquery_options,
            Some(out.clone()),
            env,
            output_type,
            action_filters,
        ))
    };
    let text = |output_type: action_graph_text_output_formatter_callback::OutputType| -> Box<
        dyn NamedThreadSafeOutputFormatterCallback<KeyedConfiguredTargetValue> + 'a,
    > {
        Box::new(ActionGraphTextOutputFormatterCallback::new(
            event_handler.clone(),
            aquery_options,
            out.clone(),
            env,
            output_type,
            action_filters,
        ))
    };
    vec![
        proto(OutputType::BINARY),
        proto(OutputType::DELIMITED_BINARY),
        proto(OutputType::TEXT),
        proto(OutputType::JSON),
        text(action_graph_text_output_formatter_callback::OutputType::TEXT),
        text(action_graph_text_output_formatter_callback::OutputType::COMMANDS),
        Box::new(ActionGraphSummaryOutputFormatterCallback::new(
            event_handler.clone(),
            aquery_options,
            Some(out.clone()),
            env,
            action_filters,
        )),
    ]
}
