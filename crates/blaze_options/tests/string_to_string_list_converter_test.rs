//! Port of src/test/java/com/google/devtools/common/options/StringToStringListConverterTest.java.

use blaze_options::converters::StringToStringListConverter;

fn convert(input: &str) -> (String, Vec<String>) {
    StringToStringListConverter.convert_entry(input).unwrap()
}

fn entry(k: &str, v: &[&str]) -> (String, Vec<String>) {
    (k.to_string(), v.iter().map(|s| s.to_string()).collect())
}

// Java: StringToStringListConverterTest.nameEqualsValue
#[test]
fn name_equals_value() {
    assert_eq!(convert("name=value"), entry("name", &["value"]));
}

// Java: StringToStringListConverterTest.nameEqualsMultipleValues
#[test]
fn name_equals_multiple_values() {
    assert_eq!(
        convert("name=value1,value2"),
        entry("name", &["value1", "value2"])
    );
}

// Java: StringToStringListConverterTest.nameEqualsNoValue_setsEmptyValue
#[test]
fn name_equals_no_value_sets_empty_value() {
    assert_eq!(convert("name="), entry("name", &[]));
}

// Java: StringToStringListConverterTest.equalsValue_setsEmptyKey
#[test]
fn equals_value_sets_empty_key() {
    assert_eq!(convert("=value"), entry("", &["value"]));
}

// Java: StringToStringListConverterTest.justValue_setsEmptyKey
#[test]
fn just_value_sets_empty_key() {
    assert_eq!(convert("value"), entry("", &["value"]));
}

// Java: StringToStringListConverterTest.noNameMultipleValues
#[test]
fn no_name_multiple_values() {
    assert_eq!(convert("value1,value2"), entry("", &["value1", "value2"]));
}
