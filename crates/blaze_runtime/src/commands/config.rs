//! `blaze config`: displays details of the configurations in the server's Skyframe graph.
//!
//! Ports `runtime/commands/ConfigCommand.java` and `runtime/commands/ConfigCommandOutputFormatter.java`.
//! The output data of `analysis/config/output/` (`ConfigurationForOutput`, `FragmentForOutput`,
//! `FragmentOptionsForOutput`), the text writer of one configuration and the JSON writer of one
//! configuration are `blaze_build_config::config_output`'s (shared with cquery's proto output);
//! the writers it does not have (the ids, the list of configurations as JSON, the diff) are
//! here.
//!
//! The configurations are the `BuildConfigurationValue`s done in the server's graph, which
//! earlier commands left there ([`crate::commands::dump::server_evaluator`]). PORT: a server
//! that keeps no graph (decision D2) finds none and reports "No configurations found", as Bazel
//! does in a fresh server or with `--batch`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::{BuildConfigurationValue, GlobalStateProvider};
use blaze_build_config::config_output::{self, ConfigurationForOutput, FragmentOptionsForOutput};
use blaze_build_config::fragment::FragmentRegistry;
use blaze_events::{Event, EventHandler as _};
use blaze_optdefs::fragment::config_command::{
    ConfigCommandConfigOptions, ConfigCommandOutputType,
};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_skyframe::{Evaluator, Key};

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::options_result::OptionsParsingResult;

/// Handles the 'config' command on the Blaze command line.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:84 ConfigCommand
pub struct ConfigCommand;

/// Data structure defining the difference between two `BuildConfigurationValue`s from the point
/// of this command's output.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:122 ConfigCommand.ConfigurationDiffForOutput
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConfigurationDiffForOutput {
    pub config_hash1: String,
    pub config_hash2: String,
    pub fragments_diff: Vec<FragmentDiffForOutput>,
}

/// Data structure defining the difference between two `BuildConfigurationValue`s for a given
/// `FragmentOptions` from the point of this command's output.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:155 ConfigCommand.FragmentDiffForOutput
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FragmentDiffForOutput {
    pub name: String,
    /// `Map<String, Pair<String, String>>` (an `ImmutableSortedMap`).
    pub options_diff: BTreeMap<String, (String, String)>,
}

impl BlazeCommand for ConfigCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &crate::command_registry::CONFIG_COMMAND
    }

    /// Main entry point into the `blaze config` command.
    ///
    /// Its purpose is to parse all options, figure out what variation of the command that
    /// implies, run the right logic, and return the right exit code.
    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:184 ConfigCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        let evaluator = crate::commands::dump::server_evaluator(env);
        let configurations = find_configurations(&evaluator);
        if configurations.is_empty() {
            let message = "No configurations found. This can happen if the 'config' subcommand \
                           is used after files, including their metadata, have changed since the \
                           last invocation of another subcommand. Try running a 'build' or \
                           'cquery' directly followed by 'config'.";
            env.get_reporter().handle(&Event::error(message));
            return create_failure_result(
                message,
                failure_details::config_command::Code::ConfigurationNotFound,
            );
        }

        // new PrintWriter(new OutputStreamWriter(outErr.getOutputStream(), UTF_8)), closed (and
        // so flushed) at the end of the try block.
        let mut writer = String::new();
        let config_command_options: ConfigCommandConfigOptions = options
            .get_options::<ConfigCommandConfigOptions>()
            .expect("NullPointerException: ConfigOptions");
        let output_formatter: &dyn ConfigCommandOutputFormatter =
            if config_command_options.output_type == ConfigCommandOutputType::TEXT {
                &TextOutputFormatter
            } else {
                &JsonOutputFormatter
            };
        let rule_class_provider = BazelRuleClassProvider::create();
        // getFragmentDefs(...getFragmentRegistry()): config_output reads the registry's
        // fragments and their required options in that order.
        // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:270 ConfigCommand.getFragmentDefs
        let fragment_defs = rule_class_provider.get_fragment_registry();

        let residue = options.get_residue();
        let result = if residue.is_empty() {
            if config_command_options.dump_all {
                report_all_configurations(
                    output_formatter,
                    &mut writer,
                    &for_output(&configurations, fragment_defs),
                )
            } else {
                report_configuration_ids(
                    output_formatter,
                    &mut writer,
                    &for_output(&configurations, fragment_defs),
                )
            }
        } else if residue.len() == 1 {
            let config_hash = &residue[0];
            report_single_configuration(
                output_formatter,
                &mut writer,
                env,
                &for_output(&configurations, fragment_defs),
                config_hash,
            )
        } else if residue.len() == 2 {
            let config_hash1 = &residue[0];
            let config_hash2 = &residue[1];
            report_configuration_diff(
                &for_output(&configurations, fragment_defs),
                config_hash1,
                config_hash2,
                output_formatter,
                &mut writer,
                env,
            )
        } else {
            let message = "Too many config ids.";
            env.get_reporter().handle(&Event::error(message));
            create_failure_result(
                message,
                failure_details::config_command::Code::TooManyConfigIds,
            )
        };
        // The UTF-8 encoder of the writer: an internal string is encoded char by char.
        let out_err = env.get_reporter().get_out_err();
        let _ = out_err.get_output_stream().write(writer.as_bytes());
        let _ = out_err.get_output_stream().flush();
        result
    }
}

