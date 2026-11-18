//! Port of src/test/java/com/google/devtools/common/options/PercentageConverterTest.java.

use blaze_options::Converter;
use blaze_options::converters::PercentageConverter;

fn convert(input: &str) -> Result<i32, blaze_options::OptionsParsingException> {
    PercentageConverter
        .convert(Some(input), None)
        .map(|v| v.unwrap().get::<i32>())
}

// Java: PercentageConverterTest.shouldReturnIntegerValue
#[test]
fn should_return_integer_value() {
    assert_eq!(convert("50").unwrap(), 50);
}

// Java: PercentageConverterTest.throwsExceptionWhenInputIsLessThanZero
#[test]
fn throws_exception_when_input_is_less_than_zero() {
    assert_eq!(
        convert("-1").unwrap_err().get_message(),
        "'-1' should be >= 0"
    );
}

// Java: PercentageConverterTest.throwsExceptionWhenInputIsGreaterThanHundred
#[test]
fn throws_exception_when_input_is_greater_than_hundred() {
    assert_eq!(
        convert("101").unwrap_err().get_message(),
        "'101' should be <= 100"
    );
}

// Java: PercentageConverterTest.throwsExceptionWhenInputIsNotANumber
#[test]
fn throws_exception_when_input_is_not_a_number() {
    assert_eq!(
        convert("oops - not a number.").unwrap_err().get_message(),
        "'oops - not a number.' is not an int"
    );
}
