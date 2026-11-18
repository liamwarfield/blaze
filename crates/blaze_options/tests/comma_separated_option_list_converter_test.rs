//! Port of src/test/java/com/google/devtools/common/options/CommaSeparatedOptionListConverterTest.java.

use blaze_options::Converter;
use blaze_options::converters::CommaSeparatedOptionListConverter;

fn convert(input: &str) -> Vec<String> {
    CommaSeparatedOptionListConverter
        .convert(Some(input), None)
        .unwrap()
        .unwrap()
        .as_list()
        .expect("a list")
        .iter()
        .map(|v| v.get::<String>())
        .collect()
}

// Java: CommaSeparatedOptionListConverterTest.emptyStringYieldsEmptyList
#[test]
fn empty_string_yields_empty_list() {
    assert!(convert("").is_empty());
}

// Java: CommaSeparatedOptionListConverterTest.commaTwoEmptyStrings
#[test]
fn comma_two_empty_strings() {
    assert_eq!(convert(","), vec!["", ""]);
}

// Java: CommaSeparatedOptionListConverterTest.leadingCommaYieldsLeadingSpace
#[test]
fn leading_comma_yields_leading_space() {
    assert_eq!(convert(",leading,comma"), vec!["", "leading", "comma"]);
}

// Java: CommaSeparatedOptionListConverterTest.trailingCommaYieldsTrailingSpace
#[test]
fn trailing_comma_yields_trailing_space() {
    assert_eq!(convert("trailing,comma,"), vec!["trailing", "comma", ""]);
}

// Java: CommaSeparatedOptionListConverterTest.singleWord
#[test]
fn single_word() {
    assert_eq!(convert("lonely"), vec!["lonely"]);
}

// Java: CommaSeparatedOptionListConverterTest.multiWords
#[test]
fn multi_words() {
    assert_eq!(convert("one,two,three"), vec!["one", "two", "three"]);
}

// Java: CommaSeparatedOptionListConverterTest.spaceIsIgnored
#[test]
fn space_is_ignored() {
    assert_eq!(convert("one two three"), vec!["one two three"]);
}