/// Returns all `BuildConfigurationValue`s in Skyframe as a map from their
/// `BuildConfigurationKey` to instance, sorted by the key's options checksum.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:248 ConfigCommand.findConfigurations
fn find_configurations(
    evaluator: &Evaluator,
) -> Vec<(BuildConfigurationKey, Arc<BuildConfigurationValue>)> {
    let mut configurations: Vec<(BuildConfigurationKey, Arc<BuildConfigurationValue>)> = evaluator
        .get_done_values()
        .into_iter()
        .filter(|(k, _)| k.function_name() == BuildConfigurationKey::FUNCTION)
        .map(|(k, v)| {
            (
                k.downcast::<BuildConfigurationKey>()
                    .expect("ClassCastException: BuildConfigurationKey")
                    .clone(),
                v.downcast::<BuildConfigurationValue>()
                    .expect("ClassCastException: BuildConfigurationValue"),
            )
        })
        .collect();
    // toImmutableSortedMap(comparing(e -> e.getOptions().checksum()), ...): a duplicate key
    // throws.
    configurations.sort_by(|a, b| {
        a.0.get_options()
            .checksum()
            .cmp(b.0.get_options().checksum())
    });
    for pair in configurations.windows(2) {
        if pair[0].0.get_options().checksum() == pair[1].0.get_options().checksum() {
            panic!(
                "IllegalArgumentException: Multiple entries with same key: {:?}",
                pair[0].0
            );
        }
    }
    configurations
}

/// `String.compareTo`: UTF-16 code units (the class names are ASCII).
fn java_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Converts `find_configurations`'s output into `ConfigurationForOutput`s sorted by hash.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:286 ConfigCommand.forOutput
fn for_output(
    as_sky_key_map: &[(BuildConfigurationKey, Arc<BuildConfigurationValue>)],
    fragment_defs: &FragmentRegistry,
) -> Vec<ConfigurationForOutput> {
    let mut ans: Vec<ConfigurationForOutput> = Vec::new();
    for (_key, config) in as_sky_key_map {
        // getConfigurationForOutput(key, config.checksum(), config, fragmentDefs): the key is the
        // configuration's own (`config.get_key()`).
        let c = ConfigurationForOutput::get_configuration_for_output(config, fragment_defs);
        // ImmutableSortedSet.orderedBy(comparing(e -> e.getConfigHash())): the first of equal
        // elements is kept.
        if !ans.iter().any(|e| e.config_hash == c.config_hash) {
            ans.push(c);
        }
    }
    ans.sort_by(|a, b| java_compare(&a.config_hash, &b.config_hash));
    ans
}

