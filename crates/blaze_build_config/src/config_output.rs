//! Ports of `analysis/config/output/ConfigurationForOutput.java` (+ `FragmentForOutput`,
//! `FragmentOptionsForOutput`) and the text and JSON writers of
//! `runtime/commands/ConfigCommandOutputFormatter.java` (`bazel config` output).
//!
//! PORT: the `config` command itself (`ConfigCommand`) belongs in blaze_runtime; this module is
//! the output model and formats it prints.

use std::collections::BTreeMap;
use std::fmt::Write;

use blaze_optdefs::fragment::core_options::CoreOptions;

use crate::build_configuration_value::BuildConfigurationValue;
use crate::fragment::{FragmentRegistry, required_options};

/// `FragmentForOutput`: a fragment and the names of the FragmentOptions it requires.
// Java: com/google/devtools/build/lib/analysis/config/output/FragmentForOutput.java:31 FragmentForOutput
// Java: com/google/devtools/build/lib/analysis/config/output/FragmentForOutput.java:38 FragmentForOutput.<init>
// Java: com/google/devtools/build/lib/analysis/config/output/FragmentForOutput.java:53 FragmentForOutput.equals
// PORT: Java's equals and hashCode compare both fields, as the derived ones do.
// Java: com/google/devtools/build/lib/analysis/config/output/FragmentForOutput.java:61 FragmentForOutput.hashCode
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FragmentForOutput {
    // Java: com/google/devtools/build/lib/analysis/config/output/FragmentForOutput.java:43 FragmentForOutput.getName
    pub name: String,
    /// The names of the FragmentOptions, sorted.
    // Java: com/google/devtools/build/lib/analysis/config/output/FragmentForOutput.java:48 FragmentForOutput.getFragmentOptions
    pub fragment_options: Vec<String>,
}

/// `FragmentOptionsForOutput`: a FragmentOptions and its option values.
// Java: com/google/devtools/build/lib/analysis/config/output/FragmentOptionsForOutput.java:27 FragmentOptionsForOutput
// Java: com/google/devtools/build/lib/analysis/config/output/FragmentOptionsForOutput.java:31 FragmentOptionsForOutput.<init>
// PORT: Java's equals and hashCode compare both fields, as the derived ones do.
// Java: com/google/devtools/build/lib/analysis/config/output/FragmentOptionsForOutput.java:53 FragmentOptionsForOutput.equals
// Java: com/google/devtools/build/lib/analysis/config/output/FragmentOptionsForOutput.java:61 FragmentOptionsForOutput.hashCode
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FragmentOptionsForOutput {
    // Java: com/google/devtools/build/lib/analysis/config/output/FragmentOptionsForOutput.java:36 FragmentOptionsForOutput.getName
    pub name: String,
    /// Sorted by option name (`ImmutableSortedMap`, natural String order).
    // Java: com/google/devtools/build/lib/analysis/config/output/FragmentOptionsForOutput.java:40 FragmentOptionsForOutput.getOptions
    pub options: BTreeMap<String, String>,
}

impl FragmentOptionsForOutput {
    // Java: com/google/devtools/build/lib/analysis/config/output/FragmentOptionsForOutput.java:44 FragmentOptionsForOutput.optionNames
    pub fn option_names(&self) -> impl Iterator<Item = &String> {
        self.options.keys()
    }

    // Java: com/google/devtools/build/lib/analysis/config/output/FragmentOptionsForOutput.java:48 FragmentOptionsForOutput.getOption
    pub fn get_option(&self, option_name: &str) -> Option<&String> {
        self.options.get(option_name)
    }
}

/// `UserDefinedFragment.DESCRIPTIVE_NAME`.
pub const USER_DEFINED_DESCRIPTIVE_NAME: &str = "user-defined";

// Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:50 ConfigurationForOutput
/// Data structure defining a `BuildConfigurationValue` for the purpose of returning user output
/// about the configuration.
// Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:58 ConfigurationForOutput.<init>
#[derive(Clone, Debug)]
pub struct ConfigurationForOutput {
    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:73 ConfigurationForOutput.getSkyKey
    pub sky_key: String,
    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:77 ConfigurationForOutput.getConfigHash
    pub config_hash: String,
    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:81 ConfigurationForOutput.getMnemonic
    pub mnemonic: String,
    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:85 ConfigurationForOutput.isExec
    pub is_exec: bool,
    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:95 ConfigurationForOutput.getFragments
    pub fragments: Vec<FragmentForOutput>,
    /// The union of `FragmentOptionsForOutput` used by the Fragments associated with this
    /// configuration, sorted by FragmentOptionsForOutput name.
    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:103 ConfigurationForOutput.getFragmentOptions
    pub fragment_options: Vec<FragmentOptionsForOutput>,
}

// Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:122 ConfigurationForOutput.equals
impl PartialEq for ConfigurationForOutput {
    fn eq(&self, other: &Self) -> bool {
        other.sky_key == self.sky_key
            && other.config_hash == self.config_hash
            && other.fragments == self.fragments
            && other.fragment_options == self.fragment_options
    }
}

impl Eq for ConfigurationForOutput {}

// Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:133 ConfigurationForOutput.hashCode
impl std::hash::Hash for ConfigurationForOutput {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.sky_key.hash(state);
        self.config_hash.hash(state);
        self.fragments.hash(state);
        self.fragment_options.hash(state);
    }
}

impl ConfigurationForOutput {
    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:108 ConfigurationForOutput.fragment
    pub fn fragment(&self, fragment_name: &str) -> Option<&FragmentOptionsForOutput> {
        self.fragment_options
            .iter()
            .find(|fo| fo.name == fragment_name)
    }

