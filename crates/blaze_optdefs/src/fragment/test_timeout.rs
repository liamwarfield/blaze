//! Port of `packages/TestTimeout.java` (the parts option parsing and test configuration use).

use std::collections::BTreeMap;
use std::time::Duration;

use blaze_options::{
    ConversionContext, Converter, OptionScalar, OptionsParsingException, Value,
    converter::non_null, java_parse_int, option_field_type, value::java_duration_to_string,
};

use super::converters::ConvResult;

// Java: com/google/devtools/build/lib/packages/TestTimeout.java:39 TestTimeout
/// Symbolic labels of test timeout. Borrows heavily from `TestSize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(clippy::upper_case_acronyms)]
pub enum TestTimeout {
    // These symbolic labels are used in the build files.
    SHORT,
    MODERATE,
    LONG,
    ETERNAL,
}

use TestTimeout::*;

// Java: com/google/devtools/build/lib/packages/TestTimeout.java:50 TestTimeout.COVERAGE_CMD_TIMEOUT
/// Default --test_timeout flag, used when collecting code coverage.
pub const COVERAGE_CMD_TIMEOUT: &str = "--test_timeout=300,600,1200,3600";

/// For the largest timeout, cap suggested and fuzzy ranges at one year.
const MAX_TIMEOUT: i32 = 365 * 24 * 60 * 60;

impl TestTimeout {
    /// `TestTimeout.values()`.
    pub const VALUES: [TestTimeout; 4] = [SHORT, MODERATE, LONG, ETERNAL];

    fn timeout(self) -> i32 {
        match self {
            SHORT => 60,
            MODERATE => 300,
            LONG => 900,
            ETERNAL => 3600,
        }
    }

    // Java: com/google/devtools/build/lib/packages/TestTimeout.java:67 TestTimeout.<clinit>
    /// `(suggested range, fuzzy range)` of this timeout, both `[min, max)`, as computed by the
    /// Java static initializer.
    fn ranges(self) -> ((i32, i32), (i32, i32)) {
        let mut previous_max_suggested = 0;
        let mut previous_timeout = 0;
        let mut it = Self::VALUES.iter().peekable();
        while let Some(&timeout) = it.next() {
            let has_next = it.peek().is_some();
            // This should be exactly the previous max because there should be exactly one
            // suggested timeout for any given time.
            let min_suggested = previous_max_suggested;
            // Only suggest timeouts that are less than 75% of the actual timeout (unless there are
            // no higher timeouts). This should be low enough to prevent suggested times from
            // causing test timeout flakiness.
            let max_suggested = if has_next {
                (timeout.timeout() as f64 * 0.75) as i32
            } else {
                MAX_TIMEOUT
            };
            // Set fuzzy minimum timeout to half the previous timeout. If the test is that fast, it
            // should be safe to use the shorter timeout.
            let min_fuzzy = previous_timeout / 2;
            // Set fuzzy maximum timeout to 90% of the timeout. A test this close to the limit can
            // easily become timeout flaky.
            let max_fuzzy = if has_next {
                (timeout.timeout() as f64 * 0.9) as i32
            } else {
                MAX_TIMEOUT
            };
            if timeout == self {
                return ((min_suggested, max_suggested), (min_fuzzy, max_fuzzy));
            }
            previous_max_suggested = max_suggested;
            previous_timeout = timeout.timeout();
        }
        unreachable!()
    }

    // Java: com/google/devtools/build/lib/packages/TestTimeout.java:123 TestTimeout.getTestTimeout
    /// Returns the enum associated with a test's timeout or null if the tag is not lower case or
    /// an unknown size.
    pub fn get_test_timeout(attr: &str) -> Option<TestTimeout> {
        if attr != attr.to_lowercase() {
            return None;
        }
        // TestTimeout.valueOf(attr.toUpperCase(Locale.ENGLISH))
        Self::VALUES
            .into_iter()
            .find(|t| t.name() == attr.to_uppercase())
    }