/// `InvalidConfigurationException` of [`get_configuration`].
struct InvalidConfigurationException(String);

/// Returns the configuration matching a hash prefix (an error unless exactly one matches).
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:311 ConfigCommand.getConfiguration
fn get_configuration<'a>(
    configurations: &'a [ConfigurationForOutput],
    config_prefix: &str,
) -> Result<&'a ConfigurationForOutput, InvalidConfigurationException> {
    let matches: Vec<&ConfigurationForOutput> = configurations
        .iter()
        .filter(|config| does_config_match(config, config_prefix))
        .collect();
    if matches.is_empty() {
        return Err(InvalidConfigurationException(format!(
            "No configuration found with ID prefix {}",
            config_prefix
        )));
    } else if matches.len() > 1 {
        return Err(InvalidConfigurationException(format!(
            "Configuration identifier '{}' is ambiguous.\n'{}' is a prefix of multiple \
             configurations:\n {}\n\nUse a sufficient prefix to uniquely identify one \
             configuration.",
            config_prefix,
            config_prefix,
            matches
                .iter()
                .map(|c| c.config_hash.as_str())
                .collect::<Vec<_>>()
                .join("\n ")
        )));
    }
    Ok(matches[0])
}

// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:337 ConfigCommand.doesConfigMatch
fn does_config_match(config: &ConfigurationForOutput, config_prefix: &str) -> bool {
    config.config_hash.starts_with(config_prefix)
}

/// Reports the result of `blaze config --dump_all`.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:345 ConfigCommand.reportAllConfigurations
fn report_all_configurations(
    writer: &dyn ConfigCommandOutputFormatter,
    out: &mut String,
    configurations: &[ConfigurationForOutput],
) -> BlazeCommandResult {
    writer.write_configurations(out, configurations);
    BlazeCommandResult::success()
}

/// Reports the result of `blaze config`.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:355 ConfigCommand.reportConfigurationIds
fn report_configuration_ids(
    writer: &dyn ConfigCommandOutputFormatter,
    out: &mut String,
    configurations: &[ConfigurationForOutput],
) -> BlazeCommandResult {
    writer.write_configuration_ids(out, configurations);
    BlazeCommandResult::success()
}

/// Reports the result of `blaze config <configHash>`.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:366 ConfigCommand.reportSingleConfiguration
fn report_single_configuration(
    writer: &dyn ConfigCommandOutputFormatter,
    out: &mut String,
    env: &CommandEnvironment,
    all_configurations: &[ConfigurationForOutput],
    config_hash: &str,
) -> BlazeCommandResult {
    env.get_reporter().handle(&Event::info(format!(
        "Displaying config with id {}",
        config_hash
    )));
    match get_configuration(all_configurations, config_hash) {
        Ok(configuration) => {
            writer.write_configuration(out, configuration);
            BlazeCommandResult::success()
        }
        Err(InvalidConfigurationException(message)) => {
            env.get_reporter().handle(&Event::error(message.clone()));
            create_failure_result(
                &message,
                failure_details::config_command::Code::ConfigurationNotFound,
            )
        }
    }
}

/// Reports the result of `blaze config <configHash1> <configHash2>`.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:386 ConfigCommand.reportConfigurationDiff
fn report_configuration_diff(
    all_configs: &[ConfigurationForOutput],
    config_hash1: &str,
    config_hash2: &str,
    writer: &dyn ConfigCommandOutputFormatter,
    out: &mut String,
    env: &CommandEnvironment,
) -> BlazeCommandResult {
    env.get_reporter().handle(&Event::info(format!(
        "Displaying diff between configs {} and {}",
        config_hash1, config_hash2
    )));
    let configs = get_configuration(all_configs, config_hash1)
        .and_then(|c1| get_configuration(all_configs, config_hash2).map(|c2| (c1, c2)));
    match configs {
        Ok((config1, config2)) => {
            let diffs = diff_configurations(config1, config2);
            writer.write_configuration_diff(
                out,
                &get_configuration_diff_for_output(config_hash1, config_hash2, diffs),
            );
            BlazeCommandResult::success()
        }
        Err(InvalidConfigurationException(message)) => {
            env.get_reporter().handle(&Event::error(message.clone()));
            create_failure_result(
                &message,
                failure_details::config_command::Code::ConfigurationNotFound,
            )
        }
    }
}

