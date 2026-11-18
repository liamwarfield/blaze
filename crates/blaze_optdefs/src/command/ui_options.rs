//! Transcription of `com/google/devtools/build/lib/runtime/UiOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/UiOptions.java:35 UiOptions
options! {
    /// Command-line UI options.
    pub struct UiOptions {
        #[option(
            name = "show_progress",
            default_value = "true",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Display progress messages during a build.",
        )]
        pub show_progress: bool,

        // A nice middle ground; snappy but not too spammy in logs.
        #[option(
            name = "show_progress_rate_limit",
            default_value = "0.2",
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
            name = "curses",
            default_value = "auto",
            converter = UseCursesConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Use terminal cursor controls to minimize scrolling output.",
        )]
        pub use_curses_enum: UseCurses,

        #[option(
            name = "terminal_columns",
            default_value = "80",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            metadata_tags = [HIDDEN],
            help = "A system-generated parameter which specifies the terminal width in columns.",
        )]
        pub terminal_columns: i32,

        #[option(
            name = "isatty",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            metadata_tags = [HIDDEN],
            help = concat!(
                "A system-generated parameter which is used to notify the ",
                "server whether this client is running in a terminal. ",
                "If this is set to false, then '--color=auto' will be treated as '--color=no'. ",
                "If this is set to true, then '--color=auto' will be treated as '--color=yes'.",
            ),
        )]
        pub is_a_tty: bool,

        #[option(
            name = "emacs",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "A system-generated parameter which is true iff EMACS=t or INSIDE_EMACS is set ",
                "in the environment of the client.  This option controls certain display ",
                "features.",
            ),
        )]
        pub running_in_emacs: bool,

        #[option(
            name = "show_timestamps",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Include timestamps in messages",
        )]
        pub show_timestamp: bool,

        #[option(
            name = "progress_in_terminal_title",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Show the command progress in the terminal title. ",
                "Useful to see what bazel is doing when having multiple terminal tabs.",
            ),
        )]
        pub progress_in_term_title: bool,

        #[option(
            name = "attempt_to_print_relative_paths",
            old_name = "experimental_ui_attempt_to_print_relative_paths",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "When printing the location part of messages, attempt to use a path relative to the ",
                "workspace directory or one of the directories specified by --package_path.",
            ),
        )]
        pub attempt_to_print_relative_paths: bool,

        #[option(
            name = "experimental_ui_debug_all_events",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            metadata_tags = [HIDDEN],
            help = "Report all events known to the Bazel UI.",
        )]
        pub experimental_ui_debug_all_events: bool,

        #[option(
            name = "ui_event_filters",
            default_value = "null",
            converter = EventFiltersConverter,
            allow_multiple = true,
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "Specifies which events to show in the UI. It is possible to add or remove events ",
                "to the default ones using leading +/-, or override the default ",
                "set completely with direct assignment. The set of supported event kinds ",
                "include INFO, DEBUG, ERROR and more.",
            ),
        )]
        pub event_kind_filters: Vec<EventKindFilters>,

        #[option(
            name = "ui_actions_shown",
            old_name = "experimental_ui_actions_shown",
            default_value = "8",
            documentation_category = UNCATEGORIZED,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "Number of concurrent actions shown in the detailed progress bar; each ",
                "action is shown on a separate line. The progress bar always shows ",
                "at least one one, all numbers less than 1 are mapped to 1.",
            ),
        )]
        pub ui_actions_shown: i32,

        #[option(
            name = "experimental_ui_max_stdouterr_bytes",
            default_value = "1048576",
            converter = MaxStdoutErrBytesConverter,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = concat!(
                "The maximum size of the stdout / stderr files that will be printed to the console. ",
                "-1 implies no limit.",
            ),
        )]
        pub max_stdout_err_bytes: i32,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const UI_OPTIONS_OPTION_NAMES: &[&str] = &[
        "show_progress",
        "show_progress_rate_limit",
        "color",
        "curses",
        "terminal_columns",
        "isatty",
        "emacs",
        "show_timestamps",
        "progress_in_terminal_title",
        "attempt_to_print_relative_paths",
        "experimental_ui_debug_all_events",
        "ui_event_filters",
        "ui_actions_shown",
        "experimental_ui_max_stdouterr_bytes",
    ];

    #[test]
    fn ui_options_option_names() {
        super::super::test_util::check_option_names(UiOptions::class(), UI_OPTIONS_OPTION_NAMES);
    }

    #[test]
    fn ui_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<UiOptions>(SKIP_ROUND_TRIP);
    }
}