    /// `Enum.name()`.
    pub fn name(self) -> &'static str {
        match self {
            SHORT => "SHORT",
            MODERATE => "MODERATE",
            LONG => "LONG",
            ETERNAL => "ETERNAL",
        }
    }

    // Java: com/google/devtools/build/lib/packages/TestTimeout.java:167 TestTimeout.prettyPrint
    /// We print to upper case to make the test timeout warnings more readable.
    pub fn pretty_print(self) -> String {
        self.name().to_string()
    }

    // Java: com/google/devtools/build/lib/packages/TestTimeout.java:172 TestTimeout.getTimeoutSeconds
    pub fn get_timeout_seconds(self) -> i32 {
        self.timeout()
    }

    // Java: com/google/devtools/build/lib/packages/TestTimeout.java:176 TestTimeout.getTimeout
    pub fn get_timeout(self) -> Duration {
        Duration::from_secs(self.timeout() as u64)
    }

    // Java: com/google/devtools/build/lib/packages/TestTimeout.java:187 TestTimeout.isInRangeFuzzy
    /// Returns true iff the given time is not close to the upper bound timeout and is so short
    /// that it should be assigned a different timeout.
    pub fn is_in_range_fuzzy(self, time_in_seconds: i32) -> bool {
        let (_, (lo, hi)) = self.ranges();
        lo <= time_in_seconds && time_in_seconds < hi
    }

    // Java: com/google/devtools/build/lib/packages/TestTimeout.java:197 TestTimeout.getSuggestedTestTimeout
    /// Returns suggested test size for the given time in seconds.
    ///
    /// Will suggest times that are unlikely to result in timeout flakiness even if the test has a
    /// significant amount of time variance.
    pub fn get_suggested_test_timeout(time_in_seconds: i32) -> Option<TestTimeout> {
        Self::VALUES.into_iter().find(|t| {
            let ((lo, hi), _) = t.ranges();
            lo <= time_in_seconds && time_in_seconds < hi
        })
    }
}

impl std::fmt::Display for TestTimeout {
    // Java: com/google/devtools/build/lib/packages/TestTimeout.java:160 TestTimeout.toString
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name().to_lowercase())
    }
}

/// `Map<TestTimeout, Duration>` (an `EnumMap`, iterated in declaration order), the value of
/// [`TestTimeoutConverter`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TestTimeoutMap(pub BTreeMap<TestTimeout, Duration>);

impl OptionScalar for TestTimeoutMap {
    // Java: java/util/EnumMap.java toString (AbstractMap: `{short=PT1M, ...}`)
    fn java_to_string(&self) -> String {
        let parts: Vec<String> = self
            .0
            .iter()
            .map(|(k, v)| format!("{}={}", k, java_duration_to_string(*v)))
            .collect();
        format!("{{{}}}", parts.join(", "))
    }
}

option_field_type!(TestTimeoutMap, "java.util.Map");

// Java: com/google/devtools/build/lib/packages/TestTimeout.java:202 TestTimeout.TestTimeoutConverter
/// Converter for the --test_timeout option.
#[derive(Clone, Copy, Debug, Default)]
pub struct TestTimeoutConverter;

impl TestTimeoutConverter {
    // Java: com/google/devtools/build/lib/packages/TestTimeout.java:207 TestTimeout.TestTimeoutConverter.convert
    pub fn convert_map(&self, input: &str) -> Result<TestTimeoutMap, OptionsParsingException> {
        let mut values: Vec<Duration> = Vec::new();
        // Splitter.on(',').limit(6).split(input)
        for token in input.splitn(6, ',') {
            // Handle the case of "2," which is accepted as legal... Because Splitter.split is
            // lazy, there's no way of knowing if an empty string is a trailing or an intermediate
            // one, so we can't fully emulate String.split(String, 0).
            if !token.is_empty() || values.len() > 1 {
                match java_parse_int(token) {
                    // Duration.ofSeconds(int): negative values are kept (then replaced below).
                    Some(v) => values.push(duration_of_seconds(v)),
                    None => {
                        return Err(OptionsParsingException::with_cause(
                            format!("'{}' is not an int", input),
                            format!("For input string: \"{}\"", token),
                        ));
                    }
                }
            }
        }
        let mut timeouts = BTreeMap::new();
        if values.len() == 1 {
            timeouts.insert(SHORT, values[0]);
            timeouts.insert(MODERATE, values[0]);
            timeouts.insert(LONG, values[0]);
            timeouts.insert(ETERNAL, values[0]);
        } else if values.len() == 4 {
            timeouts.insert(SHORT, values[0]);
            timeouts.insert(MODERATE, values[1]);
            timeouts.insert(LONG, values[2]);
            timeouts.insert(ETERNAL, values[3]);
        } else {
            return Err(OptionsParsingException::new(
                "Invalid number of comma-separated entries",
            ));
        }
        for label in TestTimeout::VALUES {
            if timeouts.get(&label).is_none_or(|d| d.is_zero()) {
                timeouts.insert(label, label.get_timeout());
            }
        }
        Ok(TestTimeoutMap(timeouts))
    }
}

