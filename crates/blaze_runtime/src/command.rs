//! `@Command` annotations, the `BlazeCommand` interface and `BlazeCommandResult` (re-exported
//! from `blaze_server`).

use blaze_options::definition::OptionsClassRef;
use blaze_options::rc::handler::Command as RcCommand;

use crate::command_environment::CommandEnvironment;
use crate::options_result::OptionsParsingResult;

// Java: com/google/devtools/build/lib/runtime/Command.java:71 Command.BuildPhase
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildPhase {
    NONE,
    LOADS,
    ANALYZES,
    EXECUTES,
}

// runtime/BuildPhase.java (the outcome of a build for its summary) has no user at the pin.
// PORT: skipped, unused at the pin: BuildPhase.getMessage, BuildPhase.getSuccess
impl BuildPhase {
    // Java: com/google/devtools/build/lib/runtime/Command.java:100 Command.BuildPhase.executes
    pub fn executes(self) -> bool {
        self == BuildPhase::EXECUTES
    }

    // Java: com/google/devtools/build/lib/runtime/Command.java:105 Command.BuildPhase.analyzes
    pub fn analyzes(self) -> bool {
        self == BuildPhase::ANALYZES || self == BuildPhase::EXECUTES
    }

    // Java: com/google/devtools/build/lib/runtime/Command.java:113 Command.BuildPhase.loads
    pub fn loads(self) -> bool {
        self == BuildPhase::LOADS || self == BuildPhase::ANALYZES || self == BuildPhase::EXECUTES
    }
}

/// A command's `@Command` annotation (Java: `com.google.devtools.build.lib.runtime.Command`).
///
/// `rc` is the part `BlazeOptionHandler` needs (name and option-inheritance chain), kept as a
/// `blaze_options` value so the handler can walk it; `inherits_options_from` is the same chain
/// with the full annotations.
// Java: com/google/devtools/build/lib/runtime/Command.java:28 Command
#[derive(Debug)]
pub struct CommandAnnotation {
    /// `name()` and `inheritsOptionsFrom()` as `BlazeOptionHandler` sees them.
    // Java: com/google/devtools/build/lib/runtime/Command.java:32 Command.name
    // Java: com/google/devtools/build/lib/runtime/Command.java:44 Command.inheritsOptionsFrom
    pub rc: RcCommand,
    /// `options()`: the options classes of the command itself (a function because option class
    /// infos are created lazily).
    // Java: com/google/devtools/build/lib/runtime/Command.java:38 Command.options
    pub options: fn() -> Vec<OptionsClassRef>,
    /// `inheritsOptionsFrom()`.
    pub inherits_options_from: &'static [&'static CommandAnnotation],
    // Java: com/google/devtools/build/lib/runtime/Command.java:49 Command.shortDescription
    pub short_description: &'static str,
    // Java: com/google/devtools/build/lib/runtime/Command.java:54 Command.usesConfigurationOptions
    pub uses_configuration_options: bool,
    // Java: com/google/devtools/build/lib/runtime/Command.java:63 Command.buildPhase
    pub build_phase: BuildPhase,
    // Java: com/google/devtools/build/lib/runtime/Command.java:121 Command.hidden
    pub hidden: bool,
    // Java: com/google/devtools/build/lib/runtime/Command.java:128 Command.allowResidue
    pub allow_residue: bool,
    // Java: com/google/devtools/build/lib/runtime/Command.java:134 Command.hasSensitiveResidue
    pub has_sensitive_residue: bool,
    // Java: com/google/devtools/build/lib/runtime/Command.java:142 Command.binaryStdOut
    pub binary_std_out: bool,
    // Java: com/google/devtools/build/lib/runtime/Command.java:150 Command.binaryStdErr
    pub binary_std_err: bool,
    // Java: com/google/devtools/build/lib/runtime/Command.java:157 Command.writeCommandLog
    pub write_command_log: bool,
    /// `help()`: either the help text or `resource:<file>`.
    // Java: com/google/devtools/build/lib/runtime/Command.java:164 Command.help
    pub help: &'static str,
    // Java: com/google/devtools/build/lib/runtime/Command.java:171 Command.mustRunInWorkspace
    pub must_run_in_workspace: bool,
    // Java: com/google/devtools/build/lib/runtime/Command.java:180 Command.completion
    pub completion: &'static str,
}

impl CommandAnnotation {
    /// Default values of the optional annotation members, for use with struct update syntax.
    pub const DEFAULTS: CommandAnnotationDefaults = CommandAnnotationDefaults;

    pub fn name(&self) -> &'static str {
        self.rc.name
    }
}

/// Marker for documentation: the Java defaults of `@Command` are `options = {}`,
/// `inheritsOptionsFrom = {}`, `usesConfigurationOptions = false`, `buildPhase = NONE`,
/// `hidden = false`, `allowResidue = false`, `hasSensitiveResidue = false`,
/// `binaryStdOut = false`, `binaryStdErr = false`, `writeCommandLog = true`,
/// `mustRunInWorkspace = true`, `completion = ""`.
#[derive(Debug)]
pub struct CommandAnnotationDefaults;

/// Interface implemented by Blaze commands.
// Java: com/google/devtools/build/lib/runtime/BlazeCommand.java:23 BlazeCommand
pub trait BlazeCommand: Send + Sync {
    /// The command's `@Command` annotation.
    fn annotation(&self) -> &'static CommandAnnotation;

    /// This method provides the imperative portion of the command. It takes a
    /// [`CommandEnvironment`] and the parsed options, and returns the result.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommand.java:40 BlazeCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult;

    /// Allows the command to provide command-specific option defaults and/or requirements.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommand.java:48 BlazeCommand.editOptions
    fn edit_options(&self, _options_parser: &mut blaze_options::OptionsParser) {}
}

/// The result of a command: `blaze_server`'s port of `BlazeCommandResult`.
///
/// PORT: Java's `BlazeCommandResult` is in `lib/runtime`; it lives in `blaze_server`, below this
/// crate, because `GrpcServerImpl` needs it (decision D3), and is re-exported here so that no
/// caller changes. `createShutdown(Crash)` there takes the crash's `DetailedExitCode`.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandResult.java:36 BlazeCommandResult
pub use blaze_server::BlazeCommandResult;