// ---- hand-written below this line (preserved by the generator) ----

// Java: com/google/devtools/build/lib/runtime/UiOptions.java:38 UiOptions.UseColor
option_enum! {
    /// Enum to select whether color output is enabled or not.
    pub enum UseColor { YES, NO, AUTO }
}

// Java: com/google/devtools/build/lib/runtime/UiOptions.java:45 UiOptions.UseCurses
option_enum! {
    /// Enum to select whether curses output is enabled or not.
    pub enum UseCurses { YES, NO, AUTO }
}

// Java: com/google/devtools/build/lib/runtime/UiOptions.java:119 UiOptions.UseColorConverter
enum_converter!(pub UseColorConverter, UseColor, "--color setting");

// Java: com/google/devtools/build/lib/runtime/UiOptions.java:126 UiOptions.UseCursesConverter
enum_converter!(pub UseCursesConverter, UseCurses, "--curses setting");

// Java: com/google/devtools/build/lib/runtime/UiOptions.java:56 UiOptions.EventFiltersConverter.EventKindFilters
/// Container for an EventKind input filter.
// PORT: java-hash-order. Java builds both sets with ImmutableSet.copyOf(HashSet<EventKind>),
// whose order follows identity hash codes (not reproducible); declaration order is used here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventKindFilters {
    pub filtered_event_kinds: std::collections::BTreeSet<EventKind>,
    pub unfiltered_event_kinds: std::collections::BTreeSet<EventKind>,
}

impl OptionScalar for EventKindFilters {
    fn java_to_string(&self) -> String {
        let set = |s: &std::collections::BTreeSet<EventKind>| {
            blaze_options::value::java_list_to_string(s.iter().map(OptionScalar::java_to_string))
        };
        format!(
            "EventKindFilters[filteredEventKinds={}, unfilteredEventKinds={}]",
            set(&self.filtered_event_kinds),
            set(&self.unfiltered_event_kinds)
        )
    }
}
option_field_type!(
    EventKindFilters,
    "com.google.devtools.build.lib.runtime.UiOptions$EventFiltersConverter$EventKindFilters",
    raw = EventKindFilters {
        filtered_event_kinds: Default::default(),
        unfiltered_event_kinds: Default::default(),
    }
);

// Java: com/google/devtools/build/lib/runtime/UiOptions.java:52 UiOptions.EventFiltersConverter
/// Converter for [`EventKind`] filters.
#[derive(Clone, Copy, Debug, Default)]
pub struct EventFiltersConverter;

impl EventFiltersConverter {
    // Java: com/google/devtools/build/lib/runtime/UiOptions.java:78 UiOptions.EventFiltersConverter.convert
    pub fn convert_filters(
        &self,
        input: &str,
    ) -> Result<EventKindFilters, OptionsParsingException> {
        use super::event_kind::ALL_EVENTS;
        use std::collections::BTreeSet;
        if input.is_empty() {
            // This method is not called to convert the default value
            // Empty list means that the user wants to filter all events
            return Ok(EventKindFilters {
                filtered_event_kinds: ALL_EVENTS.iter().copied().collect(),
                unfiltered_event_kinds: BTreeSet::new(),
            });
        }
        let filters = CommaSeparatedOptionListConverter.convert_list(input)?;
        let event_kind_converter =
            blaze_options::converters::EnumConverter::<EventKind>::new("event kind");
        let mut filtered_event_kinds = BTreeSet::new();
        let mut unfiltered_event_kinds = BTreeSet::new();
        for filter in &filters {
            if !filter.starts_with('+') && !filter.starts_with('-') {
                filtered_event_kinds.extend(ALL_EVENTS.iter().copied());
                unfiltered_event_kinds.clear();
            }
            if !filter.is_empty() {
                // filter.replaceFirst("^[+-]", "")
                let name = filter
                    .strip_prefix('+')
                    .or_else(|| filter.strip_prefix('-'))
                    .unwrap_or(filter);
                let kind = event_kind_converter.convert_enum(name)?;
                if filter.starts_with('-') {
                    filtered_event_kinds.insert(kind);
                    unfiltered_event_kinds.remove(&kind);
                } else {
                    unfiltered_event_kinds.insert(kind);
                    filtered_event_kinds.remove(&kind);
                }
            }
        }
        Ok(EventKindFilters {
            filtered_event_kinds,
            unfiltered_event_kinds,
        })
    }
}

