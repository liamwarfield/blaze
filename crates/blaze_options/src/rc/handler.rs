//! Server-side rc handling: the rc/`--config` parts of `BlazeOptionHandler`, `ConfigExpander`
//! and `RcChunkOfArgs`.

use std::collections::HashSet;
use std::sync::Arc;

use indexmap::{IndexMap, IndexSet};

use super::client_options::{ClientOptions, OptionOverride};
use crate::data::OptionsData;
use crate::definition::{OptionDefinitionRef, OptionsClassRef};
use blaze_protos::blaze::invocation_policy::InvocationPolicy;
use prost::Message as _;

use crate::exception::OptionsParsingException;
use crate::invocation_policy::{InvocationPolicyEnforcer, OptionAndRawValue};
use crate::parser::{ArgAndFallbackData, OptionsParser};
use crate::priority::PriorityCategory;

// Java: com/google/devtools/build/lib/runtime/RcChunkOfArgs.java:28 RcChunkOfArgs
/// The args of one rc file (or of the part of it between imports) for one command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RcChunkOfArgs {
    rc_file: String,
    args: Vec<String>,
}

impl RcChunkOfArgs {
    pub fn new(rc_file: impl Into<String>, args: Vec<String>) -> Self {
        RcChunkOfArgs {
            rc_file: rc_file.into(),
            args,
        }
    }

    // Java: com/google/devtools/build/lib/runtime/RcChunkOfArgs.java:50 RcChunkOfArgs.getRcFile
    pub fn get_rc_file(&self) -> &str {
        &self.rc_file
    }

    // Java: com/google/devtools/build/lib/runtime/RcChunkOfArgs.java:59 RcChunkOfArgs.getArgs
    pub fn get_args(&self) -> &[String] {
        &self.args
    }
}

/// `ListMultimap<String, RcChunkOfArgs>` keyed by command or `command:config`.
/// PORT: Java uses ArrayListMultimap (hash-ordered keys); only per-key order is observable.
pub type CommandToRcArgs = IndexMap<String, Vec<RcChunkOfArgs>>;

/// Severity of a [`HandlerEvent`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Error,
    Warning,
    Info,
}

/// An event reported through the `EventHandler` (`Event.error`/`Event.warn`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandlerEvent {
    pub kind: EventKind,
    pub message: String,
    pub tag: Option<String>,
}

impl HandlerEvent {
    pub fn warn(message: impl Into<String>) -> Self {
        HandlerEvent {
            kind: EventKind::Warning,
            message: message.into(),
            tag: None,
        }
    }
    pub fn error(message: impl Into<String>) -> Self {
        HandlerEvent {
            kind: EventKind::Error,
            message: message.into(),
            tag: None,
        }
    }
}

/// Collects events (Java: `StoredEventHandler`).
#[derive(Clone, Debug, Default)]
pub struct StoredEventHandler {
    events: Vec<HandlerEvent>,
}

impl StoredEventHandler {
    pub fn handle(&mut self, e: HandlerEvent) {
        self.events.push(e);
    }
    pub fn get_events(&self) -> &[HandlerEvent] {
        &self.events
    }
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

// Java: com/google/devtools/build/lib/runtime/Command.java:34 Command
/// The parts of a command's `@Command` annotation that option handling needs.
#[derive(Debug)]
pub struct Command {
    pub name: &'static str,
    pub inherits_options_from: &'static [&'static Command],
}

// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:77 BlazeOptionHandler.INTERNAL_COMMAND_OPTIONS
const INTERNAL_COMMAND_OPTIONS: [&str; 7] = [
    "rc_source",
    "default_override",
    "isatty",
    "terminal_columns",
    "ignore_client_env",
    "client_env",
    "client_cwd",
];

// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:89 BlazeOptionHandler.ALWAYS_PSEUDO_COMMAND
pub const ALWAYS_PSEUDO_COMMAND: &str = "always";
// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:93 BlazeOptionHandler.COMMON_PSEUDO_COMMAND
pub const COMMON_PSEUDO_COMMAND: &str = "common";
// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:96 BlazeOptionHandler.STARTUP_PSEUDO_COMMAND
pub const STARTUP_PSEUDO_COMMAND: &str = "startup";
// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:98 BlazeOptionHandler.BUILD_COMMAND_ANCESTORS
const BUILD_COMMAND_ANCESTORS: [&str; 3] = ["build", COMMON_PSEUDO_COMMAND, ALWAYS_PSEUDO_COMMAND];
// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:102 BlazeOptionHandler.BAD_OPTION_TAG
pub const BAD_OPTION_TAG: &str = "invalidOption";
// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:104 BlazeOptionHandler.ERROR_SEPARATOR
pub const ERROR_SEPARATOR: &str = " :: ";
// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:67 CoreOptionConverters.BLAZE_ALIASING_FLAG
const BLAZE_ALIASING_FLAG: &str = "flag_alias";

/// A failed `parseOptions` (Java: a non-success `DetailedExitCode` with code
/// `OPTIONS_PARSE_FAILURE`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionsParseFailure {
    /// `FailureDetail.message`.
    pub message: String,
}

// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:73 BlazeOptionHandler
/// Handles parsing the command arguments: rc options, configs and command-line options.
/// PORT: project files and the workspace check are the caller's (runtime) responsibility and
/// are not run here. `BlazeCommand.editOptions` and the invocation policy run here only when the
/// caller supplies them ([`BlazeOptionHandler::with_invocation_policy`]); the runtime does.
pub struct BlazeOptionHandler<'p> {
    product_name: String,
    command: &'static Command,
    valid_commands: IndexSet<String>,
    options_parser: &'p mut OptionsParser,
    rcfile_notes: Vec<String>,
    all_options_classes: Vec<OptionsClassRef>,
    /// `runtime.getModuleInvocationPolicy()`, the Java constructor's `invocationPolicy` and
    /// `command.editOptions`; `None` when the caller does not run them through the handler.
    policy_and_edit_options: Option<PolicyAndEditOptions<'p>>,
    /// The `invocationPolicyFlagListBuilder` the parse methods fill.
    invocation_policy_flags: Vec<OptionAndRawValue>,
}

/// What the runtime gives the handler for the last steps of `parseOptionsInternal`.
struct PolicyAndEditOptions<'p> {
    module_invocation_policy: InvocationPolicy,
    invocation_policy: InvocationPolicy,
    edit_options: Box<dyn FnMut(&mut OptionsParser) + 'p>,
}

impl<'p> BlazeOptionHandler<'p> {
    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:115 BlazeOptionHandler.<init>
    /// `valid_commands` are the names of all commands (`runtime.getCommandMap().keySet()`);
    /// `all_options_classes` the options classes of all commands, deduplicated.
    pub fn new(
        product_name: &str,
        command: &'static Command,
        valid_commands: impl IntoIterator<Item = impl Into<String>>,
        all_options_classes: Vec<OptionsClassRef>,
        options_parser: &'p mut OptionsParser,
    ) -> Self {
        BlazeOptionHandler {
            product_name: product_name.to_string(),
            command,
            valid_commands: valid_commands.into_iter().map(Into::into).collect(),
            options_parser,
            rcfile_notes: Vec::new(),
            all_options_classes,
            policy_and_edit_options: None,
            invocation_policy_flags: Vec::new(),
        }
    }

    /// The Java constructor's `invocationPolicy`, with `runtime.getModuleInvocationPolicy()` and
    /// `command.editOptions`: `parseOptionsInternal` then runs `editOptions` and enforces the
    /// combined policy after parsing, before the warnings, as Java does.
    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:115 BlazeOptionHandler.<init>
    pub fn with_invocation_policy(
        mut self,
        module_invocation_policy: InvocationPolicy,
        invocation_policy: InvocationPolicy,
        edit_options: Box<dyn FnMut(&mut OptionsParser) + 'p>,
    ) -> Self {
        self.policy_and_edit_options = Some(PolicyAndEditOptions {
            module_invocation_policy,
            invocation_policy,
            edit_options,
        });
        self
    }

    /// The flags the invocation policy applied while parsing (Java: the
    /// `invocationPolicyFlagListBuilder` passed to `parseOptions`).
    pub fn get_invocation_policy_flags(&self) -> &[OptionAndRawValue] {
        &self.invocation_policy_flags
    }