/// `Table<String, String, Pair<Object, Object>>` (a `HashBasedTable`): fragment name to option
/// name to the two values (`None` is Java's null).
///
/// PORT: the table's row and column order is never observable: `getConfigurationDiffForOutput`
/// sorts both.
type DiffTable = BTreeMap<String, BTreeMap<String, (Option<String>, Option<String>)>>;

// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:412 ConfigCommand.diffConfigurations
fn diff_configurations(
    config1: &ConfigurationForOutput,
    config2: &ConfigurationForOutput,
) -> DiffTable {
    let mut diffs: DiffTable = BTreeMap::new();
    // Sets.union(config1.fragmentOptionNames(), config2.fragmentOptionNames())
    let fragment_names: BTreeSet<String> = config1
        .fragment_options
        .iter()
        .chain(&config2.fragment_options)
        .map(|f| f.name.clone())
        .collect();
    // ConfigurationForOutput.fragment(name): the first with that name.
    let fragment = |config: &'_ ConfigurationForOutput, name: &str| {
        config
            .fragment_options
            .iter()
            .find(|f| f.name == name)
            .cloned()
    };
    for fragment_name in fragment_names {
        let options1 = fragment(config1, &fragment_name);
        let options2 = fragment(config2, &fragment_name);
        let row = diff_options(options1.as_ref(), options2.as_ref());
        // diffs.row(fragmentName).putAll(...): an empty row adds no cell, so the fragment is not
        // a row key.
        if !row.is_empty() {
            diffs.entry(fragment_name).or_default().extend(row);
        }
    }
    diffs
}

// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:425 ConfigCommand.diffOptions
fn diff_options(
    options1: Option<&FragmentOptionsForOutput>,
    options2: Option<&FragmentOptionsForOutput>,
) -> BTreeMap<String, (Option<String>, Option<String>)> {
    let option_names1: BTreeSet<&String> = options1
        .map(|o| o.options.keys().collect())
        .unwrap_or_default();
    let option_names2: BTreeSet<&String> = options2
        .map(|o| o.options.keys().collect())
        .unwrap_or_default();
    let mut diffs = BTreeMap::new();
    for option_name in option_names1.union(&option_names2) {
        let value1 = options1.and_then(|o| o.options.get(*option_name)).cloned();
        let value2 = options2.and_then(|o| o.options.get(*option_name)).cloned();
        if value1 != value2 {
            diffs.insert((*option_name).clone(), (value1, value2));
        }
    }
    diffs
}

// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:443 ConfigCommand.getConfigurationDiffForOutput
fn get_configuration_diff_for_output(
    config_hash1: &str,
    config_hash2: &str,
    diffs: DiffTable,
) -> ConfigurationDiffForOutput {
    // ImmutableSortedSet.orderedBy(comparing(e -> e.name)) over rowKeySet(), each row's cells as
    // an ImmutableSortedMap with natural ordering.
    let mut fragment_diffs: Vec<FragmentDiffForOutput> = diffs
        .into_iter()
        .map(|(fragment_name, row)| FragmentDiffForOutput {
            name: fragment_name,
            options_diff: row
                .into_iter()
                .map(|(k, v)| (k, to_nullable_string_pair(v)))
                .collect(),
        })
        .collect();
    fragment_diffs.sort_by(|a, b| java_compare(&a.name, &b.name));
    ConfigurationDiffForOutput {
        config_hash1: config_hash1.to_string(),
        config_hash2: config_hash2.to_string(),
        fragments_diff: fragment_diffs,
    }
}

