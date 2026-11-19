//! `blaze help`.

use std::sync::Arc;

use blaze_events::{Event, EventHandler, OutErr, Reporter};
use blaze_optdefs::command::help_command::HelpCommandOptions;
use blaze_options::definition::{OptionDefinitionRef, OptionsClassRef};
use blaze_options::html_utils::{HtmlUtils, html_escaper};
use blaze_options::option_filter_descriptions::OptionFilterDescriptions;
use blaze_options::{HelpVerbosity, OptionEffectTag, OptionMetadataTag, OptionsParser};
use blaze_protos::JavaMessage;
use blaze_protos::bazel_flags;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use indexmap::IndexMap;

use crate::blaze_command_utils::{BlazeCommandUtils, ResourceFileLoader};
use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::commands::info::InfoCommand;
use crate::options_result::OptionsParsingResult;
use crate::runtime::BlazeRuntime;
use crate::version_info::BlazeVersionInfo;

/// The package of `HelpCommand`, where its help topics live.
const HELP_COMMAND_PACKAGE: &str = "com/google/devtools/build/lib/runtime/commands";

/// The 'blaze help' command, which prints all available commands as well as specific help pages.
// Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:92 HelpCommand
pub struct HelpCommand;

/// A visitor for Blaze commands and their respective command line options: the command's name,
/// its annotation and an `OptionsParser` over all its options.
// Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:620 HelpCommand.CommandOptionVisitor
type CommandOptionVisitor<'a> = dyn FnMut(&str, &CommandAnnotation, &OptionsParser) + 'a;

