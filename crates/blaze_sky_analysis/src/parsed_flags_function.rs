//! Port of `skyframe/config/ParsedFlagsFunction.java` (PARSED_FLAGS): converts a list of
//! command-line flags (like `--compilation_mode=dbg` or `--//custom/starlark:flag=foo`) into a
//! `NativeAndStarlarkFlags` instance. This is intended as preparation for using the flags to
//! create or update a build configuration in Bazel.
//!
//! The key and value are `blaze_build_config::parsed_flags`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_build_config::fragment_options_class::FragmentOptionsClass;
use blaze_build_config::parsed_flags::{NativeAndStarlarkFlags, ParsedFlagsKey, ParsedFlagsValue};
use blaze_cmdline::{
    Label, LabelPackageContext, LabelRepoContext, TargetParsingException, TargetPatternsCode,
};
use blaze_options::parser::STARLARK_SKIPPED_PREFIXES;
use blaze_options::{OptionsParser, OptionsParsingException};
use blaze_packages::exceptions::NoSuchPackageException;
use blaze_packages::target::Target;
use blaze_sky_loading::package_function::PackageKey;
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, Interrupted, SkyFunctionException, Transience,
};

use crate::starlark_options_parser::{
    BuildSettingLoadError, BuildSettingLoader, StarlarkOptionsParser, StarlarkOptionsParsingError,
};

/// Registers PARSED_FLAGS with the fragment options classes of the rule class provider
/// (Java: `new ParsedFlagsFunction(ruleClassProvider.getFragmentRegistry().getOptionsClasses())`).
pub fn register(
    builder: EvaluatorBuilder,
    options_classes: Vec<FragmentOptionsClass>,
) -> EvaluatorBuilder {
    let options_classes = Arc::new(options_classes);
    builder.typed_function::<ParsedFlagsKey, _>(move |key, env| compute(&options_classes, key, env))
}

// Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsFunction.java:54 ParsedFlagsFunction.compute
fn compute(
    options_classes: &[FragmentOptionsClass],
    key: &ParsedFlagsKey,
    env: &mut Environment<'_>,
) -> Result<Option<ParsedFlagsValue>, ComputeError> {
    let mut native_flags: Vec<String> = Vec::new();
    let mut starlark_flags: Vec<String> = Vec::new();
    let flag_alias_mappings = key.flag_alias_mappings();
    for flag_setting in key.raw_flags() {
        let mut flag_setting = flag_setting.clone();
        if !flag_setting.starts_with("--") {
            // This is either something like "-c" or an invalid setting. Let options parsing
            // handle it.
            native_flags.push(flag_setting);
            continue;
        }
        let flag_name: String;
        let mut flag_value = String::new();
        let delimiter_index = flag_setting.find('=');
        let mut no_prefix = false;
        if let Some(delimiter_index) = delimiter_index {
            flag_name = flag_setting[2..delimiter_index].to_string(); // --flag=value
            flag_value = flag_setting[delimiter_index + 1..].to_string();
        } else if flag_setting.starts_with("--no") {
            flag_name = flag_setting[4..].to_string(); // --no<flag>
            no_prefix = true;
        } else {
            flag_name = flag_setting[2..].to_string(); // --<flag>
        }
        // If --flag_alias=foo=//bar and we see --foo=1, use the canonical setting --//bar=1.
        if let Some((_, actual_flag)) = flag_alias_mappings.iter().find(|(k, _)| *k == flag_name) {
            flag_setting = format!(
                "--{}{}{}",
                if no_prefix { "no" } else { "" },
                actual_flag.get_unambiguous_canonical_form(),
                if delimiter_index.is_none() {
                    String::new()
                } else {
                    format!("={}", flag_value)
                }
            );
        }
        if STARLARK_SKIPPED_PREFIXES
            .iter()
            .all(|prefix| !flag_setting.starts_with(prefix))
        {
            native_flags.push(flag_setting);
        } else {
            starlark_flags.push(flag_setting);
        }
    }
    // The StarlarkOptionsParser needs a native options parser to handle some forms of value
    // conversion and as a place to inject the flag values.
    // TODO: https://github.com/bazelbuild/bazel/issues/22365 - Clean this up as part of a general
    // rewrite.
    let mut fake_native_parser = OptionsParser::builder()
        .with_conversion_context(Arc::new(key.package_context().clone()))
        .build()
        .unwrap_or_else(|e| panic!("{}", e.get_message()));
    let loader = SkyframeTargetLoader {
        env: RefCell::new(env),
        package_context: key.package_context().clone(),
    };
    let (starlark_options, scopes_attributes, default_values) = {
        let mut starlark_flag_parser = StarlarkOptionsParser::builder()
            .build_setting_loader(&loader)
            .native_options_parser(&mut fake_native_parser)
            .include_default_values(key.include_default_values())
            .build();
        match starlark_flag_parser.parse_given_args(&starlark_flags) {
            Ok(true) => {}
            Ok(false) => return Ok(None),
            Err(StarlarkOptionsParsingError::Interrupted) => return Err(Interrupted.into()),
            Err(StarlarkOptionsParsingError::OptionsParsing(e)) => {
                return Err(parsed_flags_function_exception(e));
            }
        }
        (
            starlark_flag_parser.get_starlark_options().clone(),
            starlark_flag_parser.get_scopes_attributes().clone(),
            starlark_flag_parser.get_default_values().clone(),
        )
    };
    let mut flags = NativeAndStarlarkFlags {
        native_flags,
        starlark_flags: starlark_options,
        scopes_attributes,
        starlark_flag_defaults: BTreeMap::new(),
        options_classes: options_classes.to_vec(),
        repo_mapping: Some(key.package_context().repo_mapping().clone()),
    };

    if key.include_default_values() {
        // ImmutableMap.copyOf(starlarkFlagParser.getDefaultValues())
        flags.starlark_flag_defaults = default_values.into_iter().collect();
    }

    match ParsedFlagsValue::parse_and_create(flags) {
        Ok(value) => Ok(Some(value)),
        Err(e) => Err(parsed_flags_function_exception(e)),
    }
}

// Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsFunction.java:166 ParsedFlagsFunction.ParsedFlagsFunctionException
fn parsed_flags_function_exception(e: OptionsParsingException) -> ComputeError {
    SkyFunctionException::new(e, Transience::Persistent).into()
}

// Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsFunction.java:132 ParsedFlagsFunction.SkyframeTargetLoader
/// Lets `StarlarkOptionsParser` convert flag names to `Target`s through a Skyframe
/// `PackageValue` lookup.
struct SkyframeTargetLoader<'e, 'a> {
    env: RefCell<&'e mut Environment<'a>>,
    package_context: LabelPackageContext,
}

impl BuildSettingLoader for SkyframeTargetLoader<'_, '_> {
    // Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsFunction.java:143 ParsedFlagsFunction.SkyframeTargetLoader.loadBuildSetting
    fn load_build_setting(&self, name: &str) -> Result<Option<Target>, BuildSettingLoadError> {
        let as_label = match Label::parse_with_package_context(name, &self.package_context) {
            Ok(label) => label,
            Err(e) => panic!("java.lang.IllegalArgumentException: {}", e),
        };
        let failed_to_load = || {
            BuildSettingLoadError::TargetParsing(TargetParsingException::new(
                format!("Failed to load {}", name),
                TargetPatternsCode::DependencyNotFound,
            ))
        };
        let pkg_key = PackageKey::key(as_label.get_package_identifier().clone());
        let mut env = self.env.borrow_mut();
        let pkg = match env.get_value_or_throw::<PackageKey, NoSuchPackageException>(pkg_key) {
            Ok(Ok(Some(pkg))) => pkg,
            Ok(Ok(None)) => return Ok(None),
            Ok(Err(_)) => return Err(failed_to_load()),
            Err(Interrupted) => return Err(BuildSettingLoadError::Interrupted),
        };
        match pkg.get_package().get_target(as_label.get_name()) {
            Ok(target) => Ok(Some(target.clone())),
            Err(_) => Err(failed_to_load()),
        }
    }
}
