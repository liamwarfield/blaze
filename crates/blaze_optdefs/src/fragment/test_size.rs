//! Port of `packages/TestSize.java` (the parts option parsing and test configuration use).

use super::test_timeout::TestTimeout;

// Java: com/google/devtools/build/lib/packages/TestSize.java:28 TestSize
/// Possible test sizes.
///
/// Test size may affect the way how test is executed - e.g., it will determine default timeout
/// value and estimated local resource usage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(clippy::upper_case_acronyms)]
pub enum TestSize {
    /// Small tests use small amount of memory, but CPU intensive.
    SMALL,
    /// Medium tests tend to use larger amount of memory.
    MEDIUM,
    /// All other tests estimated to use fairly large amount of memory.
    LARGE,
    ENORMOUS,
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

    /// `Enum.name()`.
    pub fn name(self) -> &'static str {
        match self {
            TestSize::SMALL => "SMALL",
            TestSize::MEDIUM => "MEDIUM",
            TestSize::LARGE => "LARGE",
            TestSize::ENORMOUS => "ENORMOUS",
        }
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
