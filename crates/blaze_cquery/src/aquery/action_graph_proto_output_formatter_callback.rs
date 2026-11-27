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
use blaze_protos::JavaMessage;
use blaze_protos::analysis;
use blaze_protos::text_format::message_to_string;
use blaze_query::query::output::output_formatter::OutputStream;
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, InterruptedException, OutputFormatterCallback, QueryError,
    ThreadSafeOutputFormatterCallback,
};
use blaze_util::string_encoding::latin1_encode_lossy;

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
// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphProtoOutputFormatterCallback.java:49 ActionGraphProtoOutputFormatterCallback
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
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphProtoOutputFormatterCallback.java:63 ActionGraphProtoOutputFormatterCallback.<init>
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

    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphProtoOutputFormatterCallback.java:134 ActionGraphProtoOutputFormatterCallback.processSingleEntry
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
            for aspect_value in self.accessor.get_aspect_values(configured_target_value) {
                // actionGraphDump.dumpAspect(aspectValue, configuredTargetValue)
                // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/ActionGraphDump.java:298 ActionGraphDump.dumpAspect
                // (ConfiguredTarget configuredTarget = configuredTargetValue.getConfiguredTarget();
                // its label and the actual target's rule class are `dumped_target`.)
                let aspect_actions: Option<Vec<Arc<dyn Action>>> =
                    aspect_value.as_ref().map(|v| v.get_actions().to_vec());
                action_graph_dump.dump_aspect(aspect_actions.as_deref(), &dumped_target)?;
            }
        }
        Ok(())
    }
}

/// Constructs the output handler of an output type.
// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphProtoOutputFormatterCallback.java:86 ActionGraphProtoOutputFormatterCallback.constructAqueryOutputHandler
pub fn construct_aquery_output_handler(
    output_type: OutputType,
    out: OutputStream,
) -> Box<dyn AqueryOutputHandler> {
    match output_type {
        OutputType::BINARY | OutputType::DELIMITED_BINARY | OutputType::TEXT => {
            Box::new(StreamedConsumingOutputHandler::new(output_type, out))
        }
        OutputType::JSON => Box::new(MonolithicOutputHandler::new(PrintStream(out))),
    }
}

/// `CodedOutputStream.writeUInt32NoTag` (a varint).
fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        if value < 0x80 {
            out.push(value as u8);
            return;
        }
        out.push(((value & 0x7f) | 0x80) as u8);
        value >>= 7;
    }
}

/// `ActionGraphContainer` field numbers (Java `ActionGraphContainer.*_FIELD_NUMBER`).
mod field_number {
    pub const ARTIFACTS: u32 = 1;
    pub const ACTIONS: u32 = 2;
    pub const TARGETS: u32 = 3;
    pub const DEP_SET_OF_FILES: u32 = 4;
    pub const CONFIGURATION: u32 = 5;
    pub const ASPECT_DESCRIPTORS: u32 = 6;
    pub const RULE_CLASSES: u32 = 7;
    pub const PATH_FRAGMENTS: u32 = 8;
}

/// The output handler for the `proto`, `streamed_proto` and `textproto` formats: each message is
/// printed as it is produced.
///
/// PORT: Java hands each message to a single consumer thread through a bounded
/// `LinkedBlockingQueue` (`startConsumer` / `stopConsumer`, a poison pill); the consumer prints
/// the tasks in queue order, which is the order of the `output*` calls. Blaze prints each task
/// on the calling thread, in the same order. The class belongs to
/// `skyframe/actiongraph/v2` (blaze_actions); it is ported next to its only user.
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/StreamedConsumingOutputHandler.java:43 StreamedConsumingOutputHandler
pub struct StreamedConsumingOutputHandler {
    output_type: OutputType,
    output_stream: OutputStream,
    /// Java `CodedOutputStream.newInstance(out, OUTPUT_BUFFER_SIZE)` (BINARY) and
    /// `PrintStream(out)` (TEXT): bytes not yet flushed to `output_stream`.
    buffer: Vec<u8>,
}

// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphProtoOutputFormatterCallback.java:61 ActionGraphProtoOutputFormatterCallback.OUTPUT_BUFFER_SIZE
const OUTPUT_BUFFER_SIZE: usize = 16384;

impl StreamedConsumingOutputHandler {
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/StreamedConsumingOutputHandler.java:55 StreamedConsumingOutputHandler.<init>
    pub fn new(output_type: OutputType, output_stream: OutputStream) -> Self {
        blaze_util::check_argument!(
            matches!(
                output_type,
                OutputType::BINARY | OutputType::DELIMITED_BINARY | OutputType::TEXT
            ),
            "Only proto, streamed_proto and textproto outputs should be streamed."
        );
        StreamedConsumingOutputHandler {
            output_type,
            output_stream,
            buffer: Vec::new(),
        }
    }

