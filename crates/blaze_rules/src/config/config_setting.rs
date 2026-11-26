//! Port of `rules/config/ConfigSetting.java`: the `config_setting` configured target factory.

use std::sync::{Arc, LazyLock};

use crate::config::config_feature_flag_provider::ConfigFeatureFlagProvider;
use blaze_analysis::config::build_option_details::BuildOptionDetails;
use blaze_analysis::config::build_setting_provider::BuildSettingProvider;
use blaze_analysis::config::config_matching_provider::{ConfigMatchingProvider, Diff, MatchResult};
use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::info::downcast_info;
use blaze_analysis::platform::constraint_collection::ConstraintCollection;
use blaze_analysis::platform::constraint_value_info::ConstraintValueInfo;
use blaze_analysis::providers::alias_provider::AliasProvider;
use blaze_analysis::providers::file_provider::{
    FileProvider, FilesToRunProvider, RunfilesProvider,
};
use blaze_analysis::rule::{RuleConfiguredTargetBuilder, RuleContext, RuleErrorConsumer};
use blaze_analysis::toolchains::toolchain_context::ToolchainContext;
use blaze_build_config::build_options::StarlarkOptionValue;
use blaze_build_config::fragment_options_class::TEST_OPTIONS;
use blaze_cmdline::label_constants::COMMAND_LINE_OPTION_PREFIX;
use blaze_cmdline::{Label, PackageIdentifier};
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_options::{OptionsParser, OptionsParsingException, Value as OptionValue};
use blaze_packages::attribute::{AttributeMap, NonconfigurableAttributeMapper};
use blaze_packages::native_rules::config_rule_classes::ConfigSettingRule;
use blaze_packages::starlark_defs::platform_common::CONSTRAINT_VALUE_INFO;
use blaze_packages::target::RuleOrMacroInstance;
use blaze_packages::types::{AttrValue, Type};
use indexmap::{IndexMap, IndexSet};

/// Flags we'd like to remove once there are no more repo references.
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:80 ConfigSetting.DEPRECATED_PRE_PLATFORMS_FLAGS
const DEPRECATED_PRE_PLATFORMS_FLAGS: &[&str] = &["cpu", "host_cpu", "crosstool_top"];

/// User error when value settings can't be properly parsed.
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:381 ConfigSetting.PARSE_ERROR_MESSAGE
const PARSE_ERROR_MESSAGE: &str = "error while parsing configuration settings: ";

/// The settings this `config_setting` expects.
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:90 ConfigSetting.Settings
struct Settings {
    /// native flags that match this rule (defined in Bazel code)
    native_flag_settings: Vec<(String, String)>,
    /// user-defined flags that match this rule (defined in Starlark)
    user_defined_flag_settings: IndexMap<Label, String>,
    /// the current platform's expected `constraint_value`s
    constraint_value_settings: Vec<Label>,
}

/// The `config_setting` factory (Java `ConfigSetting`).
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:77 ConfigSetting
pub struct ConfigSetting;

/// Java `PlatformProviderUtils.constraintValue(dep)`.
fn constraint_value(dep: &ConfiguredTarget) -> Option<ConstraintValueInfo> {
    let info = dep.get(&CONSTRAINT_VALUE_INFO.key())?;
    downcast_info::<ConstraintValueInfo>(&info).cloned()
}

fn string_dict(attributes: &dyn AttributeMap, name: &str) -> IndexMap<String, String> {
    match attributes.get(name, Type::StringDict) {
        Some(AttrValue::StringDict(d)) => d,
        _ => IndexMap::new(),
    }
}

impl RuleConfiguredTargetFactory for ConfigSetting {
    // Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:96 ConfigSetting.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let rule = rule_context.get_rule().clone();
        let attributes = NonconfigurableAttributeMapper::of(&*rule as &dyn RuleOrMacroInstance);

        let likely_label_invalid_setting =
            string_dict(&attributes, ConfigSettingRule::SETTINGS_ATTRIBUTE)
                .keys()
                .find(|s| s.starts_with('@') || s.starts_with("//") || s.starts_with(':'))
                .cloned();
        if let Some(setting) = likely_label_invalid_setting {
            rule_context.attribute_error(
                ConfigSettingRule::SETTINGS_ATTRIBUTE,
                &format!(
                    "'{}' is not a valid setting name, but appears to be a label. Did you mean to place it in {} instead?",
                    setting,
                    ConfigSettingRule::FLAG_SETTINGS_ATTRIBUTE
                ),
            );
            return Ok(None);
        }

        let settings = get_settings(rule_context, &attributes);
        // Check that this config_setting contains at least one of {values, define_values,
        // constraint_values}
        if !values_are_set(&settings, rule_context) {
            return Ok(None);
        }

