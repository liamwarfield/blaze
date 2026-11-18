//! Port of src/test/java/com/google/devtools/common/options/LogLevelConverterTest.java.

use blaze_options::Converter;
use blaze_options::converters::{LogLevel, LogLevelConverter};

fn convert(input: &str) -> Result<LogLevel, blaze_options::OptionsParsingException> {
    LogLevelConverter
        .convert(Some(input), None)
        .map(|v| v.unwrap().get::<LogLevel>())
}

// Java: LogLevelConverterTest.convertsIntsToLevels
#[test]
fn converts_ints_to_levels() {
    for (level_id, level) in LogLevelConverter::LEVELS.iter().enumerate() {
        assert_eq!(convert(&level_id.to_string()).unwrap(), *level);
    }
}

// Java: LogLevelConverterTest.throwsExceptionWhenInputIsNotANumber
#[test]
fn throws_exception_when_input_is_not_a_number() {
    let e = convert("oops - not a number.").unwrap_err();
    assert_eq!(e.get_message(), "Not a log level: oops - not a number.");
}

// Java: LogLevelConverterTest.throwsExceptionWhenInputIsInvalidInteger
#[test]
fn throws_exception_when_input_is_invalid_integer() {
    for example in [-1, 100, 50000] {
        let e = convert(&example.to_string()).unwrap_err();
        assert_eq!(e.get_message(), format!("Not a log level: {}", example));
    }
}
