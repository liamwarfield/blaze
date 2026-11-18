//! Port of src/test/java/com/google/devtools/common/options/AssignmentConverterTest.java.

use blaze_options::converters::{AssignmentConverter, EnvVar, EnvVarsConverter};
use blaze_options::{Converter, OptionsParsingException, Value};

mod mandatory_assignment_converter_test {
    use super::*;

    fn convert(input: &str) -> Result<(String, String), OptionsParsingException> {
        AssignmentConverter
            .convert(Some(input), None)
            .map(|v| v.unwrap().get::<(String, String)>())
    }

    fn entry(k: &str, v: &str) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    // Java: AssignmentConverterTest.MandatoryAssignmentConverterTest.assignment
    #[test]
    fn assignment() {
        assert_eq!(convert("A=1").unwrap(), entry("A", "1"));
        assert_eq!(convert("A=ABC").unwrap(), entry("A", "ABC"));
        assert_eq!(convert("A=").unwrap(), entry("A", ""));
        assert_eq!(convert("A=B,C=D").unwrap(), entry("A", "B,C=D"));
    }

    // Java: AssignmentConverterTest.MandatoryAssignmentConverterTest.missingName
    #[test]
    fn missing_name() {
        assert!(convert("=VALUE").is_err());
    }

    // Java: AssignmentConverterTest.MandatoryAssignmentConverterTest.missingValue
    #[test]
    fn missing_value() {
        assert!(convert("NAME").is_err());
    }

    // Java: AssignmentConverterTest.MandatoryAssignmentConverterTest.immutability
    #[test]
    #[ignore = "needs Java Map.Entry.setValue; Rust tuples are owned values, nothing to port"]
    fn immutability() {}

    // Java: AssignmentConverterTest.MandatoryAssignmentConverterTest.emptyString
    #[test]
    fn empty_string() {
        assert!(convert("").is_err());
    }
}

mod env_vars_converter_test {
    use super::*;

    fn convert_value(input: &str) -> Value {
        EnvVarsConverter
            .convert(Some(input), None)
            .unwrap()
            .unwrap()
    }

    fn convert(input: &str) -> EnvVar {
        convert_value(input).get::<EnvVar>()
    }

    fn set(n: &str, v: &str) -> EnvVar {
        EnvVar::Set {
            name: n.into(),
            value: v.into(),
        }
    }

    // Java: AssignmentConverterTest.EnvVarsConverterTest.assignment
    #[test]
    fn assignment() {
        assert_eq!(convert("A=1"), set("A", "1"));
        assert_eq!(convert("A=ABC"), set("A", "ABC"));
        assert_eq!(convert("A="), set("A", ""));
        assert_eq!(convert("A=B,C=D"), set("A", "B,C=D"));
    }

    // Java: AssignmentConverterTest.EnvVarsConverterTest.missingName
    #[test]
    fn missing_name() {
        assert_eq!(
            convert("=NAME"),
            EnvVar::Unset {
                name: "NAME".into()
            }
        );
    }

    // Java: AssignmentConverterTest.EnvVarsConverterTest.missingValue
    #[test]
    fn missing_value() {
        assert_eq!(
            convert("NAME"),
            EnvVar::Inherit {
                name: "NAME".into()
            }
        );
    }

    // Java: AssignmentConverterTest.EnvVarsConverterTest.reverseConversionForStarlark
    #[test]
    fn reverse_conversion_for_starlark() {
        assert_eq!(
            EnvVarsConverter.reverse_for_starlark(&convert_value("a")),
            "a"
        );
        assert_eq!(
            EnvVarsConverter.reverse_for_starlark(&convert_value("a=1")),
            "a=1"
        );
        assert_eq!(
            EnvVarsConverter.reverse_for_starlark(&convert_value("=a")),
            "=a"
        );
    }
}