        let option_details =
            BuildOptionDetails::for_options(rule_context.get_configuration().get_options());
        let native_flags_result = diff_native_flags(
            &settings.native_flag_settings,
            &option_details,
            rule_context,
        );
        let user_defined_flags = UserDefinedFlagMatch::from_attribute_value_and_prerequisites(
            &settings.user_defined_flag_settings,
            &option_details,
            rule_context,
        );
        let constraint_values_result = diff_constraint_values(rule_context);

        if rule_context.has_errors() {
            return Ok(None);
        }

        let config_matcher = ConfigMatchingProvider::create(
            rule_context.get_label().clone(),
            settings.native_flag_settings.clone(),
            user_defined_flags.specified_flag_values.clone(),
            get_specified_constraint_values(rule_context),
            [
                user_defined_flags.result.clone(),
                native_flags_result,
                constraint_values_result,
            ]
            .into_iter()
            .reduce(MatchResult::combine)
            .expect("three results"),
        );

        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder
            .add_provider(RunfilesProvider::empty())
            .add_provider_arc(FileProvider::empty())
            .add_provider_arc(FilesToRunProvider::empty())
            .add_provider(config_matcher);
        Ok(builder.build()?)
    }

    // Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:261 ConfigSetting.addRuleImplSpecificRequiredConfigFragments
    fn add_rule_impl_specific_required_config_fragments(
        &self,
        required_fragments: &mut blaze_analysis::providers::required_config_fragments_provider::RequiredConfigFragmentsProviderBuilder,
        attributes: &blaze_analysis::config::configured_attribute_mapper::ConfiguredAttributeMapper,
        configuration: &blaze_build_config::build_configuration_value::BuildConfigurationValue,
    ) {
        // values
        for (option_name, value) in string_dict(attributes, ConfigSettingRule::SETTINGS_ATTRIBUTE) {
            if option_name == "define" {
                // value.indexOf('='), in chars.
                let equals_index = value.chars().position(|c| c == '=');
                match equals_index {
                    Some(i) if i > 0 => {
                        required_fragments.add_define(&value.chars().take(i).collect::<String>())
                    }
                    _ => required_fragments.add_define(&value),
                };
            } else if let Some(options_class) =
                BuildOptionDetails::for_options(configuration.get_options())
                    .get_option_class(&option_name)
            {
                required_fragments.add_options_class(
                    blaze_analysis::required_fragments_util::get_simple_name_with_outer(
                        options_class.get_name(),
                    ),
                );
            }
        }

        // define_values
        required_fragments.add_defines(
            string_dict(attributes, ConfigSettingRule::DEFINE_SETTINGS_ATTRIBUTE)
                .keys()
                .map(|s| s.as_str()),
        );

        // flag_values
        if let Some(AttrValue::LabelKeyedStringDict(d)) = attributes.get(
            ConfigSettingRule::FLAG_SETTINGS_ATTRIBUTE,
            Type::LabelKeyedStringDict,
        ) {
            required_fragments.add_starlark_options(d.into_keys());
        }
    }
}

