//! Compiles and exercises the worked example of TRANSCRIBE.md.

use blaze_options::{OptionsClass, OptionsParser, enum_converter, option_enum, options};

// Java: com/google/devtools/build/lib/runtime/KeepGoingOption.java:22 KeepGoingOption
options! {
    /// Defines the --keep_going option which is used by multiple commands.
    pub struct KeepGoingOption {
        #[option(
            name = "keep_going",
            abbrev = 'k',
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EAGERNESS_TO_EXIT],
            help = "Continue as much as possible after an error.  While the target that failed \
                    and those that depend on it cannot be analyzed, other prerequisites of these \
                    targets can be.",
        )]
        pub keep_going: bool,
    }
}

option_enum! {
    /// Enum to select whether color output is enabled or not.
    pub enum UseColor { YES, NO, AUTO }
}

// Java: com/google/devtools/build/lib/runtime/UiOptions.java:119 UiOptions.UseColorConverter
enum_converter!(pub UseColorConverter, UseColor, "--color setting");

// Java: com/google/devtools/build/lib/runtime/UiOptions.java:35 UiOptions
options! {
    pub struct UiOptions {
        #[option(
            name = "show_progress_rate_limit",
            default_value = "0.2", // A nice middle ground; snappy but not too spammy in logs.
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Minimum number of seconds between progress messages in the output.",
        )]
        pub show_progress_rate_limit: f64,

        #[option(
            name = "color",
            default_value = "auto",
            converter = UseColorConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Use terminal controls to colorize output.",
        )]
        pub use_color_enum: UseColor,

        #[option(
            name = "terminal_columns",
            default_value = "80",
            metadata_tags = [HIDDEN],
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = "A system-generated parameter which specifies the terminal width in columns.",
        )]
        pub terminal_columns: i32,
    }
}

#[test]
fn worked_example_parses() {
    let mut parser = OptionsParser::builder()
        .options_classes(&[KeepGoingOption::class(), UiOptions::class()])
        .build()
        .unwrap();
    parser.parse(&["-k", "--color=yes"]).unwrap();
    assert!(parser.get_options::<KeepGoingOption>().unwrap().keep_going);
    let ui = parser.get_options::<UiOptions>().unwrap();
    assert_eq!(ui.use_color_enum, UseColor::YES);
    assert_eq!(ui.show_progress_rate_limit, 0.2);
    assert_eq!(ui.terminal_columns, 80);
    let d = KeepGoingOption::class().definitions().next().unwrap();
    assert_eq!(
        d.get_help_text(),
        "Continue as much as possible after an error.  While the target that failed and those \
         that depend on it cannot be analyzed, other prerequisites of these targets can be."
    );
    let e = parser.parse(&["--color=blue"]).unwrap_err();
    assert_eq!(
        e.get_message(),
        "While parsing option --color=blue: Not a valid --color setting: 'blue' (should be yes, no or auto)"
    );
    assert_eq!(UiOptions::default().use_color_enum, UseColor::AUTO);
}