/// `Duration.ofSeconds(int)` for the converter: `std::time::Duration` cannot be negative, and a
/// non-positive value is always replaced by the default afterwards, so it maps to zero.
// PORT: Java keeps a negative Duration only transiently (compareTo(ZERO) <= 0 replaces it).
fn duration_of_seconds(v: i32) -> Duration {
    if v <= 0 {
        Duration::ZERO
    } else {
        Duration::from_secs(v as u64)
    }
}

impl Converter for TestTimeoutConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_map(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        "a single integer or comma-separated list of 4 integers".into()
    }
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/packages/TestTimeoutTest.java
    use super::*;

    #[test]
    fn test_basic_conversion() {
        assert_eq!(TestTimeout::get_test_timeout("short"), Some(SHORT));
        assert_eq!(TestTimeout::get_test_timeout("moderate"), Some(MODERATE));
        assert_eq!(TestTimeout::get_test_timeout("long"), Some(LONG));
        assert_eq!(TestTimeout::get_test_timeout("eternal"), Some(ETERNAL));
    }

    #[test]
    fn test_suggested_test_size() {
        let s = TestTimeout::get_suggested_test_timeout;
        assert_eq!(s(0), Some(SHORT));
        assert_eq!(s(30), Some(SHORT));
        assert_eq!(s(50), Some(MODERATE));
        assert_eq!(s(250), Some(LONG));
        assert_eq!(s(700), Some(ETERNAL));
        assert_eq!(s(60 * 60 * 24 * 360), Some(ETERNAL));
    }

    #[test]
    fn test_all_times_have_suggestions() {
        for timeout in 0..ETERNAL.get_timeout_seconds() {
            let suggested = TestTimeout::get_suggested_test_timeout(timeout).unwrap_or_else(|| {
                panic!("No suggested TestTimeout found for timeout {}", timeout)
            });
            assert!(
                suggested.is_in_range_fuzzy(timeout),
                "Suggested timeout {} is not in the fuzzy range for {}",
                suggested,
                timeout
            );
        }
    }

    #[test]
    fn test_is_in_range_fuzzy() {
        assert!(SHORT.is_in_range_fuzzy(0));
        assert!(SHORT.is_in_range_fuzzy(30));
        assert!(!SHORT.is_in_range_fuzzy(55));
        assert!(!MODERATE.is_in_range_fuzzy(10));
        assert!(MODERATE.is_in_range_fuzzy(40));
        assert!(!MODERATE.is_in_range_fuzzy(290));
        assert!(!LONG.is_in_range_fuzzy(30));
        assert!(LONG.is_in_range_fuzzy(200));
        assert!(!LONG.is_in_range_fuzzy(890));
        assert!(!ETERNAL.is_in_range_fuzzy(50));
        assert!(ETERNAL.is_in_range_fuzzy(500));
        assert!(ETERNAL.is_in_range_fuzzy(3500));
        assert!(ETERNAL.is_in_range_fuzzy(60 * 60 * 24 * 360));
    }

    #[test]
    fn test_all_fuzzy_ranges_covered() {
        for timeout in 0..ETERNAL.get_timeout_seconds() {
            assert!(
                TestTimeout::VALUES
                    .iter()
                    .any(|t| t.is_in_range_fuzzy(timeout)),
                "Timeout {} is not in any fuzzy range.",
                timeout
            );
        }
    }

    #[test]
    fn test_timeout_converter() {
        let c = TestTimeoutConverter;
        assert_eq!(
            c.convert_map("-1").unwrap().java_to_string(),
            "{short=PT1M, moderate=PT5M, long=PT15M, eternal=PT1H}"
        );
        assert_eq!(
            c.convert_map("10").unwrap().java_to_string(),
            "{short=PT10S, moderate=PT10S, long=PT10S, eternal=PT10S}"
        );
        assert_eq!(
            c.convert_map("1,0,3,4").unwrap().java_to_string(),
            "{short=PT1S, moderate=PT5M, long=PT3S, eternal=PT4S}"
        );
        assert_eq!(c.convert_map("2,").unwrap(), c.convert_map("2").unwrap());
        assert_eq!(
            c.convert_map("1,2").unwrap_err().get_message(),
            "Invalid number of comma-separated entries"
        );
        assert_eq!(
            c.convert_map("x").unwrap_err().get_message(),
            "'x' is not an int"
        );
    }
}