/// Returns this `config_setting`'s expected settings.
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:149 ConfigSetting.getSettings
fn get_settings(rule_context: &mut RuleContext, attributes: &dyn AttributeMap) -> Settings {
    // Collect expected flags from "values" and "define_values" attributes.
    let mut native_value_attributes: Vec<(String, String)> =
        string_dict(attributes, ConfigSettingRule::SETTINGS_ATTRIBUTE)
            .into_iter()
            .collect();
    native_value_attributes.extend(
        string_dict(attributes, ConfigSettingRule::DEFINE_SETTINGS_ATTRIBUTE)
            .into_iter()
            .map(|(k, v)| ("define".to_owned(), format!("{}={}", k, v))),
    );

    // Find --flag_alias=foo=//bar settings. When these are set, "--foo" isn't a native flag but
    // an alias to "//bar". Since Bazel's options parsing replaces "--foo" with "//bar", we want
    // to do the same here to match the parsed options.
    let command_line_flag_aliases: IndexMap<String, Label> = rule_context
        .get_configuration()
        .get_options()
        .get::<CoreOptions>()
        .expect("CoreOptions")
        .get_command_line_flag_aliases()
        .into_iter()
        .collect();

    // Partition expected "--foo" settings (native flag style) by whether they're flag aliases.
    let (aliased, not_aliased): (Vec<_>, Vec<_>) = native_value_attributes
        .into_iter()
        .partition(|(k, _)| command_line_flag_aliases.contains_key(k));

    // Collect actual native flags that aren't flag aliases.
    // Java: MultimapBuilder.linkedHashKeys().arrayListValues(): values grouped by first key
    // occurrence.
    let mut grouped: IndexMap<String, Vec<String>> = IndexMap::new();
    for (k, v) in not_aliased {
        grouped.entry(k).or_default().push(v);
    }
    let native_flag_settings: Vec<(String, String)> = grouped
        .into_iter()
        .flat_map(|(k, vs)| vs.into_iter().map(move |v| (k.clone(), v)))
        .collect();

    // Collect user-defined flags.
    let mut user_defined_flag_settings: IndexMap<Label, String> = match attributes.get(
        ConfigSettingRule::FLAG_SETTINGS_ATTRIBUTE,
        Type::LabelKeyedStringDict,
    ) {
        Some(AttrValue::LabelKeyedStringDict(d)) => d,
        _ => IndexMap::new(),
    };
    for (flag_alias_key, alias_value) in aliased {
        let user_defined_flag = command_line_flag_aliases[&flag_alias_key].clone();
        if let Some(flag_settings_attribute_value) =
            user_defined_flag_settings.get(&user_defined_flag)
        {
            if *flag_settings_attribute_value != alias_value {
                rule_context.rule_error(&format!(
                    "\nConflicting flag value expectations:\n - {} has '{} = {{\"{}\": \"{}\"}}'.\n - Because --{} is a flag alias for --{}, this translates to '{} = {{\"{}: \"{}\"}}'.\n - {} also has '{} = {{\"{}\": \"{}\"}}', which matches a different value.\n\nEither remove one of these settings or ensure they match the same value.\n\n",
                    rule_context.get_label(),
                    ConfigSettingRule::SETTINGS_ATTRIBUTE,
                    flag_alias_key,
                    alias_value,
                    flag_alias_key,
                    user_defined_flag,
                    ConfigSettingRule::FLAG_SETTINGS_ATTRIBUTE,
                    user_defined_flag,
                    alias_value,
                    rule_context.get_label(),
                    ConfigSettingRule::FLAG_SETTINGS_ATTRIBUTE,
                    user_defined_flag,
                    flag_settings_attribute_value
                ));
            }
        }
        user_defined_flag_settings.insert(user_defined_flag, alias_value);
    }

    // Collect platform constraint settings.
    let constraint_value_settings = match attributes.get(
        ConfigSettingRule::CONSTRAINT_VALUES_ATTRIBUTE,
        Type::LabelList,
    ) {
        Some(AttrValue::LabelList(l)) => l,
        _ => Vec::new(),
    };

    Settings {
        native_flag_settings,
        user_defined_flag_settings,
        constraint_value_settings,
    }
}

/// Returns whether all `constraint_values` settings are valid and match this configuration.
/// May generate rule errors on bad settings (e.g. wrong target types).
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:285 ConfigSetting.diffConstraintValues
fn diff_constraint_values(rule_context: &mut RuleContext) -> MatchResult {
    let mut constraint_values: Vec<ConstraintValueInfo> = Vec::new();
    for dep in rule_context.get_prerequisites(ConfigSettingRule::CONSTRAINT_VALUES_ATTRIBUTE) {
        match constraint_value(&dep) {
            None => rule_context.attribute_error(
                ConfigSettingRule::CONSTRAINT_VALUES_ATTRIBUTE,
                &format!("{} is not a constraint_value", dep.get_label()),
            ),
            Some(c) => constraint_values.push(c),
        }
    }
    if rule_context.has_errors() {
        return MatchResult::already_reported_no_match();
    }

    if constraint_values.is_empty() {
        return MatchResult::Match;
    }

    // The set of constraint_values in a config_setting should never contain multiple
    // constraint_values that map to the same constraint_setting. This method checks if there
    // are duplicates and records an error if so.
    if let Err(e) = ConstraintCollection::validate_constraints(&constraint_values) {
        rule_context.rule_error(e.get_message());
        return MatchResult::already_reported_no_match();
    }

    let Some(toolchain_context) = rule_context.get_toolchain_context() else {
        rule_context.attribute_error(
            ConfigSettingRule::CONSTRAINT_VALUES_ATTRIBUTE,
            "No target platform is present",
        );
        return MatchResult::already_reported_no_match();
    };

    let target_platform_constraints = ToolchainContext::target_platform(toolchain_context)
        .constraints()
        .clone();
    if target_platform_constraints.contains_all(&constraint_values) {
        return MatchResult::Match;
    }

    let mut diffs = Vec::new();
    for rule_constraint_value in &constraint_values {
        let setting = rule_constraint_value.constraint();
        let target_platform_value = target_platform_constraints.get(setting);
        if target_platform_value.as_ref() != Some(rule_constraint_value) {
            diffs.push(Diff::new(
                setting.label().clone(),
                match &target_platform_value {
                    Some(v) => v.label().get_name().to_owned(),
                    None => "<unset>".to_owned(),
                },
                rule_constraint_value.label().get_name().to_owned(),
            ));
        }
    }
    MatchResult::NoMatch(diffs)
}

/// Returns whether the given label falls under the `//tools` package (including subpackages)
/// of the tools repository.
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:357 ConfigSetting.isUnderToolsPackage
pub fn is_under_tools_package(
    label: &Label,
    tools_repository: &blaze_cmdline::RepositoryName,
) -> bool {
    let package_id = label.get_package_identifier();
    if package_id.get_repository() != tools_repository {
        return false;
    }
    let fragment = package_id.get_package_fragment().get_path_string();
    if fragment.is_empty() {
        // Top-level package (//).
        return false;
    }
    fragment.split('/').next() == Some("tools")
}