    /// Sorted in natural String order (`toImmutableSortedSet(Ordering.natural())`).
    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:115 ConfigurationForOutput.fragmentOptionNames
    pub fn fragment_option_names(&self) -> std::collections::BTreeSet<&String> {
        self.fragment_options.iter().map(|fo| &fo.name).collect()
    }

    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:89 ConfigurationForOutput.hasTestConfig
    pub fn has_test_config(&self) -> bool {
        self.fragment_options
            .iter()
            .any(|f| f.name.contains("TestConfiguration"))
    }

    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:160 ConfigurationForOutput.getConfigurationForOutput
    /// Constructs a `ConfigurationForOutput` from the given input data. `fragment_defs` are the
    /// rule class provider's fragments (`ConfigCommand` passes all registered fragments).
    pub fn get_configuration_for_output(
        config: &BuildConfigurationValue,
        fragment_defs: &FragmentRegistry,
    ) -> ConfigurationForOutput {
        // ImmutableSortedSet ordered by name: a later add with an existing name is ignored.
        let mut fragments: BTreeMap<String, FragmentForOutput> = BTreeMap::new();
        for fragment_class in fragment_defs.get_all_fragments().iter() {
            let mut options: Vec<&'static str> = required_options(fragment_class)
                .iter()
                .map(|o| o.get_name())
                .collect();
            options.sort_by(|a, b| crate::build_options::java_compare(a, b));
            options.dedup();
            fragments
                .entry(fragment_class.get_name().to_string())
                .or_insert(FragmentForOutput {
                    name: fragment_class.get_name().to_string(),
                    fragment_options: options.into_iter().map(str::to_string).collect(),
                });
        }

        let mut fragment_options: BTreeMap<String, FragmentOptionsForOutput> = BTreeMap::new();
        for (class, instance) in config.get_options().get_native_options() {
            fragment_options
                .entry(class.get_name().to_string())
                .or_insert(FragmentOptionsForOutput {
                    name: class.get_name().to_string(),
                    options: Self::get_ordered_native_options(class.get_simple_name(), instance),
                });
        }
        fragment_options
            .entry(USER_DEFINED_DESCRIPTIVE_NAME.to_string())
            .or_insert(FragmentOptionsForOutput {
                name: USER_DEFINED_DESCRIPTIVE_NAME.to_string(),
                options: Self::get_ordered_user_defined_options(config),
            });

        ConfigurationForOutput {
            sky_key: format!("{:?}", config.get_key()),
            config_hash: config.checksum().to_string(),
            mnemonic: config.get_mnemonic().to_string(),
            is_exec: config.is_exec_configuration(),
            fragments: fragments.into_values().collect(),
            fragment_options: fragment_options.into_values().collect(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:215 ConfigurationForOutput.getOrderedNativeOptions
    /// Returns a `FragmentOptions`'s native option settings in canonical order.
    fn get_ordered_native_options(
        simple_name: &str,
        options: &dyn blaze_options::definition::OptionsBase,
    ) -> BTreeMap<String, String> {
        options
            .as_map()
            .into_iter()
            // While technically part of CoreOptions, --define is practically a user-definable
            // flag so we include it in the user-defined fragment for clarity.
            .filter(|(k, _)| !(simple_name == "CoreOptions" && k == "define"))
            .map(|(k, v)| (k, blaze_options::value::java_string_of(v.as_ref())))
            .collect()
    }

    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:234 ConfigurationForOutput.getOrderedUserDefinedOptions
    /// Returns a configuration's user-definable settings in canonical order.
    fn get_ordered_user_defined_options(
        config: &BuildConfigurationValue,
    ) -> BTreeMap<String, String> {
        let mut ans = BTreeMap::new();
        // Starlark-defined options:
        for (k, v) in config.get_options().get_starlark_options() {
            ans.insert(k.to_string(), v.java_to_string());
        }
        // --define:
        for (k, v) in config
            .get_options()
            .get::<CoreOptions>()
            .expect("CoreOptions")
            .get_normalized_command_line_build_variables()
        {
            ans.insert(format!("--define:{}", k), v);
        }
        ans
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:45 TextOutputFormatter.writeConfigurationIDs
pub fn text_write_configuration_ids<'a>(
    configurations: impl IntoIterator<Item = &'a ConfigurationForOutput>,
) -> String {
    let mut out = String::from("Available configurations:\n");
    for config in configurations {
        let suffix = if config.is_exec {
            " (exec)"
        } else if !config.has_test_config() {
            " (test-trimmed)"
        } else {
            ""
        };
        let _ = writeln!(out, "{} {}{}", config.config_hash, config.mnemonic, suffix);
    }
    out
}

// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:48 TextOutputFormatter.writeConfiguration
pub fn text_write_configuration(configuration: &ConfigurationForOutput) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "BuildConfigurationValue {}:",
        configuration.config_hash
    );
    let _ = writeln!(out, "Skyframe Key: {}", configuration.sky_key);
    let mut fragments = String::new();
    for fragment in &configuration.fragments {
        let _ = write!(
            fragments,
            "{}: [{}], ",
            fragment.name,
            fragment.fragment_options.join(",")
        );
    }
    let _ = writeln!(out, "Fragments: {}", fragments);
    for fragment in &configuration.fragment_options {
        let _ = writeln!(out, "FragmentOptions {} {{", fragment.name);
        for (k, v) in &fragment.options {
            let _ = writeln!(out, "  {}: {}", k, v);
        }
        let _ = writeln!(out, "}}");
    }
    out
}

/// Gson's `toJson` of a string with default (HTML-safe) escaping.
pub fn gson_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '<' | '>' | '&' | '=' | '\'' | '\u{2028}' | '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommandOutputFormatter.java:48 JsonOutputFormatter.writeConfiguration
/// `new Gson().toJson(configuration)`: fields in declaration order, compact.
pub fn json_write_configuration(c: &ConfigurationForOutput) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "{{\"skyKey\":{},\"configHash\":{},\"mnemonic\":{},\"isExec\":{},\"fragments\":[",
        gson_string(&c.sky_key),
        gson_string(&c.config_hash),
        gson_string(&c.mnemonic),
        c.is_exec
    );
    for (i, f) in c.fragments.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"name\":{},\"fragmentOptions\":[{}]}}",
            gson_string(&f.name),
            f.fragment_options
                .iter()
                .map(|o| gson_string(o))
                .collect::<Vec<_>>()
                .join(",")
        );
    }
    out.push_str("],\"fragmentOptions\":[");
    for (i, f) in c.fragment_options.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"name\":{},\"options\":{{{}}}}}",
            gson_string(&f.name),
            f.options
                .iter()
                .map(|(k, v)| format!("{}:{}", gson_string(k), gson_string(v)))
                .collect::<Vec<_>>()
                .join(",")
        );
    }
    out.push_str("]}\n");
    out
}
