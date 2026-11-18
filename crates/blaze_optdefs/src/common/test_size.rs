//! Port of `com/google/devtools/build/lib/packages/TestSize.java` (the enum, the parts option
//! parsing and test configuration use, and its filter converter).

use blaze_options::converters::JavaEnum;
use blaze_options::{ConversionContext, Converter, OptionsParsingException, Value, option_enum};

use super::enum_filter_converter::EnumFilterConverter;
use super::test_timeout::TestTimeout;

type ConvResult = Result<Option<Value>, OptionsParsingException>;

// Java: com/google/devtools/build/lib/packages/TestSize.java:28 TestSize
option_enum! {
    /// Possible test sizes.
    ///
    /// Test size may affect the way how test is executed - e.g., it will determine default timeout
    /// value and estimated local resource usage. `toString()` is the lower-case name.
    pub enum TestSize {
        /// Small tests use small amount of memory, but CPU intensive.
        SMALL = "small",
        /// Medium tests tend to use larger amount of memory.
        MEDIUM = "medium",
        /// All other tests estimated to use fairly large amount of memory.
        LARGE = "large",
        ENORMOUS = "enormous",
    }
}

impl TestSize {
    /// `TestSize.values()`.
    pub const VALUES: [TestSize; 4] = [
        TestSize::SMALL,
        TestSize::MEDIUM,
        TestSize::LARGE,
        TestSize::ENORMOUS,
    ];

    /// `Enum.ordinal()`.
    pub fn ordinal(self) -> usize {
        self as usize
    }

    // Java: com/google/devtools/build/lib/packages/TestSize.java:60 TestSize.getDefaultTimeout
    /// Returns default timeout in seconds.
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

    // Java: com/google/devtools/build/lib/packages/TestSize.java:82 TestSize.getTestSize
    /// Returns [`TestSize`] matching the given timeout or null if the given timeout doesn't match
    /// any [`TestSize`].
    pub fn get_test_size_for_timeout(timeout: TestTimeout) -> Option<TestSize> {
        Self::VALUES
            .into_iter()
            .find(|s| s.get_default_timeout() == timeout)
    }

    // Java: com/google/devtools/build/lib/packages/TestSize.java:96 TestSize.getTestSize
    /// Returns the enum associated with a test's size or null if the tag is not lower case or an
    /// unknown size.
    pub fn get_test_size(attr: &str) -> Option<TestSize> {
        if attr != attr.to_lowercase() {
            return None;
        }
        Self::VALUES
            .into_iter()
            .find(|s| s.name().to_lowercase() == attr)
    }
}

impl std::fmt::Display for TestSize {
    // Java: com/google/devtools/build/lib/packages/TestSize.java:109 TestSize.toString
    /// Normal practice is to always use size tags as lower case strings.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name().to_lowercase())
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
    use blaze_options::OptionScalar;

    #[test]
    fn filter_converter() {
        let c = EnumFilterConverter::<TestSize>::new("test size");
        assert_eq!(
            c.convert_set("small,-large").unwrap().java_to_string(),
            "[small]"
        );
        assert_eq!(
            c.convert_set("-small").unwrap().java_to_string(),
            "[medium, large, enormous]"
        );
        assert_eq!(c.convert_set("").unwrap().java_to_string(), "[]");
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