/// Check to make sure this config_setting contains and sets least one of {values,
/// define_values, flag_value or constraint_values}.
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:387 ConfigSetting.valuesAreSet
fn values_are_set(settings: &Settings, errors: &mut RuleContext) -> bool {
    if settings.native_flag_settings.is_empty()
        && settings.user_defined_flag_settings.is_empty()
        && settings.constraint_value_settings.is_empty()
    {
        errors.rule_error(&format!(
            "Either {}, {} or {} must be specified and non-empty",
            ConfigSettingRule::SETTINGS_ATTRIBUTE,
            ConfigSettingRule::FLAG_SETTINGS_ATTRIBUTE,
            ConfigSettingRule::CONSTRAINT_VALUES_ATTRIBUTE
        ));
        return false;
    }
    true
}

/// Given a list of [flagName, flagValue] pairs for native Blaze flags, returns whether
/// flagName == flagValue for every item in the list under this configuration.
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:405 ConfigSetting.diffNativeFlags
fn diff_native_flags(
    expected_settings: &[(String, String)],
    options: &BuildOptionDetails,
    rule_context: &mut RuleContext,
) -> MatchResult {
    // Rather than returning fast when we find a mismatch, continue looking at the other flags to
    // check they're indeed valid flag specifications.
    expected_settings
        .iter()
        .map(|(option_name, expected_raw_value)| {
            check_option_value(options, rule_context, option_name, expected_raw_value)
        })
        .fold(MatchResult::Match, MatchResult::combine)
}

/// Returns whether the option is set to the expected value in the configuration.
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:422 ConfigSetting.checkOptionValue
fn check_option_value(
    options: &BuildOptionDetails,
    rule_context: &mut RuleContext,
    option_name: &str,
    expected_raw_value: &str,
) -> MatchResult {
    let disabled_select_options = rule_context
        .get_configuration()
        .get_options()
        .get::<CoreOptions>()
        .expect("CoreOptions")
        .disabled_select_options
        .clone();
    if disabled_select_options.iter().any(|o| o == option_name)
        || options.is_non_configurable(option_name)
    {
        let mut message = format!(
            "{}select() on '{}' is not allowed.",
            PARSE_ERROR_MESSAGE, option_name
        );
        if DEPRECATED_PRE_PLATFORMS_FLAGS.contains(&option_name) {
            message += " Use platform constraints instead: https://bazel.build/docs/configurable-attributes#platforms.";
        }
        rule_context.attribute_error(ConfigSettingRule::SETTINGS_ATTRIBUTE, &message);
        return MatchResult::already_reported_no_match();
    }

    if DEPRECATED_PRE_PLATFORMS_FLAGS.contains(&option_name)
        && rule_context.get_label().get_repository().is_main()
    {
        rule_context.rule_warning(&format!(
            "select() on {} is deprecated. Use platform constraints instead: https://bazel.build/docs/configurable-attributes#platforms.",
            option_name
        ));
    }
    // If option --foo has oldName --old_foo and the config_setting references --old_foo, get the
    // canonical name, which is where the actual option is stored.
    let canonical_option_name = options.get_canonical_name(option_name).to_owned();
    let Some(option_class) = options.get_option_class(&canonical_option_name) else {
        if is_test_option(&canonical_option_name) {
            // If TestOptions isn't present then they were trimmed, so any test options set are
            // considered unset by default.
            return MatchResult::no_match(Diff::new(
                to_option_label(option_name),
                "<test option trimmed>",
                expected_raw_value,
            ));
        }

        // Report the unknown option as an error.
        rule_context.attribute_error(
            ConfigSettingRule::SETTINGS_ATTRIBUTE,
            &format!("{}unknown option: '{}'", PARSE_ERROR_MESSAGE, option_name),
        );
        return MatchResult::already_reported_no_match();
    };

    let parsed = (|| {
        let mut parser = OptionsParser::builder()
            .options_classes(&[option_class.options_class()])
            .build()
            .map_err(|e| e.to_string())?;
        parser
            .parse(&[&format!("--{}={}", option_name, expected_raw_value)])
            .map_err(|e| e.get_message().to_owned())?;
        Ok::<_, String>(parser)
    })();
    let parser = match parsed {
        Ok(p) => p,
        Err(message) => {
            rule_context.attribute_error(
                ConfigSettingRule::SETTINGS_ATTRIBUTE,
                &format!("{}{}", PARSE_ERROR_MESSAGE, message),
            );
            return MatchResult::already_reported_no_match();
        }
    };

    let parsed_options = parser
        .get_options_dyn(option_class.options_class())
        .expect("parsed options class");
    let expected_parsed_value = option_class
        .get_option_definitions()
        .find(|d| d.get_option_name() == canonical_option_name)
        .and_then(|d| parsed_options.get_value_from_definition(d));
    option_matches(options, &canonical_option_name, expected_parsed_value)
}

