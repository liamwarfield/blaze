//! `LinkCommandLine` (ported in `blaze_cc_features`) as a `blaze_actions` command line, for the
//! `Args` that `cc_internal.get_link_args` returns (`Args.forRegisteredAction`).

use std::fmt;

use blaze_actions::actions::action::{CommandLineError, InputMetadataProvider};
use blaze_actions::actions::command_line::CommandLine;
use blaze_actions::actions::path_mapper::PathMapper;
use blaze_cc_features::actions::NOOP as CC_NOOP;
use blaze_cc_features::link_command_line::LinkCommandLine;
use blaze_collect::command_line_item::{CommandLineExpansionException, MapFnError};

use super::cc_artifact::{CcInputMetadataProvider, CcPathMapper};

/// A `LinkCommandLine` as an `AbstractCommandLine`.
// Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:43 LinkCommandLine
pub struct LinkCommandLineAdapter(pub LinkCommandLine);

impl fmt::Debug for LinkCommandLineAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LinkCommandLine({:?})", self.0.get_action_name())
    }
}

fn wrap(e: blaze_cc_features::actions::CommandLineExpansionException) -> CommandLineError {
    MapFnError::CommandLineExpansion(CommandLineExpansionException::new(e.message))
}

impl CommandLine for LinkCommandLineAdapter {
    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:157 LinkCommandLine.arguments
    fn arguments(&self) -> Result<Vec<String>, CommandLineError> {
        self.0.arguments_with(None, &CC_NOOP).map_err(wrap)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:162 LinkCommandLine.arguments
    fn arguments_with(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, CommandLineError> {
        let imp = input_metadata_provider.map(CcInputMetadataProvider);
        let imp_ref: Option<&dyn blaze_cc_features::actions::InputMetadataProvider> = imp
            .as_ref()
            .map(|p| p as &dyn blaze_cc_features::actions::InputMetadataProvider);
        self.0
            .arguments_with(imp_ref, &CcPathMapper(path_mapper))
            .map_err(wrap)
    }
}
