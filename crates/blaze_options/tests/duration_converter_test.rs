//! Port of src/test/java/com/google/devtools/common/options/DurationConverterTest.java.

use std::time::Duration;

use blaze_options::Converter;
use blaze_options::converters::DurationConverter;

fn convert(input: &str) -> Result<Duration, blaze_options::OptionsParsingException> {
    DurationConverter
        .convert(Some(input), None)
        .map(|v| v.unwrap().get::<Duration>())
}

// Java: DurationConverterTest.testDurationConverter_zero
#[test]
fn test_duration_converter_zero() {
    for s in ["0", "0d", "0h", "0m", "0s", "0ms"] {
        assert_eq!(convert(s).unwrap(), Duration::ZERO);
    }
}

// Java: DurationConverterTest.testDurationConverter_basic
#[test]
fn test_duration_converter_basic() {
    assert_eq!(convert("10d").unwrap(), Duration::from_secs(10 * 86400));
    assert_eq!(convert("20h").unwrap(), Duration::from_secs(20 * 3600));
    assert_eq!(convert("30m").unwrap(), Duration::from_secs(30 * 60));
    assert_eq!(convert("40s").unwrap(), Duration::from_secs(40));
    assert_eq!(convert("50ms").unwrap(), Duration::from_millis(50));
    assert_eq!(convert("60ns").unwrap(), Duration::from_nanos(60));
}

// Java: DurationConverterTest.testDurationConverter_invalidInputs
#[test]
fn test_duration_converter_invalid_inputs() {
    assert!(convert("").is_err());
    assert!(convert("-10d").is_err());
    assert!(convert("h").is_err());
    assert!(convert("1g").is_err());
}