/// Special hard-coded check to allow config_setting to handle test options even when the test
/// configuration has been trimmed.
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:483 ConfigSetting.isTestOption
fn is_test_option(option_name: &str) -> bool {
    TEST_OPTIONS
        .get_option_definitions()
        .any(|d| d.get_option_name() == option_name)
}

fn java_string_of(v: &OptionValue) -> String {
    v.java_to_string()
}

/// For single-value options, returns whether the option's value matches the expected value.
/// For multi-value List options returns whether any of the option's values matches the
/// expected value(s). For multi-value Map options, returns whether the last instance with the
/// same key as the expected key has the same value.
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:505 ConfigSetting.optionMatches
fn option_matches(
    options: &BuildOptionDetails,
    option_name: &str,
    expected_value: Option<OptionValue>,
) -> MatchResult {
    let Some(actual_value) = options.get_option_value(option_name) else {
        return match expected_value {
            None => MatchResult::Match,
            Some(expected) => MatchResult::no_match(Diff::new(
                to_option_label(option_name),
                "null",
                java_string_of(&expected),
            )),
        };
    };
    if !options.allows_multiple_values(option_name) {
        // Single-value case:
        return if Some(actual_value) == expected_value.as_ref() {
            MatchResult::Match
        } else {
            MatchResult::no_match(Diff::new(
                to_option_label(option_name),
                java_string_of(actual_value),
                expected_value
                    .as_ref()
                    .map(java_string_of)
                    .unwrap_or_else(|| "null".to_owned()),
            ))
        };
    }

    // Multi-value case:
    let actual_list = actual_value
        .as_list()
        .expect("IllegalStateException: allowMultiple option value is a List");
    let expected_value = expected_value.expect("IllegalStateException: expected value");
    let expected_list = expected_value
        .as_list()
        .expect("IllegalStateException: expected value is a List");

    if actual_list.is_empty() || expected_list.is_empty() {
        return if actual_list.is_empty() && expected_list.is_empty() {
            MatchResult::Match
        } else {
            MatchResult::no_match(Diff::new(
                to_option_label(option_name),
                if actual_list.is_empty() {
                    "<empty>".to_owned()
                } else {
                    actual_value.java_to_string()
                },
                if expected_list.is_empty() {
                    "<empty>".to_owned()
                } else {
                    expected_value.java_to_string()
                },
            ))
        };
    }

    // Multi-value map:
    if actual_list[0].downcast_ref::<(String, String)>().is_some() {
        // The config_setting's expected value *must* be a single map entry.
        blaze_util::check_argument!(expected_list.len() == 1);
        let expected_entry = expected_list[0]
            .downcast_ref::<(String, String)>()
            .expect("map entry")
            .clone();
        for elem in actual_list.iter().rev() {
            let actual_entry = elem.downcast_ref::<(String, String)>().expect("map entry");
            if actual_entry.0 == expected_entry.0 {
                // Found a key match!
                return if actual_entry.1 == expected_entry.1 {
                    MatchResult::Match
                } else {
                    MatchResult::no_match(Diff::new(
                        to_option_label(option_name),
                        format!("{}={}", actual_entry.0, actual_entry.1),
                        format!("{}={}", expected_entry.0, expected_entry.1),
                    ))
                };
            }
        }
        return MatchResult::no_match(Diff::new(
            to_option_label(option_name),
            format!("<key {} not found>", expected_entry.0),
            format!("{}={}", expected_entry.0, expected_entry.1),
        ));
    }

    // Multi-value list:
    if expected_list.iter().all(|e| actual_list.contains(e)) {
        MatchResult::Match
    } else {
        MatchResult::no_match(Diff::new(
            to_option_label(option_name),
            actual_value.java_to_string(),
            expected_value.java_to_string(),
        ))
    }
}

// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:582 ConfigSetting.COMMAND_LINE_OPTIONS_PACKAGE
static COMMAND_LINE_OPTIONS_PACKAGE: LazyLock<PackageIdentifier> = LazyLock::new(|| {
    PackageIdentifier::create_in_main_repo_str(COMMAND_LINE_OPTION_PREFIX.trim_end_matches(':'))
});

// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:586 ConfigSetting.toOptionLabel
fn to_option_label(option_name: &str) -> Label {
    Label::create_unvalidated(COMMAND_LINE_OPTIONS_PACKAGE.clone(), option_name)
}

/// Java `ConfigSetting.UserDefinedFlagMatch`.
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:590 ConfigSetting.UserDefinedFlagMatch
struct UserDefinedFlagMatch {
    result: MatchResult,
    /// The specified flag values, with aliases converted to their original targets' labels.
    specified_flag_values: IndexMap<Label, String>,
}

