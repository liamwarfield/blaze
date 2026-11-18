//! Port of `profiler/CommandProfilerModule.java` (the `OptionsBase` subclass and what it uses).

use blaze_options::{enum_converter, option_enum, options};

// Java: com/google/devtools/build/lib/profiler/CommandProfilerModule.java:41 CommandProfilerModule.ProfileType
option_enum! {
    /// The type of profile to capture.
    pub enum ProfileType { CPU = "cpu", WALL = "wall", ALLOC = "alloc", LOCK = "lock" }
}

// Java: com/google/devtools/build/lib/profiler/CommandProfilerModule.java:54 CommandProfilerModule.ProfileTypeConverter
enum_converter!(
    pub ProfileTypeConverter,
    ProfileType,
    "--experimental_command_profile setting"
);

// Java: com/google/devtools/build/lib/profiler/CommandProfilerModule.java:62 CommandProfilerModule.Options
options! {
    /// CommandProfilerModule options.
    pub struct CommandProfilerModuleOptions {
        #[option(
            name = "experimental_command_profile",
            default_value = "null",
            converter = ProfileTypeConverter,
            documentation_category = LOGGING,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Records a Java Flight Recorder profile for the duration of the command. One of the",
                " supported profiling event types (cpu, wall, alloc or lock) must be given as an",
                " argument. The profile is written to a file named after the event type under the",
                " output base directory.",
                " The syntax and semantics of this flag might change in the future to support",
                " additional profile types or output formats; use at your own risk.",
            ),
        )]
        pub profile_type: Option<ProfileType>,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<CommandProfilerModuleOptions>(&["experimental_command_profile"]);
    }

    #[test]
    fn parses_flag() {
        let o = parse::<CommandProfilerModuleOptions>(&["--experimental_command_profile=WALL"])
            .unwrap();
        assert_eq!(o.profile_type, Some(ProfileType::WALL));
        assert_eq!(CommandProfilerModuleOptions::default().profile_type, None);
        assert_eq!(
            parse::<CommandProfilerModuleOptions>(&["--experimental_command_profile=x"])
                .unwrap_err()
                .get_message(),
            "While parsing option --experimental_command_profile=x: Not a valid \
             --experimental_command_profile setting: 'x' (should be cpu, wall, alloc or lock)"
        );
    }
}