    pub fn options_parser(&mut self) -> &mut OptionsParser {
        self.options_parser
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:148 BlazeOptionHandler.getRcfileNotes
    pub fn get_rcfile_notes(&self) -> &[String] {
        &self.rcfile_notes
    }

    fn fallback_data(&self) -> Option<Arc<OptionsData>> {
        Some(
            OptionsParser::get_fallback_options_data(&self.all_options_classes)
                .unwrap_or_else(|e| e.throw()),
        )
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:200 BlazeOptionHandler.parseRcOptions
    /// Parses the unconditional rc options for the current command, in command-inheritance
    /// order (`always`, `common`, ancestors, the command) and rc order within each.
    pub fn parse_rc_options(
        &mut self,
        _event_handler: &mut StoredEventHandler,
        command_to_rc_args: &CommandToRcArgs,
    ) -> Result<(), OptionsParsingException> {
        for command_to_parse in get_command_names_to_parse(self.command) {
            let Some(chunks) = command_to_rc_args.get(&command_to_parse) else {
                continue;
            };
            for rc_args in chunks {
                if !rc_args.get_args().is_empty() {
                    let inherited = if command_to_parse == self.command.name {
                        ""
                    } else {
                        "Inherited "
                    };
                    let source = if rc_args.get_rc_file() == "client" {
                        "Options provided by the client".to_string()
                    } else {
                        format!(
                            "Reading rc options for '{}' from {}",
                            self.command.name,
                            rc_args.get_rc_file()
                        )
                    };
                    self.rcfile_notes.push(format!(
                        "{}:\n  {}'{}' options: {}",
                        source,
                        inherited,
                        command_to_parse,
                        rc_args.get_args().join(" ")
                    ));
                }
                let rc_file = rc_args.get_rc_file().to_string();
                if command_to_parse == COMMON_PSEUDO_COMMAND {
                    // Pass in options data for all commands supported by the runtime so that
                    // options that apply to some but not the current command can be ignored.
                    let fallback = self.fallback_data();
                    let ignored_args = self.options_parser.parse_with_source_function(
                        PriorityCategory::RC_FILE,
                        &move |_| Some(rc_file.clone()),
                        rc_args.get_args().to_vec(),
                        fallback,
                    )?;
                    if !ignored_args.is_empty() {
                        let note = self.rcfile_notes.last_mut().unwrap();
                        note.push_str(&format!(
                            "\n  Ignored as unsupported by '{}': {}",
                            self.command.name,
                            ignored_args.join(" ")
                        ));
                    }
                } else {
                    self.options_parser.parse_with_priority(
                        PriorityCategory::RC_FILE,
                        Some(&rc_file),
                        rc_args.get_args().to_vec(),
                    )?;
                }
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:258 BlazeOptionHandler.parseArgsAndConfigs
    fn parse_args_and_configs(
        &mut self,
        args: &[String],
        event_handler: &mut StoredEventHandler,
    ) -> Result<CommandToRcArgs, OptionsParsingException> {
        let launcher_source = format!("options generated by {} launcher", self.product_name);
        let command_option_source_function = move |option: OptionDefinitionRef| {
            if INTERNAL_COMMAND_OPTIONS.contains(&option.get_option_name()) {
                Some(launcher_source.clone())
            } else {
                Some("command line options".to_string())
            }
        };

        let cmd_line_after_command = &args[1..];
        let (default_overrides_and_rc_sources, remaining_cmd_line) =
            partition_command_line_args(cmd_line_after_command);

        // Parses options needed to parse rcfiles properly.
        self.options_parser.parse_with_source_function(
            PriorityCategory::COMMAND_LINE,
            &command_option_source_function,
            default_overrides_and_rc_sources,
            None,
        )?;

        // Command-specific options from .blazerc passed in via --default_override and
        // --rc_source.
        let rc_file_options = self
            .options_parser
            .get_options::<ClientOptions>()
            .expect("ClientOptions must be registered with the parser");
        let command_to_rc_args = structure_rc_options_and_configs(
            event_handler,
            &rc_file_options.rc_source,
            &rc_file_options.options_overrides,
            &self.valid_commands,
        )?;
        self.parse_rc_options(event_handler, &command_to_rc_args)?;

        // Parses the remaining command-line options.
        self.options_parser.parse_with_source_function(
            PriorityCategory::COMMAND_LINE,
            &command_option_source_function,
            remaining_cmd_line,
            None,
        )?;

        // PORT: ProjectFileSupport.handleProjectFiles (commands that analyze) is not ported.

        self.expand_config_options(event_handler, &command_to_rc_args)?;
        Ok(command_to_rc_args)
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:466 BlazeOptionHandler.parseOptions
    /// Parses the options (`args[0]` is the command name). Errors are reported as events.
    /// PORT: Java's `invocationPolicyFlagListBuilder` argument is the handler's
    /// [`BlazeOptionHandler::get_invocation_policy_flags`].
    pub fn parse_options(
        &mut self,
        args: &[String],
        event_handler: &mut StoredEventHandler,
    ) -> Result<(), OptionsParseFailure> {
        let (result, _) =
            self.parse_options_internal(args, event_handler, /* get_config_definitions= */ false);
        if result.is_err() {
            self.options_parser.set_error();
        }
        result
    }

    /// `parseOptionsAndGetConfigDefinitions`: also returns the `--config` definitions that apply
    /// to this command (config name -> definitions).
    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:483 BlazeOptionHandler.parseOptionsAndGetConfigDefinitions
    pub fn parse_options_and_get_config_definitions(
        &mut self,
        args: &[String],
        event_handler: &mut StoredEventHandler,
    ) -> (
        Result<(), OptionsParseFailure>,
        Vec<(String, RcChunkOfArgs)>,
    ) {
        let (result, config_definitions) =
            self.parse_options_internal(args, event_handler, /* get_config_definitions= */ true);
        if result.is_err() {
            self.options_parser.set_error();
        }
        (result, config_definitions)
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:496 BlazeOptionHandler.parseOptionsInternal
    // PORT: checkCwdInWorkspace runs in the runtime, before the handler is created.
    fn parse_options_internal(
        &mut self,
        args: &[String],
        event_handler: &mut StoredEventHandler,
        get_config_definitions: bool,
    ) -> (
        Result<(), OptionsParseFailure>,
        Vec<(String, RcChunkOfArgs)>,
    ) {
        let mut rc_definitions = CommandToRcArgs::new();
        let exit_code = (|| -> Result<(), OptionsParsingException> {
            rc_definitions = self.parse_args_and_configs(args, event_handler)?;
            if let Some(p) = &mut self.policy_and_edit_options {
                // Allow the command to edit the options.
                (p.edit_options)(self.options_parser);
                // Merge the invocation policy that is user-supplied, from the command line, and
                // any invocation policy that was added by a module. The module one goes 'first,'
                // so the user one has priority. (`Builder.mergeFrom(Message)` is the merge of
                // the serialized forms.)
                let mut combined_policy = p.module_invocation_policy.clone();
                combined_policy
                    .merge(p.invocation_policy.encode_to_vec().as_slice())
                    .expect("re-encoded InvocationPolicy");
                let options_policy_enforcer = InvocationPolicyEnforcer::new(
                    combined_policy,
                    self.options_parser.get_conversion_context().cloned(),
                );
                // Enforce the invocation policy. It is intentional that this is the last step in
                // preparing the options. The invocation policy is used in security-critical
                // contexts, and may be used as a last resort to override flags. That means that
                // the policy can override flags set in BlazeCommand.editOptions, so the code
                // needs to be safe regardless of the actual flag values. At the time of this
                // writing, editOptions was only used as a convenience feature or to improve the
                // user experience, but not required for safety or correctness.
                options_policy_enforcer.enforce(
                    self.options_parser,
                    self.command.name,
                    &mut self.invocation_policy_flags,
                )?;
            }
            // Print warnings for odd options usage
            for warning in self.options_parser.get_warnings() {
                event_handler.handle(HandlerEvent::warn(warning));
            }
            // PORT: Java reads CommonCommandOptions.deprecationWarnings; looked up by option
            // name here because CommonCommandOptions lives in the runtime crate.
            if let Some(d) = self
                .options_parser
                .options_data()
                .get_option_definition_from_name("unconditional_warning")
            {
                let value = self
                    .options_parser
                    .get_option_value_description(d.get_option_name())
                    .and_then(|v| v.get_value());
                if let Some(list) = value.as_ref().and_then(|v| v.as_list()) {
                    for warning in list {
                        event_handler.handle(HandlerEvent::warn(warning.get::<String>()));
                    }
                }
            }
            Ok(())
        })();
        let exit_code = exit_code.map_err(|e| {
            let log_message = "Error parsing options";
            process_options_parsing_exception(event_handler, &e, log_message)
        });

        if !get_config_definitions {
            return (exit_code, Vec::new());
        }
        // Transforms all rc definitions into valid --config definitions for this command. For
        // example, "build:asan" is valid for a build command but not "test:asan". Rc definitions
        // like "build" aren't included because those aren't --config definitions.
        let matching_rc_commands = get_command_names_to_parse(self.command);
        let mut valid_config_defs = Vec::new();
        for (rc_key, chunks) in &rc_definitions {
            let Some(first_colon) = rc_key.find(':') else {
                continue;
            };
            let cmd = &rc_key[..first_colon];
            let config_name = &rc_key[first_colon + 1..];
            if matching_rc_commands.iter().any(|c| c == cmd) {
                for chunk in chunks {
                    valid_config_defs.push((config_name.to_string(), chunk.clone()));
                }
            }
        }
        (exit_code, valid_config_defs)
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:594 BlazeOptionHandler.expandConfigOptions
    /// Expands `--config` values per the rc definitions applicable to this command.
    pub fn expand_config_options(
        &mut self,
        event_handler: &mut StoredEventHandler,
        command_to_rc_args: &CommandToRcArgs,
    ) -> Result<(), OptionsParsingException> {
        let fallback = self.fallback_data();
        let commands_to_parse = get_command_names_to_parse(self.command);
        ConfigExpander::expand_config_options(
            event_handler,
            command_to_rc_args,
            self.command.name,
            &commands_to_parse,
            &mut self.rcfile_notes,
            self.options_parser,
            fallback,
        )
    }
}

// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:607 BlazeOptionHandler.getCommandNamesToParse
/// `always`, `common`, then the command's inheritance chain (ancestors first), then the command.
pub fn get_command_names_to_parse(command: &Command) -> Vec<String> {
    let mut result = vec![
        ALWAYS_PSEUDO_COMMAND.to_string(),
        COMMON_PSEUDO_COMMAND.to_string(),
    ];
    get_command_names_to_parse_helper(command, &mut result);
    result
}

// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:615 BlazeOptionHandler.getCommandNamesToParseHelper
fn get_command_names_to_parse_helper(command: &Command, accumulator: &mut Vec<String>) {
    for base in command.inherits_options_from {
        get_command_names_to_parse_helper(base, accumulator);
    }
    accumulator.push(command.name.to_string());
}

// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:623 BlazeOptionHandler.processOptionsParsingException
fn process_options_parsing_exception(
    event_handler: &mut StoredEventHandler,
    e: &OptionsParsingException,
    log_message: &str,
) -> OptionsParseFailure {
    let error = match e.get_invalid_argument() {
        Some(arg) => HandlerEvent {
            kind: EventKind::Error,
            message: format!("{}{}{}", arg, ERROR_SEPARATOR, e.get_message()),
            tag: Some(BAD_OPTION_TAG.to_string()),
        },
        None => HandlerEvent::error(e.get_message()),
    };
    event_handler.handle(error);
    OptionsParseFailure {
        message: format!("{}: {}", log_message, e.get_message()),
    }
}

// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:664 BlazeOptionHandler.structureRcOptionsAndConfigs
/// Groups the client's line-by-line rc transfer by command, then by rc chunk.
pub fn structure_rc_options_and_configs(
    event_handler: &mut StoredEventHandler,
    rc_files: &[String],
    raw_overrides: &[OptionOverride],
    valid_commands: &IndexSet<String>,
) -> Result<CommandToRcArgs, OptionsParsingException> {
    let mut command_to_rc_args: CommandToRcArgs = IndexMap::new();
    let mut last_rc_file: Option<String> = None;
    let mut command_to_arg_map_for_last_rc: IndexMap<String, Vec<String>> = IndexMap::new();

    let flush = |command_to_rc_args: &mut CommandToRcArgs,
                 rc: &str,
                 map: &IndexMap<String, Vec<String>>| {
        for (command_key, args) in map {
            command_to_rc_args
                .entry(command_key.clone())
                .or_default()
                .push(RcChunkOfArgs::new(rc, args.clone()));
        }
    };

    for override_ in raw_overrides {
        if override_.blaze_rc < 0 || override_.blaze_rc as usize >= rc_files.len() {
            event_handler.handle(HandlerEvent::warn(
                "inconsistency in generated command line args. Ignoring bogus argument\n",
            ));
            continue;
        }
        let rc_file = &rc_files[override_.blaze_rc as usize];
        // The canonicalize-flags command only inherits bazelrc "build" commands. Restrict
        // --flag_alias accordingly.
        if (override_
            .option
            .starts_with(&format!("--{}=", BLAZE_ALIASING_FLAG))
            || override_.option == format!("--{}", BLAZE_ALIASING_FLAG))
            && !BUILD_COMMAND_ANCESTORS.contains(&override_.command.as_str())
        {
            return Err(OptionsParsingException::new(format!(
                "{}: \"{} {}\" disallowed. --{} only supports these commands: {}",
                rc_file,
                override_.command,
                override_.option,
                BLAZE_ALIASING_FLAG,
                BUILD_COMMAND_ANCESTORS.join(", ")
            )));
        }
        let mut command = override_.command.as_str();
        if let Some(index) = command.find(':')
            && index > 0
        {
            command = &command[..index];
        }
        if !valid_commands.contains(command)
            && command != ALWAYS_PSEUDO_COMMAND
            && command != COMMON_PSEUDO_COMMAND
            && command != STARTUP_PSEUDO_COMMAND
        {
            event_handler.handle(HandlerEvent::warn(format!(
                "while reading option defaults file '{}':\n  invalid command name '{}'.",
                rc_file, override_.command
            )));
            continue;
        }

        // We've moved on to another rc file "chunk", store the accumulated args from the last.
        if last_rc_file.as_deref() != Some(rc_file.as_str()) {
            if let Some(last) = &last_rc_file {
                flush(
                    &mut command_to_rc_args,
                    last,
                    &command_to_arg_map_for_last_rc,
                );
            }
            last_rc_file = Some(rc_file.clone());
            command_to_arg_map_for_last_rc = IndexMap::new();
        }
        command_to_arg_map_for_last_rc
            .entry(override_.command.clone())
            .or_default()
            .push(override_.option.clone());
    }
    if let Some(last) = &last_rc_file {
        flush(
            &mut command_to_rc_args,
            last,
            &command_to_arg_map_for_last_rc,
        );
    }
    Ok(command_to_rc_args)
}

// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:752 BlazeOptionHandler.partitionCommandLineArgs
/// Splits `--rc_source`/`--default_override` (and their separate values) from the rest.
fn partition_command_line_args(cmd_line: &[String]) -> (Vec<String>, Vec<String>) {
    let mut default_overrides_and_rc_sources = Vec::new();
    let mut remaining_cmd_line = Vec::new();
    let mut it = cmd_line.iter();
    while let Some(option) = it.next() {
        if option.starts_with("--rc_source=") || option.starts_with("--default_override=") {
            default_overrides_and_rc_sources.push(option.clone());
        } else if option == "--rc_source" || option == "--default_override" {
            let possible_argument = it.next();
            default_overrides_and_rc_sources.push(option.clone());
            if let Some(a) = possible_argument {
                default_overrides_and_rc_sources.push(a.clone());
            }
        } else {
            remaining_cmd_line.push(option.clone());
        }
    }
    (default_overrides_and_rc_sources, remaining_cmd_line)
}

// Java: com/google/devtools/build/lib/runtime/ConfigExpander.java:38 ConfigExpander
/// `--config` expansion.
pub struct ConfigExpander;

impl ConfigExpander {
    // Java: com/google/devtools/build/lib/runtime/ConfigExpander.java:42 ConfigExpander.getPlatformName
    /// Linux only (PORT: skipped, other OSes).
    pub fn get_platform_name() -> &'static str {
        "linux"
    }

    // Java: com/google/devtools/build/lib/runtime/ConfigExpander.java:62 ConfigExpander.shouldEnablePlatformSpecificConfig
    fn should_enable_platform_specific_config(
        enable_platform_specific_config_description: Option<&crate::OptionValueDescription>,
        command_to_rc_args: &CommandToRcArgs,
        commands_to_parse: &[String],
    ) -> bool {
        let Some(d) = enable_platform_specific_config_description else {
            return false;
        };
        if d.get_value()
            .and_then(|v| v.downcast_ref::<bool>().copied())
            != Some(true)
        {
            return false;
        }
        commands_to_parse.iter().any(|command_name| {
            command_to_rc_args
                .get(&format!("{}:{}", command_name, Self::get_platform_name()))
                .is_some_and(|v| !v.is_empty())
        })
    }

    // Java: com/google/devtools/build/lib/runtime/ConfigExpander.java:89 ConfigExpander.expandConfigOptions
    pub fn expand_config_options(
        event_handler: &mut StoredEventHandler,
        command_to_rc_args: &CommandToRcArgs,
        current_command: &str,
        commands_to_parse: &[String],
        rc_file_notes: &mut Vec<String>,
        options_parser: &mut OptionsParser,
        fallback_data: Option<Arc<OptionsData>>,
    ) -> Result<(), OptionsParsingException> {
        let config_value_description = options_parser.get_option_value_description("config");
        if let Some(instances) = config_value_description
            .as_ref()
            .and_then(|d| d.get_canonical_instances())
        {
            // Expand the configs that are mentioned in the input. Flatten these expansions before
            // parsing them, to preserve order.
            for config_instance in instances {
                let config_value_to_expand = config_instance
                    .get_converted_value()?
                    .map(|v| v.get::<String>())
                    .unwrap_or_default();
                let expansion = Self::get_expansion(
                    event_handler,
                    command_to_rc_args,
                    commands_to_parse,
                    &config_value_to_expand,
                    rc_file_notes,
                    fallback_data.clone(),
                )?;
                let ignored_args = options_parser.parse_args_as_expansion_of_option(
                    &config_instance,
                    &format!("expanded from --config={}", config_value_to_expand),
                    expansion,
                )?;
                if !ignored_args.is_empty() {
                    rc_file_notes.push(format!(
                        "Ignored as unsupported by '{}': {}",
                        current_command,
                        ignored_args.join(" ")
                    ));
                }
            }
        }

        let enable_platform_specific_config_description =
            options_parser.get_option_value_description("enable_platform_specific_config");
        if Self::should_enable_platform_specific_config(
            enable_platform_specific_config_description.as_ref(),
            command_to_rc_args,
            commands_to_parse,
        ) {
            let expansion = Self::get_expansion(
                event_handler,
                command_to_rc_args,
                commands_to_parse,
                Self::get_platform_name(),
                rc_file_notes,
                fallback_data.clone(),
            )?;
            let instances = enable_platform_specific_config_description
                .unwrap()
                .get_canonical_instances()
                .unwrap_or_default();
            assert!(
                instances.len() == 1,
                "IllegalArgumentException: expected one element"
            );
            let option_to_expand = &instances[0];
            let ignored_args = options_parser.parse_args_as_expansion_of_option(
                option_to_expand,
                "enabled by --enable_platform_specific_config",
                expansion,
            )?;
            if !ignored_args.is_empty() {
                rc_file_notes.push(format!(
                    "Ignored as unsupported by '{}': {}",
                    current_command,
                    ignored_args.join(" ")
                ));
            }
        }

        // Identify duplicates, if any, to warn about re-application.
        let configs: Vec<String> = options_parser
            .get_option_value_description("config")
            .and_then(|d| d.get_value())
            .and_then(|v| {
                v.as_list()
                    .map(|l| l.iter().map(|e| e.get::<String>()).collect())
            })
            .unwrap_or_default();
        let mut config_set = HashSet::new();
        let mut duplicate_configs: IndexSet<String> = IndexSet::new();
        for config_value in configs {
            if !config_set.insert(config_value.clone()) {
                duplicate_configs.insert(config_value);
            }
        }
        if !duplicate_configs.is_empty() {
            event_handler.handle(HandlerEvent::warn(format!(
                "The following configs were expanded more than once: [{}]. For repeatable flags, \
                 repeats are counted twice and may lead to unexpected behavior.",
                duplicate_configs.into_iter().collect::<Vec<_>>().join(", ")
            )));
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/runtime/ConfigExpander.java:183 ConfigExpander.getExpansion
    fn get_expansion(
        event_handler: &mut StoredEventHandler,
        command_to_rc_args: &CommandToRcArgs,
        commands_to_parse: &[String],
        config_to_expand: &str,
        rc_file_notes: &mut Vec<String>,
        fallback_data: Option<Arc<OptionsData>>,
    ) -> Result<Vec<ArgAndFallbackData>, OptionsParsingException> {
        let mut config_ancestor_set: IndexSet<String> = IndexSet::new();
        config_ancestor_set.insert(config_to_expand.to_string());
        let mut longest_chain: Vec<String> = Vec::new();
        let final_expansion = Self::get_expansion_rec(
            command_to_rc_args,
            commands_to_parse,
            &config_ancestor_set,
            config_to_expand,
            &mut longest_chain,
            rc_file_notes,
            &fallback_data,
        )?;
        if longest_chain.len() >= 10 {
            event_handler.handle(HandlerEvent::warn(format!(
                "There is a recursive chain of configs {} configs long: [{}]. This seems \
                 excessive, and might be hiding errors.",
                longest_chain.len(),
                longest_chain.join(", ")
            )));
        }
        Ok(final_expansion)
    }

    // Java: com/google/devtools/build/lib/runtime/ConfigExpander.java:226 ConfigExpander.getExpansion
    fn get_expansion_rec(
        command_to_rc_args: &CommandToRcArgs,
        commands_to_parse: &[String],
        config_ancestor_set: &IndexSet<String>,
        config_to_expand: &str,
        longest_chain: &mut Vec<String>,
        rc_file_notes: &mut Vec<String>,
        fallback_data: &Option<Arc<OptionsData>>,
    ) -> Result<Vec<ArgAndFallbackData>, OptionsParsingException> {
        let mut expansion = Vec::new();
        let mut found_definition = false;
        // The expansion order of rc files is first by command priority, and then in the order
        // the rc files were read, respecting import statement placement.
        for command_to_parse in commands_to_parse {
            let config_def = format!("{}:{}", command_to_parse, config_to_expand);
            let Some(chunks) = command_to_rc_args.get(&config_def) else {
                continue;
            };
            for rc_args in chunks {
                found_definition = true;
                rc_file_notes.push(format!(
                    "Found applicable config definition {} in file {}: {}",
                    config_def,
                    rc_args.get_rc_file(),
                    rc_args.get_args().join(" ")
                ));
                for arg in rc_args.get_args() {
                    expansion.push(ArgAndFallbackData::new(
                        arg.clone(),
                        if command_to_parse == COMMON_PSEUDO_COMMAND {
                            fallback_data.clone()
                        } else {
                            None
                        },
                    ));
                    if arg.len() >= 8 && arg.starts_with("--config") {
                        // Only --config=value is accepted for recursive configs.
                        let Some(char_of_config_value) = arg.find('=') else {
                            return Err(OptionsParsingException::new(format!(
                                "In file {}, the definition of config {} expands to another \
                                 config that either has no value or is not in the form \
                                 --config=value. For recursive config definitions, please do \
                                 not provide the value in a separate token, such as in the form \
                                 '--config value'.",
                                rc_args.get_rc_file(),
                                config_to_expand
                            )));
                        };
                        let new_config_value = &arg[char_of_config_value + 1..];
                        let mut extended = config_ancestor_set.clone();
                        if !extended.insert(new_config_value.to_string()) {
                            return Err(OptionsParsingException::new(format!(
                                "Config expansion has a cycle: config value {} expands to \
                                 itself, see inheritance chain [{}]",
                                new_config_value,
                                extended.iter().cloned().collect::<Vec<_>>().join(", ")
                            )));
                        }
                        if extended.len() > longest_chain.len() {
                            *longest_chain = extended.iter().cloned().collect();
                        }
                        expansion.extend(Self::get_expansion_rec(
                            command_to_rc_args,
                            commands_to_parse,
                            &extended,
                            new_config_value,
                            longest_chain,
                            rc_file_notes,
                            fallback_data,
                        )?);
                    }
                }
            }
        }
        if !found_definition {
            return Err(OptionsParsingException::new(format!(
                "Config value '{}' is not defined in any .rc file",
                config_to_expand
            )));
        }
        Ok(expansion)
    }
}