/// Java `BuildSettingProvider.getDefaultValue()` / `optionDetails.getOptionValue(label)` in the
/// `BuildOptions` form, for comparison with a converted expected value.
fn build_setting_value_of(default: Option<&AttrValue>) -> Option<StarlarkOptionValue> {
    default.map(blaze_analysis::config::starlark_transition::attr_value_to_starlark_option)
}

/// Java `BUILD_SETTING_CONVERTERS.get(type).convert(value, /* conversionContext= */ null)`: the
/// converted object in its `BuildOptions` form (the shared port of the converters is
/// `blaze_build_config::core_option_converters`).
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:697 ConfigSetting.UserDefinedFlagMatch.fromAttributeValueAndPrerequisites
fn convert_build_setting_value(
    type_: Type,
    value: &str,
) -> Result<StarlarkOptionValue, OptionsParsingException> {
    let converter = blaze_analysis::config::starlark_transition::build_setting_converter(type_)
        .unwrap_or_else(|| {
            panic!(
                "NullPointerException: BUILD_SETTING_CONVERTERS has no converter for type {}",
                type_
            )
        });
    converter.convert(value, /* conversion_context= */ None)
}

/// Java `String.valueOf` of a nullable string (`"null"` for null).
fn java_string(s: Option<&str>) -> String {
    s.unwrap_or("null").to_owned()
}