    /// Construct the printing task and print it (Java: put it in the queue, whose consumer
    /// prints it).
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/StreamedConsumingOutputHandler.java:133 StreamedConsumingOutputHandler.addTaskToQueue
    fn add_task_to_queue<M: JavaMessage>(
        &mut self,
        message: M,
        field_number: u32,
        message_label: &str,
        wrap: impl FnOnce(M) -> analysis::ActionGraphContainer,
    ) {
        match self.output_type {
            OutputType::BINARY => {
                // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/PrintTask.java:54 PrintTask.ProtoPrintTask.print
                // codedOutputStream.writeMessage(fieldNumber, message)
                encode_varint(u64::from((field_number << 3) | 2), &mut self.buffer);
                message.write_delimited_to(&mut self.buffer);
                if self.buffer.len() >= OUTPUT_BUFFER_SIZE {
                    self.flush_buffer();
                }
            }
            OutputType::DELIMITED_BINARY => {
                // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/PrintTask.java:77 PrintTask.StreamedProtoPrintTask.print
                let mut bytes = Vec::new();
                wrap(message).write_delimited_to(&mut bytes);
                // builder.build().writeDelimitedTo(out): straight to the stream.
                let _ = self.output_stream.borrow_mut().write_all(&bytes);
            }
            OutputType::TEXT => {
                // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/PrintTask.java:129 PrintTask.TextProtoPrintTask.print
                let text = format!("{message_label} {{\n{}}}\n", message_to_string(&message));
                // PrintStream(out): the default charset (ISO-8859-1 on Bazel's server).
                self.buffer.extend_from_slice(&latin1_encode_lossy(&text));
            }
            OutputType::JSON => panic!("IllegalStateException: Unknown outputType: JSON"),
        }
    }

    fn flush_buffer(&mut self) {
        let bytes = std::mem::take(&mut self.buffer);
        let _ = self.output_stream.borrow_mut().write_all(&bytes);
    }
}

impl AqueryOutputHandler for StreamedConsumingOutputHandler {
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/StreamedConsumingOutputHandler.java:72 StreamedConsumingOutputHandler.outputArtifact
    fn output_artifact(&mut self, message: analysis::Artifact) {
        self.add_task_to_queue(message, field_number::ARTIFACTS, "artifacts", |m| {
            analysis::ActionGraphContainer {
                artifacts: vec![m],
                ..Default::default()
            }
        });
    }
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/StreamedConsumingOutputHandler.java:77 StreamedConsumingOutputHandler.outputAction
    fn output_action(&mut self, message: analysis::Action) {
        self.add_task_to_queue(message, field_number::ACTIONS, "actions", |m| {
            analysis::ActionGraphContainer {
                actions: vec![m],
                ..Default::default()
            }
        });
    }
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/StreamedConsumingOutputHandler.java:82 StreamedConsumingOutputHandler.outputTarget
    fn output_target(&mut self, message: analysis::Target) {
        self.add_task_to_queue(message, field_number::TARGETS, "targets", |m| {
            analysis::ActionGraphContainer {
                targets: vec![m],
                ..Default::default()
            }
        });
    }
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/StreamedConsumingOutputHandler.java:87 StreamedConsumingOutputHandler.outputDepSetOfFiles
    fn output_dep_set_of_files(&mut self, message: analysis::DepSetOfFiles) {
        self.add_task_to_queue(
            message,
            field_number::DEP_SET_OF_FILES,
            "dep_set_of_files",
            |m| analysis::ActionGraphContainer {
                dep_set_of_files: vec![m],
                ..Default::default()
            },
        );
    }
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/StreamedConsumingOutputHandler.java:92 StreamedConsumingOutputHandler.outputConfiguration
    fn output_configuration(&mut self, message: analysis::Configuration) {
        self.add_task_to_queue(message, field_number::CONFIGURATION, "configuration", |m| {
            analysis::ActionGraphContainer {
                configuration: vec![m],
                ..Default::default()
            }
        });
    }
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/StreamedConsumingOutputHandler.java:97 StreamedConsumingOutputHandler.outputAspectDescriptor
    fn output_aspect_descriptor(&mut self, message: analysis::AspectDescriptor) {
        self.add_task_to_queue(
            message,
            field_number::ASPECT_DESCRIPTORS,
            "aspect_descriptors",
            |m| analysis::ActionGraphContainer {
                aspect_descriptors: vec![m],
                ..Default::default()
            },
        );
    }
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/StreamedConsumingOutputHandler.java:103 StreamedConsumingOutputHandler.outputRuleClass
    fn output_rule_class(&mut self, message: analysis::RuleClass) {
        self.add_task_to_queue(message, field_number::RULE_CLASSES, "rule_classes", |m| {
            analysis::ActionGraphContainer {
                rule_classes: vec![m],
                ..Default::default()
            }
        });
    }
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/StreamedConsumingOutputHandler.java:108 StreamedConsumingOutputHandler.outputPathFragment
    fn output_path_fragment(&mut self, message: analysis::PathFragment) {
        self.add_task_to_queue(
            message,
            field_number::PATH_FRAGMENTS,
            "path_fragments",
            |m| analysis::ActionGraphContainer {
                path_fragments: vec![m],
                ..Default::default()
            },
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/StreamedConsumingOutputHandler.java:160 StreamedConsumingOutputHandler.close
    fn close(&mut self) -> std::io::Result<()> {
        // codedOutputStream.flush() / outputStream.flush() / printStream.flush()
        self.flush_buffer();
        self.output_stream.borrow_mut().flush()
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
        // PORT: processOutputInParallel (AqueryConsumingOutputHandler: proto, streamed_proto and
        // textproto) is reached through UnportedStreamedConsumingOutputHandler, which fails
        // loudly on its first use.
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
