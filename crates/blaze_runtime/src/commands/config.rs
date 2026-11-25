//! `blaze config`: displays details of the configurations in the server's Skyframe graph.
//!
//! Ports `runtime/commands/ConfigCommand.java`, `runtime/commands/ConfigCommandOutputFormatter.java`
//! and the output data of `analysis/config/output/` (`ConfigurationForOutput`,
//! `FragmentForOutput`, `FragmentOptionsForOutput`).
//!
//! PORT: the `analysis.config.output` classes belong to `blaze_build_config`'s package; they are
//! kept here, next to their only user.
//!
//! PORT: the configurations are the `BuildConfigurationValue`s done in the server's graph, which
//! earlier commands left there. Blaze keeps no graph between commands (decision D2, see
//! [`crate::commands::dump::server_evaluator`]), so a command always finds none and reports
//! "No configurations found", as Bazel does in a fresh server or with `--batch`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::{BuildConfigurationValue, GlobalStateProvider};
use blaze_build_config::fragment::{FragmentClass, FragmentRegistry, required_options};
use blaze_build_config::fragment_options_class::FragmentOptionsClass;
use blaze_events::{Event, EventHandler as _};
use blaze_optdefs::fragment::config_command::{
    ConfigCommandConfigOptions, ConfigCommandOutputType,
};
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_options::value::java_string_of;
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
        let evaluator = crate::commands::dump::server_evaluator();
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
        let fragment_defs = get_fragment_defs(rule_class_provider.get_fragment_registry());

        let residue = options.get_residue();
        let result = if residue.is_empty() {
            if config_command_options.dump_all {
                report_all_configurations(
                    output_formatter,
                    &mut writer,
                    &for_output(&configurations, &fragment_defs),
                )
            } else {
                report_configuration_ids(
                    output_formatter,
                    &mut writer,
                    &for_output(&configurations, &fragment_defs),
                )
            }
        } else if residue.len() == 1 {
            let config_hash = &residue[0];
            report_single_configuration(
                output_formatter,
                &mut writer,
                env,
                &for_output(&configurations, &fragment_defs),
                config_hash,
            )
        } else if residue.len() == 2 {
            let config_hash1 = &residue[0];
            let config_hash2 = &residue[1];
            report_configuration_diff(
                &for_output(&configurations, &fragment_defs),
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

/// The fragments Blaze "knows about" (the rule class provider's registry), each with the
/// `FragmentOptions` it requires, sorted by class name.
///
/// These are the fragments that Blaze "knows about", not necessarily the fragments in a
/// `BuildConfigurationValue`. Trimming, in particular, strips fragments out of actual
/// configurations. It's safe to assume untrimmed configuration have all fragments listed here.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:270 ConfigCommand.getFragmentDefs
pub fn get_fragment_defs(
    fragment_registry: &FragmentRegistry,
) -> Vec<(FragmentClass, Vec<FragmentOptionsClass>)> {
    let mut defs: Vec<(FragmentClass, Vec<FragmentOptionsClass>)> = fragment_registry
        .get_all_fragments()
        .iter()
        .map(|fragment| {
            // ImmutableSortedSet.copyOf(comparing(Class::getName), requiredOptions(fragment))
            let mut options = required_options(fragment);
            options.sort_by(|a, b| java_compare(a.get_name(), b.get_name()));
            options.dedup();
            (fragment, options)
        })
        .collect();
    // FragmentClassSet.LEXICAL_FRAGMENT_SORTER: by class name.
    defs.sort_by(|a, b| java_compare(a.0.get_name(), b.0.get_name()));
    defs
}

/// `String.compareTo`: UTF-16 code units (the class names are ASCII).
fn java_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Converts `find_configurations`'s output into `ConfigurationForOutput`s sorted by hash.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:286 ConfigCommand.forOutput
fn for_output(
    as_sky_key_map: &[(BuildConfigurationKey, Arc<BuildConfigurationValue>)],
    fragment_defs: &[(FragmentClass, Vec<FragmentOptionsClass>)],
) -> Vec<ConfigurationForOutput> {
    let mut ans: Vec<ConfigurationForOutput> = Vec::new();
    for (key, config) in as_sky_key_map {
        let c = ConfigurationForOutput::get_configuration_for_output(
            key,
            config.checksum(),
            config,
            fragment_defs,
        );
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
                .map(|c| c.get_config_hash())
                .collect::<Vec<_>>()
                .join("\n ")
        )));
    }
    Ok(matches[0])
}

// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:337 ConfigCommand.doesConfigMatch
fn does_config_match(config: &ConfigurationForOutput, config_prefix: &str) -> bool {
    config.get_config_hash().starts_with(config_prefix)
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
    let fragment_names: BTreeSet<String> = config1
        .fragment_option_names()
        .union(&config2.fragment_option_names())
        .cloned()
        .collect();
    for fragment_name in fragment_names {
        let options1 = config1.fragment(&fragment_name);
        let options2 = config2.fragment(&fragment_name);
        let row = diff_options(options1, options2);
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
        .map(|o| o.option_names().collect())
        .unwrap_or_default();
    let option_names2: BTreeSet<&String> = options2
        .map(|o| o.option_names().collect())
        .unwrap_or_default();
    let mut diffs = BTreeMap::new();
    for option_name in option_names1.union(&option_names2) {
        let value1 = options1.and_then(|o| o.get_option(option_name)).cloned();
        let value2 = options2.and_then(|o| o.get_option(option_name)).cloned();
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
// analysis/config/output

/// Data structure defining a `BuildConfigurationValue` for the purpose of returning user output
/// about the configuration.
///
/// Includes all data representing a "configuration" and defines their relative structure and
/// list order. A [`ConfigCommandOutputFormatter`] uses this to lightly format output from a
/// logically consistent core structure.
// Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:46 ConfigurationForOutput
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConfigurationForOutput {
    sky_key: String,
    config_hash: String,
    mnemonic: String,
    is_exec: bool,
    fragments: Vec<FragmentForOutput>,
    fragment_options: Vec<FragmentOptionsForOutput>,
}

impl ConfigurationForOutput {
    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:54 ConfigurationForOutput.<init>
    pub fn new(
        sky_key: String,
        config_hash: String,
        mnemonic: String,
        is_exec: bool,
        fragments: Vec<FragmentForOutput>,
        fragment_options: Vec<FragmentOptionsForOutput>,
    ) -> Self {
        ConfigurationForOutput {
            sky_key,
            config_hash,
            mnemonic,
            is_exec,
            fragments,
            fragment_options,
        }
    }

    pub fn get_sky_key(&self) -> &str {
        &self.sky_key
    }

    pub fn get_config_hash(&self) -> &str {
        &self.config_hash
    }

    pub fn get_mnemonic(&self) -> &str {
        &self.mnemonic
    }

    pub fn is_exec(&self) -> bool {
        self.is_exec
    }

    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:86 ConfigurationForOutput.hasTestConfig
    pub fn has_test_config(&self) -> bool {
        self.fragment_options
            .iter()
            .any(|f| f.get_name().contains("TestConfiguration"))
    }

    pub fn get_fragments(&self) -> &[FragmentForOutput] {
        &self.fragments
    }

    /// The union of `FragmentOptionsForOutput` used by the Fragments associated with this
    /// configuration, sorted by FragmentOptionsForOutput name.
    pub fn get_fragment_options(&self) -> &[FragmentOptionsForOutput] {
        &self.fragment_options
    }

    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:104 ConfigurationForOutput.fragment
    pub fn fragment(&self, fragment_name: &str) -> Option<&FragmentOptionsForOutput> {
        self.fragment_options
            .iter()
            .find(|fo| fo.get_name() == fragment_name)
    }

    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:111 ConfigurationForOutput.fragmentOptionNames
    pub fn fragment_option_names(&self) -> BTreeSet<String> {
        self.fragment_options
            .iter()
            .map(|f| f.get_name().to_string())
            .collect()
    }

    /// Constructs a `ConfigurationForOutput` from the given input data.
    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:157 ConfigurationForOutput.getConfigurationForOutput
    pub fn get_configuration_for_output(
        sky_key: &BuildConfigurationKey,
        config_hash: &str,
        config: &BuildConfigurationValue,
        fragment_defs: &[(FragmentClass, Vec<FragmentOptionsClass>)],
    ) -> ConfigurationForOutput {
        // ImmutableSortedSet.orderedBy(comparing(e -> e.getName())): an element equal by name to
        // one already added is dropped.
        let mut fragments: Vec<FragmentForOutput> = Vec::new();
        let mut add_fragment = |f: FragmentForOutput| {
            if !fragments.iter().any(|e| e.name == f.name) {
                fragments.push(f);
            }
        };
        for (fragment, options) in fragment_defs {
            add_fragment(FragmentForOutput::new(
                fragment.get_name().to_string(),
                options.iter().map(|c| c.get_name().to_string()).collect(),
            ));
        }
        for (fragment, options) in fragment_defs
            .iter()
            .filter(|(fragment, _)| config.has_fragment(fragment))
        {
            add_fragment(FragmentForOutput::new(
                fragment.get_name().to_string(),
                options.iter().map(|c| c.get_name().to_string()).collect(),
            ));
        }
        fragments.sort_by(|a, b| java_compare(&a.name, &b.name));

        let mut fragment_options: Vec<FragmentOptionsForOutput> = Vec::new();
        let mut add_options = |f: FragmentOptionsForOutput| {
            if !fragment_options.iter().any(|e| e.name == f.name) {
                fragment_options.push(f);
            }
        };
        let build_options = config.get_options();
        for options_class in build_options.get_fragment_classes() {
            let fragment_options_instance = build_options
                .get_dyn(options_class)
                .expect("BuildOptions.get of one of its classes");
            // fragmentOptionsInstance.getClass().getName()
            add_options(FragmentOptionsForOutput::new(
                options_class.get_name().to_string(),
                get_ordered_native_options(options_class, fragment_options_instance),
            ));
        }
        add_options(FragmentOptionsForOutput::new(
            USER_DEFINED_FRAGMENT_DESCRIPTIVE_NAME.to_string(),
            get_ordered_user_defined_options(config),
        ));
        fragment_options.sort_by(|a, b| java_compare(&a.name, &b.name));

        ConfigurationForOutput::new(
            format!("{:?}", sky_key),
            config_hash.to_string(),
            config.get_mnemonic().to_string(),
            config.is_exec_configuration(),
            fragments,
            fragment_options,
        )
    }
}

/// `UserDefinedFragment.DESCRIPTIVE_NAME`: Starlark options don't have configuration
/// fragments; this keeps their output consistent with native options.
// Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:259 ConfigurationForOutput.UserDefinedFragment
const USER_DEFINED_FRAGMENT_DESCRIPTIVE_NAME: &str = "user-defined";

/// Returns a `FragmentOptions`'s native option settings in canonical order.
///
/// While actual option values are objects, we serialize them to strings to prevent command
/// output from interpreting them more deeply than we want for simple "name=value" output.
// Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:214 ConfigurationForOutput.getOrderedNativeOptions
fn get_ordered_native_options(
    options_class: FragmentOptionsClass,
    options: &dyn blaze_options::definition::OptionsBase,
) -> BTreeMap<String, String> {
    let is_core_options = options_class.get_name() == CORE_OPTIONS_CLASS_NAME;
    let mut map = BTreeMap::new();
    for (key, value) in options.as_map() {
        // While technically part of CoreOptions, --define is practically a user-definable flag
        // so we include it in the user-defined fragment for clarity. See
        // getOrderedUserDefinedOptions.
        if is_core_options && key == "define" {
            continue;
        }
        // toImmutableSortedMap throws on a duplicate key; option names are unique.
        map.insert(key, java_string_of(value.as_ref()));
    }
    map
}

/// `CoreOptions.class.getName()`.
const CORE_OPTIONS_CLASS_NAME: &str = "com.google.devtools.build.lib.analysis.config.CoreOptions";

/// Returns a configuration's user-definable settings in canonical order.
// Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:232 ConfigurationForOutput.getOrderedUserDefinedOptions
fn get_ordered_user_defined_options(config: &BuildConfigurationValue) -> BTreeMap<String, String> {
    // ImmutableSortedMap.naturalOrder(); buildOrThrow fails on a duplicate key.
    let mut ans: BTreeMap<String, String> = BTreeMap::new();
    let mut put = |k: String, v: String| {
        if ans.insert(k.clone(), v).is_some() {
            panic!(
                "IllegalArgumentException: Multiple entries with same key: {}",
                k
            );
        }
    };
    // Starlark-defined options:
    for (label, value) in config.get_options().get_starlark_options() {
        put(label.to_string(), value.java_to_string());
    }
    // --define:
    let core_options = config
        .get_options()
        .get::<CoreOptions>()
        .expect("NullPointerException: CoreOptions");
    for (key, value) in core_options.get_normalized_command_line_build_variables() {
        put(format!("--define:{}", key), value);
    }
    ans
}

/// Data structure defining a `Fragment` for the purpose of creating user output.
// Java: com/google/devtools/build/lib/analysis/config/output/FragmentForOutput.java:32 FragmentForOutput
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FragmentForOutput {
    name: String,
    /// The names of the FragmentOptions, sorted. (The names, not `FragmentOptionsForOutput`s,
    /// because multiple fragments may use the same FragmentOptions.)
    fragment_options: Vec<String>,
}

impl FragmentForOutput {
    pub fn new(name: String, fragment_options: Vec<String>) -> Self {
        FragmentForOutput {
            name,
            fragment_options,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_fragment_options(&self) -> &[String] {
        &self.fragment_options
    }
}

/// Data structure defining a `FragmentOptions` for creating user output.
// Java: com/google/devtools/build/lib/analysis/config/output/FragmentOptionsForOutput.java:28 FragmentOptionsForOutput
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FragmentOptionsForOutput {
    name: String,
    options: BTreeMap<String, String>,
}

impl FragmentOptionsForOutput {
    pub fn new(name: String, options: BTreeMap<String, String>) -> Self {
        FragmentOptionsForOutput { name, options }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_options(&self) -> &BTreeMap<String, String> {
        &self.options
    }

    pub fn option_names(&self) -> impl Iterator<Item = &String> {
        self.options.keys()
    }

    pub fn get_option(&self, option_name: &str) -> Option<&String> {
        self.options.get(option_name)
    }
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

impl TextOutputFormatter {
    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:76 ConfigCommandOutputFormatter.TextOutputFormatter.getSuffix
    fn get_suffix(config: &ConfigurationForOutput) -> &'static str {
        if config.is_exec() {
            " (exec)"
        } else if !config.has_test_config() {
            " (test-trimmed)"
        } else {
            ""
        }
    }
}

impl ConfigCommandOutputFormatter for TextOutputFormatter {
    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:68 ConfigCommandOutputFormatter.TextOutputFormatter.writeConfigurationIDs
    fn write_configuration_ids(
        &self,
        writer: &mut String,
        configurations: &[ConfigurationForOutput],
    ) {
        writer.push_str("Available configurations:\n");
        for config in configurations {
            // printf("%s %s%s%n"): %n is the platform line separator.
            writer.push_str(&format!(
                "{} {}{}\n",
                config.get_config_hash(),
                config.get_mnemonic(),
                Self::get_suffix(config)
            ));
        }
    }

    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:86 ConfigCommandOutputFormatter.TextOutputFormatter.writeConfiguration
    fn write_configuration(&self, writer: &mut String, configuration: &ConfigurationForOutput) {
        writer.push_str(&format!(
            "BuildConfigurationValue {}:\n",
            configuration.get_config_hash()
        ));
        writer.push_str(&format!("Skyframe Key: {}\n", configuration.get_sky_key()));

        let mut fragments = String::new();
        for fragment in configuration.get_fragments() {
            fragments.push_str(fragment.get_name());
            fragments.push_str(": [");
            fragments.push_str(&fragment.get_fragment_options().join(","));
            fragments.push_str("], ");
        }

        writer.push_str(&format!("Fragments: {}\n", fragments));
        for fragment in configuration.get_fragment_options() {
            writer.push_str(&format!("FragmentOptions {} {{\n", fragment.get_name()));
            for (key, value) in fragment.get_options() {
                writer.push_str(&format!("  {}: {}\n", key, value));
            }
            writer.push_str("}\n");
        }
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
        gson_string(&mut json, "configuration-IDs");
        json.push_str(":[");
        for (i, config) in configurations.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            gson_string(&mut json, config.get_config_hash());
        }
        json.push_str("]}");
        writer.push_str(&json);
        writer.push('\n');
    }

    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:151 ConfigCommandOutputFormatter.JsonOutputFormatter.writeConfiguration
    fn write_configuration(&self, writer: &mut String, configuration: &ConfigurationForOutput) {
        gson_configuration(writer, configuration);
        writer.push('\n');
    }

    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:156 ConfigCommandOutputFormatter.JsonOutputFormatter.writeConfigurations
    fn write_configurations(&self, writer: &mut String, configurations: &[ConfigurationForOutput]) {
        writer.push('[');
        for (i, config) in configurations.iter().enumerate() {
            if i > 0 {
                writer.push(',');
            }
            gson_configuration(writer, config);
        }
        writer.push(']');
        writer.push('\n');
    }

    // Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:161 ConfigCommandOutputFormatter.JsonOutputFormatter.writeConfigurationDiff
    fn write_configuration_diff(&self, writer: &mut String, diff: &ConfigurationDiffForOutput) {
        // Gson's reflective adapter: the declared fields in order. Pair's fields are `first`
        // and `second`.
        writer.push('{');
        gson_string(writer, "configHash1");
        writer.push(':');
        gson_string(writer, &diff.config_hash1);
        writer.push(',');
        gson_string(writer, "configHash2");
        writer.push(':');
        gson_string(writer, &diff.config_hash2);
        writer.push(',');
        gson_string(writer, "fragmentsDiff");
        writer.push_str(":[");
        for (i, fragment_diff) in diff.fragments_diff.iter().enumerate() {
            if i > 0 {
                writer.push(',');
            }
            writer.push('{');
            gson_string(writer, "name");
            writer.push(':');
            gson_string(writer, &fragment_diff.name);
            writer.push(',');
            gson_string(writer, "optionsDiff");
            writer.push_str(":{");
            for (j, (key, (first, second))) in fragment_diff.options_diff.iter().enumerate() {
                if j > 0 {
                    writer.push(',');
                }
                gson_string(writer, key);
                writer.push_str(":{");
                gson_string(writer, "first");
                writer.push(':');
                gson_string(writer, first);
                writer.push(',');
                gson_string(writer, "second");
                writer.push(':');
                gson_string(writer, second);
                writer.push('}');
            }
            writer.push_str("}}");
        }
        writer.push_str("]}");
        writer.push('\n');
    }
}

/// `gson.toJson(configuration)`: the reflective adapter writes the declared fields in order
/// (`skyKey`, `configHash`, `mnemonic`, `isExec`, `fragments`, `fragmentOptions`).
fn gson_configuration(writer: &mut String, configuration: &ConfigurationForOutput) {
    writer.push('{');
    gson_string(writer, "skyKey");
    writer.push(':');
    gson_string(writer, &configuration.sky_key);
    writer.push(',');
    gson_string(writer, "configHash");
    writer.push(':');
    gson_string(writer, &configuration.config_hash);
    writer.push(',');
    gson_string(writer, "mnemonic");
    writer.push(':');
    gson_string(writer, &configuration.mnemonic);
    writer.push(',');
    gson_string(writer, "isExec");
    writer.push(':');
    writer.push_str(if configuration.is_exec {
        "true"
    } else {
        "false"
    });
    writer.push(',');
    gson_string(writer, "fragments");
    writer.push_str(":[");
    for (i, fragment) in configuration.fragments.iter().enumerate() {
        if i > 0 {
            writer.push(',');
        }
        writer.push('{');
        gson_string(writer, "name");
        writer.push(':');
        gson_string(writer, &fragment.name);
        writer.push(',');
        gson_string(writer, "fragmentOptions");
        writer.push_str(":[");
        for (j, name) in fragment.fragment_options.iter().enumerate() {
            if j > 0 {
                writer.push(',');
            }
            gson_string(writer, name);
        }
        writer.push_str("]}");
    }
    writer.push_str("],");
    gson_string(writer, "fragmentOptions");
    writer.push_str(":[");
    for (i, fragment_options) in configuration.fragment_options.iter().enumerate() {
        if i > 0 {
            writer.push(',');
        }
        writer.push('{');
        gson_string(writer, "name");
        writer.push(':');
        gson_string(writer, &fragment_options.name);
        writer.push(',');
        gson_string(writer, "options");
        writer.push_str(":{");
        for (j, (key, value)) in fragment_options.options.iter().enumerate() {
            if j > 0 {
                writer.push(',');
            }
            gson_string(writer, key);
            writer.push(':');
            gson_string(writer, value);
        }
        writer.push_str("}}");
    }
    writer.push_str("]}");
}

/// Gson's `JsonWriter.string` with HTML-safe escaping (the default of `new Gson()`).
// Java (Gson 2.10): com/google/gson/stream/JsonWriter.java JsonWriter.string
fn gson_string(writer: &mut String, value: &str) {
    writer.push('"');
    for c in value.chars() {
        match c {
            '"' => writer.push_str("\\\""),
            '\\' => writer.push_str("\\\\"),
            '\t' => writer.push_str("\\t"),
            '\u{8}' => writer.push_str("\\b"),
            '\n' => writer.push_str("\\n"),
            '\r' => writer.push_str("\\r"),
            '\u{c}' => writer.push_str("\\f"),
            // HTML_SAFE_REPLACEMENT_CHARS.
            '<' | '>' | '&' | '=' | '\'' => writer.push_str(&format!("\\u{:04x}", c as u32)),
            '\u{2028}' => writer.push_str("\\u2028"),
            '\u{2029}' => writer.push_str("\\u2029"),
            c if (c as u32) < 0x20 => writer.push_str(&format!("\\u{:04x}", c as u32)),
            c => writer.push(c),
        }
    }
    writer.push('"');
}

#[cfg(test)]
mod tests {
    //! `ConfigCommandTest` (src/test/java/com/google/devtools/build/lib/runtime/commands): every
    //! test runs `build --nobuild` and then `config` in one server, reading the configurations
    //! the build left in the graph.

    use super::*;

    const NEEDS_GRAPH: &str =
        "needs the Skyframe graph kept between commands (Blaze discards it, decision D2)";

    macro_rules! ignored {
        ($($name:ident),* $(,)?) => {
            $(
                #[test]
                #[ignore = "needs the Skyframe graph kept between commands (Blaze discards it, decision D2)"]
                fn $name() {
                    let _ = NEEDS_GRAPH;
                }
            )*
        };
    }

    ignored!(
        show_config_ids,
        show_single_config,
        show_single_config_hash_prefix,
        unknown_hash_prefix,
        show_all_configs,
        compare_configs,
        compare_configs_hash_prefix,
        starlark_flags_in_user_defined_fragment,
        define_flags_individually_listed_in_user_defined_fragment,
        conflicting_defines_last_wins,
    );

    #[test]
    fn gson_string_escapes_html_characters() {
        // Gson's default (HTML-safe) escaping: com/google/gson/stream/JsonWriter.java
        // HTML_SAFE_REPLACEMENT_CHARS.
        let mut s = String::new();
        gson_string(&mut s, "a=<b>&'c'\"\\\n");
        assert_eq!(
            s,
            "\"a\\u003d\\u003cb\\u003e\\u0026\\u0027c\\u0027\\\"\\\\\\n\""
        );
    }
}