impl UserDefinedFlagMatch {
    /// The `flag_values` attribute takes a label->string dictionary of feature flags and
    /// starlark-defined settings to their values in string form.
    // Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:631 ConfigSetting.UserDefinedFlagMatch.fromAttributeValueAndPrerequisites
    fn from_attribute_value_and_prerequisites(
        attribute_value: &IndexMap<Label, String>,
        option_details: &BuildOptionDetails,
        rule_context: &mut RuleContext,
    ) -> UserDefinedFlagMatch {
        let mut specified_flag_values: IndexMap<Label, String> = IndexMap::new();

        let mut diffs: Vec<Diff> = Vec::new();
        // Only configuration-dependent errors should be deferred.
        let mut deferred_errors: Vec<String> = Vec::new();
        let mut found_duplicate = false;

        // Get the actual targets the 'flag_values' keys reference.
        // PORT: Java's LinkedHashSet of TransitiveInfoCollections dedupes by identity.
        let mut prerequisites: Vec<ConfiguredTarget> = Vec::new();
        for attr in [
            ConfigSettingRule::FLAG_SETTINGS_ATTRIBUTE,
            ConfigSettingRule::FLAG_ALIAS_SETTINGS_ATTRIBUTE,
        ] {
            for p in rule_context.get_prerequisites(attr) {
                if !prerequisites.iter().any(|q| same_configured_target(q, &p)) {
                    prerequisites.push(p);
                }
            }
        }

        for target in &prerequisites {
            let actual_label = target.get_label();
            let specified_label = AliasProvider::get_dependency_label(target);
            let specified_value = maybe_canonicalize_label(
                attribute_value.get(&specified_label).cloned(),
                target,
                rule_context,
            );
            if specified_flag_values.contains_key(&actual_label) {
                found_duplicate = true;
            }
            specified_flag_values.insert(
                actual_label.clone(),
                specified_value.clone().unwrap_or_default(),
            );

            // Java: target.satisfies(ConfigFeatureFlagProvider.REQUIRE_CONFIG_FEATURE_FLAG_PROVIDER)
            if let Some(info) = ConfigFeatureFlagProvider::from_target(target) {
                // config_feature_flag
                let provider = ConfigFeatureFlagProvider::of(&info);
                // PORT: Java passes the nullable specifiedValue; Rust's predicate takes a str.
                let specified = specified_value.clone().unwrap_or_default();
                if !provider.is_valid_value(&specified) {
                    // This is a configuration-independent error on the attributes of
                    // config_setting. So, is appropriate to error immediately.
                    rule_context.attribute_error(
                        ConfigSettingRule::FLAG_SETTINGS_ATTRIBUTE,
                        &format!(
                            "error while parsing user-defined configuration values: '{}' is not a valid value for '{}'",
                            java_string(specified_value.as_deref()),
                            specified_label
                        ),
                    );
                    continue;
                }
                if let Some(error) = provider.get_error().filter(|e| !e.is_empty()) {
                    deferred_errors.push(error.to_owned());
                    continue;
                } else if provider.get_flag_value() != specified_value.as_deref() {
                    diffs.push(Diff::new(
                        specified_label.clone(),
                        java_string(specified_value.as_deref()),
                        java_string(provider.get_flag_value()),
                    ));
                }
            } else if let Some(provider) = target.get_provider::<BuildSettingProvider>() {
                // build setting
                let configuration_value: Option<StarlarkOptionValue> =
                    match option_details.get_starlark_option_value(provider.get_label()) {
                        Some(v) => Some(v.clone()),
                        None => build_setting_value_of(provider.get_default_value()),
                    };
                let specified = specified_value.clone().unwrap_or_default();

                // We don't need to supply a base package or repo mapping for the conversion
                // here, because `specifiedValue` is already canonicalized.
                let converted_specified_value = match convert_build_setting_value(
                    provider.get_type(),
                    &specified,
                ) {
                    Ok(v) => v,
                    Err(_) => {
                        // This is a configuration-independent error on the attributes of
                        // config_setting. So, is appropriate to error immediately.
                        rule_context.attribute_error(
                                ConfigSettingRule::FLAG_SETTINGS_ATTRIBUTE,
                                &format!(
                                    "error while parsing user-defined configuration values: '{}' cannot be converted to {} type {}",
                                    specified,
                                    specified_label,
                                    provider.get_type()
                                ),
                            );
                        continue;
                    }
                };

                let configuration_value =
                    configuration_value.expect("NullPointerException: build setting value");
                // Java: configurationValue instanceof List || configurationValue instanceof Set
                if let StarlarkOptionValue::List(configuration_list) = &configuration_value {
                    // If the build_setting is a list or set, it's either an allow-multiple
                    // string-typed build setting, a string_list-typed build setting or a
                    // string_set-typed build setting. We use the same semantics as for
                    // multi-value native flags: if *any* entry in the list matches the
                    // config_setting's expected entry, it's a match.

                    // If this is an allow-multiple build setting, the converter will have
                    // converted the config settings value to a singular object, if it's a
                    // string_list or string_set build setting the converter will have converted
                    // it to a list or set respectively.
                    let specified_value_as_iterable: Vec<StarlarkOptionValue> =
                        if provider.allows_multiple() {
                            vec![converted_specified_value.clone()]
                        } else {
                            match &converted_specified_value {
                                StarlarkOptionValue::List(l) => l.items().to_vec(),
                                other => panic!(
                                    "ClassCastException: {} cannot be cast to Iterable",
                                    other.java_to_string()
                                ),
                            }
                        };
                    if specified_value_as_iterable.len() != 1 {
                        rule_context.attribute_error(
                            ConfigSettingRule::FLAG_SETTINGS_ATTRIBUTE,
                            &format!(
                                "\"{}\" not a valid value for flag {}. Only single, exact values are allowed. If you want to match multiple values, consider Skylib's selects.config_setting_group",
                                specified, specified_label
                            ),
                        );
                    } else if !configuration_list
                        .iter()
                        .any(|entry| specified_value_as_iterable[0].java_equals(entry))
                    {
                        diffs.push(Diff::new(
                            specified_label.clone(),
                            converted_specified_value.java_to_string(),
                            configuration_value.java_to_string(),
                        ));
                    }
                } else if !configuration_value.java_equals(&converted_specified_value) {
                    diffs.push(Diff::new(
                        specified_label.clone(),
                        converted_specified_value.java_to_string(),
                        configuration_value.java_to_string(),
                    ));
                }
            } else {
                // This should be configuration-independent error on the attributes of
                // config_setting. So, is appropriate to error immediately.
                rule_context.attribute_error(
                    ConfigSettingRule::FLAG_SETTINGS_ATTRIBUTE,
                    &format!(
                        "error while parsing user-defined configuration values: {} keys must be build settings or feature flags and {} is not",
                        ConfigSettingRule::FLAG_SETTINGS_ATTRIBUTE,
                        specified_label
                    ),
                );
            }
        }

        if found_duplicate {
            // Java: collectAliases(prerequisites)
            let mut aliases: IndexMap<Label, Vec<Label>> = IndexMap::new();
            for target in &prerequisites {
                aliases
                    .entry(target.get_label())
                    .or_default()
                    .push(AliasProvider::get_dependency_label(target));
            }
            for (actual_label, alias_list) in &aliases {
                if alias_list.len() > 1 {
                    rule_context.attribute_error(
                        ConfigSettingRule::FLAG_SETTINGS_ATTRIBUTE,
                        &format!(
                            "flag '{}' referenced multiple times as ['{}']",
                            actual_label,
                            alias_list
                                .iter()
                                .map(|l| l.to_string())
                                .collect::<Vec<_>>()
                                .join("', '")
                        ),
                    );
                }
            }
        }
        let match_result = if !deferred_errors.is_empty() {
            MatchResult::InError(deferred_errors)
        } else if rule_context.has_errors() {
            MatchResult::already_reported_no_match()
        } else if !diffs.is_empty() {
            MatchResult::NoMatch(diffs)
        } else {
            MatchResult::Match
        };
        UserDefinedFlagMatch {
            result: match_result,
            specified_flag_values,
        }
    }
}

