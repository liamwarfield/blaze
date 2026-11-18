//! Ports of `query2/aquery/ActionGraphProtoOutputFormatterCallback.java` and
//! `query2/aquery/AqueryThreadsafeCallback.java`.

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::sync::Arc;

use blaze_actions::actions::action::Action;
use blaze_actions::skyframe::actiongraph::v2::action_graph_dump::{
    ActionGraphDump, ActionGraphDumpError, DumpedTarget,
};
use blaze_actions::skyframe::actiongraph::v2::aquery_output_handler::{
    AqueryOutputHandler, MonolithicOutputHandler, OutputType,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::aquery_options::AqueryOptions;
use blaze_query::query::output::output_formatter::OutputStream;
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, InterruptedException, OutputFormatterCallback, QueryError,
    ThreadSafeOutputFormatterCallback,
};

use crate::aquery::action_graph_query_environment::{
    ActionGraphQueryEnvironment, KeyedConfiguredTargetValue,
};
use crate::aquery::aquery_action_filter::AqueryActionFilter;
use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;

/// `java.io.PrintStream` over the query output stream.
pub struct PrintStream(pub OutputStream);

impl Write for PrintStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.borrow_mut().flush()
    }
}

/// Default output callback for aquery, prints proto output.
// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphProtoOutputFormatterCallback.java:48 ActionGraphProtoOutputFormatterCallback
pub struct ActionGraphProtoOutputFormatterCallback<'a> {
    // AqueryThreadsafeCallback
    // Java: com/google/devtools/build/lib/query2/aquery/AqueryThreadsafeCallback.java:24 AqueryThreadsafeCallback
    #[allow(dead_code)]
    event_handler: Arc<dyn ExtendedEventHandler>,
    options: &'a AqueryOptions,
    accessor: &'a ActionGraphQueryEnvironment,
    // ActionGraphProtoOutputFormatterCallback
    output_type: OutputType,
    action_filters: &'a AqueryActionFilter,
    aquery_output_handler: RefCell<Option<Box<dyn AqueryOutputHandler>>>,
    processed: Cell<bool>,
    io_exception: IoExceptionSlot,
}

