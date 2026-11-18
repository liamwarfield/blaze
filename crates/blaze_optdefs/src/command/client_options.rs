//! Transcription of `com/google/devtools/build/lib/runtime/ClientOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/ClientOptions.java:35 ClientOptions
options! {
    /// Options that the Bazel client passes to server, which are then incorporated into the environment.
    ///
    /// <p>The rc file options are parsed in their own right and appear, if applicable, in the final
    /// value of the parsed options. The environment variables update the stored values in the
    /// CommandEnvironment. These options should never be accessed directly from this class after command
    /// environment initialization.
    pub struct ClientOptions {
        #[option(
            name = "client_env",
            default_value = "null",
            converter = AssignmentConverter,
            allow_multiple = true,
            documentation_category = UNDOCUMENTED,
            effect_tags = [CHANGES_INPUTS],
            metadata_tags = [HIDDEN],
            help = "A system-generated parameter which specifies the client's environment",
        )]
        pub client_env: Vec<(String, String)>,

        /// These are the actual default overrides. Each value is a tuple of (bazelrc index, command name,
        /// value). The blazerc index is a number used to find the blazerc in --rc_source's values.
        ///
        /// <p>For example: "--default_override=rc:build=--cpu=piii"
        #[option(
            name = "default_override",
            default_value = "null",
            converter = OptionOverrideConverter,
            allow_multiple = true,
            documentation_category = UNDOCUMENTED,
            effect_tags = [CHANGES_INPUTS],
            metadata_tags = [HIDDEN],
            help = "",
        )]
        pub options_overrides: Vec<OptionOverride>,

        /// This is the filename that the Blaze client parsed.
        #[option(
            name = "rc_source",
            default_value = "null",
            allow_multiple = true,
            documentation_category = UNDOCUMENTED,
            effect_tags = [CHANGES_INPUTS],
            metadata_tags = [HIDDEN],
            help = "",
        )]
        pub rc_source: Vec<String>,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const CLIENT_OPTIONS_OPTION_NAMES: &[&str] = &["client_env", "default_override", "rc_source"];

    #[test]
    fn client_options_option_names() {
        super::super::test_util::check_option_names(
            ClientOptions::class(),
            CLIENT_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn client_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<ClientOptions>(SKIP_ROUND_TRIP);
    }
}

// ---- hand-written below this line (preserved by the generator) ----

// Java: com/google/devtools/build/lib/runtime/ClientOptions.java:40 ClientOptions.OptionOverride
/// A class representing a blazerc option. blazeRc is serial number of the rc file this option came
/// from, option is the name of the option and value is its value (or null if not specified).
// PORT: Java has no equals() (identity); structural equality is used here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionOverride {
    pub blaze_rc: i32,
    pub command: String,
    pub option: String,
}

// Java: com/google/devtools/build/lib/runtime/ClientOptions.java:50 ClientOptions.OptionOverride.toString
impl OptionScalar for OptionOverride {
    fn java_to_string(&self) -> String {
        format!("{}:{}={}", self.blaze_rc, self.command, self.option)
    }
}
option_field_type!(
    OptionOverride,
    "com.google.devtools.build.lib.runtime.ClientOptions$OptionOverride",
    raw = OptionOverride {
        blaze_rc: 0,
        command: String::new(),
        option: String::new(),
    }
);

// Java: com/google/devtools/build/lib/runtime/ClientOptions.java:58 ClientOptions.OptionOverrideConverter
/// Converter for --default_override. The format is: --default_override=blazerc:command=option.
#[derive(Clone, Copy, Debug, Default)]
pub struct OptionOverrideConverter;

impl OptionOverrideConverter {
    const ERROR_MESSAGE: &str =
        "option overrides must be in form rcfile:command=option, where rcfile is a nonzero integer";

    // Java: com/google/devtools/build/lib/runtime/ClientOptions.java:65 ClientOptions.OptionOverrideConverter.convert
    pub fn convert_override(&self, input: &str) -> Result<OptionOverride, OptionsParsingException> {
        let colon_pos = input.find(':');
        let assignment_pos = input.find('=');
        let Some(colon_pos) = colon_pos else {
            return Err(OptionsParsingException::new(Self::ERROR_MESSAGE));
        };
        // Java: indexOf returns -1 when '=' is absent.
        let assignment_pos = assignment_pos.map(|p| p as i64).unwrap_or(-1);
        if assignment_pos <= colon_pos as i64 + 1 {
            return Err(OptionsParsingException::new(Self::ERROR_MESSAGE));
        }
        let assignment_pos = assignment_pos as usize;
        // Java: Integer.valueOf.
        let Some(blaze_rc) = java_parse_int(&input[..colon_pos]) else {
            return Err(OptionsParsingException::new(Self::ERROR_MESSAGE));
        };
        if blaze_rc < 0 {
            return Err(OptionsParsingException::new(Self::ERROR_MESSAGE));
        }
        let command = input[colon_pos + 1..assignment_pos].to_string();
        let option = input[assignment_pos + 1..].to_string();
        Ok(OptionOverride {
            blaze_rc,
            command,
            option,
        })
    }
}

impl Converter for OptionOverrideConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_override(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        "blazerc option override".into()
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
    fn default_override() {
        let o = parse::<ClientOptions>(&[
            "--default_override=0:build=--jobs=2",
            "--default_override=1:test=--foo",
        ]);
        assert_eq!(
            o.options_overrides,
            vec![
                OptionOverride {
                    blaze_rc: 0,
                    command: "build".into(),
                    option: "--jobs=2".into()
                },
                OptionOverride {
                    blaze_rc: 1,
                    command: "test".into(),
                    option: "--foo".into()
                },
            ]
        );
        assert_eq!(
            parse_err::<ClientOptions>(&["--default_override=x:build=y"]),
            "While parsing option --default_override=x:build=y: option overrides must be in form rcfile:command=option, where rcfile is a nonzero integer"
        );
    }
}
