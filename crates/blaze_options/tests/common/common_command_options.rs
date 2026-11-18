//! The options of runtime CommonCommandOptions that rc/--config handling reads (subset).

use blaze_options::options;

options! {
    /// The options of CommonCommandOptions that rc/--config handling reads.
    pub struct CommonCommandOptions {
        #[option(
            name = "enable_platform_specific_config",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "If true, Bazel picks up host-OS-specific config lines from bazelrc files.",
        )]
        pub enable_platform_specific_config: bool,

        #[option(
            name = "config",
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            allow_multiple = true,
            help = "Selects additional config sections from the rc files.",
        )]
        pub configs: Vec<String>,

        #[option(
            name = "unconditional_warning",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [TERMINAL_OUTPUT],
            allow_multiple = true,
            help = "A warning that will unconditionally get printed with build warnings and errors.",
        )]
        pub deprecation_warnings: Vec<String>,
    }
}
