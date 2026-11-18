//! Port of `analysis/config/PerLabelOptions.java`.

use blaze_cmdline::Label;
use blaze_options::{
    ConversionContext, Converter, OptionScalar, OptionsParsingException, Value,
    converter::non_null, option_field_type, value::java_list_to_string,
};

use super::converters::ConvResult;
use super::regex_filter::{RegexFilter, RegexFilterConverter, split_unescaped};

// Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:32 PerLabelOptions
/// Models options that can be added to a command line when a label matches a given
/// [`RegexFilter`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PerLabelOptions {
    /// The filter used to match labels
    regex_filter: RegexFilter,
    /// The list of options to add when the filter matches a label
    options_list: Vec<String>,
}

// Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:46 PerLabelOptions.PerLabelOptionsConverter
/// Converts a String to a [`PerLabelOptions`] object. The syntax of the string is
/// `regex_filter@option_1,option_2,...,option_n`. Where regex_filter stands for the String
/// representation of a [`RegexFilter`], and `option_1` to `option_n` stand for arbitrary command
/// line options. If an option contains a comma it has to be quoted with a backslash. Options can
/// contain @. Only the first @ is used to split the string.
#[derive(Clone, Copy, Debug, Default)]
pub struct PerLabelOptionsConverter;

/// Java `String.trim()`: strips leading and trailing chars `<= ' '`.
pub(crate) fn java_string_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

impl PerLabelOptionsConverter {
    // Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:49 PerLabelOptions.PerLabelOptionsConverter.convert
    pub fn convert_options(&self, input: &str) -> Result<PerLabelOptions, OptionsParsingException> {
        let converter = RegexFilterConverter;
        match input.find('@') {
            None => Ok(PerLabelOptions::new(
                converter.convert_filter(input)?,
                vec![],
            )),
            Some(at_index) => {
                let filter_piece = &input[..at_index];
                let options_piece = &input[at_index + 1..];
                let mut options_list = Vec::new();
                // Split on ',' but not on '\,'
                for option in split_unescaped(options_piece, ',') {
                    if !java_string_trim(&option).is_empty() {
                        options_list.push(option.replace("\\,", ","));
                    }
                }
                Ok(PerLabelOptions::new(
                    converter.convert_filter(filter_piece)?,
                    options_list,
                ))
            }
        }
    }
}

impl Converter for PerLabelOptionsConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_options(non_null(input))?)))
    }
    // Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:68 PerLabelOptions.PerLabelOptionsConverter.starlarkConvertible
    fn starlark_convertible(&self) -> bool {
        true
    }
    // Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:73 PerLabelOptions.PerLabelOptionsConverter.reverseForStarlark
    fn reverse_for_starlark(&self, converted: &Value) -> String {
        let typed_value = converted.get::<PerLabelOptions>();
        format!(
            "{}@{}",
            // String.format("%s", null) prints "null".
            typed_value
                .get_regex_filter()
                .to_original_string()
                .unwrap_or("null"),
            typed_value.get_options().join(",")
        )
    }
    fn get_type_description(&self) -> String {
        "a comma-separated list of regex expressions with prefix '-' specifying excluded paths \
         followed by an @ and a comma separated list of options"
            .into()
    }
}

impl PerLabelOptions {
    // Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:88 PerLabelOptions.<init>
    pub fn new(regex_filter: RegexFilter, options_list: Vec<String>) -> Self {
        PerLabelOptions {
            regex_filter,
            options_list,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:96 PerLabelOptions.isIncluded
    /// Returns true if the given label is matched by the [`RegexFilter`].
    pub fn is_included(&self, label: &Label) -> bool {
        self.regex_filter.is_included(&label.to_string())
    }

    // Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:104 PerLabelOptions.isIncluded
    /// Returns true if the execution path (which includes the base name of the file) of the given
    /// file is matched by the [`RegexFilter`].
    // PORT: Java takes an Artifact and uses artifact.getExecPathString(); the Artifact class is
    // not ported yet, so callers pass the exec path string.
    pub fn is_included_exec_path(&self, exec_path_string: &str) -> bool {
        self.regex_filter.is_included(exec_path_string)
    }

    // Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:111 PerLabelOptions.getOptions
    /// Returns the list of options to add to a command line.
    pub fn get_options(&self) -> &[String] {
        &self.options_list
    }

    // Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:115 PerLabelOptions.getRegexFilter
    pub fn get_regex_filter(&self) -> &RegexFilter {
        &self.regex_filter
    }
}

impl std::fmt::Display for PerLabelOptions {
    // Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:120 PerLabelOptions.toString
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} Options: {}",
            self.regex_filter,
            java_list_to_string(self.options_list.iter().cloned())
        )
    }
}

impl OptionScalar for PerLabelOptions {
    fn java_to_string(&self) -> String {
        self.to_string()
    }
}

option_field_type!(
    PerLabelOptions,
    "com.google.devtools.build.lib.analysis.config.PerLabelOptions",
    raw = PerLabelOptions::new(RegexFilterConverter.convert_filter("").unwrap(), vec![])
);

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/analysis/config/PerLabelOptionsTest.java
    use super::*;

    fn create_options(s: &str) -> PerLabelOptions {
        PerLabelOptionsConverter.convert_options(s).unwrap()
    }

    fn assert_regex_parsing(options: &PerLabelOptions, filter: &str) {
        let regex_filter = RegexFilterConverter.convert_filter(filter).unwrap();
        assert_eq!(
            options.get_regex_filter().to_string(),
            regex_filter.to_string()
        );
    }

    fn assert_options(pattern: &str, opts: &str, expected_options: &[&str]) {
        let options = create_options(&format!("{}@{}", pattern, opts));
        assert_regex_parsing(&options, pattern);
        assert_eq!(options.get_options(), expected_options);
    }

    #[test]
    fn test_empty() {
        let options = create_options("");
        assert_regex_parsing(&options, "");
        assert!(options.get_options().is_empty());
    }

    #[test]
    fn test_parsing() {
        assert_options("", "", &[]);
        assert_options("", ", ,\t,", &[]);
        assert_options("a/b,+^c,_test$", ", ,\t,", &[]);
        assert_options("a/b,+^c,_test$", "", &[]);
        assert_options("a/b,+^c,_test$", "-g,-O0", &["-g", "-O0"]);
        assert_options("a/b,+^c,_test$", "-g@,-O0", &["-g@", "-O0"]);
        assert_options("a/b,+^c,_test$", "-g\\,,-O0", &["-g,", "-O0"]);
        assert_options("a/b,+^c,_test$", "-g\\,,,,,-O0,,,@,", &["-g,", "-O0", "@"]);
    }

    #[test]
    fn test_equals() {
        let a1 = create_options("a/b,+^c,_test$@-g,-O0");
        let a2 = create_options("a/b,+^c,_test$@-g,-O0");
        let b = create_options("a/b,+^c,_test$@-O0");
        assert_eq!(a1, a2);
        assert_ne!(a1, b);
        assert_ne!(a2, b);
    }
}
