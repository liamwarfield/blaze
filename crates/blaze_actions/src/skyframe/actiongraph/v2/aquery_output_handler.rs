//! Ports of `AqueryOutputHandler.java` and `MonolithicOutputHandler.java`.

use blaze_protos::analysis;
use blaze_protos::json::JsonFormat;

/// Interface for the output handlers of aquery (`jsonproto`, `proto`, `textproto`,
/// `streamed_proto`).
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:27 AqueryOutputHandler
pub trait AqueryOutputHandler {
    fn output_artifact(&mut self, message: analysis::Artifact);
    fn output_action(&mut self, message: analysis::Action);
    fn output_target(&mut self, message: analysis::Target);
    fn output_dep_set_of_files(&mut self, message: analysis::DepSetOfFiles);
    fn output_configuration(&mut self, message: analysis::Configuration);
    fn output_aspect_descriptor(&mut self, message: analysis::AspectDescriptor);
    fn output_rule_class(&mut self, message: analysis::RuleClass);
    fn output_path_fragment(&mut self, message: analysis::PathFragment);
    /// Writes the output (for monolithic handlers) and closes it.
    fn close(&mut self) -> std::io::Result<()>;
}

/// The output format names.
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:29 AqueryOutputHandler.OutputType
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputType {
    BINARY,
    DELIMITED_BINARY,
    TEXT,
    JSON,
}

impl OutputType {
    pub fn format_name(self) -> &'static str {
        match self {
            OutputType::BINARY => "proto",
            OutputType::DELIMITED_BINARY => "streamed_proto",
            OutputType::TEXT => "textproto",
            OutputType::JSON => "jsonproto",
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:45 OutputType.fromString
    pub fn from_string(string: &str) -> Result<OutputType, String> {
        match string {
            "proto" => Ok(OutputType::BINARY),
            "streamed_proto" => Ok(OutputType::DELIMITED_BINARY),
            "textproto" => Ok(OutputType::TEXT),
            "jsonproto" => Ok(OutputType::JSON),
            _ => Err(format!("Invalid aquery output format: {string}")),
        }
    }
}

/// Collects everything into one `ActionGraphContainer` and prints it on close.
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/MonolithicOutputHandler.java:30 MonolithicOutputHandler
pub struct MonolithicOutputHandler<W: std::io::Write> {
    action_graph_container: analysis::ActionGraphContainer,
    print_stream: W,
}

impl<W: std::io::Write> MonolithicOutputHandler<W> {
    pub fn new(print_stream: W) -> Self {
        MonolithicOutputHandler {
            action_graph_container: analysis::ActionGraphContainer::default(),
            print_stream,
        }
    }

    pub fn container(&self) -> &analysis::ActionGraphContainer {
        &self.action_graph_container
    }

    pub fn into_inner(self) -> W {
        self.print_stream
    }
}

impl<W: std::io::Write> AqueryOutputHandler for MonolithicOutputHandler<W> {
    fn output_artifact(&mut self, message: analysis::Artifact) {
        self.action_graph_container.artifacts.push(message);
    }
    fn output_action(&mut self, message: analysis::Action) {
        self.action_graph_container.actions.push(message);
    }
    fn output_target(&mut self, message: analysis::Target) {
        self.action_graph_container.targets.push(message);
    }
    fn output_dep_set_of_files(&mut self, message: analysis::DepSetOfFiles) {
        self.action_graph_container.dep_set_of_files.push(message);
    }
    fn output_configuration(&mut self, message: analysis::Configuration) {
        self.action_graph_container.configuration.push(message);
    }
    fn output_aspect_descriptor(&mut self, message: analysis::AspectDescriptor) {
        self.action_graph_container.aspect_descriptors.push(message);
    }
    fn output_rule_class(&mut self, message: analysis::RuleClass) {
        self.action_graph_container.rule_classes.push(message);
    }
    fn output_path_fragment(&mut self, message: analysis::PathFragment) {
        self.action_graph_container.path_fragments.push(message);
    }

    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/MonolithicOutputHandler.java:81 MonolithicOutputHandler.close
    fn close(&mut self) -> std::io::Result<()> {
        let json = JsonFormat::printer()
            .print(&self.action_graph_container)
            .map_err(|e| std::io::Error::other(e.get_message().to_string()))?;
        // printStream.println() after jsonPrinter.appendTo.
        // Java: the `PrintStream` of `AqueryThreadsafeCallback` (`new PrintStream(out)`) encodes
        // with the default charset, ISO-8859-1 (`blaze.cc:431` `-Dfile.encoding`), so the
        // internal strings of the messages come out as their bytes, one per char.
        self.print_stream
            .write_all(&blaze_util::string_encoding::latin1_encode_lossy(&json))?;
        self.print_stream.write_all(b"\n")?;
        self.print_stream.flush()
    }
}
