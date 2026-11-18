//! Port of `analysis/config/RunUnder.java` and `analysis/config/RunUnderConverter.java`.

use blaze_cmdline::Label;
use blaze_options::{
    ConversionContext, Converter, OptionScalar, OptionsParsingException, Value,
    converter::non_null, option_field_type, value::java_list_to_string,
};

use super::converters::{ConvResult, LabelConverter};
use super::shell_utils;

// Java: com/google/devtools/build/lib/analysis/config/RunUnder.java:22 RunUnder
/// Components of the `--run_under` option.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RunUnder {
    // Java: com/google/devtools/build/lib/analysis/config/RunUnder.java:53 RunUnder.LabelRunUnder
    /// Represents a value of `--run_under` whose first word (according to shell tokenization)
    /// starts with `"//"` or `"@"`. It is treated as a label referencing a target that should be
    /// used as the `--run_under` executable.
    LabelRunUnder {
        value: String,
        options: Vec<String>,
        label: Label,
    },
    // Java: com/google/devtools/build/lib/analysis/config/RunUnder.java:61 RunUnder.CommandRunUnder
    /// Represents a value of `--run_under` whose first word (according to shell tokenization)
    /// does not start with `"//"` or `"@"`. It is treated as a shell command.
    CommandRunUnder {
        value: String,
        options: Vec<String>,
        command: String,
    },
}

impl RunUnder {
    // Java: com/google/devtools/build/lib/analysis/config/RunUnder.java:26 RunUnder.value
    /// Returns the whole value passed to --run_under option.
    pub fn value(&self) -> &str {
        match self {
            RunUnder::LabelRunUnder { value, .. } | RunUnder::CommandRunUnder { value, .. } => {
                value
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/RunUnder.java:32 RunUnder.options
    /// Returns everything except the first word (according to shell tokenization) passed to
    /// `--run_under`.
    pub fn options(&self) -> &[String] {
        match self {
            RunUnder::LabelRunUnder { options, .. } | RunUnder::CommandRunUnder { options, .. } => {
                options
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/RunUnder.java:39 RunUnder.trimForNonTestConfiguration
    /// Returns a new instance that only retains the information that is relevant for the analysis
    /// of non-test targets.
    pub fn trim_for_non_test_configuration(run_under: Option<&RunUnder>) -> Option<RunUnder> {
        match run_under {
            Some(RunUnder::LabelRunUnder { label, .. }) => Some(RunUnder::LabelRunUnder {
                value: String::new(),
                options: vec![],
                label: label.clone(),
            }),
            _ => None,
        }
    }
}

impl OptionScalar for RunUnder {
    // Java record toString.
    fn java_to_string(&self) -> String {
        match self {
            RunUnder::LabelRunUnder {
                value,
                options,
                label,
            } => format!(
                "LabelRunUnder[value={}, options={}, label={}]",
                value,
                java_list_to_string(options.iter().cloned()),
                label
            ),
            RunUnder::CommandRunUnder {
                value,
                options,
                command,
            } => format!(
                "CommandRunUnder[value={}, options={}, command={}]",
                value,
                java_list_to_string(options.iter().cloned()),
                command
            ),
        }
    }
}

option_field_type!(
    RunUnder,
    "com.google.devtools.build.lib.analysis.config.RunUnder",
    raw = RunUnder::CommandRunUnder {
        value: String::new(),
        options: vec![],
        command: String::new(),
    }
);

// Java: com/google/devtools/build/lib/analysis/config/RunUnderConverter.java:29 RunUnderConverter
/// --run_under options converter.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunUnderConverter;

impl RunUnderConverter {
    // Java: com/google/devtools/build/lib/analysis/config/RunUnderConverter.java:33 RunUnderConverter.convert
    pub fn convert_run_under(
        &self,
        input: &str,
        conversion_context: Option<&ConversionContext>,
    ) -> Result<RunUnder, OptionsParsingException> {
        let mut run_under_list = Vec::new();
        if let Err(e) = shell_utils::tokenize(&mut run_under_list, input) {
            return Err(OptionsParsingException::new(format!(
                "Not a valid command prefix {}",
                e.get_message()
            )));
        }
        if run_under_list.is_empty() {
            return Err(OptionsParsingException::new("Empty command"));
        }
        let run_under_command = run_under_list[0].clone();
        let run_under_suffix = run_under_list[1..].to_vec();
        if run_under_command.starts_with("//") || run_under_command.starts_with('@') {
            match LabelConverter.convert_label(&run_under_command, conversion_context) {
                Ok(run_under_label) => Ok(RunUnder::LabelRunUnder {
                    value: input.to_string(),
                    options: run_under_suffix,
                    label: run_under_label,
                }),
                Err(e) => Err(OptionsParsingException::new(format!(
                    "Not a valid label {}",
                    e.get_message()
                ))),
            }
        } else {
            Ok(RunUnder::CommandRunUnder {
                value: input.to_string(),
                options: run_under_suffix,
                command: run_under_command,
            })
        }
    }
}

impl Converter for RunUnderConverter {
    fn convert(&self, input: Option<&str>, ctx: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(
            self.convert_run_under(non_null(input), ctx)?,
        )))
    }
    fn get_type_description(&self) -> String {
        "a prefix in front of command".into()
    }
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/analysis/config/RunUnderConverterTest.java
    use super::*;

    fn assert_equals_run_under(
        input: &str,
        label: Option<&str>,
        command: Option<&str>,
        options: &[&str],
    ) {
        let run_under = RunUnderConverter.convert_run_under(input, None).unwrap();
        let options: Vec<String> = options.iter().map(|s| s.to_string()).collect();
        match label {
            None => assert_eq!(
                run_under,
                RunUnder::CommandRunUnder {
                    value: input.to_string(),
                    options,
                    command: command.unwrap().to_string(),
                }
            ),
            Some(label) => assert_eq!(
                run_under,
                RunUnder::LabelRunUnder {
                    value: input.to_string(),
                    options,
                    label: Label::parse_canonical_unchecked(label),
                }
            ),
        }
    }

    fn assert_run_under_fails(input: &str, expected_error: &str) {
        let e = RunUnderConverter
            .convert_run_under(input, None)
            .unwrap_err();
        assert_eq!(e.get_message(), expected_error);
    }

    #[test]
    fn test_converter() {
        assert_equals_run_under("command", None, Some("command"), &[]);
        assert_equals_run_under("command -c", None, Some("command"), &["-c"]);
        assert_equals_run_under(
            "command -c --out=all",
            None,
            Some("command"),
            &["-c", "--out=all"],
        );
        assert_equals_run_under("//run:under", Some("//run:under"), None, &[]);
        assert_equals_run_under("//run:under -c", Some("//run:under"), None, &["-c"]);
        assert_equals_run_under(
            "//run:under -c --out=all",
            Some("//run:under"),
            None,
            &["-c", "--out=all"],
        );

        assert_run_under_fails("", "Empty command");
    }
}
