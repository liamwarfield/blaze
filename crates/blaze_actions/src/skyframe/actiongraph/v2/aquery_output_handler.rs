//! Ports of `AqueryOutputHandler.java` and `MonolithicOutputHandler.java`.

use blaze_protos::analysis;
use blaze_protos::json::JsonFormat;

/// Interface for the output handlers of aquery (`jsonproto`, `proto`, `textproto`,
/// `streamed_proto`).
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:27 AqueryOutputHandler
pub trait AqueryOutputHandler {
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:61 AqueryOutputHandler.outputArtifact
    fn output_artifact(&mut self, message: analysis::Artifact);
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:63 AqueryOutputHandler.outputAction
    fn output_action(&mut self, message: analysis::Action);
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:65 AqueryOutputHandler.outputTarget
    fn output_target(&mut self, message: analysis::Target);
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:67 AqueryOutputHandler.outputDepSetOfFiles
    fn output_dep_set_of_files(&mut self, message: analysis::DepSetOfFiles);
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:69 AqueryOutputHandler.outputConfiguration
    fn output_configuration(&mut self, message: analysis::Configuration);
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:71 AqueryOutputHandler.outputAspectDescriptor
    fn output_aspect_descriptor(&mut self, message: analysis::AspectDescriptor);
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:73 AqueryOutputHandler.outputRuleClass
    fn output_rule_class(&mut self, message: analysis::RuleClass);
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:75 AqueryOutputHandler.outputPathFragment
    fn output_path_fragment(&mut self, message: analysis::PathFragment);
    /// Writes the output (for monolithic handlers) and closes it.
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:78 AqueryOutputHandler.close
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

// PORT: skipped, the format name of each variant is `format_name`'s match: AqueryOutputHandler.OutputType.<init>
impl OutputType {
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:41 AqueryOutputHandler.OutputType.formatName
    pub fn format_name(self) -> &'static str {
        match self {
            OutputType::BINARY => "proto",
            OutputType::DELIMITED_BINARY => "streamed_proto",
            OutputType::TEXT => "textproto",
            OutputType::JSON => "jsonproto",
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/AqueryOutputHandler.java:45 AqueryOutputHandler.OutputType.fromString
    /// The error is the message of Java's `InvalidAqueryOutputFormatException`.
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/InvalidAqueryOutputFormatException.java:19 InvalidAqueryOutputFormatException.<init>
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
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/MonolithicOutputHandler.java:36 MonolithicOutputHandler.<init>
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
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/MonolithicOutputHandler.java:40 MonolithicOutputHandler.outputArtifact
    fn output_artifact(&mut self, message: analysis::Artifact) {
        self.action_graph_container.artifacts.push(message);
    }
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/MonolithicOutputHandler.java:45 MonolithicOutputHandler.outputAction
    fn output_action(&mut self, message: analysis::Action) {
        self.action_graph_container.actions.push(message);
    }
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/MonolithicOutputHandler.java:50 MonolithicOutputHandler.outputTarget
    fn output_target(&mut self, message: analysis::Target) {
        self.action_graph_container.targets.push(message);
    }
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/MonolithicOutputHandler.java:55 MonolithicOutputHandler.outputDepSetOfFiles
    fn output_dep_set_of_files(&mut self, message: analysis::DepSetOfFiles) {
        self.action_graph_container.dep_set_of_files.push(message);
    }
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/MonolithicOutputHandler.java:60 MonolithicOutputHandler.outputConfiguration
    fn output_configuration(&mut self, message: analysis::Configuration) {
        self.action_graph_container.configuration.push(message);
    }
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/MonolithicOutputHandler.java:65 MonolithicOutputHandler.outputAspectDescriptor
    fn output_aspect_descriptor(&mut self, message: analysis::AspectDescriptor) {
        self.action_graph_container.aspect_descriptors.push(message);
    }
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/MonolithicOutputHandler.java:70 MonolithicOutputHandler.outputRuleClass
    fn output_rule_class(&mut self, message: analysis::RuleClass) {
        self.action_graph_container.rule_classes.push(message);
    }
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/MonolithicOutputHandler.java:75 MonolithicOutputHandler.outputPathFragment
    fn output_path_fragment(&mut self, message: analysis::PathFragment) {
        self.action_graph_container.path_fragments.push(message);
    }

    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/MonolithicOutputHandler.java:80 MonolithicOutputHandler.close
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
