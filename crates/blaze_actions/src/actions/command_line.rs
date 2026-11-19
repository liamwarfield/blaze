//! Ports of `lib/actions/CommandLine.java`, `AbstractCommandLine.java` and `ArgChunk.java`.

use std::fmt;
use std::sync::Arc;

use blaze_util::Fingerprint;

use super::action::{CommandLineError, InputMetadataProvider};
use super::action_key_context::ActionKeyContext;
use super::path_mapper::{OutputPathsMode, PathMapper, for_action_key};

/// A chunk of command line arguments, possibly lazily expanded with a `PathMapper`.
// Java: com/google/devtools/build/lib/actions/ArgChunk.java:23 ArgChunk
pub trait ArgChunk {
    /// Returns the arguments of this chunk, mapped with `path_mapper`.
    fn arguments(&self, path_mapper: &dyn PathMapper) -> Vec<String>;

    /// Returns the total length of the arguments (each plus one separator byte), mapped with
    /// `path_mapper`.
    fn total_arg_length(&self, path_mapper: &dyn PathMapper) -> i32;
}

/// An `ArgChunk` over a fixed list of arguments.
// Java: com/google/devtools/build/lib/actions/CommandLine.java:52 CommandLine.SimpleArgChunk
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimpleArgChunk {
    args: Vec<String>,
}

impl SimpleArgChunk {
    pub fn new(args: Vec<String>) -> SimpleArgChunk {
        SimpleArgChunk { args }
    }
}

impl ArgChunk for SimpleArgChunk {
    // Java: com/google/devtools/build/lib/actions/CommandLine.java:60 SimpleArgChunk.arguments
    fn arguments(&self, _path_mapper: &dyn PathMapper) -> Vec<String> {
        self.args.clone()
    }

    // Java: com/google/devtools/build/lib/actions/CommandLine.java:65 SimpleArgChunk.totalArgLength
    fn total_arg_length(&self, _path_mapper: &dyn PathMapper) -> i32 {
        let mut total: i32 = 0;
        for arg in &self.args {
            // Java `String.length()` of an internal string.
            total = total.wrapping_add(arg.chars().count() as i32 + 1);
        }
        total
    }
}

/// A representation of a list of arguments.
// Java: com/google/devtools/build/lib/actions/CommandLine.java:26 CommandLine
//
// PORT: `AbstractCommandLine` (the base of command lines that only implement `arguments()`) is
// expressed as the provided methods of this trait: an implementation that overrides only
// [`CommandLine::arguments`] gets AbstractCommandLine's `expand`, `arguments(inputMetadataProvider,
// pathMapper)` and `addToFingerprint`.
pub trait CommandLine: Send + Sync + fmt::Debug + std::any::Any {
    /// Returns the expanded command line with enclosed artifacts expanded by `input_metadata_provider`
    /// at execution time.
    ///
    /// By default, this method just delegates to [`CommandLine::arguments`], without performing
    /// any artifact expansion. Subclasses should override this method if they contain tree
    /// artifacts and need to expand them for proper argument evaluation.
    // Java: com/google/devtools/build/lib/actions/AbstractCommandLine.java:28 AbstractCommandLine.expand
    fn expand(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Box<dyn ArgChunk + '_>, CommandLineError> {
        Ok(Box::new(SimpleArgChunk::new(
            self.arguments_with(input_metadata_provider, path_mapper)?,
        )))
    }

    /// Returns the command line.
    // Java: com/google/devtools/build/lib/actions/CommandLine.java:60 CommandLine.arguments
    fn arguments(&self) -> Result<Vec<String>, CommandLineError>;