impl HelpCommand {
    pub fn new() -> Self {
        HelpCommand
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:200 HelpCommand.emitBlazeVersionInfo
    fn emit_blaze_version_info(out_err: &OutErr, product_name: &str) {
        let release_info = BlazeVersionInfo::instance().get_release_name();
        let line = format!("[{} {}]", product_name, release_info);
        out_err.print_out(&format!("{:>80}\n", line));
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:206 HelpCommand.emitStartupOptions
    fn emit_startup_options(
        out_err: &OutErr,
        help_verbosity: HelpVerbosity,
        runtime: &BlazeRuntime,
    ) {
        out_err.print_out(&BlazeCommandUtils::expand_help_topic(
            "startup_options",
            "resource:startup_options.txt",
            HELP_COMMAND_PACKAGE,
            &BlazeRuntime::get_startup_options(runtime.get_blaze_modules()),
            help_verbosity,
            runtime.get_product_name(),
        ));
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:218 HelpCommand.emitCompletionHelp
    fn emit_completion_help(
        shell: Option<&str>,
        runtime: &BlazeRuntime,
        reporter: &Reporter,
    ) -> BlazeCommandResult {
        let out_err = reporter.get_out_err();
        match shell {
            Some("bash") => {
                out_err.print_out_ln(&Self::load_completion_script("bazel-complete-header.bash"));
                Self::emit_completion_variables(runtime, out_err);
                out_err.print_out_ln(&Self::load_completion_script(
                    "bazel-complete-template.bash",
                ));
                BlazeCommandResult::success()
            }
            None => {
                // Preserved for backwards compatibility: print only the variables part of the
                // bash completion script.
                Self::emit_completion_variables(runtime, out_err);
                BlazeCommandResult::success()
            }
            Some(shell) => {
                let message = format!(
                    "The completion command only supports 'bash' as an argument, got '{}'",
                    shell
                );
                reporter.handle(&Event::error(message.clone()));
                Self::create_failure_result(
                    &message,
                    failure_details::help_command::Code::MissingArgument,
                )
            }
        }
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:243 HelpCommand.loadCompletionScript
    fn load_completion_script(basename: &str) -> String {
        let resource_name = format!("/scripts/{}", basename);
        match ResourceFileLoader::load_absolute_resource_bytes(&resource_name) {
            // new String(bytes, ISO_8859_1)
            Some(bytes) => blaze_util::string_encoding::latin1_decode(bytes).into_owned(),
            None => panic!(
                "IllegalStateException: Failed to read built-in resource {}: {} not found.",
                basename, resource_name
            ),
        }
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:258 HelpCommand.emitCompletionVariables
    fn emit_completion_variables(runtime: &BlazeRuntime, out_err: &OutErr) {
        let commands_by_name = Self::get_sorted_commands(runtime);

        out_err.print_out_ln(&format!(
            "BAZEL_COMMAND_LIST=\"{}\"",
            commands_by_name
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        ));

        out_err.print_out_ln("BAZEL_INFO_KEYS=\"");
        for name in InfoCommand::get_hardwired_info_item_names(runtime.get_product_name()) {
            out_err.print_out_ln(&name);
        }
        out_err.print_out_ln("\"");

        let mut startup_option_visitor = |parser: &OptionsParser| {
            out_err.print_out_ln("BAZEL_STARTUP_OPTIONS=\"");
            out_err.print_out(&parser.get_options_completion());
            out_err.print_out_ln("\"");
        };
        let mut command_option_visitor =
            |command_name: &str, command_annotation: &CommandAnnotation, parser: &OptionsParser| {
                // CaseFormat.LOWER_HYPHEN.to(CaseFormat.UPPER_UNDERSCORE, commandName)
                let var_name = command_name.replace('-', "_").to_ascii_uppercase();
                if !command_annotation.completion.is_empty() {
                    out_err.print_out_ln(&format!(
                        "BAZEL_COMMAND_{}_ARGUMENT=\"{}\"",
                        var_name, command_annotation.completion
                    ));
                }
                out_err.print_out_ln(&format!("BAZEL_COMMAND_{}_FLAGS=\"", var_name));
                out_err.print_out(&parser.get_options_completion());
                out_err.print_out_ln("\"");
            };

        Self::visit_all_options(
            runtime,
            &mut startup_option_visitor,
            &mut command_option_visitor,
        );
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:296 HelpCommand.emitFlagsAsProtoHelp
    fn emit_flags_as_proto_help(runtime: &BlazeRuntime, out_err: &OutErr) {
        let mut flags = JavaHashMap::new();

        {
            let mut visitor = |command_name: &str, option: OptionDefinitionRef| {
                if option
                    .get_option_metadata_tags()
                    .contains(&OptionMetadataTag::INTERNAL)
                {
                    return;
                }
                let info = flags
                    .compute_if_absent(option.get_option_name(), || Self::create_flag_info(option));
                info.commands.push(command_name.to_string());
            };
            // Both visitors need the same `visitor`; they run one after the other.
            let visitor_cell = std::cell::RefCell::new(&mut visitor);
            let mut startup_option_visitor = |parser: &OptionsParser| {
                parser.visit_options(
                    |_| true,
                    |option| (visitor_cell.borrow_mut())("startup", option),
                );
            };
            let mut command_option_visitor =
                |command_name: &str, _annotation: &CommandAnnotation, parser: &OptionsParser| {
                    parser.visit_options(
                        |_| true,
                        |option| (visitor_cell.borrow_mut())(command_name, option),
                    );
                };

            Self::visit_all_options(
                runtime,
                &mut startup_option_visitor,
                &mut command_option_visitor,
            );
        }

        let mut collection_builder = bazel_flags::FlagCollection::default();
        for info in flags.into_values() {
            collection_builder.flag_infos.push(info);
        }
        out_err.print_out(&base64_encode(&collection_builder.to_byte_array()));
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:325 HelpCommand.createFlagInfo
    fn create_flag_info(option: OptionDefinitionRef) -> bazel_flags::FlagInfo {
        let mut flag_builder = bazel_flags::FlagInfo {
            name: option.get_option_name().to_string(),
            has_negative_flag: Some(option.uses_boolean_value_syntax()),
            documentation: Some(option.get_help_text().to_string()),
            allows_multiple: Some(option.allows_multiple()),
            requires_value: Some(option.requires_value()),
            ..Default::default()
        };

        if option.get_abbreviation() != '\0' {
            flag_builder.abbreviation = Some(option.get_abbreviation().to_string());
        }
        if !option.get_old_option_name().is_empty() {
            flag_builder.old_name = Some(option.get_old_option_name().to_string());
        }

        flag_builder.effect_tags = option
            .get_option_effect_tags()
            .iter()
            .map(|t| t.name().to_string())
            .collect();

        flag_builder.metadata_tags = option
            .get_option_metadata_tags()
            .iter()
            .map(|t| t.name().to_string())
            .collect();

        // getDocumentationCategory() is never null.
        flag_builder.documentation_category =
            Some(option.get_documentation_category().name().to_string());

        if !option.is_special_null_default() {
            flag_builder.default_value = Some(option.get_unparsed_default_value().to_string());
        }

        if !option.get_deprecation_warning().is_empty() {
            flag_builder.deprecation_warning = Some(option.get_deprecation_warning().to_string());
        }

        if !option.get_option_expansion().is_empty() {
            flag_builder.option_expansions = option
                .get_option_expansion()
                .iter()
                .map(|s| s.to_string())
                .collect();
        }

        let converter = option.get_converter();
        let converter_class_name = converter.java_simple_class_name();
        if let Some(short_name) = converter_class_name.strip_suffix("Converter") {
            flag_builder.type_converter = Some(short_name.to_string());
        }
        if let Some(enum_values) = converter.enum_type_constants() {
            flag_builder.enum_values = enum_values;
        }

        flag_builder
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:383 HelpCommand.visitAllOptions
    fn visit_all_options(
        runtime: &BlazeRuntime,
        startup_option_visitor: &mut dyn FnMut(&OptionsParser),
        command_option_visitor: &mut CommandOptionVisitor<'_>,
    ) {
        // First startup_options
        let options_suppliers = runtime.get_blaze_modules();
        let commands_by_name = Self::get_sorted_commands(runtime);

        let options: Vec<OptionsClassRef> = BlazeRuntime::get_startup_options(options_suppliers);
        startup_option_visitor(&build_parser(&options));

        for (name, command) in &commands_by_name {
            let annotation = command.annotation();
            let options = crate::command_registry::get_options(annotation, options_suppliers);
            command_option_visitor(name, annotation, &build_parser(&options));
        }
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:405 HelpCommand.getSortedCommands
    fn get_sorted_commands(
        runtime: &BlazeRuntime,
    ) -> std::collections::BTreeMap<String, Arc<dyn BlazeCommand>> {
        runtime
            .get_command_map()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:409 HelpCommand.emitTargetSyntaxHelp
    fn emit_target_syntax_help(out_err: &OutErr, product_name: &str) {
        out_err.print_out(&BlazeCommandUtils::expand_help_topic(
            "target-syntax",
            "resource:target-syntax.txt",
            HELP_COMMAND_PACKAGE,
            &[],
            HelpVerbosity::MEDIUM,
            product_name,
        ));
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:420 HelpCommand.emitInfoKeysHelp
    fn emit_info_keys_help(env: &CommandEnvironment, out_err: &OutErr) {
        let empty = OptionsParser::builder()
            .build()
            .expect("OptionsParser.builder().build()");
        for item in InfoCommand::get_info_item_map(env, &empty).values() {
            out_err.print_out(&format!(
                "{:<23} {}\n",
                item.get_name(),
                item.get_description()
            ));
        }
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:427 HelpCommand.emitGenericHelp
    fn emit_generic_help(out_err: &OutErr, runtime: &BlazeRuntime) {
        let commands: Vec<(String, &'static CommandAnnotation)> = runtime
            .get_command_map()
            .iter()
            .map(|(name, command)| (name.clone(), command.annotation()))
            .collect();
        out_err.print_out(&Self::generic_help(runtime.get_product_name(), &commands));
    }

    /// The text `emitGenericHelp` prints, given the command map's names and annotations.
    fn generic_help(
        product_name: &str,
        commands: &[(String, &'static CommandAnnotation)],
    ) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Usage: {} <command> <options> ...\n\n",
            product_name
        ));
        out.push_str("Available commands:\n");

        let mut names_in_order: Vec<&(String, &'static CommandAnnotation)> =
            commands.iter().collect();
        names_in_order.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, annotation) in names_in_order {
            if annotation.hidden {
                continue;
            }
            let short_description = annotation
                .short_description
                .replace("%{product}", product_name);
            out.push_str(&format!("  {:<19} {}\n", name, short_description));
        }

        out.push('\n');
        out.push_str("Getting more help:\n");
        out.push_str(&format!("  {} help <command>\n", product_name));
        out.push_str("                   Prints help and options for <command>.\n");
        out.push_str(&format!("  {} help startup_options\n", product_name));
        out.push_str(&format!(
            "                   Options for the JVM hosting {}.\n",
            product_name
        ));
        out.push_str(&format!("  {} help target-syntax\n", product_name));
        out.push_str("                   Explains the syntax for specifying targets.\n");
        out.push_str(&format!("  {} help info-keys\n", product_name));
        out.push_str("                   Displays a list of keys used by the info command.\n");
        out
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:636 HelpCommand.createFailureResult
    fn create_failure_result(
        message: &str,
        detailed_code: failure_details::help_command::Code,
    ) -> BlazeCommandResult {
        BlazeCommandResult::failure_detail(FailureDetail {
            message: message.to_string(),
            category: Some(Category::HelpCommand(failure_details::HelpCommand {
                code: detailed_code as i32,
            })),
        })
    }
}

fn build_parser(options: &[OptionsClassRef]) -> OptionsParser {
    OptionsParser::builder()
        .options_classes(options)
        .build()
        .unwrap_or_else(|e| e.throw())
}

/// `HelpCommand.HtmlEmitter`.
// Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:461 HelpCommand.HtmlEmitter
struct HtmlEmitter<'a> {
    runtime: &'a BlazeRuntime,
}

impl HtmlEmitter<'_> {
    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:468 HelpCommand.HtmlEmitter.emit
    fn emit(&self, out_err: &OutErr) {
        let runtime = self.runtime;
        let commands_by_name = HelpCommand::get_sorted_commands(runtime);
        let mut result = String::new();
        result.push_str("<h2>Commands</h2>\n");
        result.push_str("<table>\n");
        for (name, command) in &commands_by_name {
            let annotation = command.annotation();
            if annotation.hidden {
                continue;
            }
            let short_description = annotation
                .short_description
                .replace("%{product}", runtime.get_product_name());

            result.push_str("<tr>\n");
            result.push_str(&format!(
                "  <td><a href=\"#{}\"><code>{}</code></a></td>\n",
                name, name
            ));
            result.push_str("  <td>");
            result.push_str(&html_escaper(&short_description));
            result.push_str("</td>\n");
            result.push_str("</tr>\n");
        }
        result.push_str("</table>\n");
        result.push('\n');

        result.push_str("<h2>Startup Options</h2>\n");
        self.append_options_html(
            &mut result,
            &BlazeRuntime::get_startup_options(runtime.get_blaze_modules()),
            &[],
            Some("startup_options"),
        );
        result.push('\n');

        result.push_str("<h2><a name=\"common_options\">Options Common to all Commands</a></h2>\n");
        self.append_options_html(
            &mut result,
            &crate::command_registry::get_common_options(runtime.get_blaze_modules()),
            &[],
            Some("common_options"),
        );
        result.push('\n');

        for (name, command) in &commands_by_name {
            result.push_str(&format!(
                "<h2><a name=\"{}\">{} Options</a></h2>\n",
                name,
                capitalize(name)
            ));
            let annotation = command.annotation();
            if annotation.hidden {
                continue;
            }
            let inherited_cmd_names: Vec<String> = annotation
                .inherits_options_from
                .iter()
                .map(|base| format!("<a href=\"#{}\">{}</a>", base.name(), base.name()))
                .collect();
            if !inherited_cmd_names.is_empty() {
                result.push_str("<p>Inherits all options from ");
                result.push_str(&join_english_list(&inherited_cmd_names, "and"));
                result.push_str(".</p>\n\n");
            }
            let mut options: Vec<OptionsClassRef> = Vec::new();
            for class in (annotation.options)() {
                add_class(&mut options, class);
            }
            for supplier in runtime.get_blaze_modules() {
                for class in supplier.get_command_options(annotation.name()) {
                    add_class(&mut options, class);
                }
            }
            let options_to_ignore =
                self.append_options_html(&mut result, &options, &[], Some(name));
            result.push('\n');

            // For now, we print all the configuration options in a list after all the
            // non-configuration options.
            if annotation.uses_configuration_options {
                options.clear();
                for class in (annotation.options)() {
                    add_class(&mut options, class);
                }
                for class in crate::fragment_options::fragment_options_classes() {
                    add_class(&mut options, class);
                }
                self.append_options_html(&mut result, &options, &options_to_ignore, None);
                result.push('\n');
            }
        }

        // Describe the tags once, any mentions above should link to these descriptions.
        let product_name = runtime.get_product_name();
        let effect_tag_descriptions =
            OptionFilterDescriptions::get_option_effect_tag_description(product_name);
        result.push_str("<h3>Option Effect Tags</h3>\n");
        result.push_str("<table>\n");
        for tag in OptionEffectTag::VALUES {
            let tag_description = &effect_tag_descriptions[tag];

            result.push_str("<tr>\n");
            result.push_str(&format!(
                "<td id=\"effect_tag_{}\"><code>{}</code></td>\n",
                tag.name(),
                tag.name().to_ascii_lowercase()
            ));
            result.push_str(&format!("<td>{}</td>\n", html_escaper(tag_description)));
            result.push_str("</tr>\n");
        }
        result.push_str("</table>\n");

        let metadata_tag_descriptions =
            OptionFilterDescriptions::get_option_metadata_tag_description(product_name);
        result.push_str("<h3>Option Metadata Tags</h3>\n");
        result.push_str("<table>\n");
        for tag in OptionMetadataTag::VALUES {
            // skip the tags that are reserved for undocumented flags.
            if *tag != OptionMetadataTag::HIDDEN && *tag != OptionMetadataTag::INTERNAL {
                let tag_description = &metadata_tag_descriptions[tag];

                result.push_str("<tr>\n");
                result.push_str(&format!(
                    "<td id=\"metadata_tag_{}\"><code>{}</code></td>\n",
                    tag.name(),
                    tag.name().to_ascii_lowercase()
                ));
                result.push_str(&format!("<td>{}</td>\n", html_escaper(tag_description)));
                result.push_str("</tr>\n");
            }
        }
        result.push_str("</table>\n");

        out_err.print_out(&result);
    }

    /// Returns the list of appended option names.
    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:588 HelpCommand.HtmlEmitter.appendOptionsHtml
    fn append_options_html(
        &self,
        result: &mut String,
        options_classes: &[OptionsClassRef],
        options_to_ignore: &[String],
        command_name: Option<&str>,
    ) -> Vec<String> {
        let parser = build_parser(options_classes);
        let product_name = self.runtime.get_product_name();
        result.push_str(
            &HtmlUtils::describe_options_html(
                &parser,
                html_escaper,
                options_to_ignore,
                command_name,
            )
            .replace("%{product}", product_name),
        );

        let mut option_names = Vec::new();
        for category in parser.get_options_sorted_by_category().values() {
            for option in category {
                option_names.push(option.get_option_name().to_string());
            }
        }
        option_names
    }
}

/// Adds `class` unless present (Java collects the classes in a `HashSet`; the order does not
/// reach the output, which is sorted by category and name).
fn add_class(options: &mut Vec<OptionsClassRef>, class: OptionsClassRef) {
    if !options.iter().any(|c| std::ptr::eq(*c, class)) {
        options.push(class);
    }
}

/// `StringUtilities.capitalize`.
// Java: com/google/devtools/build/lib/util/StringUtilities.java:144 StringUtilities.capitalize
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => panic!("StringIndexOutOfBoundsException: begin 0, end 1, length 0"),
    }
}

/// `StringUtil.joinEnglishList(choices, lastSeparator)` (Oxford comma, no quotes).
// Java: com/google/devtools/build/lib/util/StringUtil.java:44 StringUtil.joinEnglishList
fn join_english_list(choices: &[String], last_separator: &str) -> String {
    let mut buf = String::new();
    for (num_choices_seen, choice) in choices.iter().enumerate() {
        let has_next = num_choices_seen + 1 < choices.len();
        if !buf.is_empty() {
            if has_next || num_choices_seen >= 2 {
                buf.push(',');
            }
            if !has_next {
                buf.push(' ');
                buf.push_str(last_separator);
            }
            buf.push(' ');
        }
        buf.push_str(choice);
    }
    if buf.is_empty() {
        "nothing".to_string()
    } else {
        buf
    }
}

/// `java.util.Base64.getEncoder().encodeToString` (standard alphabet, with padding).
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// A `java.util.HashMap<String, V>` filled with `computeIfAbsent`, iterated in Java's order.
///
/// PORT: java-hash-order. The table is simulated as `HashMap` keeps it: hash `h ^ (h >>> 16)`
/// of `String.hashCode`, capacity 16 doubled by `resize` (which keeps the relative order of a
/// bucket's nodes) when `computeIfAbsent` starts with the size above 3/4 of the capacity, and a
/// new node goes to the head of its bucket. Tree bins (a bucket reaching 8 nodes once the
/// capacity is 64) are not modelled and fail loudly.
struct JavaHashMap<V> {
    entries: IndexMap<String, V>,
    /// The buckets, each a list of indices into `entries` in node order.
    table: Vec<Vec<usize>>,
}

impl<V> JavaHashMap<V> {
    fn new() -> Self {
        JavaHashMap {
            entries: IndexMap::new(),
            table: Vec::new(),
        }
    }

    /// `String.hashCode()` spread by `HashMap.hash`.
    fn hash(key: &str) -> u32 {
        let mut h: i32 = 0;
        for unit in key.encode_utf16() {
            h = h.wrapping_mul(31).wrapping_add(unit as i32);
        }
        let h = h as u32;
        h ^ (h >> 16)
    }

    /// `HashMap.resize`: the first table has 16 buckets, then the size doubles; each bucket
    /// splits into the same index and index + old capacity, keeping the node order.
    fn resize(&mut self) {
        let old_cap = self.table.len();
        if old_cap == 0 {
            self.table = vec![Vec::new(); 16];
            return;
        }
        let mut new_table = vec![Vec::new(); old_cap * 2];
        for (j, bucket) in self.table.iter().enumerate() {
            for &e in bucket {
                let h = Self::hash(self.entries.get_index(e).unwrap().0) as usize;
                if h & old_cap == 0 {
                    new_table[j].push(e);
                } else {
                    new_table[j + old_cap].push(e);
                }
            }
        }
        self.table = new_table;
    }

    /// `computeIfAbsent(key, k -> f())`.
    // Java: java/util/HashMap.java HashMap.computeIfAbsent
    fn compute_if_absent(&mut self, key: &str, f: impl FnOnce() -> V) -> &mut V {
        let threshold = self.table.len() * 3 / 4;
        if self.entries.len() > threshold || self.table.is_empty() {
            self.resize();
        }
        if let Some(i) = self.entries.get_index_of(key) {
            return &mut self.entries[i];
        }
        let n = self.table.len();
        let bucket = (Self::hash(key) as usize) & (n - 1);
        let bin_count = self.table[bucket].len();
        let (index, _) = self.entries.insert_full(key.to_string(), f());
        self.table[bucket].insert(0, index);
        if bin_count >= 8 - 1 {
            // treeifyBin: a small table is resized instead.
            if self.table.len() < 64 {
                self.resize();
            } else {
                blaze_util::unported!(
                    "HashMap.treeifyBin",
                    "8 or more keys in one bucket of the flags-as-proto map"
                );
            }
        }
        &mut self.entries[index]
    }

    /// `values()`, in Java's iteration order.
    fn into_values(self) -> Vec<V> {
        let order: Vec<usize> = self.table.iter().flatten().copied().collect();
        let mut slots: Vec<Option<V>> = self.entries.into_values().map(Some).collect();
        order
            .into_iter()
            .map(|i| slots[i].take().expect("each entry is in one bucket"))
            .collect()
    }
}

impl BlazeCommand for HelpCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &crate::command_registry::HELP_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:133 HelpCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        // PORT: `env.getEventBus().post(new NoBuildEvent())` only reaches the Build Event
        // Protocol, which is ported for build and test; a BEP file requested for this command
        // fails loudly before it runs (crate::unported_options).
        let runtime = env.get_runtime().clone();
        let out_err = env.get_reporter().get_out_err().clone();
        let help_options = options
            .get_options::<HelpCommandOptions>()
            .expect("HelpCommand.Options");
        let residue = options.get_residue();
        if residue.is_empty() {
            Self::emit_blaze_version_info(&out_err, runtime.get_product_name());
            Self::emit_generic_help(&out_err, &runtime);
            return BlazeCommandResult::success();
        }
        if residue[0] == "completion" {
            if residue.len() > 2 {
                let message = "The completion command takes at most one argument";
                env.get_reporter().handle(&Event::error(message));
                return Self::create_failure_result(
                    message,
                    failure_details::help_command::Code::MissingArgument,
                );
            }
            let shell = residue.get(1).map(String::as_str);
            return Self::emit_completion_help(shell, &runtime, env.get_reporter());
        }
        if residue.len() != 1 {
            let message = "You must specify exactly one command";
            env.get_reporter().handle(&Event::error(message));
            return Self::create_failure_result(
                message,
                failure_details::help_command::Code::MissingArgument,
            );
        }
        let help_subject = residue[0].as_str();
        let product_name = runtime.get_product_name();
        // Go through the custom subjects before going through Bazel commands.
        match help_subject {
            "startup_options" => {
                Self::emit_blaze_version_info(&out_err, product_name);
                Self::emit_startup_options(&out_err, help_options.help_verbosity, &runtime);
                return BlazeCommandResult::success();
            }
            "target-syntax" => {
                Self::emit_blaze_version_info(&out_err, product_name);
                Self::emit_target_syntax_help(&out_err, product_name);

                return BlazeCommandResult::success();
            }
            "info-keys" => {
                Self::emit_info_keys_help(env, &out_err);
                return BlazeCommandResult::success();
            }
            "flags-as-proto" => {
                Self::emit_flags_as_proto_help(&runtime, &out_err);
                return BlazeCommandResult::success();
            }
            "everything-as-html" => {
                HtmlEmitter { runtime: &runtime }.emit(&out_err);
                return BlazeCommandResult::success();
            }
            _ => {} // fall out
        }

        let Some(command) = runtime.get_command_map().get(help_subject) else {
            let message = format!("'{}' is not a known command", help_subject);
            env.get_reporter().handle(&Event::error(message.clone()));
            return Self::create_failure_result(
                &message,
                failure_details::help_command::Code::CommandNotFound,
            );
        };
        Self::emit_blaze_version_info(&out_err, product_name);
        out_err.print_out(&BlazeCommandUtils::get_usage(
            command.annotation(),
            help_options.help_verbosity,
            runtime.get_blaze_modules(),
            product_name,
        ));

        BlazeCommandResult::success()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn annotation_for_test(
        hidden: bool,
        short_description: &'static str,
    ) -> &'static CommandAnnotation {
        Box::leak(Box::new(CommandAnnotation {
            rc: blaze_options::rc::handler::Command {
                name: "x",
                inherits_options_from: &[],
            },
            options: Vec::new,
            inherits_options_from: &[],
            short_description,
            uses_configuration_options: false,
            build_phase: crate::command::BuildPhase::NONE,
            hidden,
            allow_residue: false,
            has_sensitive_residue: false,
            binary_std_out: false,
            binary_std_err: false,
            write_command_log: true,
            help: "",
            must_run_in_workspace: false,
            completion: "",
        }))
    }

    /// Format of the oracle's `bazel help` index (Bazel 9.3.2): the version line right-aligned to
    /// 80 columns, sorted non-hidden commands padded to 20, then the help topics.
    #[test]
    fn generic_help_format_matches_oracle() {
        let commands = vec![
            (
                "version".to_string(),
                annotation_for_test(false, "Prints version information for %{product}."),
            ),
            (
                "aquery".to_string(),
                annotation_for_test(
                    false,
                    "Analyzes the given targets and queries the action graph.",
                ),
            ),
            (
                "hidden-cmd".to_string(),
                annotation_for_test(true, "Hidden."),
            ),
        ];
        assert_eq!(
            HelpCommand::generic_help("bazel", &commands),
            "\
Usage: bazel <command> <options> ...

Available commands:
  aquery              Analyzes the given targets and queries the action graph.
  version             Prints version information for bazel.

Getting more help:
  bazel help <command>
                   Prints help and options for <command>.
  bazel help startup_options
                   Options for the JVM hosting bazel.
  bazel help target-syntax
                   Explains the syntax for specifying targets.
  bazel help info-keys
                   Displays a list of keys used by the info command.
"
        );
    }

    #[test]
    fn version_line_right_aligned_to_80() {
        let line = format!("[{} {}]", "bazel", "release 9.3.2");
        assert_eq!(
            format!("{:>80}\n", line),
            "                                                           [bazel release 9.3.2]\n"
        );
    }

    #[test]
    fn base64_matches_java() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn join_english_list_uses_the_oxford_comma() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(join_english_list(&s(&[]), "and"), "nothing");
        assert_eq!(join_english_list(&s(&["a"]), "and"), "a");
        assert_eq!(join_english_list(&s(&["a", "b"]), "and"), "a and b");
        assert_eq!(
            join_english_list(&s(&["a", "b", "c"]), "and"),
            "a, b, and c"
        );
    }

    #[test]
    fn java_string_hash_code() {
        // "hello".hashCode() == 99162322 in Java; HashMap.hash spreads it.
        let h: u32 = 99162322;
        assert_eq!(JavaHashMap::<()>::hash("hello"), h ^ (h >> 16));
    }
}