impl Converter for EventFiltersConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_filters(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        "Convert list of comma separated event kind to list of filters".into()
    }
}

// Java: com/google/devtools/build/lib/runtime/UiOptions.java:295 UiOptions.MaxStdoutErrBytesConverter
/// A converter for --experimental_ui_max_stdouterr_bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct MaxStdoutErrBytesConverter;

impl MaxStdoutErrBytesConverter {
    /// The maximum value of the flag must be limited to ensure conversions to UTF-8 do not trigger
    /// integer overflows.
    const MAX_VALUE: i32 = (i32::MAX - 8) >> 1;

    fn range() -> RangeConverter {
        RangeConverter::new(-1, (i32::MAX - 8) >> 1)
    }
}

impl Converter for MaxStdoutErrBytesConverter {
    // Java: com/google/devtools/build/lib/runtime/UiOptions.java:310 UiOptions.MaxStdoutErrBytesConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let value = Self::range().convert_int(non_null(input))?;
        Ok(Some(Value::new(if value >= 0 {
            value
        } else {
            Self::MAX_VALUE
        })))
    }
    fn get_type_description(&self) -> String {
        Self::range().get_type_description()
    }
}

impl UiOptions {
    // Java: com/google/devtools/build/lib/runtime/UiOptions.java:277 UiOptions.useColor
    pub fn use_color(&self) -> bool {
        self.use_color_enum == UseColor::YES
            || (self.use_color_enum == UseColor::AUTO && self.is_a_tty)
    }

    // Java: com/google/devtools/build/lib/runtime/UiOptions.java:281 UiOptions.useCursorControl
    pub fn use_cursor_control(&self) -> bool {
        self.use_curses_enum == UseCurses::YES
            || (self.use_curses_enum == UseCurses::AUTO && self.is_a_tty)
    }

    // Java: com/google/devtools/build/lib/runtime/UiOptions.java:285 UiOptions.getFilteredEventKinds
    pub fn get_filtered_event_kinds(&self) -> std::collections::BTreeSet<EventKind> {
        let mut filtered = std::collections::BTreeSet::new();
        for filters in &self.event_kind_filters {
            filtered.extend(filters.filtered_event_kinds.iter().copied());
            filtered.retain(|k| !filters.unfiltered_event_kinds.contains(k));
        }
        filtered
    }
}

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, parse_err};
    use super::*;

    #[test]
    fn color_and_filters() {
        let o = parse::<UiOptions>(&["--color=yes", "--ui_event_filters=-info,+warning"]);
        assert!(o.use_color());
        let f = o.get_filtered_event_kinds();
        assert!(f.contains(&EventKind::INFO));
        assert!(!f.contains(&EventKind::WARNING));
        let o = parse::<UiOptions>(&["--ui_event_filters="]);
        assert_eq!(o.get_filtered_event_kinds().len(), 16);
        let o = parse::<UiOptions>(&["--ui_event_filters=error"]);
        assert_eq!(o.get_filtered_event_kinds().len(), 15);
        assert_eq!(
            parse_err::<UiOptions>(&["--color=blue"]),
            "While parsing option --color=blue: Not a valid --color setting: 'blue' (should be yes, no or auto)"
        );
    }

    #[test]
    fn max_stdout_err_bytes() {
        let o = parse::<UiOptions>(&["--experimental_ui_max_stdouterr_bytes=-1"]);
        assert_eq!(o.max_stdout_err_bytes, (i32::MAX - 8) >> 1);
        let o = parse::<UiOptions>(&["--experimental_ui_max_stdouterr_bytes=5"]);
        assert_eq!(o.max_stdout_err_bytes, 5);
    }
}
