//! Port of src/test/java/com/google/devtools/common/options/RegexPatternConverterTest.java.

use blaze_options::Converter;
use blaze_options::converters::{RegexPatternConverter, RegexPatternOption};
use blaze_util::string_encoding::unicode_to_internal;

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

fn internal(s: &str) -> String {
    unicode_to_internal(s).into_owned()
}

// Java: RegexPatternConverterTest.unicodeLiteral
#[test]
fn unicode_literal() {
    // Options passed on the command line are passed to convertes in the internal encoding.
    let regex = convert(&internal("äöüÄÖÜß🌱")).unwrap();
    assert!(regex.regex_pattern().is_match("äöüÄÖÜß🌱"));
    assert!(regex.pattern_matches("äöüÄÖÜß🌱"));
    assert!(regex.matches(&internal("äöüÄÖÜß🌱")));
}

// Java: RegexPatternConverterTest.unicodeLiteral_caseInsensitive
#[test]
fn unicode_literal_case_insensitive() {
    // Options passed on the command line are passed to convertes in the internal encoding.
    let regex = convert(&internal("(?ui)äöüÄÖÜß🌱")).unwrap();
    assert!(regex.pattern_matches("ÄÖÜäöüß🌱"));
    assert!(regex.matches(&internal("ÄÖÜäöüß🌱")));
}

// Java: RegexPatternConverterTest.unicodeLiteral_suffix
#[test]
fn unicode_literal_suffix() {
    // Options passed on the command line are passed to convertes in the internal encoding.
    let regex = convert(&internal(".*äöüÄÖÜß🌱")).unwrap();
    assert!(regex.pattern_matches("äöüäöüÄÖÜß🌱"));
    assert!(regex.matches(&internal("äöüäöüÄÖÜß🌱")));
}

// Blaze: each branch of RegexUtil.asOptimizedMatchingPredicate on internal strings: `é` (C3 A9),
// `🐷` (F0 9F 90 B7) and the byte FF (not UTF-8; U+FFFD after internalToUnicode).
#[test]
fn matcher_on_internal_strings() {
    let e = internal("é");
    let pig = internal("🐷");
    // Literal: unicodeToInternal(pattern)::equals.
    let literal = convert(&format!("a{e}\\.b")).unwrap();
    assert_eq!(literal.pattern(), "aé\\.b");
    assert!(literal.matches(&format!("a{e}.b")));
    assert!(!literal.matches("aé.b"));
    // Literal suffix: endsWith.
    let suffix = convert(&format!(".*{pig}")).unwrap();
    assert!(suffix.matches(&format!("x{pig}")));
    // Alternation: matcher(internalToUnicode(s)).matches().
    let alternation = convert(&format!("{e}|x")).unwrap();
    assert!(alternation.matches(&e));
    assert!(!alternation.matches("\u{ff}"));
    // A char class: `.` is one Unicode char, so `é` matches `^.$` but FF does too (U+FFFD).
    let dot = convert("[a-z]?.").unwrap();
    assert!(dot.matches(&e));
    assert!(dot.matches("\u{ff}"));
    assert!(!dot.matches(&format!("{e}{e}")));
    // Find of the suffix after `.*`.
    let find = convert(&format!(".*[{e}]x")).unwrap();
    assert!(find.matches(&format!("ab{e}x")));
}

// Java: RegexPatternConverterTest.unicodeClass
#[test]
#[ignore = "needs Java regex syntax: \\p{IsEmoji} is not supported by the regex crate"]
fn unicode_class() {
    let regex = convert("\\p{L}{7}\\p{IsEmoji}").unwrap();
    assert!(regex.matches("äöüÄÖÜß🌱"));
}
