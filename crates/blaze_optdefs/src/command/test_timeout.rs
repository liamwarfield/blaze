//! The enum and filter converter of `com/google/devtools/build/lib/packages/TestTimeout.java`.

use super::enum_filter_converter::EnumFilterConverter;
use super::prelude::*;

// Java: com/google/devtools/build/lib/packages/TestTimeout.java:39 TestTimeout
option_enum! {
    /// Symbolic labels of test timeout (`toString()` is lower case).
    pub enum TestTimeout { SHORT = "short", MODERATE = "moderate", LONG = "long", ETERNAL = "eternal" }
}

// Java: com/google/devtools/build/lib/packages/TestTimeout.java:252 TestTimeout.TestTimeoutFilterConverter
/// Converter for the --test_timeout_filters option.
#[derive(Clone, Copy, Debug, Default)]
pub struct TestTimeoutFilterConverter;

impl Converter for TestTimeoutFilterConverter {
    fn convert(&self, input: Option<&str>, c: Option<&ConversionContext>) -> ConvResult {
        EnumFilterConverter::<TestTimeout>::new("test timeout").convert(input, c)
    }
    fn get_type_description(&self) -> String {
        EnumFilterConverter::<TestTimeout>::new("test timeout").get_type_description()
    }
}
