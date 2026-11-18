//! Port of `lib/runtime/GotOptionsEvent.java` (the `optionsParsed` build event).
//!
//! # What the runtime passes, and when it posts the event
//!
//! Java posts it from `CommandEnvironment.beforeCommand(invocationPolicy)` (called by
//! `BlazeCommandDispatcher` at line 634), right after `BuildMetadataEvent`, with:
//! - `startup_options`: `runtime.getStartupOptionsProvider()`, the server's parsed startup
//!   options;
//! - `options`: the command environment's options (`env.getOptions()`) as they are at
//!   `beforeCommand`, i.e. before the dispatcher rebuilds the parser with the main repo mapping
//!   and parses the "module resolution" `--flag_alias` options into the rebuilt parser (Java's
//!   `optionsParser.toBuilder()...build()` makes a new parser, so the `--flag_alias` options do
//!   not reach this event's `cmdLine`);
//! - `invocation_policy`: the invocation policy the command ran with (the oracle records the
//!   empty policy `{}`).
//!
//! The event waits for `started` and `unstructuredCommandLine` (`posted_after`).

use std::fmt;
use std::sync::Arc;

use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_options::OptionsParser;
use blaze_options::parsed::ParsedOptionRef;
use blaze_protos::blaze::invocation_policy::InvocationPolicy;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::{self as bes, BuildEventId, OptionsParsed};

use crate::buildeventstream::build_event::{BuildEvent, Interrupted};
use crate::buildeventstream::build_event_context::BuildEventContext;
use crate::buildeventstream::build_event_id_util;
use crate::buildeventstream::generic_build_event::proto_chaining_with;

/// Return a representation of the non-hidden specified options, as a list of string. No escaping
/// is done.
// Java: com/google/devtools/build/lib/util/OptionsUtils.java:62 OptionsUtils.asArgumentList
fn as_argument_list<'a>(
    options_list: impl IntoIterator<Item = &'a ParsedOptionRef>,
) -> Vec<String> {
    let mut builder = Vec::new();
    for option in options_list {
        if option.is_hidden() {
            continue;
        }
        builder.push(option.get_canonical_form());
    }
    builder
}

/// Return a representation of the non-hidden specified options, as a list of string. No escaping
/// is done.
// Java: com/google/devtools/build/lib/util/OptionsUtils.java:62 OptionsUtils.asArgumentList
fn as_argument_list_of_result(options: &OptionsParser) -> Vec<String> {
    as_argument_list(&options.as_complete_list_of_parsed_options())
}

/// An event in which the command line options are discovered.
// Java: com/google/devtools/build/lib/runtime/GotOptionsEvent.java:31 GotOptionsEvent
pub struct GotOptionsEvent {
    startup_options: Arc<OptionsParser>,
    options: Arc<OptionsParser>,
    invocation_policy: InvocationPolicy,
}

impl fmt::Debug for GotOptionsEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GotOptionsEvent")
            .field("invocation_policy", &self.invocation_policy)
            .finish_non_exhaustive()
    }
}

impl GotOptionsEvent {
    /// Construct the options event.
    ///
    /// `startup_options`: the parsed startup options; `options`: the parsed options.
    // Java: com/google/devtools/build/lib/runtime/GotOptionsEvent.java:43 GotOptionsEvent.<init>
    pub fn new(
        startup_options: Arc<OptionsParser>,
        options: Arc<OptionsParser>,
        invocation_policy: InvocationPolicy,
    ) -> Self {
        GotOptionsEvent {
            startup_options,
            options,
            invocation_policy,
        }
    }

    /// Returns the parsed startup options.
    // Java: com/google/devtools/build/lib/runtime/GotOptionsEvent.java:53 GotOptionsEvent.getStartupOptions
    pub fn get_startup_options(&self) -> &Arc<OptionsParser> {
        &self.startup_options
    }

    /// Returns the parsed options.
    // Java: com/google/devtools/build/lib/runtime/GotOptionsEvent.java:58 GotOptionsEvent.getOptions
    pub fn get_options(&self) -> &Arc<OptionsParser> {
        &self.options
    }

    /// Returns the invocation policy.
    // Java: com/google/devtools/build/lib/runtime/GotOptionsEvent.java:63 GotOptionsEvent.getInvocationPolicy
    pub fn get_invocation_policy(&self) -> &InvocationPolicy {
        &self.invocation_policy
    }
}

impl BuildEvent for GotOptionsEvent {
    // Java: com/google/devtools/build/lib/runtime/GotOptionsEvent.java:68 GotOptionsEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::options_parsed_id()
    }

    // Java: com/google/devtools/build/lib/runtime/GotOptionsEvent.java:73 GotOptionsEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/runtime/GotOptionsEvent.java:78 GotOptionsEvent.asStreamProto
    fn as_stream_proto(
        &self,
        _converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        let mut options_builder = OptionsParsed::default();

        let options = self.get_startup_options();
        options_builder
            .startup_options
            .extend(as_argument_list_of_result(options));
        options_builder
            .explicit_startup_options
            .extend(as_argument_list(
                options
                    .as_list_of_explicit_options()
                    .iter()
                    .filter(|input| input.get_source() != Some("default")),
            ));
        let options = self.get_options();
        options_builder
            .cmd_line
            .extend(as_argument_list_of_result(options));
        options_builder.explicit_cmd_line.extend(as_argument_list(
            options
                .as_list_of_explicit_options()
                .iter()
                .filter(|input| input.get_source() == Some("command line options")),
        ));

        options_builder.invocation_policy = Some(self.get_invocation_policy().clone());

        let common_options = self
            .get_options()
            .get_options::<CommonCommandOptions>()
            .expect("NullPointerException: CommonCommandOptions");
        options_builder.tool_tag = common_options.tool_tag;

        Ok(proto_chaining_with(
            self,
            Payload::OptionsParsed(options_builder),
        ))
    }

    // Java: com/google/devtools/build/lib/runtime/GotOptionsEvent.java:106 GotOptionsEvent.postedAfter
    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        Some(vec![
            build_event_id_util::build_started_id(),
            build_event_id_util::unstructured_commandline_id(),
        ])
    }
}

impl blaze_events::Postable for GotOptionsEvent {}
