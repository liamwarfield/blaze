//! Port of `lib/runtime/CommandLineEvent.java` (`OriginalCommandLineEvent`,
//! `CanonicalCommandLineEvent`, `ToolCommandLineEvent` and their shared base
//! `BazelCommandLineEvent`) and `lib/runtime/OriginalUnstructuredCommandLineEvent.java`.
//!
//! # What the runtime passes, and when it posts these events
//!
//! Java posts all four events from `BlazeCommandDispatcher.execExclusively` (lines 779-823),
//! after the command options are final: after `env.beforeCommand` (which posted
//! `GotOptionsEvent`), after the main repo mapping was computed and the `MODULE.bazel`
//! `flag_alias` entries were parsed into the options parser, and after
//! `optionHandler.parseStarlarkOptions`. They are posted, in this order:
//!
//! 1. [`OriginalUnstructuredCommandLineEvent`]: `new(args)` where `args` is the full argument
//!    list the server received from the client (`RunRequest.arg`: the command name, then the
//!    client's logging args `--startup_time=..`, `--command_wait_time=..`, `--binary_path=..`,
//!    `--rc_source=..`, `--default_override=..`, `--client_env=..`, `--client_cwd=..`, then the
//!    user's command arguments in their original order, residue included). For
//!    `commandName == "run" && !includeResidueInRunBepEvent` it is
//!    [`OriginalUnstructuredCommandLineEvent::redacted_unstructured_command_line_event`] instead.
//! 2. [`OriginalCommandLineEvent::new`] with
//!    - `product_name`: `runtime.getProductName()` (`"bazel"`);
//!    - `active_startup_options`: `runtime.getStartupOptionsProvider()`, the server's parsed
//!      startup options (used only when `original_startup_options` is `None`);
//!    - `command_name`: the command (`"build"`);
//!    - `residue`: `options.getResidue()` of the final command options parser;
//!    - `include_residue_in_run_bep_event`: `BuildEventProtocolOptions.includeResidueInRunBepEvent`
//!      (`false` when that options class is not registered for the command);
//!    - `explicit_options`: `options.asListOfExplicitOptions()` (only the `COMMAND_LINE` priority
//!      ones are reported, which drops rc-file options and the "module resolution"
//!      `--flag_alias` options, parsed at `RC_FILE` priority);
//!    - `explicit_starlark_options`:
//!      `options.getExplicitStarlarkOptions(OriginalCommandLineEvent::command_line_priority)`;
//!    - `original_startup_options`: `startupOptionsTaggedWithBazelRc`, i.e. the `(source, option)`
//!      pairs of `RunRequest.startup_options` exactly as the client sent them (source `""` = the
//!      command line). The 9.3.2 client calls `request.add_startup_options()` twice per startup
//!      option (blaze.cc `BlazeServer::Communicate`), so every real pair is followed by an empty
//!      `("", "")` pair; this is why the oracle's "startup options" section is
//!      `{combinedForm: ..}, {}, {combinedForm: ..}, {}, ...`. Pass the list unfiltered. Pass
//!      `None` only when there is no client request (Java `Optional.empty()`).
//! 3. [`CanonicalCommandLineEvent::new`] with the same first five arguments, then
//!    `explicit_starlark_options` (as above), `starlark_options`: `options.getStarlarkOptions()`,
//!    `canonical_options`: `options.asListOfCanonicalOptions()` (sorted by priority, so the
//!    "module resolution" `--flag_alias=..` options, parsed by `BlazeCommandDispatcher` at
//!    `RC_FILE` priority with source `"module resolution"`, come before the command-line ones),
//!    and `replaceable`: `commandAnnotation.buildPhase().analyzes()`. When it is replaceable,
//!    `BuildTool` later posts either a replacement `CanonicalCommandLineEvent` (PROJECT.scl flags)
//!    or a `ReleaseReplaceableBuildEvent` for `structured_commandline_id("canonical")`.
//! 4. [`ToolCommandLineEvent::from_option_value`] of `CommonCommandOptions.toolCommandLine`
//!    (`--experimental_tool_command_line`; the default is the empty `CommandLine`).
//!
//! All four wait for `BuildStartingEvent` (`posted_after` = `started`).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use blaze_optdefs::command::blaze_server_startup_options::BlazeServerStartupOptions;
use blaze_optdefs::command::command_line_event::ToolCommandLineEvent as OptionToolCommandLineEvent;
use blaze_options::parsed::ParsedOptionRef;
use blaze_options::{
    OptionDefinition, OptionEffectTag, OptionMetadataTag, OptionsClass, OptionsParser,
    PriorityCategory, Value,
};
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::{self as bes, BuildEventId, UnstructuredCommandLine};
use blaze_protos::command_line::command_line_section::SectionType;
use blaze_protos::command_line::{
    ChunkList, CommandLine, CommandLineSection, Option as ProtoOption, OptionList,
};

