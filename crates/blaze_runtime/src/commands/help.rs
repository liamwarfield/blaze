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
/// of `String.hashCode`, capacity 16 doubled by `resize` when `computeIfAbsent` starts with the
/// size above 3/4 of the capacity, a new node at the head of a list bucket, and the tree bins
/// (`HashMap.TreeNode`) a bucket becomes once it holds 8 nodes in a table of 64 or more: their
/// red-black tree decides where a new node goes in the bucket's node order (after its tree
/// parent) and which node leads it (the root), and `resize` splits them, keeping, rebuilding or
/// flattening the tree as Java does.
struct JavaHashMap<V> {
    entries: IndexMap<String, V>,
    /// Per entry (index into `entries`): its spread hash and its tree links.
    nodes: Vec<TreeLinks>,
    /// The buckets, each a list of indices into `entries` in node (`next`) order.
    table: Vec<Vec<usize>>,
    /// Whether each bucket is a tree bin (its first node is the tree's root).
    tree_bins: Vec<bool>,
}

/// The fields of a `HashMap.TreeNode` besides the list links.
#[derive(Clone, Copy, Default)]
struct TreeLinks {
    hash: i32,
    parent: Option<usize>,
    left: Option<usize>,
    right: Option<usize>,
    red: bool,
}

/// `HashMap.TREEIFY_THRESHOLD`, `UNTREEIFY_THRESHOLD`, `MIN_TREEIFY_CAPACITY`.
const TREEIFY_THRESHOLD: usize = 8;
const UNTREEIFY_THRESHOLD: usize = 6;
const MIN_TREEIFY_CAPACITY: usize = 64;

