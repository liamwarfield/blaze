//! Port of `com.google.devtools.build.lib.runtime.ClientOptions`.

use crate::converter::{ConversionContext, Converter, non_null};
use crate::exception::OptionsParsingException;
use crate::value::{OptionScalar, Value};

// Java: com/google/devtools/build/lib/runtime/ClientOptions.java:37 ClientOptions.OptionOverride
/// A blazerc option: the serial number of the rc file, the command and the option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionOverride {
    pub blaze_rc: i32,
    pub command: String,
    pub option: String,
}

impl OptionOverride {
    pub fn new(blaze_rc: i32, command: impl Into<String>, option: impl Into<String>) -> Self {
        OptionOverride {
            blaze_rc,
            command: command.into(),
            option: option.into(),
        }
    }
}

impl OptionScalar for OptionOverride {
    // Java: com/google/devtools/build/lib/runtime/ClientOptions.java:48 ClientOptions.OptionOverride.toString
    fn java_to_string(&self) -> String {
        format!("{}:{}={}", self.blaze_rc, self.command, self.option)
    }
}
crate::option_field_type!(
    OptionOverride,
    "com.google.devtools.build.lib.runtime.ClientOptions$OptionOverride",
    raw = OptionOverride::new(0, "", "")
);

// Java: com/google/devtools/build/lib/runtime/ClientOptions.java:54 ClientOptions.OptionOverrideConverter
/// Converter for `--default_override=blazerc:command=option`.
#[derive(Clone, Copy, Debug, Default)]
pub struct OptionOverrideConverter;

impl OptionOverrideConverter {
    pub const ERROR_MESSAGE: &'static str =
        "option overrides must be in form rcfile:command=option, where rcfile is a nonzero integer";
}

impl Converter for OptionOverrideConverter {
    // Java: com/google/devtools/build/lib/runtime/ClientOptions.java:61 ClientOptions.OptionOverrideConverter.convert
    fn convert(
        &self,
        input: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        let input = non_null(input);
        let err = || OptionsParsingException::new(Self::ERROR_MESSAGE);
        let colon_pos = input.find(':').ok_or_else(err)?;
        // Java: indexOf('=') over the whole input; -1 when absent.
        let assignment_pos = input.find('=').map(|p| p as i64).unwrap_or(-1);
        if assignment_pos <= colon_pos as i64 + 1 {
            return Err(err());
        }
        let assignment_pos = assignment_pos as usize;
        // Java Integer.valueOf: optional sign, decimal digits.
        let blaze_rc: i32 = input[..colon_pos].parse().map_err(|_| err())?;
        if blaze_rc < 0 {
            return Err(err());
        }
        let command = &input[colon_pos + 1..assignment_pos];
        let option = &input[assignment_pos + 1..];
        Ok(Some(Value::new(OptionOverride::new(
            blaze_rc, command, option,
        ))))
    }

    fn get_type_description(&self) -> String {
        "blazerc option override".into()
    }
}

crate::options! {
    // Java: com/google/devtools/build/lib/runtime/ClientOptions.java:30 ClientOptions
    /// Options that the Bazel client passes to the server.
    pub struct ClientOptions {
        #[option(
            name = "client_env",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            metadata_tags = [HIDDEN],
            effect_tags = [CHANGES_INPUTS],
            converter = crate::converters::AssignmentConverter,
            allow_multiple = true,
            help = "A system-generated parameter which specifies the client's environment",
        )]
        pub client_env: Vec<(String, String)>,

        /// These are the actual default overrides. Each value is a tuple of (bazelrc index,
        /// command name, value).
        #[option(
            name = "default_override",
            default_value = "null",
            allow_multiple = true,
            documentation_category = UNDOCUMENTED,
            effect_tags = [CHANGES_INPUTS],
            metadata_tags = [HIDDEN],
            converter = OptionOverrideConverter,
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