/// `Pair.of(String.valueOf(pair.first), String.valueOf(pair.second))`.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:462 ConfigCommand.toNullableStringPair
fn to_nullable_string_pair(pair: (Option<String>, Option<String>)) -> (String, String) {
    (
        pair.0.unwrap_or_else(|| "null".to_string()),
        pair.1.unwrap_or_else(|| "null".to_string()),
    )
}

// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:466 ConfigCommand.createFailureResult
fn create_failure_result(
    message: &str,
    detailed_code: failure_details::config_command::Code,
) -> BlazeCommandResult {
    BlazeCommandResult::failure_detail(FailureDetail {
        message: message.to_string(),
        category: Some(Category::ConfigCommand(failure_details::ConfigCommand {
            code: detailed_code as i32,
        })),
    })
}

// ---------------------------------------------------------------------------------------------
// ConfigCommandOutputFormatter

/// Formats output for [`ConfigCommand`].
///
/// The basic contract is `ConfigCommand` makes all important structural decisions: what data
/// gets reported, how different pieces of data relate to each other, and how data is ordered. A
/// formatter then outputs this in a format-appropriate way.
///
/// PORT: Java's formatters hold the `PrintWriter`; here each method is given the buffer.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:39 ConfigCommandOutputFormatter
pub trait ConfigCommandOutputFormatter {
    /// Outputs a list of configuration hash IDs.
    fn write_configuration_ids(
        &self,
        writer: &mut String,
        configurations: &[ConfigurationForOutput],
    );
    /// Outputs a single configuration.
    fn write_configuration(&self, writer: &mut String, configuration: &ConfigurationForOutput);
    /// Outputs a series of configurations.
    fn write_configurations(&self, writer: &mut String, configurations: &[ConfigurationForOutput]);
    /// Outputs the diff between two configurations.
    fn write_configuration_diff(&self, writer: &mut String, diff: &ConfigurationDiffForOutput);
}

/// A [`ConfigCommandOutputFormatter`] that outputs plain user-readable text.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:62 ConfigCommandOutputFormatter.TextOutputFormatter
pub struct TextOutputFormatter;

impl ConfigCommandOutputFormatter for TextOutputFormatter {
    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:68 ConfigCommandOutputFormatter.TextOutputFormatter.writeConfigurationIDs
    fn write_configuration_ids(
        &self,
        writer: &mut String,
        configurations: &[ConfigurationForOutput],
    ) {
        writer.push_str(&config_output::text_write_configuration_ids(configurations));
    }

    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:86 ConfigCommandOutputFormatter.TextOutputFormatter.writeConfiguration
    fn write_configuration(&self, writer: &mut String, configuration: &ConfigurationForOutput) {
        writer.push_str(&config_output::text_write_configuration(configuration));
    }

    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:108 ConfigCommandOutputFormatter.TextOutputFormatter.writeConfigurations
    fn write_configurations(&self, writer: &mut String, configurations: &[ConfigurationForOutput]) {
        for config in configurations {
            self.write_configuration(writer, config);
        }
    }

    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:115 ConfigCommandOutputFormatter.TextOutputFormatter.writeConfigurationDiff
    fn write_configuration_diff(&self, writer: &mut String, diff: &ConfigurationDiffForOutput) {
        writer.push_str(&format!(
            "Displaying diff between configs {} and {}\n",
            diff.config_hash1, diff.config_hash2
        ));
        for fragment_diff in &diff.fragments_diff {
            writer.push_str(&format!("FragmentOptions {} {{\n", fragment_diff.name));
            for (key, (first, second)) in &fragment_diff.options_diff {
                writer.push_str(&format!("  {}: {}, {}\n", key, first, second));
            }
            writer.push_str("}\n");
        }
    }
}

