//! Port of src/test/java/com/google/devtools/common/options/ByteSizeConverterTest.java.

use blaze_options::converters::ByteSizeConverter;
use blaze_options::{Converter, OptionsParsingException};

fn convert(input: &str) -> Result<i64, OptionsParsingException> {
    ByteSizeConverter
        .convert(Some(input), None)
        .map(|v| v.unwrap().get::<i64>())
}

// Java: ByteSizeConverterTest.empty
#[test]
fn empty() {
    assert!(convert("").is_err());
}

// Java: ByteSizeConverterTest.zero
#[test]
fn zero() {
    assert_eq!(convert("0").unwrap(), 0);
    assert_eq!(convert("00").unwrap(), 0);
}

// Java: ByteSizeConverterTest.negative
#[test]
fn negative() {
    assert!(convert("-1").is_err());
}

// Java: ByteSizeConverterTest.fractional
#[test]
fn fractional() {
    assert!(convert("1.1").is_err());
}

// Java: ByteSizeConverterTest.nonDecimal
#[test]
fn non_decimal() {
    assert!(convert("1f").is_err());
}

// Java: ByteSizeConverterTest.noSuffix
#[test]
fn no_suffix() {
    assert_eq!(convert("123").unwrap(), 123);
}

// Java: ByteSizeConverterTest.kiloSuffix
#[test]
fn kilo_suffix() {
    assert_eq!(convert("123K").unwrap(), 123 * 1024);
}

// Java: ByteSizeConverterTest.megaSuffix
#[test]
fn mega_suffix() {
    assert_eq!(convert("123M").unwrap(), 123 * 1024 * 1024);
}

// Java: ByteSizeConverterTest.gigaSuffix
#[test]
fn giga_suffix() {
    assert_eq!(convert("123G").unwrap(), 123 * 1024 * 1024 * 1024);
}

// Java: ByteSizeConverterTest.teraSuffix
#[test]
fn tera_suffix() {
    assert_eq!(convert("123T").unwrap(), 123 * 1024 * 1024 * 1024 * 1024);
}

// Java: ByteSizeConverterTest.noSuffixOverflow
#[test]
fn no_suffix_overflow() {
    assert!(convert(&(1u128 << 63).to_string()).is_err());
}

// Java: ByteSizeConverterTest.kiloOverflow
#[test]
fn kilo_overflow() {
    assert!(convert(&format!("{}K", 1u128 << 53)).is_err());
}

// Java: ByteSizeConverterTest.megaOverflow
#[test]
fn mega_overflow() {
    assert!(convert(&format!("{}M", 1u128 << 43)).is_err());
}

// Java: ByteSizeConverterTest.gigaOverflow
#[test]
fn giga_overflow() {
    assert!(convert(&format!("{}G", 1u128 << 33)).is_err());
}

// Java: ByteSizeConverterTest.teraOverflow
#[test]
fn tera_overflow() {
    assert!(convert(&format!("{}T", 1u128 << 23)).is_err());
}
