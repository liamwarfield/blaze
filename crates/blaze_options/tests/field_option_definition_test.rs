//! Port of src/test/java/com/google/devtools/common/options/FieldOptionDefinitionTest.java.

use std::sync::Arc;

use blaze_options::converters::AssignmentConverter;
use blaze_options::{OptionDefinition, OptionsClass, OptionsParser, StringConverter, options};

options! {
    /// Dummy options class, to test various expected failures of the OptionDefinition.
    pub struct BrokenOptions {
        #[option(
            name = "assignments",
            default_value = "foo is not an assignment",
            converter = AssignmentConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
        )]
        pub assignments: (String, String),
    }
}

fn def<C: OptionsClass>(name: &str) -> &'static OptionDefinition {
    OptionsParser::get_option_definition_by_name(C::class(), name)
}

// Java: FieldOptionDefinitionTest.optionConverterCannotParseDefaultValue
#[test]
fn option_converter_cannot_parse_default_value() {
    let option_def = def::<BrokenOptions>("assignments");
    let e = option_def.try_get_default_value(None).unwrap_err();
    // PORT: Java names the camelCase Java field; the Rust field name is used here (they agree
    // for this one-word field).
    assert!(
        e.get_message().contains(
            "OptionsParsingException while retrieving the default value for assignments: \
             Variable definitions must be in the form of a 'name=value' assignment"
        ),
        "{}",
        e
    );
}

// Java: FieldOptionDefinitionTest.optionDefinitionRejectsNonOptions
#[test]
#[ignore = "needs Java reflection: options! accepts only #[option] fields, so a non-option field \
            is a compile error rather than a NotAnOptionException"]
fn option_definition_rejects_non_options() {}

options! {
    /// Dummy options class with valid options for testing the memoization of converters and
    /// default values.
    pub struct ValidOptionUsingDefaultConverterForMocking {
        #[option(
            name = "foo",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "42",
        )]
        pub foo: i32,

        #[option(
            name = "bar",
            converter = StringConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "strings",
        )]
        pub bar: String,
    }
}

// Java: FieldOptionDefinitionTest.optionDefinitionMemoizesDefaultConverterValue
/// PORT: the Mockito call-count verifications are not portable; the observable memoization
/// (same converter instance on repeated calls, same default value) is checked.
#[test]
fn option_definition_memoizes_default_converter_value() {
    let option_def = def::<ValidOptionUsingDefaultConverterForMocking>("foo");
    assert!(!option_def.uses_boolean_value_syntax());
    let converter = option_def.get_converter().clone();
    // IntegerConverter.
    assert_eq!(converter.get_type_description(), "an integer");
    let value = option_def.get_default_value(None).unwrap().get::<i32>();
    assert_eq!(value, 42);
    let second_converter = option_def.get_converter();
    assert!(Arc::ptr_eq(&converter, second_converter));
    assert_eq!(option_def.get_default_value(None).unwrap().get::<i32>(), 42);
}

// Java: FieldOptionDefinitionTest.optionDefinitionMemoizesProvidedConverterValue
/// PORT: see option_definition_memoizes_default_converter_value.
#[test]
fn option_definition_memoizes_provided_converter_value() {
    let option_def = def::<ValidOptionUsingDefaultConverterForMocking>("bar");
    assert!(!option_def.uses_boolean_value_syntax());
    let converter = option_def.get_converter().clone();
    // StringConverter.
    assert_eq!(converter.get_type_description(), "a string");
    let value = option_def.get_default_value(None).unwrap().get::<String>();
    assert_eq!(value, "strings");
    assert!(Arc::ptr_eq(&converter, option_def.get_converter()));
    option_def.get_default_value(None);
}

options! {
    /// Dummy options class, to test defaultValue handling.
    pub struct DefaultValueTestOptions {
        #[option(
            name = "null_non_multiple_option",
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
        )]
        pub null_non_multiple_option: Option<String>,

        #[option(
            name = "null_multiple_option",
            allow_multiple = true,
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
        )]
        pub null_multiple_option: Vec<String>,

        #[option(
            name = "empty_string_multiple_option",
            allow_multiple = true,
            default_value = "",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
        )]
        pub empty_string_multiple_option: Vec<String>,

        #[option(
            name = "non_empty_string_multiple_option",
            allow_multiple = true,
            default_value = "text",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
        )]
        pub non_empty_string_multiple_option: Vec<String>,
    }
}

fn string_list(name: &str) -> Vec<String> {
    def::<DefaultValueTestOptions>(name)
        .get_default_value(None)
        .unwrap()
        .as_list()
        .unwrap()
        .iter()
        .map(|v| v.get::<String>())
        .collect()
}

// Java: FieldOptionDefinitionTest.specialDefaultValueForNonMultipleOptionShouldResultInNull
#[test]
fn special_default_value_for_non_multiple_option_should_result_in_null() {
    let result = def::<DefaultValueTestOptions>("null_non_multiple_option").get_default_value(None);
    assert!(result.is_none());
}

// Java: FieldOptionDefinitionTest.specialDefaultValueForMultipleOptionShouldResultInEmptyList
#[test]
fn special_default_value_for_multiple_option_should_result_in_empty_list() {
    assert!(string_list("null_multiple_option").is_empty());
}

// Java: FieldOptionDefinitionTest.emptyStringForMultipleOptionShouldBeConverted
#[test]
fn empty_string_for_multiple_option_should_be_converted() {
    assert_eq!(string_list("empty_string_multiple_option"), vec![""]);
}

// Java: FieldOptionDefinitionTest.nonEmptyStringForMultipleOptionShouldBeConverted
#[test]
fn non_empty_string_for_multiple_option_should_be_converted() {
    assert_eq!(
        string_list("non_empty_string_multiple_option"),
        vec!["text"]
    );
}
