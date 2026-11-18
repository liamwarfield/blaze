//! Port of `com.google.devtools.build.lib.query2.cquery`: the configured target query
//! environment, its accessor, the `config()` function and the output formatter callbacks.

pub mod config_function;
pub mod configured_target_query_environment;
pub mod cquery_options;
pub mod cquery_threadsafe_callback;
pub mod label_and_configuration_output_formatter_callback;
pub mod proto_output_formatter_callback;
pub mod unported_output_formatter_callbacks;

use std::sync::Arc;

use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::cquery_options::CqueryOptions;
use blaze_query::query::aspectresolvers::aspect_resolver;
use blaze_query::query::output::output_formatter::OutputStream;
use blaze_sky_top::skyframe_executor::SkyframeExecutor;

use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;
use configured_target_query_environment::{ConfiguredTargetQueryEnvironment, CqueryNode};
use cquery_threadsafe_callback::CqueryThreadsafeCallback;
use label_and_configuration_output_formatter_callback::LabelAndConfigurationOutputFormatterCallback;
use proto_output_formatter_callback::{OutputType, ProtoOutputFormatterCallback};
use unported_output_formatter_callbacks::UnportedOutputFormatterCallback;

/// The output formatter callbacks of cquery, in Java's order.
// Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:175 ConfiguredTargetQueryEnvironment.getDefaultOutputFormatters
pub fn get_default_output_formatters<'a>(
    env: &'a ConfiguredTargetQueryEnvironment,
    cquery_options: &'a CqueryOptions,
    event_handler: Arc<dyn ExtendedEventHandler>,
    out: OutputStream,
    skyframe_executor: &'a SkyframeExecutor,
) -> Vec<Box<dyn NamedThreadSafeOutputFormatterCallback<CqueryNode> + 'a>> {
    // cqueryOptions.aspectDeps.createResolver(packageManager, eventHandler)
    let aspect_resolver = aspect_resolver::create_resolver(cquery_options.aspect_deps);
    let base = |uniquify_results: bool| {
        CqueryThreadsafeCallback::new(
            event_handler.clone(),
            cquery_options,
            Some(out.clone()),
            skyframe_executor,
            env,
            uniquify_results,
        )
    };
    let hash_function = skyframe_executor.get_digest_function();
    let label_printer = env.get_label_printer().clone();
    vec![
        Box::new(LabelAndConfigurationOutputFormatterCallback::new(
            base(false),
            true,
            label_printer.clone(),
        )),
        Box::new(LabelAndConfigurationOutputFormatterCallback::new(
            base(false),
            false,
            label_printer.clone(),
        )),
        Box::new(UnportedOutputFormatterCallback::transitions()),
        Box::new(ProtoOutputFormatterCallback::new(
            base(false),
            aspect_resolver.clone(),
            OutputType::BINARY,
            label_printer.clone(),
            hash_function,
        )),
        Box::new(ProtoOutputFormatterCallback::new(
            base(false),
            aspect_resolver.clone(),
            OutputType::DELIMITED_BINARY,
            label_printer.clone(),
            hash_function,
        )),
        Box::new(ProtoOutputFormatterCallback::new(
            base(false),
            aspect_resolver.clone(),
            OutputType::TEXT,
            label_printer.clone(),
            hash_function,
        )),
        Box::new(ProtoOutputFormatterCallback::new(
            base(false),
            aspect_resolver,
            OutputType::JSON,
            label_printer,
            hash_function,
        )),
        Box::new(UnportedOutputFormatterCallback::build()),
        Box::new(UnportedOutputFormatterCallback::graph()),
        Box::new(UnportedOutputFormatterCallback::starlark()),
        Box::new(UnportedOutputFormatterCallback::files()),
    ]
}