/// A [`ConfigCommandOutputFormatter`] that outputs structured JSON (`new Gson().toJson`).
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:133 ConfigCommandOutputFormatter.JsonOutputFormatter
pub struct JsonOutputFormatter;

impl ConfigCommandOutputFormatter for JsonOutputFormatter {
    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:142 ConfigCommandOutputFormatter.JsonOutputFormatter.writeConfigurationIDs
    fn write_configuration_ids(
        &self,
        writer: &mut String,
        configurations: &[ConfigurationForOutput],
    ) {
        // gson.toJson(ImmutableMap.of("configuration-IDs", configurationIDs))
        let mut json = String::from("{");
        json.push_str(&config_output::gson_string("configuration-IDs"));
        json.push_str(":[");
        for (i, config) in configurations.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&config_output::gson_string(&config.config_hash));
        }
        json.push_str("]}");
        writer.push_str(&json);
        writer.push('\n');
    }

    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:151 ConfigCommandOutputFormatter.JsonOutputFormatter.writeConfiguration
    fn write_configuration(&self, writer: &mut String, configuration: &ConfigurationForOutput) {
        // writer.println(gson.toJson(configuration))
        writer.push_str(&config_output::json_write_configuration(configuration));
    }

    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:156 ConfigCommandOutputFormatter.JsonOutputFormatter.writeConfigurations
    fn write_configurations(&self, writer: &mut String, configurations: &[ConfigurationForOutput]) {
        // writer.println(gson.toJson(configurations)): a JSON array of the same objects.
        writer.push('[');
        for (i, config) in configurations.iter().enumerate() {
            if i > 0 {
                writer.push(',');
            }
            let json = config_output::json_write_configuration(config);
            writer.push_str(json.strip_suffix('\n').unwrap_or(&json));
        }
        writer.push(']');
        writer.push('\n');
    }

    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:161 ConfigCommandOutputFormatter.JsonOutputFormatter.writeConfigurationDiff
    fn write_configuration_diff(&self, writer: &mut String, diff: &ConfigurationDiffForOutput) {
        // Gson's reflective adapter: the declared fields in order. Pair's fields are `first`
        // and `second`.
        writer.push('{');
        writer.push_str(&config_output::gson_string("configHash1"));
        writer.push(':');
        writer.push_str(&config_output::gson_string(&diff.config_hash1));
        writer.push(',');
        writer.push_str(&config_output::gson_string("configHash2"));
        writer.push(':');
        writer.push_str(&config_output::gson_string(&diff.config_hash2));
        writer.push(',');
        writer.push_str(&config_output::gson_string("fragmentsDiff"));
        writer.push_str(":[");
        for (i, fragment_diff) in diff.fragments_diff.iter().enumerate() {
            if i > 0 {
                writer.push(',');
            }
            writer.push('{');
            writer.push_str(&config_output::gson_string("name"));
            writer.push(':');
            writer.push_str(&config_output::gson_string(&fragment_diff.name));
            writer.push(',');
            writer.push_str(&config_output::gson_string("optionsDiff"));
            writer.push_str(":{");
            for (j, (key, (first, second))) in fragment_diff.options_diff.iter().enumerate() {
                if j > 0 {
                    writer.push(',');
                }
                writer.push_str(&config_output::gson_string(key));
                writer.push_str(":{");
                writer.push_str(&config_output::gson_string("first"));
                writer.push(':');
                writer.push_str(&config_output::gson_string(first));
                writer.push(',');
                writer.push_str(&config_output::gson_string("second"));
                writer.push(':');
                writer.push_str(&config_output::gson_string(second));
                writer.push('}');
            }
            writer.push_str("}}");
        }
        writer.push_str("]}");
        writer.push('\n');
    }
}

// The tests (ConfigCommandTest) are in tests/config_command_test.rs: they run several commands on
// one runtime (blaze_buildtool_testing::BuildIntegrationTestCase).