use crate::buildeventstream::build_event::{BuildEvent, Interrupted};
use crate::buildeventstream::build_event_context::BuildEventContext;
use crate::buildeventstream::build_event_id_util;
use crate::buildeventstream::generic_build_event::proto_chaining_with;
pub use crate::events::buildtool_events::{
    CANONICAL_COMMAND_LINE_LABEL, ORIGINAL_COMMAND_LINE_LABEL, TOOL_COMMAND_LINE_LABEL,
};

/// `CommandLineEvent.getChildrenEvents`.
// Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:57 CommandLineEvent.getChildrenEvents
fn command_line_event_children_events() -> Vec<BuildEventId> {
    Vec::new()
}

/// `CommandLineEvent.postedAfter`.
// Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:62 CommandLineEvent.postedAfter
fn command_line_event_posted_after() -> Option<Vec<BuildEventId>> {
    Some(vec![build_event_id_util::build_started_id()])
}

/// A CommandLineEvent that stores functions and values common to both Bazel command lines.
// Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:67 CommandLineEvent.BazelCommandLineEvent
pub struct BazelCommandLineEvent {
    product_name: String,
    active_startup_options: Arc<OptionsParser>,
    command_name: String,
    residue: Vec<String>,
    include_residue_in_run_bep_event: bool,
}

impl fmt::Debug for BazelCommandLineEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BazelCommandLineEvent")
            .field("product_name", &self.product_name)
            .field("command_name", &self.command_name)
            .field("residue", &self.residue)
            .field(
                "include_residue_in_run_bep_event",
                &self.include_residue_in_run_bep_event,
            )
            .finish_non_exhaustive()
    }
}