impl<V> JavaHashMap<V> {
    fn new() -> Self {
        JavaHashMap {
            entries: IndexMap::new(),
            nodes: Vec::new(),
            table: Vec::new(),
            tree_bins: Vec::new(),
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

    /// The tree order of two distinct keys (`treeify` / `putTreeVal`): the spread hashes as
    /// signed ints, then `String.compareTo` (`comparableClassFor` is `String`; the keys are
    /// distinct, so `tieBreakOrder` is never needed). Negative goes left.
    fn dir(&self, x: usize, p: usize) -> i32 {
        let (h, ph) = (self.nodes[x].hash, self.nodes[p].hash);
        if ph > h {
            -1
        } else if ph < h {
            1
        } else {
            let k = self.entries.get_index(x).unwrap().0;
            let pk = self.entries.get_index(p).unwrap().0;
            match k.encode_utf16().cmp(pk.encode_utf16()) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Greater => 1,
                std::cmp::Ordering::Equal => unreachable!("distinct keys"),
            }
        }
    }

    // Java: java/util/HashMap.java HashMap.TreeNode.rotateLeft
    fn rotate_left(&mut self, mut root: usize, p: usize) -> usize {
        if let Some(r) = self.nodes[p].right {
            let rl = self.nodes[r].left;
            self.nodes[p].right = rl;
            if let Some(rl) = rl {
                self.nodes[rl].parent = Some(p);
            }
            let pp = self.nodes[p].parent;
            self.nodes[r].parent = pp;
            match pp {
                None => {
                    root = r;
                    self.nodes[r].red = false;
                }
                Some(pp) if self.nodes[pp].left == Some(p) => self.nodes[pp].left = Some(r),
                Some(pp) => self.nodes[pp].right = Some(r),
            }
            self.nodes[r].left = Some(p);
            self.nodes[p].parent = Some(r);
        }
        root
    }

    // Java: java/util/HashMap.java HashMap.TreeNode.rotateRight
    fn rotate_right(&mut self, mut root: usize, p: usize) -> usize {
        if let Some(l) = self.nodes[p].left {
            let lr = self.nodes[l].right;
            self.nodes[p].left = lr;
            if let Some(lr) = lr {
                self.nodes[lr].parent = Some(p);
            }
            let pp = self.nodes[p].parent;
            self.nodes[l].parent = pp;
            match pp {
                None => {
                    root = l;
                    self.nodes[l].red = false;
                }
                Some(pp) if self.nodes[pp].right == Some(p) => self.nodes[pp].right = Some(l),
                Some(pp) => self.nodes[pp].left = Some(l),
            }
            self.nodes[l].right = Some(p);
            self.nodes[p].parent = Some(l);
        }
        root
    }

    // Java: java/util/HashMap.java HashMap.TreeNode.balanceInsertion
    fn balance_insertion(&mut self, mut root: usize, mut x: usize) -> usize {
        self.nodes[x].red = true;
        loop {
            let Some(mut xp) = self.nodes[x].parent else {
                self.nodes[x].red = false;
                return x;
            };
            let xpp = match self.nodes[xp].parent {
                Some(xpp) if self.nodes[xp].red => xpp,
                _ => return root,
            };
            let xppl = self.nodes[xpp].left;
            if Some(xp) == xppl {
                let xppr = self.nodes[xpp].right;
                if let Some(xppr) = xppr
                    && self.nodes[xppr].red
                {
                    self.nodes[xppr].red = false;
                    self.nodes[xp].red = false;
                    self.nodes[xpp].red = true;
                    x = xpp;
                } else {
                    let mut xpp = Some(xpp);
                    if Some(x) == self.nodes[xp].right {
                        x = xp;
                        root = self.rotate_left(root, x);
                        xp = self.nodes[x].parent.expect("rotated under a parent");
                        xpp = self.nodes[xp].parent;
                    }
                    self.nodes[xp].red = false;
                    if let Some(xpp) = xpp {
                        self.nodes[xpp].red = true;
                        root = self.rotate_right(root, xpp);
                    }
                }
            } else if let Some(xppl) = xppl
                && self.nodes[xppl].red
            {
                self.nodes[xppl].red = false;
                self.nodes[xp].red = false;
                self.nodes[xpp].red = true;
                x = xpp;
            } else {
                let mut xpp = Some(xpp);
                if Some(x) == self.nodes[xp].left {
                    x = xp;
                    root = self.rotate_right(root, x);
                    xp = self.nodes[x].parent.expect("rotated under a parent");
                    xpp = self.nodes[xp].parent;
                }
                self.nodes[xp].red = false;
                if let Some(xpp) = xpp {
                    self.nodes[xpp].red = true;
                    root = self.rotate_left(root, xpp);
                }
            }
        }
    }

    /// `moveRootToFront`: the root becomes the bucket's first node; the others keep their
    /// order.
    // Java: java/util/HashMap.java HashMap.TreeNode.moveRootToFront
    fn move_root_to_front(&mut self, bucket: usize, root: usize) {
        let list = &mut self.table[bucket];
        let pos = list
            .iter()
            .position(|&e| e == root)
            .expect("root in its bucket");
        if pos != 0 {
            list.remove(pos);
            list.insert(0, root);
        }
    }

    /// `treeify` of the nodes of `bucket`, in their list order.
    // Java: java/util/HashMap.java HashMap.TreeNode.treeify
    fn treeify(&mut self, bucket: usize) {
        let list = self.table[bucket].clone();
        let mut root: Option<usize> = None;
        for x in list {
            self.nodes[x].left = None;
            self.nodes[x].right = None;
            match root {
                None => {
                    self.nodes[x].parent = None;
                    self.nodes[x].red = false;
                    root = Some(x);
                }
                Some(r) => {
                    let mut p = r;
                    loop {
                        let dir = self.dir(x, p);
                        let xp = p;
                        let next = if dir <= 0 {
                            self.nodes[p].left
                        } else {
                            self.nodes[p].right
                        };
                        match next {
                            Some(n) => p = n,
                            None => {
                                self.nodes[x].parent = Some(xp);
                                if dir <= 0 {
                                    self.nodes[xp].left = Some(x);
                                } else {
                                    self.nodes[xp].right = Some(x);
                                }
                                root = Some(self.balance_insertion(r, x));
                                break;
                            }
                        }
                    }
                }
            }
        }
        self.tree_bins[bucket] = true;
        if let Some(root) = root {
            self.move_root_to_front(bucket, root);
        }
    }

    /// `putTreeVal` of the new node `x` into the tree bin `bucket`: it becomes a child of the
    /// node where the search ends and follows that node in the list.
    // Java: java/util/HashMap.java HashMap.TreeNode.putTreeVal
    fn put_tree_val(&mut self, bucket: usize, x: usize) {
        let root = self.table[bucket][0];
        let mut p = root;
        loop {
            let dir = self.dir(x, p);
            let xp = p;
            let next = if dir <= 0 {
                self.nodes[p].left
            } else {
                self.nodes[p].right
            };
            match next {
                Some(n) => p = n,
                None => {
                    if dir <= 0 {
                        self.nodes[xp].left = Some(x);
                    } else {
                        self.nodes[xp].right = Some(x);
                    }
                    self.nodes[x].parent = Some(xp);
                    let list = &mut self.table[bucket];
                    let pos = list
                        .iter()
                        .position(|&e| e == xp)
                        .expect("xp in its bucket");
                    list.insert(pos + 1, x);
                    let root = self.balance_insertion(root, x);
                    self.move_root_to_front(bucket, root);
                    return;
                }
            }
        }
    }

    /// `HashMap.resize`: the first table has 16 buckets, then the size doubles; each bucket
    /// splits into the same index and index + old capacity, keeping the node order; a tree bin
    /// (`TreeNode.split`) becomes a list at 6 nodes or fewer, keeps its tree when all of its
    /// nodes stay together, and is treeified again otherwise.
    fn resize(&mut self) {
        let old_cap = self.table.len();
        if old_cap == 0 {
            self.table = vec![Vec::new(); 16];
            self.tree_bins = vec![false; 16];
            return;
        }
        let old_table = std::mem::replace(&mut self.table, vec![Vec::new(); old_cap * 2]);
        let old_trees = std::mem::replace(&mut self.tree_bins, vec![false; old_cap * 2]);
        for (j, bucket) in old_table.into_iter().enumerate() {
            let (lo, hi): (Vec<usize>, Vec<usize>) = bucket
                .into_iter()
                .partition(|&e| (self.nodes[e].hash as u32 as usize) & old_cap == 0);
            let (lo_present, hi_present) = (!lo.is_empty(), !hi.is_empty());
            let (lc, hc) = (lo.len(), hi.len());
            self.table[j] = lo;
            self.table[j + old_cap] = hi;
            if !old_trees[j] {
                continue;
            }
            // Java: java/util/HashMap.java HashMap.TreeNode.split
            if lo_present && lc > UNTREEIFY_THRESHOLD {
                if hi_present {
                    self.treeify(j);
                } else {
                    self.tree_bins[j] = true;
                }
            }
            if hi_present && hc > UNTREEIFY_THRESHOLD {
                if lo_present {
                    self.treeify(j + old_cap);
                } else {
                    self.tree_bins[j + old_cap] = true;
                }
            }
        }
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
        let hash = Self::hash(key);
        let bucket = (hash as usize) & (n - 1);
        let (index, _) = self.entries.insert_full(key.to_string(), f());
        self.nodes.push(TreeLinks {
            hash: hash as i32,
            ..TreeLinks::default()
        });
        if self.tree_bins[bucket] {
            self.put_tree_val(bucket, index);
        } else {
            let bin_count = self.table[bucket].len();
            self.table[bucket].insert(0, index);
            if bin_count >= TREEIFY_THRESHOLD - 1 {
                // treeifyBin: a small table is resized instead.
                // Java: java/util/HashMap.java HashMap.treeifyBin
                if self.table.len() < MIN_TREEIFY_CAPACITY {
                    self.resize();
                } else {
                    self.treeify(bucket);
                }
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
        // `env.getEventBus().post(new NoBuildEvent())` only reaches the Build Event Protocol;
        // BazelBuildEventServiceModule creates no stream for this command (it is not in
        // ALLOWED_COMMANDS, crate::commands::bep), so nothing subscribes to the event.
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

    #[test]
    fn java_hash_map_tree_bins_follow_java_order() {
        // The keys and the expected order were recorded with java.util.HashMap (OpenJDK 25;
        // HashMap's tree bins are unchanged since JDK 8) by
        // ~/.cache/blaze/work/commands-fix-1/hashprobe/HashOrder.java: 64 keys of one
        // String.hashCode (one tree bin at every size) and 40 keys equal modulo 64 (one tree bin
        // at 64 buckets, split by each later resize), among 300 others. Each number is the
        // insertion index of the key at that position of `values()`.
        let mut colliding: Vec<String> = Vec::new();
        for i in 0..64 {
            let mut sb = String::new();
            for b in 0..6 {
                sb.push_str(if (i >> b) & 1 == 0 { "Aa" } else { "BB" });
            }
            colliding.push(sb);
        }
        let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        let mut low: Vec<String> = Vec::new();
        for x in alphabet.chars() {
            for y in alphabet.chars() {
                let s = format!("{x}{y}");
                let h = (x as i32).wrapping_mul(31).wrapping_add(y as i32);
                if h & 63 == 5 && low.len() < 40 {
                    low.push(s);
                }
            }
        }
        let mut keys: Vec<String> = Vec::new();
        let (mut c, mut l) = (0, 0);
        for i in 0..300 {
            keys.push(format!("flag_{i}"));
            if i % 4 == 0 && c < colliding.len() {
                keys.push(colliding[c].clone());
                c += 1;
            }
            if i >= 30 && i % 3 == 0 && l < low.len() {
                keys.push(low[l].clone());
                l += 1;
            }
        }
        keys.extend(colliding[c..].iter().cloned());
        keys.extend(low[l..].iter().cloned());
        assert_eq!(keys.len(), 404);
        let mut map = JavaHashMap::new();
        for (i, key) in keys.iter().enumerate() {
            map.compute_if_absent(key, || i);
        }
        let expected: Vec<usize> = vec![
            163, 10, 9, 12, 5, 4, 8, 7, 0, 3, 2, 167, 172, 44, 39, 177, 182, 53, 49, 191, 186, 63,
            58, 104, 107, 102, 103, 201, 196, 72, 68, 114, 116, 111, 113, 108, 109, 122, 123, 119,
            121, 117, 132, 128, 130, 126, 127, 140, 141, 136, 138, 133, 220, 210, 205, 82, 135, 91,
            77, 146, 147, 142, 145, 224, 215, 106, 96, 87, 51, 52, 47, 50, 45, 46, 41, 43, 38, 40,
            54, 69, 65, 66, 62, 64, 59, 60, 56, 57, 70, 71, 120, 110, 101, 83, 84, 79, 81, 76, 78,
            73, 75, 88, 89, 85, 100, 97, 98, 94, 95, 90, 92, 134, 125, 115, 148, 139, 129, 303,
            304, 299, 300, 297, 298, 294, 23, 295, 24, 292, 20, 293, 22, 18, 19, 15, 17, 13, 14,
            302, 290, 37, 34, 35, 32, 33, 29, 30, 27, 28, 25, 153, 144, 330, 166, 332, 168, 328,
            329, 165, 327, 161, 164, 324, 159, 325, 160, 322, 155, 323, 157, 319, 152, 320, 154, 6,
            26, 36, 236, 162, 336, 256, 86, 316, 188, 356, 276, 137, 31, 80, 181, 351, 271, 131,
            311, 231, 55, 156, 331, 251, 105, 291, 207, 21, 219, 143, 321, 241, 67, 301, 169, 341,
            261, 118, 16, 112, 296, 213, 74, 175, 346, 266, 124, 306, 226, 61, 11, 48, 150, 326,
            246, 99, 286, 200, 1, 93, 281, 194, 42, 317, 149, 318, 151, 315, 313, 314, 310, 312,
            308, 309, 305, 307, 158, 358, 202, 359, 203, 355, 198, 357, 199, 353, 354, 197, 352,
            193, 195, 349, 190, 350, 192, 347, 187, 348, 189, 344, 184, 345, 185, 342, 180, 343,
            183, 340, 179, 338, 176, 339, 178, 335, 173, 337, 174, 333, 170, 334, 171, 380, 234,
            381, 235, 378, 232, 379, 233, 376, 229, 377, 230, 374, 375, 228, 373, 225, 227, 371,
            222, 372, 223, 369, 218, 370, 221, 367, 216, 368, 217, 365, 212, 366, 214, 364, 211,
            362, 208, 363, 209, 360, 204, 361, 206, 394, 395, 253, 402, 262, 403, 263, 400, 259,
            401, 260, 398, 257, 399, 258, 396, 254, 397, 255, 393, 250, 252, 384, 391, 248, 392,
            249, 389, 245, 390, 247, 387, 243, 388, 244, 385, 240, 386, 242, 239, 382, 237, 383,
            238, 279, 280, 278, 289, 287, 288, 284, 285, 282, 283, 265, 267, 275, 277, 273, 274,
            270, 272, 268, 269, 264,
        ];
        assert_eq!(map.into_values(), expected);
    }
}
