//! `CommandLineEvent.ToolCommandLineEvent` and its option converter, from
//! `com/google/devtools/build/lib/runtime/CommandLineEvent.java`.

use super::prelude::*;

/// The parts of the `command_line.CommandLine` proto the converter can produce.
// PORT: the proto messages are modelled with the fields this converter fills in.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CommandLine {
    pub command_line_label: String,
    /// Sections holding a `chunk_list`.
    pub sections: Vec<Vec<String>>,
}

impl CommandLine {
    /// Protobuf `TextFormat` output of the message (`Message.toString()`).
    fn to_text_format(&self) -> String {
        let mut out = String::new();
        if !self.command_line_label.is_empty() {
            out.push_str(&format!(
                "command_line_label: \"{}\"\n",
                escape_bytes(&self.command_line_label)
            ));
        }
        for chunks in &self.sections {
            out.push_str("sections {\n  chunk_list {\n");
            for chunk in chunks {
                out.push_str(&format!("    chunk: \"{}\"\n", escape_bytes(chunk)));
            }
            out.push_str("  }\n}\n");
        }
        out
    }
}

/// Protobuf `TextFormat.escapeBytes` over the UTF-8 bytes of `s`.
fn escape_bytes(s: &str) -> String {
    let mut out = String::new();
    for &b in s.as_bytes() {
        match b {
            0x07 => out.push_str("\\a"),
            0x08 => out.push_str("\\b"),
            0x0c => out.push_str("\\f"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x0b => out.push_str("\\v"),
            b'\\' => out.push_str("\\\\"),
            b'\'' => out.push_str("\\'"),
            b'"' => out.push_str("\\\""),
            0x20..=0x7e => out.push(b as char),
            _ => out.push_str(&format!("\\{:03o}", b)),
        }
    }
    out
}

/// Guava `BaseEncoding.base64().decode` accepts `input` (standard alphabet, trailing padding).
fn is_valid_base64(input: &str) -> bool {
    let trimmed = input.trim_end_matches('=');
    // isValidPaddingStartPosition: a 4-char chunk may end after 0, 2 or 3 characters.
    if trimmed.len() % 4 == 1 {
        return false;
    }
    trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}

// Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:496 CommandLineEvent.ToolCommandLineEvent
/// The command line of the tool that invoked Bazel (`--experimental_tool_command_line`).
// PORT: Java has no equals() (identity); structural equality is used here.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ToolCommandLineEvent {
    pub command_line: CommandLine,
}

// Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:497 CommandLineEvent.ToolCommandLineEvent.LABEL
pub const TOOL_COMMAND_LINE_EVENT_LABEL: &str = "tool";

// Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:561 CommandLineEvent.ToolCommandLineEvent.toString
impl OptionScalar for ToolCommandLineEvent {
    fn java_to_string(&self) -> String {
        format!(
            "ToolCommandLineEvent{{commandLine={}}}",
            self.command_line.to_text_format()
        )
    }
}
option_field_type!(
    ToolCommandLineEvent,
    "com.google.devtools.build.lib.runtime.CommandLineEvent$ToolCommandLineEvent"
);

// Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:523 CommandLineEvent.ToolCommandLineEvent.Converter
/// The converter for the option value. We accept the command line both in base64 encoded proto
/// form and as unstructured strings.
#[derive(Clone, Copy, Debug, Default)]
pub struct ToolCommandLineEventConverter;

impl Converter for ToolCommandLineEventConverter {
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:527 CommandLineEvent.ToolCommandLineEvent.Converter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        if input.is_empty() {
            return Ok(Some(Value::new(ToolCommandLineEvent::default())));
        }
        if is_valid_base64(input) {
            // Try decoding the input as a base64 encoded binary proto.
            unported!("CommandLine.parseFrom (base64-encoded --experimental_tool_command_line)");
        }
        // If the value was not recognized as a base64-encoded proto, store the flag value as a
        // single string chunk.
        Ok(Some(Value::new(ToolCommandLineEvent {
            command_line: CommandLine {
                command_line_label: TOOL_COMMAND_LINE_EVENT_LABEL.to_string(),
                sections: vec![vec![input.to_string()]],
            },
        })))
    }
    fn get_type_description(&self) -> String {
        "A command line, either as a simple string, or as a base64-encoded binary form of a CommandLine proto"
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_string_becomes_one_chunk() {
        let v = ToolCommandLineEventConverter
            .convert(Some("my tool --x"), None)
            .unwrap()
            .unwrap();
        assert_eq!(
            v.java_to_string(),
            "ToolCommandLineEvent{commandLine=command_line_label: \"tool\"\nsections {\n  chunk_list {\n    chunk: \"my tool --x\"\n  }\n}\n}"
        );
        let empty = ToolCommandLineEventConverter
            .convert(Some(""), None)
            .unwrap()
            .unwrap();
        assert_eq!(empty.java_to_string(), "ToolCommandLineEvent{commandLine=}");
        assert!(is_valid_base64("abcd"));
        assert!(is_valid_base64("ab=="));
        assert!(!is_valid_base64("a"));
        assert!(!is_valid_base64("a b"));
    }
}