impl BazelCommandLineEvent {
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:74 BazelCommandLineEvent.<init>
    fn new(
        product_name: String,
        active_startup_options: Arc<OptionsParser>,
        command_name: String,
        residue: Vec<String>,
        include_residue_in_run_bep_event: bool,
    ) -> Self {
        BazelCommandLineEvent {
            product_name,
            active_startup_options,
            command_name,
            residue,
            include_residue_in_run_bep_event,
        }
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:87 BazelCommandLineEvent.getExecutableSection
    fn get_executable_section(&self) -> CommandLineSection {
        CommandLineSection {
            section_label: "executable".to_string(),
            section_type: Some(SectionType::ChunkList(ChunkList {
                chunk: vec![self.product_name.clone()],
            })),
        }
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:94 BazelCommandLineEvent.getCommandSection
    fn get_command_section(&self) -> CommandLineSection {
        CommandLineSection {
            section_label: "command".to_string(),
            section_type: Some(SectionType::ChunkList(ChunkList {
                chunk: vec![self.command_name.clone()],
            })),
        }
    }

    /// Convert an array of tags to the equivalent proto-generated enum values.
    ///
    /// The proto type is duplicate in order to not burden the OptionsParser with the proto
    /// dependency. A test guarantees that the two enum types are kept in sync with matching
    /// indices.
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:107 BazelCommandLineEvent.getProtoEffectTags
    pub fn get_proto_effect_tags(tag_array: &[OptionEffectTag]) -> Vec<i32> {
        let mut effect_tags = Vec::with_capacity(tag_array.len());
        for tag in tag_array {
            // PORT: OptionFilters.OptionEffectTag.forNumber(n) is stored as its number n.
            effect_tags.push(tag.get_value());
        }
        effect_tags
    }

    /// Convert an array of tags to the equivalent proto-generated enum values.
    ///
    /// The proto type is duplicate in order to not burden the OptionsParser with the proto
    /// dependency. A test guarantees that the two enum types are kept in sync with matching
    /// indices.
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:121 BazelCommandLineEvent.getProtoMetadataTags
    pub fn get_proto_metadata_tags(tag_array: &[OptionMetadataTag]) -> Vec<i32> {
        let mut metadata_tags = Vec::with_capacity(tag_array.len());
        for tag in tag_array {
            // PORT: OptionFilters.OptionMetadataTag.forNumber(n) is stored as its number n.
            metadata_tags.push(tag.get_value());
        }
        metadata_tags
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:130 BazelCommandLineEvent.getOptionListFromParsedOptionDescriptions
    fn get_option_list_from_parsed_option_descriptions(
        &self,
        parsed_option_descriptions: &[ParsedOptionRef],
    ) -> Vec<ProtoOption> {
        let mut options = Vec::new();
        for parsed_option in parsed_option_descriptions {
            options.push(Self::create_option(
                parsed_option.get_option_definition(),
                parsed_option.get_source(),
                // Option.Builder.setCombinedForm(null) throws NullPointerException.
                parsed_option
                    .get_command_line_form()
                    .expect("NullPointerException: combinedForm"),
                parsed_option.get_unconverted_value(),
            ));
        }
        options
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:135 BazelCommandLineEvent.createOption
    fn create_option(
        option_definition: &OptionDefinition,
        source: Option<&str>,
        combined_form: &str,
        value: Option<&str>,
    ) -> ProtoOption {
        let mut option = ProtoOption {
            combined_form: combined_form.to_string(),
            option_name: option_definition.get_option_name().to_string(),
            ..Default::default()
        };
        if let Some(value) = value {
            option.option_value = value.to_string();
        }
        option.effect_tags.extend(Self::get_proto_effect_tags(
            option_definition.get_option_effect_tags(),
        ));
        option.metadata_tags.extend(Self::get_proto_metadata_tags(
            option_definition.get_option_metadata_tags(),
        ));
        if let Some(source) = source {
            option.source = source.to_string();
        }
        option
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:163 BazelCommandLineEvent.createStarlarkOption
    fn create_starlark_option(starlark_flag: &str, value: Option<&Value>) -> ProtoOption {
        // String.format("--%s=%s", starlarkFlag, value): a null value formats as "null".
        let value_string = value.map(Value::java_to_string);
        let combined_form = format!(
            "--{}={}",
            starlark_flag,
            value_string.as_deref().unwrap_or("null")
        );
        let mut option = ProtoOption {
            combined_form,
            option_name: starlark_flag.to_string(),
            ..Default::default()
        };
        if let Some(value_string) = value_string {
            option.option_value = value_string;
        }
        option
    }

    /// Returns the startup option section of the command line for the startup options as the
    /// server received them at its startup. Since not all client options get passed to the
    /// server as startup options, this might not represent the actual list of startup options as
    /// the user provided them.
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:180 BazelCommandLineEvent.getActiveStartupOptions
    fn get_active_startup_options(&self) -> CommandLineSection {
        CommandLineSection {
            section_label: "startup options".to_string(),
            section_type: Some(SectionType::OptionList(OptionList {
                option: self.get_option_list_from_parsed_option_descriptions(
                    &self
                        .active_startup_options
                        .as_complete_list_of_parsed_options(),
                ),
            })),
        }
    }

    /// Returns the final part of the command line, containing whatever was left after obtaining
    /// the command and its options.
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:195 BazelCommandLineEvent.getResidual
    fn get_residual(&self) -> CommandLineSection {
        // Potential further split: how the residual, if any is accepted, gets interpreted depends
        // on the command. For example, for build commands, we might want to consider separating
        // out project files, as in runtime.commands.ProjectFileSupport. To properly report this,
        // we would need to let the command customize how the residual is listed. This catch-all
        // could serve as a default in this case.
        let chunk_list = if self.command_name == "run"
            && !self.include_residue_in_run_bep_event
            && !self.residue.is_empty()
        {
            let target = self.residue[0].clone();
            let mut residual = ChunkList {
                chunk: vec![target],
            };
            if self.residue.len() > 1 {
                residual.chunk.push("REDACTED".to_string());
            }
            residual
        } else {
            ChunkList {
                chunk: self.residue.clone(),
            }
        };
        CommandLineSection {
            section_label: "residual".to_string(),
            section_type: Some(SectionType::ChunkList(chunk_list)),
        }
    }
}

/// This reports a reassembled version of the command line as Bazel received it.
// Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:218 CommandLineEvent.OriginalCommandLineEvent
#[derive(Debug)]
pub struct OriginalCommandLineEvent {
    base: BazelCommandLineEvent,
    explicit_options: Vec<ParsedOptionRef>,
    explicit_starlark_options: BTreeMap<String, Value>,
    /// PORT: Java `Optional<List<Pair<String, String>>>` (source, option). The pair's source is
    /// never null in Java (GrpcServerImpl builds it from proto strings), so it is a `String`.
    original_startup_options: Option<Vec<(String, String)>>,
}

impl OriginalCommandLineEvent {
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:219 OriginalCommandLineEvent.LABEL
    pub const LABEL: &'static str = ORIGINAL_COMMAND_LINE_LABEL;

    /// PORT: Java's public constructor takes the `BlazeRuntime` and reads
    /// `runtime.getProductName()` and `runtime.getStartupOptionsProvider()`; the runtime passes
    /// those two values here (the `@VisibleForTesting` constructor).
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:224 OriginalCommandLineEvent.<init>
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:224 OriginalCommandLineEvent.<init>
    pub fn new(
        product_name: String,
        active_startup_options: Arc<OptionsParser>,
        command_name: String,
        residue: Vec<String>,
        include_residue_in_run_bep_event: bool,
        explicit_options: Vec<ParsedOptionRef>,
        explicit_starlark_options: BTreeMap<String, Value>,
        original_startup_options: Option<Vec<(String, String)>>,
    ) -> Self {
        OriginalCommandLineEvent {
            base: BazelCommandLineEvent::new(
                product_name,
                active_startup_options,
                command_name,
                residue,
                include_residue_in_run_bep_event,
            ),
            explicit_options,
            explicit_starlark_options,
            original_startup_options,
        }
    }

    /// Returns the literal command line options as received. These are not the final parsed
    /// values, but are passed as is from the client, so we do not have the full
    /// OptionDefinition information. In this form, only set the "combinedForm" field.
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:269 OriginalCommandLineEvent.getStartupOptionSection
    fn get_startup_option_section(&self) -> CommandLineSection {
        if let Some(original_startup_options) = &self.original_startup_options {
            let mut options = Vec::new();
            for (source, option) in original_startup_options {
                // Only add the options that were added by the command line.
                // TODO(b/19881919) decide the format that option source information should take
                // and then add all options, tagged with the source, instead of filtering out the
                // rc options.
                if source.is_empty() {
                    options.push(ProtoOption {
                        combined_form: option.clone(),
                        ..Default::default()
                    });
                }
            }
            CommandLineSection {
                section_label: "startup options".to_string(),
                section_type: Some(SectionType::OptionList(OptionList { option: options })),
            }
        } else {
            // If we were not provided with the startup options, fallback to reporting the active
            // ones stored by the Bazel Runtime.
            self.base.get_active_startup_options()
        }
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:292 OriginalCommandLineEvent.commandLinePriority
    pub fn command_line_priority(parsed_option_description: &ParsedOptionRef) -> bool {
        parsed_option_description
            .get_priority()
            .get_priority_category()
            == PriorityCategory::COMMAND_LINE
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:297 OriginalCommandLineEvent.getExplicitCommandOptions
    fn get_explicit_command_options(&self) -> CommandLineSection {
        let explicit_options_command_line_priority: Vec<ParsedOptionRef> = self
            .explicit_options
            .iter()
            .filter(|o| Self::command_line_priority(o))
            .cloned()
            .collect();
        let starlark_options: Vec<ProtoOption> = self
            .explicit_starlark_options
            .iter()
            .map(|(k, v)| BazelCommandLineEvent::create_starlark_option(k, Some(v)))
            .collect();
        let mut option = self.base.get_option_list_from_parsed_option_descriptions(
            &explicit_options_command_line_priority,
        );
        option.extend(starlark_options);
        CommandLineSection {
            section_label: "command options".to_string(),
            section_type: Some(SectionType::OptionList(OptionList { option })),
        }
    }
}

impl BuildEvent for OriginalCommandLineEvent {
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:260 OriginalCommandLineEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::structured_commandline_id(Self::LABEL)
    }

    fn get_children_events(&self) -> Vec<BuildEventId> {
        command_line_event_children_events()
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:317 OriginalCommandLineEvent.asStreamProto
    fn as_stream_proto(
        &self,
        _converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        Ok(proto_chaining_with(
            self,
            Payload::StructuredCommandLine(CommandLine {
                command_line_label: Self::LABEL.to_string(),
                sections: vec![
                    self.base.get_executable_section(),
                    self.get_startup_option_section(),
                    self.base.get_command_section(),
                    self.get_explicit_command_options(),
                    self.base.get_residual(),
                ],
            }),
        ))
    }

    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        command_line_event_posted_after()
    }
}

impl blaze_events::Postable for OriginalCommandLineEvent {}

/// This reports the canonical form of the command line.
// Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:333 CommandLineEvent.CanonicalCommandLineEvent
#[derive(Debug)]
pub struct CanonicalCommandLineEvent {
    base: BazelCommandLineEvent,
    #[allow(dead_code)]
    explicit_starlark_options: BTreeMap<String, Value>,
    starlark_options: BTreeMap<String, Value>,
    canonical_options: Vec<ParsedOptionRef>,
    replaceable: bool,
}

impl CanonicalCommandLineEvent {
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:335 CanonicalCommandLineEvent.LABEL
    pub const LABEL: &'static str = CANONICAL_COMMAND_LINE_LABEL;

    /// PORT: Java's public constructor takes the `BlazeRuntime` and reads
    /// `runtime.getProductName()` and `runtime.getStartupOptionsProvider()`; the runtime passes
    /// those two values here (the `@VisibleForTesting` constructor).
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:341 CanonicalCommandLineEvent.<init>
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:341 CanonicalCommandLineEvent.<init>
    pub fn new(
        product_name: String,
        active_startup_options: Arc<OptionsParser>,
        command_name: String,
        residue: Vec<String>,
        include_residue_in_run_bep_event: bool,
        explicit_starlark_options: BTreeMap<String, Value>,
        starlark_options: BTreeMap<String, Value>,
        canonical_options: Vec<ParsedOptionRef>,
        replaceable: bool,
    ) -> Self {
        CanonicalCommandLineEvent {
            base: BazelCommandLineEvent::new(
                product_name,
                active_startup_options,
                command_name,
                residue,
                include_residue_in_run_bep_event,
            ),
            explicit_starlark_options,
            starlark_options,
            canonical_options,
            replaceable,
        }
    }

    /// Returns the effective startup options.
    ///
    /// Since in this command line the command options include invocation policy's and rcs'
    /// contents expanded fully, the list of startup options should prevent reapplication of these
    /// contents.
    ///
    /// The options parser does not understand the effect of these flags, since the relationship
    /// between these startup options and the command options is not held within the options
    /// parser, so instead, we add a small hack. Remove any explicit mentions of these flags, and
    /// explicitly add the options that prevent Blaze from looking for the default rc files.
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:402 CanonicalCommandLineEvent.getCanonicalStartupOptions
    fn get_canonical_startup_options(&self) -> CommandLineSection {
        let unfiltered_options = match self.base.get_active_startup_options().section_type {
            Some(SectionType::OptionList(list)) => list.option,
            _ => Vec::new(),
        };
        // Create the fake ones to prevent reapplication of the original rc file contents.
        let mut fake_options = OptionsParser::builder()
            .options_classes(&[BlazeServerStartupOptions::class()])
            .build()
            .unwrap_or_else(|e| panic!("{e:?}"));
        if let Err(e) = fake_options.parse(&["--ignore_all_rc_files"]) {
            // Unless someone changes the definition of these flags, this is impossible.
            panic!("IllegalStateException: {e:?}");
        }

        // Remove any instances of the applied, and add the new blocking ones.
        let mut option: Vec<ProtoOption> = unfiltered_options
            .into_iter()
            .filter(|option| {
                let option_name = option.option_name.as_str();
                option_name != "ignore_all_rc_files"
                    && option_name != "blazerc"
                    && option_name != "master_blazerc"
                    && option_name != "bazelrc"
                    && option_name != "master_bazelrc"
                    && option_name != "invocation_policy"
            })
            .collect();
        option.extend(self.base.get_option_list_from_parsed_option_descriptions(
            &fake_options.as_complete_list_of_parsed_options(),
        ));
        CommandLineSection {
            section_label: "startup options".to_string(),
            section_type: Some(SectionType::OptionList(OptionList { option })),
        }
    }

    /// Returns the canonical command options, overridden and default values are not listed.
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:439 CanonicalCommandLineEvent.getCanonicalCommandOptions
    fn get_canonical_command_options(&self) -> CommandLineSection {
        let starlark_options_as_list: Vec<ProtoOption> = self
            .starlark_options
            .iter()
            .map(|(k, v)| BazelCommandLineEvent::create_starlark_option(k, Some(v)))
            .collect();
        let mut option = self
            .base
            .get_option_list_from_parsed_option_descriptions(&self.canonical_options);
        option.extend(starlark_options_as_list);
        CommandLineSection {
            section_label: "command options".to_string(),
            section_type: Some(SectionType::OptionList(OptionList { option })),
        }
    }

    /// Hash including the explicit command line options as well as the residue, e.g. the
    /// targets.
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:456 CanonicalCommandLineEvent.getExplicitCommandLineHash
    pub fn get_explicit_command_line_hash(&self) -> i64 {
        let mut hash: i64 = 0;
        for (key, value) in &self.starlark_options {
            // Map.Entry.toString(): "key=value".
            let entry = format!("{}={}", key, value.java_to_string());
            hash = hash
                .wrapping_mul(31)
                .wrapping_add(java_string_hash_code(&entry) as i64);
        }
        for canonical_option_desc in &self.canonical_options {
            if canonical_option_desc.is_hidden()
                || canonical_option_desc.get_source() != Some("command line options")
            {
                continue;
            }
            hash = hash.wrapping_mul(31).wrapping_add(java_string_hash_code(
                &canonical_option_desc.get_canonical_form(),
            ) as i64);
        }
        for r in &self.base.residue {
            hash = hash
                .wrapping_mul(31)
                .wrapping_add(java_string_hash_code(r) as i64);
        }
        hash
    }
}

/// Java `String.hashCode`.
///
/// PORT: StringEncoding identity. These strings are Bazel "internal" strings (one Java `char`
/// per UTF-8 byte), so the hash runs over the UTF-8 bytes.
fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for &b in s.as_bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as i32);
    }
    h
}

impl BuildEvent for CanonicalCommandLineEvent {
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:381 CanonicalCommandLineEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::structured_commandline_id(Self::LABEL)
    }

    fn get_children_events(&self) -> Vec<BuildEventId> {
        command_line_event_children_events()
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:476 CanonicalCommandLineEvent.asStreamProto
    fn as_stream_proto(
        &self,
        _converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        Ok(proto_chaining_with(
            self,
            Payload::StructuredCommandLine(CommandLine {
                command_line_label: Self::LABEL.to_string(),
                sections: vec![
                    self.base.get_executable_section(),
                    self.get_canonical_startup_options(),
                    self.base.get_command_section(),
                    self.get_canonical_command_options(),
                    self.base.get_residual(),
                ],
            }),
        ))
    }

    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        command_line_event_posted_after()
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:386 CanonicalCommandLineEvent.replaceable
    fn replaceable(&self) -> Option<bool> {
        Some(self.replaceable)
    }
}

impl blaze_events::Postable for CanonicalCommandLineEvent {}

/// A command line that Bazel accepts via flag (yes, we see the irony there).
///
/// Permits Bazel to report command lines from the tool that invoked it, if such a tool exists.
///
/// PORT: the option value (`CommonCommandOptions.toolCommandLine`) and its converter live in
/// `blaze_optdefs::command::command_line_event`, which models the proto with the fields the
/// converter fills in; [`ToolCommandLineEvent::from_option_value`] turns that value into this
/// event.
// Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:496 CommandLineEvent.ToolCommandLineEvent
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCommandLineEvent {
    command_line: CommandLine,
}

impl ToolCommandLineEvent {
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:497 ToolCommandLineEvent.LABEL
    pub const LABEL: &'static str = TOOL_COMMAND_LINE_LABEL;

    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:500 ToolCommandLineEvent.<init>
    pub fn new(command_line: CommandLine) -> Self {
        ToolCommandLineEvent { command_line }
    }

    /// The event for the converted `--experimental_tool_command_line` value
    /// (`commonOptions.toolCommandLine`, which Java posts directly).
    pub fn from_option_value(value: &OptionToolCommandLineEvent) -> Self {
        let model = &value.command_line;
        ToolCommandLineEvent::new(CommandLine {
            command_line_label: model.command_line_label.clone(),
            sections: model
                .sections
                .iter()
                .map(|chunks| CommandLineSection {
                    section_label: String::new(),
                    section_type: Some(SectionType::ChunkList(ChunkList {
                        chunk: chunks.clone(),
                    })),
                })
                .collect(),
        })
    }

    pub fn get_command_line(&self) -> &CommandLine {
        &self.command_line
    }
}

impl BuildEvent for ToolCommandLineEvent {
    /// The label of this command line event is always "tool," so that the BuildStartingEvent
    /// correctly tracks its children. The provided command line may have its own label that will
    /// be more descriptive.
    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:515 ToolCommandLineEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::structured_commandline_id(Self::LABEL)
    }

    fn get_children_events(&self) -> Vec<BuildEventId> {
        command_line_event_children_events()
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLineEvent.java:505 ToolCommandLineEvent.asStreamProto
    fn as_stream_proto(
        &self,
        _converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        Ok(proto_chaining_with(
            self,
            Payload::StructuredCommandLine(self.command_line.clone()),
        ))
    }

    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        command_line_event_posted_after()
    }
}

impl blaze_events::Postable for ToolCommandLineEvent {}

/// A build event reporting the original commandline by which bazel was invoked.
// Java: com/google/devtools/build/lib/runtime/OriginalUnstructuredCommandLineEvent.java:28 OriginalUnstructuredCommandLineEvent
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginalUnstructuredCommandLineEvent {
    args: Vec<String>,
}

impl OriginalUnstructuredCommandLineEvent {
    /// `REDACTED_UNSTRUCTURED_COMMAND_LINE_EVENT`.
    // Java: com/google/devtools/build/lib/runtime/OriginalUnstructuredCommandLineEvent.java:29 OriginalUnstructuredCommandLineEvent.REDACTED_UNSTRUCTURED_COMMAND_LINE_EVENT
    pub fn redacted_unstructured_command_line_event() -> Self {
        OriginalUnstructuredCommandLineEvent::new(vec!["REDACTED".to_string()])
    }

    // Java: com/google/devtools/build/lib/runtime/OriginalUnstructuredCommandLineEvent.java:33 OriginalUnstructuredCommandLineEvent.<init>
    pub fn new(args: Vec<String>) -> Self {
        OriginalUnstructuredCommandLineEvent { args }
    }
}

impl BuildEvent for OriginalUnstructuredCommandLineEvent {
    // Java: com/google/devtools/build/lib/runtime/OriginalUnstructuredCommandLineEvent.java:38 OriginalUnstructuredCommandLineEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::unstructured_commandline_id()
    }

    // Java: com/google/devtools/build/lib/runtime/OriginalUnstructuredCommandLineEvent.java:43 OriginalUnstructuredCommandLineEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/runtime/OriginalUnstructuredCommandLineEvent.java:48 OriginalUnstructuredCommandLineEvent.postedAfter
    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        Some(vec![build_event_id_util::build_started_id()])
    }

    // Java: com/google/devtools/build/lib/runtime/OriginalUnstructuredCommandLineEvent.java:53 OriginalUnstructuredCommandLineEvent.asStreamProto
    fn as_stream_proto(
        &self,
        _converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        Ok(proto_chaining_with(
            self,
            Payload::UnstructuredCommandLine(UnstructuredCommandLine {
                args: self.args.clone(),
            }),
        ))
    }
}

impl blaze_events::Postable for OriginalUnstructuredCommandLineEvent {}