/// Given a `flag_values = {"//ref:to:flagTarget": "expectedValue"}` pair, if expectedValue is
/// a relative label and flagTarget's value(s) are label-typed, returns an absolute form of the
/// label under the config_setting's package. Else returns the original value unchanged.
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:808 ConfigSetting.maybeCanonicalizeLabel
fn maybe_canonicalize_label(
    expected_value: Option<String>,
    flag_target: &ConfiguredTarget,
    rule_context: &RuleContext,
) -> Option<String> {
    let Some(provider) = flag_target.get_provider::<BuildSettingProvider>() else {
        return expected_value;
    };
    if !provider.get_type().is_label_type() {
        return expected_value;
    }
    let expected = expected_value?;
    match Label::parse_with_package_context(&expected, &rule_context.get_package_context()) {
        Ok(l) => Some(l.get_unambiguous_canonical_form()),
        // Swallow this: the subsequent type conversion already checks for this.
        Err(_) => Some(expected),
    }
}

/// Returns the labels of all prerequisite constraint values for this rule, resolving aliases to
/// their concrete targets (needed for specialization checking in select() statements).
// Java: com/google/devtools/build/lib/rules/config/ConfigSetting.java:836 ConfigSetting.getSpecifiedConstraintValues
fn get_specified_constraint_values(rule_context: &RuleContext) -> IndexSet<Label> {
    let mut result = IndexSet::new();
    for dep in rule_context.get_prerequisites(ConfigSettingRule::CONSTRAINT_VALUES_ATTRIBUTE) {
        result.insert(dep.get_label());
        if let Some(constraint_value) = constraint_value(&dep) {
            result.extend(constraint_value.constraint().refinement_chain());
        }
    }
    result
}

/// Java identity of two `TransitiveInfoCollection`s (`LinkedHashSet` membership).
///
/// PORT: a configured target is unique per (original label, configuration) in one build, so
/// that pair stands for the object identity.
fn same_configured_target(a: &ConfiguredTarget, b: &ConfiguredTarget) -> bool {
    a.get_original_label() == b.get_original_label()
        && a.get_configuration_key() == b.get_configuration_key()
}

/// Registers the factory.
pub fn factory() -> Arc<dyn RuleConfiguredTargetFactory> {
    Arc::new(ConfigSetting)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Java: com/google/devtools/build/lib/rules/config/ConfigSettingTest.java isUnderToolsPackage
    #[test]
    fn is_under_tools_package_checks() {
        let tools = blaze_cmdline::repository_name::BAZEL_TOOLS.clone();
        assert!(is_under_tools_package(
            &Label::parse_canonical_unchecked("@bazel_tools//tools:foo"),
            &tools
        ));
        assert!(is_under_tools_package(
            &Label::parse_canonical_unchecked("@bazel_tools//tools/cpp:foo"),
            &tools
        ));
        assert!(!is_under_tools_package(
            &Label::parse_canonical_unchecked("@bazel_tools//:foo"),
            &tools
        ));
        assert!(!is_under_tools_package(
            &Label::parse_canonical_unchecked("@bazel_tools//toolsfoo:foo"),
            &tools
        ));
        assert!(!is_under_tools_package(
            &Label::parse_canonical_unchecked("//tools:foo"),
            &tools
        ));
    }

    #[test]
    #[ignore = "expected value is not from Java or the oracle: Java's createInMainRepo(\"//command_line_option\") makes an absolute package fragment, so the label prints as ///command_line_option:cpu; needs an oracle recording"]
    fn to_option_label_is_in_command_line_option_package() {
        assert_eq!(
            to_option_label("cpu").to_string(),
            "//command_line_option:cpu"
        );
    }

    // ConfigSettingTest's analysis tests are in crates/blaze_sky_top/src/tests/config_setting_test.rs.

    // Expected values: the oracle's transcript of scripts/w4/scenarios/flags-transitions
    // (steps 27 to 32): `flag_values = {":level": "0x10"}` matches `--//:level=16`; "010", " 5"
    // and the boolean "maybe" "cannot be converted"; "no" is the boolean false.
    #[test]
    fn convert_build_setting_value_uses_the_build_setting_converters() {
        assert_eq!(
            convert_build_setting_value(Type::Integer, "0x10"),
            Ok(StarlarkOptionValue::Int(16))
        );
        assert!(convert_build_setting_value(Type::Integer, "010").is_err());
        assert!(convert_build_setting_value(Type::Integer, " 5").is_err());
        assert_eq!(
            convert_build_setting_value(Type::Boolean, "no"),
            Ok(StarlarkOptionValue::Bool(false))
        );
        assert!(convert_build_setting_value(Type::Boolean, "maybe").is_err());
        assert_eq!(
            convert_build_setting_value(Type::String, " a "),
            Ok(StarlarkOptionValue::Str(" a ".to_owned()))
        );
        assert_eq!(
            convert_build_setting_value(Type::StringList, "a,b"),
            Ok(StarlarkOptionValue::list(vec![
                StarlarkOptionValue::Str("a".to_owned()),
                StarlarkOptionValue::Str("b".to_owned()),
            ]))
        );
    }
}
