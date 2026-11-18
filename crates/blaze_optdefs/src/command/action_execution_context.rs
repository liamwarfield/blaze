//! `ActionExecutionContext.ShowSubcommands` from `com/google/devtools/build/lib/actions/ActionExecutionContext.java`.

use super::prelude::*;

// Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:209 ActionExecutionContext.ShowSubcommands
option_enum! {
    /// Enum for --subcommands flag
    pub enum ShowSubcommands { TRUE, PRETTY_PRINT, FALSE }
}

impl ShowSubcommands {
    /// `shouldShowSubcommands`.
    pub fn should_show_subcommands(self) -> bool {
        matches!(self, ShowSubcommands::TRUE | ShowSubcommands::PRETTY_PRINT)
    }

    /// `prettyPrintArgs`.
    pub fn pretty_print_args(self) -> bool {
        matches!(self, ShowSubcommands::PRETTY_PRINT)
    }
}
