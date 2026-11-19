//! The part of `BuildTool.buildTargets` that applies a project file's flags to the command's
//! options (`BuildTool.ProjectOptionsApplier` of blaze_sky_top), and the conversion of the rc
//! files' `--config` definitions into `ConfigFlagDefinitions`.

use std::sync::Arc;

use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::fragment_options_class::all_known_fragment_options_classes;
use blaze_cmdline::Label;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_options::parser::STARLARK_SKIPPED_PREFIXES;
use blaze_options::rc::handler::RcChunkOfArgs;
use blaze_options::{OptionsParser, Value};
use blaze_sky_analysis::flag_set_function::{ConfigDefinition, ConfigFlagDefinitions};
use blaze_sky_top::build_tool::{BuildToolError, ProjectOptionsApplier};
use blaze_sky_top::skyframe_executor::SkyframeExecutor;
use blaze_vfs::PathFragment;
use indexmap::IndexSet;

use crate::starlark_options_parser::{
    SkyframeExecutorTargetLoader, StarlarkOptionsParser, StarlarkOptionsParsingError,
};

/// The `--config` definitions valid for a command (Java: the end of
/// `BlazeOptionHandler.parseOptionsAndGetConfigDefinitions`: `<command>:<config>` rc entries of
/// the commands this command inherits from).
// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:569 BlazeOptionHandler.parseOptionsInternal
pub fn config_flag_definitions(definitions: &[(String, RcChunkOfArgs)]) -> ConfigFlagDefinitions {
    ConfigFlagDefinitions::new(
        definitions
            .iter()
            .map(|(name, chunk)| {
                (
                    name.clone(),
                    ConfigDefinition {
                        flags: chunk.get_args().to_vec(),
                        rc_source: chunk.get_rc_file().to_owned(),
                    },
                )
            })
            .collect(),
    )
}

/// Applies a project file's options to the command's build options.
///
/// PORT: Java parses the project file's flags into the command's own `OptionsParser` (at
/// `PriorityCategory.COMMAND_LINE`, after the command line, so they win over earlier values and
/// accumulate into `allowMultiple` options), lets the `StarlarkOptionsParser` add its Starlark
/// flags, then recreates the build options (`runtime.createBuildOptions(optionsParser)`). Blaze's
/// options parser of the command is shared and immutable here, so the project's flags are parsed
/// by a new parser over the same options classes, and each option they set is applied to the
/// command's build options with the same effect: a single-valued option takes the project's value,
/// an `allowMultiple` option appends it. The Starlark flags are parsed as Java parses them: the
/// project's, then the command's skipped args.
pub struct BuildProjectOptionsApplier {
    pub options: Arc<OptionsParser>,
    pub relative_working_directory: PathFragment,
    pub reporter: Arc<dyn ExtendedEventHandler>,
    pub base_build_options: BuildOptions,
}

impl ProjectOptionsApplier for BuildProjectOptionsApplier {
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:352 BuildTool.buildTargets
    fn apply(
        &self,
        project_file: &Label,
        options: &IndexSet<String>,
        skyframe_executor: &SkyframeExecutor,
    ) -> Result<BuildOptions, BuildToolError> {
        let parse_error = |message: String| {
            self.reporter.handle(&Event::error(message.clone()));
            BuildToolError::InvalidConfiguration(
                blaze_build_config::fragment::InvalidConfigurationException::with_code(
                    message,
                    "INVALID_BUILD_OPTIONS",
                ),
            )
        };
        // First parse the native options from the project file.
        let native: Vec<String> = options
            .iter()
            .filter(|o| !STARLARK_SKIPPED_PREFIXES.iter().any(|p| o.starts_with(p)))
            .cloned()
            .collect();
        let mut parser = self
            .options
            .to_builder()
            .build()
            .unwrap_or_else(|e| e.throw());
        // optionsParser.parse(PriorityCategory.COMMAND_LINE, projectFile.toString(), ...)
        let _ = project_file;
        parser
            .parse_list(native)
            .map_err(|e| parse_error(e.get_message().to_owned()))?;

        let mut builder = self.base_build_options.to_builder();
        for option_value in parser.all_option_values() {
            let definition = option_value.get_option_definition();
            let Some(class) = all_known_fragment_options_classes()
                .into_iter()
                .find(|c| c.type_id() == definition.declaring_class_id())
            else {
                // Not a FragmentOptions option: not part of BuildOptions.
                continue;
            };
            let Some(fragment) = builder.get_fragment_options_by_type_id(class.type_id()) else {
                continue;
            };
            let new_value = option_value.get_value();
            let value = if definition.allows_multiple() {
                // The command's values come first: the project's are parsed after them.
                let mut items: Vec<Value> = definition
                    .get_raw_value(&*fragment)
                    .and_then(|v| v.as_list().map(|l| l.to_vec()))
                    .unwrap_or_default();
                if let Some(added) = new_value.as_ref().and_then(|v| v.as_list()) {
                    items.extend(added.iter().cloned());
                }
                Some(Value::list(items))
            } else {
                new_value
            };
            if definition.set_value(fragment, value).is_err() {
                panic!(
                    "IllegalStateException: Could not set option {}",
                    definition.get_option_name()
                );
            }
            // FragmentOptions.getNormalized, as BuildOptions.of applies it.
            let normalized = class.get_normalized(&*fragment);
            builder.add_fragment_options_dyn(class, normalized.as_ref());
        }

        // Then parse the starlark options from the project file, followed by the command's own
        // (`optionsParser.getSkippedArgs()`): a user's value wins over the project's, and a
        // repeatable flag lists the project's values first.
        let starlark: Vec<String> = options
            .iter()
            .filter(|o| STARLARK_SKIPPED_PREFIXES.iter().any(|p| o.starts_with(p)))
            .cloned()
            .chain(self.options.get_skipped_args())
            .collect();
        {
            let loader = SkyframeExecutorTargetLoader::new(
                skyframe_executor,
                self.relative_working_directory.clone(),
                self.reporter.clone(),
            );
            let (parsed, scopes) = {
                let mut starlark_options_parser = StarlarkOptionsParser::builder()
                    .build_setting_loader(&loader)
                    .native_options_parser(&mut parser)
                    .build();
                match starlark_options_parser.parse_given_args(&starlark) {
                    Ok(true) => {}
                    Ok(false) => panic!("IllegalStateException: Preconditions.checkState"),
                    Err(StarlarkOptionsParsingError::OptionsParsing(e)) => {
                        return Err(parse_error(e.get_message().to_owned()));
                    }
                    Err(StarlarkOptionsParsingError::Interrupted) => {
                        return Err(BuildToolError::Interrupted);
                    }
                }
                (
                    starlark_options_parser.get_starlark_options().clone(),
                    starlark_options_parser.get_scopes_attributes().clone(),
                )
            };
            let as_values = parsed
                .iter()
                .map(|(k, v)| (k.clone(), Value::new(v.clone())))
                .collect();
            // StarlarkOptionsParser.parseGivenArgs replaces the parser's Starlark options
            // (nativeOptionsParser.setStarlarkOptions), and createBuildOptions reads them: the
            // build options hold exactly the parsed ones.
            let existing: Vec<Label> = self
                .base_build_options
                .get_starlark_options()
                .keys()
                .cloned()
                .collect();
            for label in &existing {
                builder.remove_starlark_option(label);
            }
            builder.add_starlark_options(BuildOptions::labelize_starlark_options(&as_values));
            builder
                .add_scope_type_map(BuildOptions::convert_scopes_attributes(&scopes, &as_values));
        }
        Ok(builder.build())
    }
}
