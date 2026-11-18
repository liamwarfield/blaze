//! Port of src/test/java/com/google/devtools/common/options/RegexPatternConverterTest.java.

use blaze_options::Converter;
use blaze_options::converters::{RegexPatternConverter, RegexPatternOption};

fn convert(input: &str) -> Result<RegexPatternOption, blaze_options::OptionsParsingException> {
    RegexPatternConverter
        .convert(Some(input), None)
        .map(|v| v.unwrap().get::<RegexPatternOption>())
}

/// Port of the parts of `ConverterTester.testConvert` used here: converting the same input twice
/// yields equal values; values of different groups are unequal.
fn test_convert(groups: &[&str]) {
    for (i, a) in groups.iter().enumerate() {
        assert_eq!(convert(a).unwrap(), convert(a).unwrap());
        for b in &groups[i + 1..] {
            assert_ne!(convert(a).unwrap(), convert(b).unwrap());
        }
    }
}

// Java: RegexPatternConverterTest.consistentEqualsAndHashCodeForSamePattern
#[test]
fn consistent_equals_and_hash_code_for_same_pattern() {
    test_convert(&["", ".*", "[^\\s]+"]);
}

// Java: RegexPatternConverterTest.comparisonBasedOnInputOnly
#[test]
fn comparison_based_on_input_only() {
    test_convert(&["a", "[a]"]);
}

// Java: RegexPatternConverterTest.createsProperPattern
#[test]
fn creates_proper_pattern() {
    for regex in ["", ".*", "\\s*(\\w+)", "prefix (suffix1|suffix2)"] {
        assert_eq!(convert(regex).unwrap().pattern(), regex);
    }
}

// Java: RegexPatternConverterTest.throwsForWrongPattern
#[test]
fn throws_for_wrong_pattern() {
    let e = convert("{").unwrap_err();
    assert!(
        e.get_message()
            .starts_with("Not a valid regular expression:")
    );
}

// Java: RegexPatternConverterTest.unicodeLiteral
#[test]
fn unicode_literal() {
    // PORT: StringEncoding identity.
    let regex = convert("äöüÄÖÜß🌱").unwrap();
    assert!(regex.matches("äöüÄÖÜß🌱"));
}

// Java: RegexPatternConverterTest.unicodeLiteral_caseInsensitive
#[test]
fn unicode_literal_case_insensitive() {
    let regex = convert("(?ui)äöüÄÖÜß🌱").unwrap();
    assert!(regex.matches("ÄÖÜäöüß🌱"));
}

// Java: RegexPatternConverterTest.unicodeLiteral_suffix
#[test]
fn unicode_literal_suffix() {
    let regex = convert(".*äöüÄÖÜß🌱").unwrap();
    assert!(regex.matches("äöüäöüÄÖÜß🌱"));
}

// Java: RegexPatternConverterTest.unicodeClass
#[test]
#[ignore = "needs Java regex syntax: \\p{IsEmoji} is not supported by the regex crate"]
fn unicode_class() {
    let regex = convert("\\p{L}{7}\\p{IsEmoji}").unwrap();
    assert!(regex.matches("äöüÄÖÜß🌱"));
}
