//! The part of `BuildTool.buildTargets` that applies a project file's flags to the command's
//! options (`BuildTool.ProjectOptionsApplier` of blaze_sky_top), and the conversion of the rc
//! files' `--config` definitions into `ConfigFlagDefinitions`.

use std::sync::Arc;

use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::top_level::create_build_options;
use blaze_cmdline::Label;
use blaze_events::ExtendedEventHandler;
use blaze_options::parser::STARLARK_SKIPPED_PREFIXES;
use blaze_options::rc::handler::RcChunkOfArgs;
use blaze_options::{OptionsParser, PriorityCategory};
use blaze_sky_analysis::flag_set_function::{ConfigDefinition, ConfigFlagDefinitions};
use blaze_sky_top::build_tool::{BuildToolError, ProjectOptionsApplier};
use blaze_sky_top::skyframe_executor::SkyframeExecutor;
use blaze_vfs::PathFragment;
use indexmap::IndexSet;
use parking_lot::Mutex;

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

/// Applies a project file's options to the command's options.
///
/// Java parses the project file's flags into the command's own `OptionsParser` (at
/// `PriorityCategory.COMMAND_LINE`, after the command line, so they win over earlier values and
/// accumulate into `allowMultiple` options), lets the `StarlarkOptionsParser` add its Starlark
/// flags to the same parser, then recreates the build options
/// (`runtime.createBuildOptions(optionsParser)`).
///
/// PORT: the command's parser is shared behind an `Arc` here, so the flags are parsed into a copy
/// of it (all its parsed state included); the command installs that copy as its options
/// ([`BuildProjectOptionsApplier::take_updated_options`]), so everything that reads the options
/// after `BuildTool.buildTargets` sees the project's values as Java's callers do.
pub struct BuildProjectOptionsApplier {
    pub options: Arc<OptionsParser>,
    pub relative_working_directory: PathFragment,
    pub reporter: Arc<dyn ExtendedEventHandler>,
    /// The parser with the project's flags, once `apply` has run.
    pub updated_options: Mutex<Option<Arc<OptionsParser>>>,
    /// Posts the final `CanonicalCommandLineEvent` of the updated parser to the command's build
    /// event stream (`None` without a stream).
    pub post_canonical_command_line: Option<Box<dyn Fn(&OptionsParser) + Send + Sync>>,
}

impl BuildProjectOptionsApplier {
    pub fn new(
        options: Arc<OptionsParser>,
        relative_working_directory: PathFragment,
        reporter: Arc<dyn ExtendedEventHandler>,
    ) -> Self {
        BuildProjectOptionsApplier {
            options,
            relative_working_directory,
            reporter,
            updated_options: Mutex::new(None),
            post_canonical_command_line: None,
        }
    }

    /// With the build event stream's poster of the final canonical command line.
    pub fn with_canonical_command_line_poster(
        mut self,
        poster: Box<dyn Fn(&OptionsParser) + Send + Sync>,
    ) -> Self {
        self.post_canonical_command_line = Some(poster);
        self
    }

    /// The command's parser after the project file's flags were parsed into it, if a project
    /// file set any.
    pub fn take_updated_options(&self) -> Option<Arc<OptionsParser>> {
        self.updated_options.lock().take()
    }
}

/// An `OptionsParsingException` thrown in `BuildTool.buildTargets`: no clause of
/// `processRequest` catches it but `catch (Throwable)`, which keeps it as `crash`; the `finally`
/// block's `stopRequest` then calls `BuildResult.setUnhandledThrowable(crash)`, whose
/// `checkState` (a crash must be a `RuntimeException` or an `Error`) throws an
/// `IllegalStateException` without a message. That is the crash Bazel reports (exit 37; oracle
/// probe with `--compilation_mode=bogus` in a PROJECT.scl buildable unit, 2026-11-25).
///
/// PORT: Blaze crashes here, where the exception is thrown; nothing Java does between the throw
/// and the `checkState` is visible.
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:1002 BuildTool.processRequest
// Java: com/google/devtools/build/lib/buildtool/BuildResult.java:125 BuildResult.setUnhandledThrowable
fn options_parsing_crash(_e: &blaze_options::OptionsParsingException) -> ! {
    panic!("IllegalStateException")
}

impl ProjectOptionsApplier for BuildProjectOptionsApplier {
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:352 BuildTool.buildTargets
    fn apply(
        &self,
        project_file: &Label,
        options: &IndexSet<String>,
        skyframe_executor: &SkyframeExecutor,
    ) -> Result<BuildOptions, BuildToolError> {
        let mut parser: OptionsParser = (*self.options).clone();
        // First parse the native options from the project file.
        let native: Vec<String> = options
            .iter()
            .filter(|o| !STARLARK_SKIPPED_PREFIXES.iter().any(|p| o.starts_with(p)))
            .cloned()
            .collect();
        if let Err(e) = parser.parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some(&project_file.to_string()),
            native,
        ) {
            options_parsing_crash(&e);
        }

        // Then parse the starlark options from the project file, followed by the command's
        // skipped args.
        let starlark: Vec<String> = options
            .iter()
            .filter(|o| STARLARK_SKIPPED_PREFIXES.iter().any(|p| o.starts_with(p)))
            .cloned()
            .chain(parser.get_skipped_args())
            .collect();
        {
            let loader = SkyframeExecutorTargetLoader::new(
                skyframe_executor,
                self.relative_working_directory.clone(),
                self.reporter.clone(),
            );
            let mut starlark_options_parser = StarlarkOptionsParser::builder()
                .build_setting_loader(&loader)
                .native_options_parser(&mut parser)
                .build();
            match starlark_options_parser.parse_given_args(&starlark) {
                Ok(true) => {}
                Ok(false) => panic!("IllegalStateException: Preconditions.checkState"),
                Err(StarlarkOptionsParsingError::OptionsParsing(e)) => options_parsing_crash(&e),
                Err(StarlarkOptionsParsingError::Interrupted) => {
                    return Err(BuildToolError::Interrupted);
                }
            }
        }
        // env.getEventBus().post(new CanonicalCommandLineEvent(..., /* replaceable= */ false)),
        // then new UpdateOptionsEvent(optionsParser), which no subscriber at the pin receives.
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:375 BuildTool.buildTargets
        if let Some(post) = &self.post_canonical_command_line {
            post(&parser);
        }
        // buildOptions = runtime.createBuildOptions(optionsParser)
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:400 BuildTool.buildTargets
        let build_options = create_build_options(&BazelRuleClassProvider::create(), &parser);
        *self.updated_options.lock() = Some(Arc::new(parser));
        Ok(build_options)
    }
}
