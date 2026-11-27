//! Port of `query2/aquery/ActionGraphTextOutputFormatterCallback.java` (`--output=text`, the
//! default of aquery, and `--output=commands`).
//!
//! The text is built in a `StringBuilder` of internal strings (labels, exec paths, file
//! contents) and written with `getBytes(UTF_8)`: an internal string's non-ASCII bytes leave
//! double encoded, as in Bazel. Arguments, environment and artifact paths go through
//! `internalToEscapedUnicode` first and are ASCII.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine;
use blaze_actions::actions::action::Action;
use blaze_actions::actions::action_key_context::ActionKeyContext;
use blaze_actions::actions::action_owner::AspectDescriptor;
use blaze_actions::skyframe::actiongraph::v2::action_graph_dump::{
    ActionGraphDumpError, get_action_inputs, matches_aquery_filters,
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
use blaze_shell::util::command_description_form::CommandDescriptionForm;
use blaze_shell::util::command_failure_utils::CommandFailureUtils;
use blaze_shell::util::script_util::ScriptUtil;
use blaze_shell::util::shell_escaper::ShellEscaper;
use blaze_util::string_encoding::internal_to_unicode;

use crate::aquery::action_graph_query_environment::{
    ActionGraphQueryEnvironment, KeyedConfiguredTargetValue,
};
use crate::aquery::aquery_action_filter::AqueryActionFilter;
use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;

// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:62 ActionGraphTextOutputFormatterCallback.OutputType
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputType {
    TEXT,
    COMMANDS,
}

impl OutputType {
    /// `formatName`.
    pub fn format_name(self) -> &'static str {
        match self {
            OutputType::TEXT => "text",
            OutputType::COMMANDS => "commands",
        }
    }
}

/// Output callback for aquery, prints human readable output.
// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:61 ActionGraphTextOutputFormatterCallback
pub struct ActionGraphTextOutputFormatterCallback<'a> {
    // AqueryThreadsafeCallback
    // Java: com/google/devtools/build/lib/query2/aquery/AqueryThreadsafeCallback.java:24 AqueryThreadsafeCallback
    #[allow(dead_code)]
    event_handler: Arc<dyn ExtendedEventHandler>,
    options: &'a AqueryOptions,
    /// `printStream` (`new PrintStream(out)`); `writeAction` writes bytes to it.
    print_stream: OutputStream,
    accessor: &'a ActionGraphQueryEnvironment,
    // ActionGraphTextOutputFormatterCallback
    action_key_context: ActionKeyContext,
    output_type: OutputType,
    action_filters: &'a AqueryActionFilter,
    /// `paramFileNameToContentMap` (lazily created in Java).
    param_file_name_to_content_map: RefCell<HashMap<String, String>>,
    io_exception: IoExceptionSlot,
}

impl<'a> ActionGraphTextOutputFormatterCallback<'a> {
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:79 ActionGraphTextOutputFormatterCallback.<init>
    pub fn new(
        event_handler: Arc<dyn ExtendedEventHandler>,
        options: &'a AqueryOptions,
        out: OutputStream,
        accessor: &'a ActionGraphQueryEnvironment,
        output_type: OutputType,
        action_filters: &'a AqueryActionFilter,
    ) -> Self {
        // PORT: Java's LabelPrinter argument is the environment's (`accessor.get_label_printer()`).
        ActionGraphTextOutputFormatterCallback {
            event_handler,
            options,
            print_stream: out,
            accessor,
            action_key_context: ActionKeyContext::new(),
            output_type,
            action_filters,
            param_file_name_to_content_map: RefCell::new(HashMap::new()),
            io_exception: IoExceptionSlot::new(),
        }
    }