impl<'a> ActionGraphProtoOutputFormatterCallback<'a> {
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphProtoOutputFormatterCallback.java:62 ActionGraphProtoOutputFormatterCallback.<init>
    pub fn new(
        event_handler: Arc<dyn ExtendedEventHandler>,
        options: &'a AqueryOptions,
        out: Option<OutputStream>,
        accessor: &'a ActionGraphQueryEnvironment,
        output_type: OutputType,
        action_filters: &'a AqueryActionFilter,
    ) -> Self {
        ActionGraphProtoOutputFormatterCallback {
            event_handler,
            options,
            accessor,
            output_type,
            action_filters,
            aquery_output_handler: RefCell::new(
                out.map(|out| construct_aquery_output_handler(output_type, out)),
            ),
            processed: Cell::new(false),
            io_exception: IoExceptionSlot::new(),
        }
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphProtoOutputFormatterCallback.java:133 ActionGraphProtoOutputFormatterCallback.processSingleEntry
    fn process_single_entry(
        &self,
        action_graph_dump: &mut ActionGraphDump<'_>,
        configured_target_value: &KeyedConfiguredTargetValue,
    ) -> Result<(), ActionGraphDumpError> {
        let configured_target = configured_target_value.get_configured_target();
        if !matches!(configured_target, ConfiguredTarget::Rule(_)) {
            // We have to include non-rule values in the graph to visit their dependencies, but
            // they don't have any actions to print out.
            return Ok(());
        }
        // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/ActionGraphDump.java:319 ActionGraphDump.dumpConfiguredTarget
        // Dereferencing aliases (dumpSingleAction: configuredTarget.getActual()).
        let actual = configured_target.get_actual();
        let ConfiguredTarget::Rule(rule_configured_target) = &actual else {
            panic!("IllegalStateException: not a RuleConfiguredTarget");
        };
        let dumped_target = DumpedTarget {
            label: actual.get_label().to_string(),
            rule_class_string: Some(rule_configured_target.get_rule_class_string().to_owned()),
        };
        let actions: Vec<Arc<dyn Action>> = configured_target_value.get_actions().to_vec();
        action_graph_dump.dump_configured_target(&dumped_target, &actions)?;
        if self.options.use_aspects {
            // Java: actionGraphDump.dumpAspect(aspectValue, configuredTargetValue) for each.
            for aspect_value in self.accessor.get_aspect_values(configured_target_value) {
                match aspect_value {}
            }
        }
        Ok(())
    }
}

/// Constructs the output handler of an output type.
// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphProtoOutputFormatterCallback.java:84 ActionGraphProtoOutputFormatterCallback.constructAqueryOutputHandler
pub fn construct_aquery_output_handler(
    output_type: OutputType,
    out: OutputStream,
) -> Box<dyn AqueryOutputHandler> {
    match output_type {
        OutputType::BINARY | OutputType::DELIMITED_BINARY | OutputType::TEXT => {
            Box::new(UnportedStreamedConsumingOutputHandler)
        }
        OutputType::JSON => Box::new(MonolithicOutputHandler::new(PrintStream(out))),
    }
}

/// `StreamedConsumingOutputHandler` (proto, streamed_proto and textproto output) is not ported.
struct UnportedStreamedConsumingOutputHandler;

impl AqueryOutputHandler for UnportedStreamedConsumingOutputHandler {
    fn output_artifact(&mut self, _message: blaze_protos::analysis::Artifact) {
        blaze_util::unported!("StreamedConsumingOutputHandler")
    }
    fn output_action(&mut self, _message: blaze_protos::analysis::Action) {
        blaze_util::unported!("StreamedConsumingOutputHandler")
    }
    fn output_target(&mut self, _message: blaze_protos::analysis::Target) {
        blaze_util::unported!("StreamedConsumingOutputHandler")
    }
    fn output_dep_set_of_files(&mut self, _message: blaze_protos::analysis::DepSetOfFiles) {
        blaze_util::unported!("StreamedConsumingOutputHandler")
    }
    fn output_configuration(&mut self, _message: blaze_protos::analysis::Configuration) {
        blaze_util::unported!("StreamedConsumingOutputHandler")
    }
    fn output_aspect_descriptor(&mut self, _message: blaze_protos::analysis::AspectDescriptor) {
        blaze_util::unported!("StreamedConsumingOutputHandler")
    }
    fn output_rule_class(&mut self, _message: blaze_protos::analysis::RuleClass) {
        blaze_util::unported!("StreamedConsumingOutputHandler")
    }
    fn output_path_fragment(&mut self, _message: blaze_protos::analysis::PathFragment) {
        blaze_util::unported!("StreamedConsumingOutputHandler")
    }
    fn close(&mut self) -> std::io::Result<()> {
        blaze_util::unported!("StreamedConsumingOutputHandler")
    }
}

impl NamedThreadSafeOutputFormatterCallback<KeyedConfiguredTargetValue>
    for ActionGraphProtoOutputFormatterCallback<'_>
{
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphProtoOutputFormatterCallback.java:101 ActionGraphProtoOutputFormatterCallback.getName
    fn get_name(&self) -> String {
        self.output_type.format_name().to_owned()
    }
}

impl ThreadSafeOutputFormatterCallback<KeyedConfiguredTargetValue>
    for ActionGraphProtoOutputFormatterCallback<'_>
{
}

impl Callback<KeyedConfiguredTargetValue> for ActionGraphProtoOutputFormatterCallback<'_> {
    fn process(&self, partial_result: &[KeyedConfiguredTargetValue]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<KeyedConfiguredTargetValue>
    for ActionGraphProtoOutputFormatterCallback<'_>
{
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphProtoOutputFormatterCallback.java:106 ActionGraphProtoOutputFormatterCallback.close
    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        if !fail_fast {
            if let Some(handler) = self.aquery_output_handler.borrow_mut().as_mut() {
                handler.close()?;
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphProtoOutputFormatterCallback.java:115 ActionGraphProtoOutputFormatterCallback.processOutput
    fn process_output(
        &self,
        partial_result: &[KeyedConfiguredTargetValue],
    ) -> Result<(), OutputFormatterError> {
        // PORT: Java builds the ActionGraphDump in the constructor; it borrows the output
        // handler here. PostAnalysisQueryProcessor calls processOutput exactly once, so the
        // dump's id caches span the whole output as in Java.
        blaze_util::check_state!(
            !self.processed.replace(true),
            "processOutput called more than once"
        );
        let mut handler = self.aquery_output_handler.borrow_mut();
        let Some(handler) = handler.as_mut() else {
            return Ok(());
        };
        // PORT: AqueryConsumingOutputHandler (processOutputInParallel) is not ported.
        let options = self.options;
        // Enabling includeParamFiles should enable includeCommandline by default.
        // PORT: Java mutates options.includeCommandline after the dump was constructed with the
        // original value, so the dump does not see it.
        let mut action_graph_dump = ActionGraphDump::new(
            options.include_commandline,
            options.include_artifacts,
            options.include_pruned_inputs,
            Some(self.action_filters),
            options.include_param_files,
            options.include_file_write_contents,
            &mut **handler,
        );
        for configured_target_value in partial_result {
            match self.process_single_entry(&mut action_graph_dump, configured_target_value) {
                Ok(()) => {}
                Err(ActionGraphDumpError::Interrupted) => {
                    return Err(OutputFormatterError::Interrupted(
                        InterruptedException::new(),
                    ));
                }
                Err(ActionGraphDumpError::Io(e)) => return Err(e.into()),
                // CommandLineExpansionException | TemplateExpansionException
                Err(e) => return Err(std::io::Error::other(e.to_string()).into()),
            }
        }
        Ok(())
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}
