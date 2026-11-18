//! Port of `analysis/test/TestShardingStrategy.java`, `TestShardingStrategyNotForced.java` and
//! `TestShardingStrategyForced.java`.

use blaze_options::{
    ConversionContext, Converter, IntegerConverter, OptionScalar, OptionsParsingException, Value,
    converter::non_null, option_field_type,
};

use super::converters::ConvResult;

// Java: com/google/devtools/build/lib/analysis/test/TestShardingStrategy.java:22 TestShardingStrategy
/// A strategy for running the same tests in many processes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(clippy::upper_case_acronyms)]
pub enum TestShardingStrategy {
    // Java: com/google/devtools/build/lib/analysis/test/TestShardingStrategyNotForced.java:20 TestShardingStrategyNotForced.EXPLICIT
    EXPLICIT,
    // Java: com/google/devtools/build/lib/analysis/test/TestShardingStrategyNotForced.java:27 TestShardingStrategyNotForced.DISABLED
    DISABLED,
    // Java: com/google/devtools/build/lib/analysis/test/TestShardingStrategyForced.java:17 TestShardingStrategyForced
    Forced(i32),
}

impl TestShardingStrategy {
    /// `TestShardingStrategyNotForced.values()`.
    const NOT_FORCED: [TestShardingStrategy; 2] = [
        TestShardingStrategy::EXPLICIT,
        TestShardingStrategy::DISABLED,
    ];

    // Java: com/google/devtools/build/lib/analysis/test/TestShardingStrategy.java:23 TestShardingStrategy.getNumberOfShards
    pub fn get_number_of_shards(&self, shard_count_from_attr: i32) -> i32 {
        match self {
            TestShardingStrategy::EXPLICIT => shard_count_from_attr.max(0),
            TestShardingStrategy::DISABLED => 0,
            TestShardingStrategy::Forced(n) => *n,
        }
    }
}

impl std::fmt::Display for TestShardingStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TestShardingStrategy::EXPLICIT => f.write_str("EXPLICIT"),
            TestShardingStrategy::DISABLED => f.write_str("DISABLED"),
            // Java: com/google/devtools/build/lib/analysis/test/TestShardingStrategyForced.java:30 TestShardingStrategyForced.toString
            TestShardingStrategy::Forced(n) => write!(f, "forced={}", n),
        }
    }
}

impl OptionScalar for TestShardingStrategy {
    fn java_to_string(&self) -> String {
        self.to_string()
    }
}

option_field_type!(
    TestShardingStrategy,
    "com.google.devtools.build.lib.analysis.test.TestShardingStrategy",
    raw = TestShardingStrategy::EXPLICIT
);

// Java: com/google/devtools/build/lib/analysis/test/TestShardingStrategy.java:26 TestShardingStrategy.ShardingStrategyConverter
/// Converts to [`TestShardingStrategy`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ShardingStrategyConverter;

const FORCED_PREFIX: &str = "forced=";

impl ShardingStrategyConverter {
    // Java: com/google/devtools/build/lib/analysis/test/TestShardingStrategy.java:35 TestShardingStrategy.ShardingStrategyConverter.convert
    pub fn convert_strategy(
        &self,
        input: &str,
    ) -> Result<TestShardingStrategy, OptionsParsingException> {
        for value in TestShardingStrategy::NOT_FORCED {
            if value.to_string().eq_ignore_ascii_case(input) {
                return Ok(value);
            }
        }
        if input.to_ascii_lowercase().starts_with(FORCED_PREFIX) {
            let forced_shards_count = IntegerConverter
                .convert(Some(&input[FORCED_PREFIX.len()..]), None)?
                .expect("IntegerConverter returns a value")
                .get::<i32>();
            if forced_shards_count < 0 {
                return Err(OptionsParsingException::new(
                    "Forced shards count cannot be negative.",
                ));
            }
            return Ok(TestShardingStrategy::Forced(forced_shards_count));
        }
        Err(OptionsParsingException::new(format!(
            "Not a valid test sharding strategy: '{}' (should be {})",
            input,
            self.get_type_description()
        )))
    }
}

impl Converter for ShardingStrategyConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_strategy(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        "explicit, disabled or forced=k where k is the number of shards to enforce".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_all_forms() {
        let c = ShardingStrategyConverter;
        assert_eq!(
            c.convert_strategy("explicit").unwrap(),
            TestShardingStrategy::EXPLICIT
        );
        assert_eq!(
            c.convert_strategy("DISABLED").unwrap(),
            TestShardingStrategy::DISABLED
        );
        assert_eq!(
            c.convert_strategy("Forced=3").unwrap(),
            TestShardingStrategy::Forced(3)
        );
        assert_eq!(
            c.convert_strategy("forced=-1").unwrap_err().get_message(),
            "Forced shards count cannot be negative."
        );
        assert_eq!(
            c.convert_strategy("x").unwrap_err().get_message(),
            "Not a valid test sharding strategy: 'x' (should be explicit, disabled or forced=k \
             where k is the number of shards to enforce)"
        );
    }
}