    /// `writeAction` with `processOutput`'s handling of its exceptions.
    fn write_action_or_fail(&self, action: &dyn Action) -> Result<(), OutputFormatterError> {
        match self.write_action(action) {
            Ok(()) => Ok(()),
            Err(ActionGraphDumpError::Interrupted) => Err(OutputFormatterError::Interrupted(
                InterruptedException::new(),
            )),
            Err(ActionGraphDumpError::Io(e)) => Err(e.into()),
            // catch (CommandLineExpansionException | EvalException e) {
            //   throw new IOException(e.getMessage()); }
            Err(e) => Err(std::io::Error::other(e.to_string()).into()),
        }
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:130 ActionGraphTextOutputFormatterCallback.writeAction
    fn write_action(&self, action: &dyn Action) -> Result<(), ActionGraphDumpError> {
        if self.options.include_param_files {
            // `action instanceof ParameterFileWriteAction parameterFileWriteAction`
            if let Some(arguments) = action.aquery_parameter_file_arguments() {
                let file_content = arguments?.join(" \\\n    ");
                let param_file_name = action
                    .get_primary_output()
                    .get_exec_path_string()
                    .to_string();

                self.param_file_name_to_content_map
                    .borrow_mut()
                    .insert(param_file_name, file_content);
            }
        }

        if !matches_aquery_filters(
            action,
            self.action_filters,
            self.options.include_pruned_inputs,
        ) {
            return Ok(());
        }

        let mut string_builder = String::new();
        match self.output_type {
            OutputType::TEXT => self.write_text(action, &mut string_builder)?,
            OutputType::COMMANDS => write_command(action, &mut string_builder)?,
        }
        // printStream.write(stringBuilder.toString().getBytes(UTF_8))
        self.print_stream
            .borrow_mut()
            .write_all(string_builder.as_bytes())?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:153 ActionGraphTextOutputFormatterCallback.writeText
    fn write_text(
        &self,
        action: &dyn Action,
        string_builder: &mut String,
    ) -> Result<(), ActionGraphDumpError> {
        let label_printer = self.accessor.get_label_printer();
        // Enabling includeParamFiles should enable includeCommandline by default
        // (processOutput: options.includeCommandline |= options.includeParamFiles).
        let include_commandline =
            self.options.include_commandline || self.options.include_param_files;
        let action_owner = action.get_owner();
        string_builder.push_str(&action.pretty_print());
        string_builder.push('\n');
        string_builder.push_str("  Mnemonic: ");
        string_builder.push_str(&action.get_mnemonic());
        string_builder.push('\n');

        // PORT: every Rust action has an owner (Java: actionOwner != null).
        {
            let configuration = action_owner
                .get_build_configuration_event()
                .expect("NullPointerException: action owner has no BuildConfigurationEvent");
            string_builder.push_str("  Target: ");
            string_builder.push_str(
                &action_owner
                    .get_label()
                    .map(|l| label_printer.to_string(l))
                    .unwrap_or_else(|| "null".to_owned()),
            );
            string_builder.push('\n');
            string_builder.push_str("  Configuration: ");
            string_builder.push_str(&configuration.configuration.mnemonic);
            string_builder.push('\n');
            if let Some(platform) = action_owner.get_execution_platform() {
                string_builder.push_str("  Execution platform: ");
                string_builder.push_str(&label_printer.to_string(platform.label()));
                string_builder.push('\n');
            }

            // In the case of aspect-on-aspect, AspectDescriptors are listed in
            // topological order of the dependency graph.
            // e.g. [A -> B] would imply that aspect A is applied on top of aspect B.
            write_aspect_descriptors(action_owner.get_aspect_descriptors(), string_builder);
        }

        if action.is_action_execution_metadata() {
            string_builder.push_str("  ActionKey: ");
            string_builder.push_str(&action.get_key(&self.action_key_context, None)?);
            string_builder.push('\n');
        }

        if self.options.include_artifacts {
            let inputs = get_action_inputs(action, self.options.include_pruned_inputs);
            let mut input_paths: Vec<String> = inputs
                .to_list()
                .iter()
                .map(|input| internal_to_escaped_unicode(input.get_exec_path_string()))
                .collect();
            input_paths.sort();
            string_builder.push_str("  Inputs: [");
            string_builder.push_str(&input_paths.join(", "));
            string_builder.push_str("]\n");

            let mut output_paths: Vec<String> = action
                .get_outputs()
                .iter()
                .map(|output| {
                    internal_to_escaped_unicode(&if output.is_tree_artifact() {
                        format!("{} (TreeArtifact)", output.get_exec_path_string())
                    } else {
                        output.get_exec_path_string().to_owned()
                    })
                })
                .collect();
            output_paths.sort();
            string_builder.push_str("  Outputs: [");
            string_builder.push_str(&output_paths.join(", "));
            string_builder.push_str("]\n");
        }

        // PORT: every Rust action is an AbstractAction; action templates (which are not) have an
        // empty fixed environment, so they print nothing here, as in Java.
        {
            // TODO(twerth): This handles the fixed environment. We probably want to output the
            // inherited environment as well.
            let fixed_environment = action.get_environment().get_fixed_env();
            if !fixed_environment.is_empty() {
                let mut entries: Vec<String> = fixed_environment
                    .iter()
                    .map(|(key, value)| internal_to_escaped_unicode(&format!("{key}={value}")))
                    .collect();
                entries.sort();
                string_builder.push_str("  Environment: [");
                string_builder.push_str(&entries.join(", "));
                string_builder.push_str("]\n");
            }
        }
        if include_commandline && let Some(command_action) = action.as_command_action() {
            let arguments: Vec<String> = command_action
                .get_arguments()?
                .iter()
                .map(|a| internal_to_escaped_unicode(a))
                .collect();
            let configuration_checksum = action_owner.get_configuration_checksum();
            string_builder.push_str("  Command Line: ");
            string_builder.push_str(&CommandFailureUtils::describe_command(
                CommandDescriptionForm::Complete,
                /* pretty_print_args= */ true,
                &arguments,
                /* environment= */ None,
                /* environment_variables_to_clear= */ None,
                /* cwd= */ None,
                Some(configuration_checksum),
                action
                    .get_owner()
                    .get_execution_platform()
                    .map(|platform| platform.label()),
                /* spawn_runner= */ None,
            ));
            string_builder.push('\n');
        }

        if self.options.include_param_files {
            // Assumption: if an Action takes a param file as an input, it will be used
            // to provide params to the command.
            let map = self.param_file_name_to_content_map.borrow();
            for input in get_action_inputs(action, self.options.include_pruned_inputs)
                .to_list()
                .iter()
            {
                let input_file_name = input.get_exec_path_string();
                if let Some(content) = map.get(input_file_name) {
                    string_builder.push_str("  Params File Content (");
                    string_builder.push_str(input_file_name);
                    string_builder.push_str("):\n    ");
                    string_builder.push_str(content);
                    string_builder.push('\n');
                }
            }
        }
        let execution_info = action.get_execution_info();
        if !execution_info.is_empty() {
            // Map.Entry.comparingByKey(): String.compareTo on internal strings is str order.
            let mut entries: Vec<(&String, &String)> = execution_info.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            string_builder.push_str("  ExecutionInfo: {");
            string_builder.push_str(
                &entries
                    .iter()
                    .map(|(key, value)| {
                        format!(
                            "{}: {}",
                            ShellEscaper::escape_string(key),
                            ShellEscaper::escape_string(value)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            string_builder.push_str("}\n");
        }

        // `action instanceof TemplateExpansionAction`
        match action.aquery_template_expansion() {
            None => {}
            Some(Err(e)) => return Err(ActionGraphDumpError::TemplateExpansion(e)),
            Some(Ok((template_content, substitutions))) => {
                string_builder.push_str("  Template: ");
                string_builder.push_str(&template_content);
                string_builder.push('\n');

                string_builder.push_str("  Substitutions: [\n");
                for (key, value) in substitutions {
                    string_builder.push_str("    {");
                    string_builder.push_str(&key);
                    string_builder.push_str(": ");
                    string_builder.push_str(&value);
                    string_builder.push_str("}\n");
                }
                string_builder.push_str("  ]\n");
            }
        }

        if let Some(file_action) = action.as_file_contents_provider() {
            string_builder.push_str(&format!(
                "  IsExecutable: {}\n",
                file_action.make_executable()
            ));
            if self.options.include_file_write_contents {
                let contents = file_action.get_file_contents()?;
                string_builder.push_str("  FileWriteContents: [");
                // Base64.getEncoder().encodeToString(contents.getBytes(UTF_8)): the UTF-8 bytes
                // of the internal string (double encoded for non-ASCII).
                string_builder.push_str(
                    &base64::engine::general_purpose::STANDARD.encode(contents.as_bytes()),
                );
                string_builder.push_str("]\n");
            }
        }

        if let Some(target) = action.aquery_unresolved_symlink_target() {
            string_builder.push_str("  UnresolvedSymlinkTarget: ");
            string_builder.push_str(&target);
            string_builder.push('\n');
        }

        string_builder.push('\n');
        Ok(())
    }
}

// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:368 ActionGraphTextOutputFormatterCallback.writeCommand
fn write_command(
    action: &dyn Action,
    string_builder: &mut String,
) -> Result<(), ActionGraphDumpError> {
    let Some(command_action) = action.as_command_action() else {
        return Ok(());
    };

    let mut first = true;
    for arg in command_action
        .get_arguments()?
        .iter()
        .map(|a| internal_to_escaped_unicode(a))
    {
        if !first {
            string_builder.push(' ');
        }
        ScriptUtil::emit_command_element(
            /* message= */ string_builder,
            /* command_element= */ &arg,
            /* is_binary= */ first,
        );
        first = false;
    }
    string_builder.push('\n');
    Ok(())
}

/// Convert an internal string (see `StringEncoding`) to a Unicode string with any character
/// outside the basic printable ASCII range escaped.
///
/// Characters other than printable ASCII but within the Basic Multilingual Plane are formatted
/// with `\\uXXXX`. Characters outside the BMP are formatted as `\\UXXXXXXXX`.
// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:405 ActionGraphTextOutputFormatterCallback.internalToEscapedUnicode
pub fn internal_to_escaped_unicode(internal: &str) -> String {
    // internal.chars(): UTF-16 units; an internal string's chars are all <= U+00FF.
    if internal.chars().all(|c| ('\u{20}'..'\u{7f}').contains(&c)) {
        return internal.to_owned();
    }

    let unicode = internal_to_unicode(internal);
    let mut sb = String::with_capacity(unicode.len() * 8);
    for c in unicode.chars() {
        let c = c as u32;
        if (0x20..0x7F).contains(&c) {
            sb.push(char::from_u32(c).expect("ASCII"));
        } else if c <= 0xFFFF {
            sb.push_str(&format!("\\u{c:04X}"));
        } else {
            sb.push_str(&format!("\\U{c:08X}"));
        }
    }
    sb
}

/// The `  AspectDescriptors: [...]` lines `writeText` appends for an action whose owner has
/// aspect descriptors (nothing otherwise).
///
/// In the case of aspect-on-aspect, AspectDescriptors are listed in topological order of the
/// dependency graph. e.g. [A -> B] would imply that aspect A is applied on top of aspect B.
///
/// PORT: the aspect part of `writeText`, separate so that it can be tested alone.
// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:153 ActionGraphTextOutputFormatterCallback.writeText
pub fn write_aspect_descriptors(
    owner_aspect_descriptors: &[AspectDescriptor],
    string_builder: &mut String,
) {
    // ImmutableList<AspectDescriptor> aspectDescriptors =
    //     actionOwner.getAspectDescriptors().reverse();
    if owner_aspect_descriptors.is_empty() {
        return;
    }
    let descriptions: Vec<String> = owner_aspect_descriptors
        .iter()
        .rev()
        .map(|aspect_descriptor| {
            let mut aspect_description = String::new();
            aspect_description.push_str(&aspect_descriptor.aspect_class_name);
            aspect_description.push('(');
            aspect_description.push_str(
                &aspect_descriptor
                    .parameters
                    .iter()
                    .map(|(key, value)| format!("{key}='{value}'"))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            aspect_description.push(')');
            aspect_description
        })
        .collect();
    string_builder.push_str("  AspectDescriptors: [");
    string_builder.push_str(&descriptions.join("\n    -> "));
    string_builder.push_str("]\n");
}

impl NamedThreadSafeOutputFormatterCallback<KeyedConfiguredTargetValue>
    for ActionGraphTextOutputFormatterCallback<'_>
{
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:94 ActionGraphTextOutputFormatterCallback.getName
    fn get_name(&self) -> String {
        self.output_type.format_name().to_owned()
    }
}

impl ThreadSafeOutputFormatterCallback<KeyedConfiguredTargetValue>
    for ActionGraphTextOutputFormatterCallback<'_>
{
}

impl Callback<KeyedConfiguredTargetValue> for ActionGraphTextOutputFormatterCallback<'_> {
    fn process(&self, partial_result: &[KeyedConfiguredTargetValue]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<KeyedConfiguredTargetValue>
    for ActionGraphTextOutputFormatterCallback<'_>
{
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:99 ActionGraphTextOutputFormatterCallback.processOutput
    fn process_output(
        &self,
        partial_result: &[KeyedConfiguredTargetValue],
    ) -> Result<(), OutputFormatterError> {
        // Enabling includeParamFiles should enable includeCommandline by default.
        // PORT: `options.includeCommandline |= options.includeParamFiles` is read only by
        // `writeText`, which computes it there (the options are shared and immutable here).

        for configured_target_value in partial_result {
            if !matches!(
                configured_target_value.get_configured_target(),
                ConfiguredTarget::Rule(_)
            ) {
                // We have to include non-rule values in the graph to visit their dependencies,
                // but they don't have any actions to print out.
                continue;
            }
            let actions: Vec<Arc<dyn Action>> = configured_target_value.get_actions().to_vec();
            for action in &actions {
                self.write_action_or_fail(&**action)?;
            }
            if self.options.use_aspects {
                for aspect_value in self
                    .accessor
                    .get_aspect_values(configured_target_value)
                    .into_iter()
                    .flatten()
                {
                    let actions: Vec<Arc<dyn Action>> = aspect_value.get_actions().to_vec();
                    for action in &actions {
                        self.write_action_or_fail(&**action)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The format names of the Java enum (`TEXT("text")`, `COMMANDS("commands")`), which
    /// `getName` returns and `--output` selects.
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:62 ActionGraphTextOutputFormatterCallback.OutputType
    fn descriptor(name: &str, parameters: &[(&str, &str)]) -> AspectDescriptor {
        AspectDescriptor {
            aspect_class_name: name.to_owned(),
            parameters: parameters
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            description: String::new(),
        }
    }

    // The format of ActionGraphTextOutputFormatterCallback.writeText (lines 182-215): the
    // owner's descriptors reversed, each `name(key='value', ...)`, joined by "\n    -> ".
    // Expected strings follow the Java StringBuilder code character by character.
    #[test]
    fn write_aspect_descriptors_format() {
        let mut out = String::new();
        write_aspect_descriptors(&[], &mut out);
        assert_eq!(out, "");

        // The upb aspects of the workerd oracle: the action owner of an action of
        // upb_c_proto_library_aspect applied on top of upb_minitable_proto_library_aspect lists
        // [minitable, c] (base aspect first); the text lists them reversed.
        let minitable = descriptor(
            "@@protobuf+//upb/bazel:upb_minitable_proto_library.bzl%upb_minitable_proto_library_aspect",
            &[],
        );
        let c = descriptor(
            "@@protobuf+//upb/bazel:upb_c_proto_library.bzl%upb_c_proto_library_aspect",
            &[("p", "v1"), ("p", "v2"), ("q", "x")],
        );
        let mut out = String::new();
        write_aspect_descriptors(&[minitable, c], &mut out);
        assert_eq!(
            out,
            "  AspectDescriptors: [\
             @@protobuf+//upb/bazel:upb_c_proto_library.bzl%upb_c_proto_library_aspect(p='v1', p='v2', q='x')\n    \
             -> @@protobuf+//upb/bazel:upb_minitable_proto_library.bzl%upb_minitable_proto_library_aspect()]\n"
        );
    }

    // The oracle's `aquery //:all --include_file_write_contents` on the byte-strings workspace
    // (~/.cache/blaze/work/bytes-out/aquery_text.txt): the genrule argument `'é🐷'` prints as
    // `\u00E9\U0001F437`, the run_shell argument `"é\377"` as `\u00E9\uFFFD` and the
    // environment entry `ENV_E=é` as `ENV_E=\u00E9`.
    #[test]
    fn internal_to_escaped_unicode_matches_the_oracle() {
        assert_eq!(
            internal_to_escaped_unicode("printf '%s' '\u{c3}\u{a9}\u{f0}\u{9f}\u{90}\u{b7}'"),
            "printf '%s' '\\u00E9\\U0001F437'"
        );
        assert_eq!(
            internal_to_escaped_unicode("\u{c3}\u{a9}\u{ff}"),
            "\\u00E9\\uFFFD"
        );
        assert_eq!(
            internal_to_escaped_unicode("ENV_E=\u{c3}\u{a9}"),
            "ENV_E=\\u00E9"
        );
        // Printable ASCII is returned as is; other ASCII is escaped.
        assert_eq!(internal_to_escaped_unicode("a b~"), "a b~");
        assert_eq!(
            internal_to_escaped_unicode("a\nb\u{7f}"),
            "a\\u000Ab\\u007F"
        );
    }

    // FileWriteContents of `ctx.actions.write(content = "é🐷\377\n")`: base64 of
    // `contents.getBytes(UTF_8)` of the internal string (oracle: `w4PCqcOwwp/CkMK3w78K`).
    #[test]
    fn file_write_contents_are_base64_of_the_internal_strings_utf8() {
        let contents = "\u{c3}\u{a9}\u{f0}\u{9f}\u{90}\u{b7}\u{ff}\n";
        assert_eq!(
            base64::engine::general_purpose::STANDARD.encode(contents.as_bytes()),
            "w4PCqcOwwp/CkMK3w78K"
        );
    }

    #[test]
    fn output_type_format_names() {
        assert_eq!(OutputType::TEXT.format_name(), "text");
        assert_eq!(OutputType::COMMANDS.format_name(), "commands");
    }
}
