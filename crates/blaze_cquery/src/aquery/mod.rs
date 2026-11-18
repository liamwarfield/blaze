//! Port of `com.google.devtools.build.lib.query2.aquery`: the action graph query environment,
//! its accessor, action filters and output formatter callbacks.
//!
//! `AqueryUtils` (`getActionInputs`, `matchesAqueryFilters`) is ported in blaze_actions next to
//! `ActionGraphDump`, its only user.

pub mod action_graph_proto_output_formatter_callback;
pub mod action_graph_query_environment;
pub mod aquery_action_filter;

use std::sync::Arc;

use blaze_actions::skyframe::actiongraph::v2::aquery_output_handler::OutputType;
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::aquery_options::AqueryOptions;
use blaze_query::query::output::output_formatter::OutputStream;
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, OutputFormatterCallback, QueryError, ThreadSafeOutputFormatterCallback,
};

use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;
use action_graph_proto_output_formatter_callback::ActionGraphProtoOutputFormatterCallback;
use action_graph_query_environment::{ActionGraphQueryEnvironment, KeyedConfiguredTargetValue};
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
    vec![
        proto(OutputType::BINARY),
        proto(OutputType::DELIMITED_BINARY),
        proto(OutputType::TEXT),
        proto(OutputType::JSON),
        Box::new(UnportedAqueryCallback::new(
            "text",
            "ActionGraphTextOutputFormatterCallback",
        )),
        Box::new(UnportedAqueryCallback::new(
            "commands",
            "ActionGraphTextOutputFormatterCallback",
        )),
        Box::new(UnportedAqueryCallback::new(
            "summary",
            "ActionGraphSummaryOutputFormatterCallback",
        )),
    ]
}

/// An aquery output formatter whose Java class is not ported
/// (`ActionGraphTextOutputFormatterCallback`, `ActionGraphSummaryOutputFormatterCallback`).
struct UnportedAqueryCallback {
    name: &'static str,
    java_class: &'static str,
    io_exception: IoExceptionSlot,
}

impl UnportedAqueryCallback {
    fn new(name: &'static str, java_class: &'static str) -> Self {
        UnportedAqueryCallback {
            name,
            java_class,
            io_exception: IoExceptionSlot::new(),
        }
    }
}

impl NamedThreadSafeOutputFormatterCallback<KeyedConfiguredTargetValue> for UnportedAqueryCallback {
    fn get_name(&self) -> String {
        self.name.to_owned()
    }
}

impl ThreadSafeOutputFormatterCallback<KeyedConfiguredTargetValue> for UnportedAqueryCallback {}

impl Callback<KeyedConfiguredTargetValue> for UnportedAqueryCallback {
    fn process(&self, partial_result: &[KeyedConfiguredTargetValue]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<KeyedConfiguredTargetValue> for UnportedAqueryCallback {
    fn start(&self) -> Result<(), blaze_query_engine::callback::IoException> {
        blaze_util::unported!(format!("{}.start", self.java_class))
    }

    fn process_output(
        &self,
        _partial_result: &[KeyedConfiguredTargetValue],
    ) -> Result<(), OutputFormatterError> {
        blaze_util::unported!(format!("{}.processOutput", self.java_class))
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}