    /// Returns the evaluated command line with enclosed artifacts expanded by
    /// `input_metadata_provider` at execution time.
    // Java: com/google/devtools/build/lib/actions/AbstractCommandLine.java:47 AbstractCommandLine.arguments
    fn arguments_with(
        &self,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        _path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, CommandLineError> {
        self.arguments()
    }

    /// Adds the command line to the provided `Fingerprint`.
    ///
    /// Some of the implementations may require the to expand provided directory in order to
    /// produce a unique key. Consequently, the result of calling this function can be different
    /// depending on whether the `input_metadata_provider` is provided.
    // Java: com/google/devtools/build/lib/actions/AbstractCommandLine.java:54 AbstractCommandLine.addToFingerprint
    fn add_to_fingerprint(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        effective_output_paths_mode: OutputPathsMode,
        fingerprint: &mut Fingerprint,
    ) -> Result<(), CommandLineError> {
        for s in self.arguments_with(None, for_action_key(effective_output_paths_mode))? {
            // Invariant: arguments are internal strings (see `fingerprint_util`).
            fingerprint.add_string(&s);
        }
        Ok(())
    }

    /// Java `toString`: the arguments joined by spaces.
    // Java: com/google/devtools/build/lib/actions/CommandLine.java:74 CommandLine.toString
    fn to_string_java(&self) -> String {
        match self.arguments() {
            Ok(args) => args.join(" "),
            Err(blaze_collect::command_line_item::MapFnError::CommandLineExpansion(_)) => {
                "Error in expanding command line".to_string()
            }
            Err(blaze_collect::command_line_item::MapFnError::Interrupted(_)) => {
                "Interrupted while expanding command line".to_string()
            }
        }
    }
}

/// Shared handle to a command line.
pub type CommandLineRef = Arc<dyn CommandLine>;

/// A command line consisting of a fixed list of arguments.
// Java: com/google/devtools/build/lib/actions/CommandLine.java:112 CommandLine.FlatCommandLine
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlatCommandLine {
    args: Vec<String>,
}

impl FlatCommandLine {
    pub fn get_args(&self) -> &[String] {
        &self.args
    }
}

impl CommandLine for FlatCommandLine {
    // Java: com/google/devtools/build/lib/actions/CommandLine.java:60 FlatCommandLine.arguments
    fn arguments(&self) -> Result<Vec<String>, CommandLineError> {
        Ok(self.args.clone())
    }
}

/// `CommandLine.empty()`.
// Java: com/google/devtools/build/lib/actions/CommandLine.java:28 CommandLine.empty
pub fn empty() -> CommandLineRef {
    Arc::new(FlatCommandLine { args: Vec::new() })
}

/// `CommandLine.of(arguments)`.
// Java: com/google/devtools/build/lib/actions/CommandLine.java:33 CommandLine.of
pub fn of(arguments: Vec<String>) -> CommandLineRef {
    Arc::new(FlatCommandLine { args: arguments })
}

/// Returns a `CommandLine` that is constructed by appending the `args` to `command_line`.
// Java: com/google/devtools/build/lib/actions/CommandLine.java:41 CommandLine.concat
pub fn concat(command_line: CommandLineRef, args: Vec<String>) -> CommandLineRef {
    if args.is_empty() {
        return command_line;
    }
    if let Some(flat) =
        (command_line.as_ref() as &dyn std::any::Any).downcast_ref::<FlatCommandLine>()
    {
        // Java: `commandLine == FlatCommandLine.EMPTY_INSTANCE`.
        if flat.args.is_empty() {
            return of(args);
        }
    }
    Arc::new(SuffixedCommandLine {
        executable_args: args,
        command_line,
    })
}

// Java: com/google/devtools/build/lib/actions/CommandLine.java:127 CommandLine.SuffixedCommandLine
#[derive(Debug)]
struct SuffixedCommandLine {
    executable_args: Vec<String>,
    command_line: CommandLineRef,
}

impl CommandLine for SuffixedCommandLine {
    // Java: com/google/devtools/build/lib/actions/CommandLine.java:60 SuffixedCommandLine.arguments
    fn arguments(&self) -> Result<Vec<String>, CommandLineError> {
        let mut v = self.command_line.arguments()?;
        v.extend(self.executable_args.iter().cloned());
        Ok(v)
    }

    // Java: com/google/devtools/build/lib/actions/CommandLine.java:60 SuffixedCommandLine.arguments
    fn arguments_with(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, CommandLineError> {
        let mut v = self
            .command_line
            .arguments_with(input_metadata_provider, path_mapper)?;
        v.extend(self.executable_args.iter().cloned());
        Ok(v)
    }
}
