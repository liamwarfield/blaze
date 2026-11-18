//! Port of `com/google/devtools/build/lib/packages/TestSize.java` (the enum and its filter converter).

use super::enum_filter_converter::EnumFilterConverter;
use super::prelude::*;

// Java: com/google/devtools/build/lib/packages/TestSize.java:28 TestSize
option_enum! {
    /// Possible test sizes. Normal practice is to always use size tags as lower case strings
    /// (`toString()` is lower case).
    pub enum TestSize { SMALL = "small", MEDIUM = "medium", LARGE = "large", ENORMOUS = "enormous" }
}

impl TestSize {
    // Java: com/google/devtools/build/lib/packages/TestSize.java:60 TestSize.getDefaultTimeout
    /// Returns default timeout.
    pub fn get_default_timeout(self) -> TestTimeout {
        match self {
            TestSize::SMALL => TestTimeout::SHORT,
            TestSize::MEDIUM => TestTimeout::MODERATE,
            TestSize::LARGE => TestTimeout::LONG,
            TestSize::ENORMOUS => TestTimeout::ETERNAL,
        }
    }

    // Java: com/google/devtools/build/lib/packages/TestSize.java:67 TestSize.getDefaultShards
    /// Returns default number of shards.
    pub fn get_default_shards(self) -> i32 {
        match self {
            TestSize::SMALL => 2,
            TestSize::MEDIUM => 10,
            TestSize::LARGE => 20,
            TestSize::ENORMOUS => 30,
        }
    }
}

// Java: com/google/devtools/build/lib/packages/TestSize.java:116 TestSize.TestSizeFilterConverter
/// Converter for the --test_size_filters option.
#[derive(Clone, Copy, Debug, Default)]
pub struct TestSizeFilterConverter;

impl Converter for TestSizeFilterConverter {
    fn convert(&self, input: Option<&str>, c: Option<&ConversionContext>) -> ConvResult {
        EnumFilterConverter::<TestSize>::new("test size").convert(input, c)
    }
    fn get_type_description(&self) -> String {
        EnumFilterConverter::<TestSize>::new("test size").get_type_description()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_converter() {
        let c = EnumFilterConverter::<TestSize>::new("test size");
        assert_eq!(
            OptionScalar::java_to_string(&c.convert_set("small,-large").unwrap()),
            "[small]"
        );
        assert_eq!(
            OptionScalar::java_to_string(&c.convert_set("-small").unwrap()),
            "[medium, large, enormous]"
        );
        assert_eq!(
            OptionScalar::java_to_string(&c.convert_set("").unwrap()),
            "[]"
        );
        assert_eq!(
            c.convert_set("small,").unwrap_err().get_message(),
            "Invalid test size filter '' in the input 'small,'"
        );
        assert_eq!(
            c.convert_set("small,-small").unwrap_err().get_message(),
            "Test size filter 'small,-small' definition cannot match any tests"
        );
        assert_eq!(
            TestSizeFilterConverter.get_type_description(),
            "comma-separated list of values: small, medium, large, or enormous"
        );
    }
}
