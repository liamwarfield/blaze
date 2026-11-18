//! Port of `com/google/devtools/build/lib/analysis/config/PerLabelOptions.java` (the parts the
//! option converters need).

use std::fmt;

use super::prelude::*;
use super::regex_filter::split_on_unescaped_commas;

// Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:32 PerLabelOptions
/// Models options that can be added to a command line when a label matches a given
/// [`RegexFilter`].
#[derive(Clone, Debug, PartialEq)]
pub struct PerLabelOptions {
    /// The filter used to match labels.
    regex_filter: RegexFilter,
    /// The list of options to add when the filter matches a label.
    options_list: Vec<String>,
}

impl PerLabelOptions {
    pub fn new(regex_filter: RegexFilter, options_list: Vec<String>) -> Self {
        PerLabelOptions {
            regex_filter,
            options_list,
        }
    }

    /// True if the given label (`Label.toString()`) is matched by the [`RegexFilter`].
    pub fn is_included_label(&self, label: &Label) -> bool {
        self.regex_filter.is_included(&label.get_canonical_form())
    }

    /// The list of options to add to a command line.
    pub fn get_options(&self) -> &[String] {
        &self.options_list
    }

    pub fn get_regex_filter(&self) -> &RegexFilter {
        &self.regex_filter
    }
}

// Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:120 PerLabelOptions.toString
impl fmt::Display for PerLabelOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} Options: {}",
            self.regex_filter,
            blaze_options::value::java_list_to_string(self.options_list.iter().cloned())
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

// Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:46 PerLabelOptions.PerLabelOptionsConverter
/// Converts a String to a [`PerLabelOptions`] object. The syntax of the string is
/// `regex_filter@option_1,option_2,...,option_n`. Where regex_filter stands for the String
/// representation of a [`RegexFilter`], and `option_1` to `option_n` stand for arbitrary command
/// line options. If an option contains a comma it has to be quoted with a backslash. Options can
/// contain @. Only the first @ is used to split the string.
#[derive(Clone, Copy, Debug, Default)]
pub struct PerLabelOptionsConverter;

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
                for option in split_on_unescaped_commas(options_piece) {
                    // Java: String.trim() strips characters <= ' '.
                    if !option.trim_matches(|c: char| c <= ' ').is_empty() {
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
    fn starlark_convertible(&self) -> bool {
        true
    }
    // Java: com/google/devtools/build/lib/analysis/config/PerLabelOptions.java:73 PerLabelOptions.PerLabelOptionsConverter.reverseForStarlark
    fn reverse_for_starlark(&self, converted: &Value) -> String {
        let typed_value = converted.get::<PerLabelOptions>();
        format!(
            "{}@{}",
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

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from src/test/java/com/google/devtools/build/lib/analysis/config/PerLabelOptionsTest.java.

    fn create_options(string: &str) -> PerLabelOptions {
        PerLabelOptionsConverter.convert_options(string).unwrap()
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
    }

    // Blaze-specific: toString and starlark reversal.
    #[test]
    fn to_string_and_reverse() {
        let p = create_options("//foo@-g,-O\\,2, ,x@y");
        assert_eq!(p.get_options(), &["-g", "-O,2", "x@y"]);
        assert_eq!(p.to_string(), "(?:(?>//foo)) Options: [-g, -O,2, x@y]");
        assert_eq!(
            PerLabelOptionsConverter.reverse_for_starlark(&Value::new(p)),
            "//foo@-g,-O,2,x@y"
        );
    }
}
