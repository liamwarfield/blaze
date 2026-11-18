//! `MemoryProfiler.MemoryProfileStableHeapParameters` and its converter, from
//! `com/google/devtools/build/lib/profiler/MemoryProfiler.java`.

use super::prelude::*;

// Java: com/google/devtools/build/lib/profiler/MemoryProfiler.java:161 MemoryProfiler.MemoryProfileStableHeapParameters
/// Parameters that control how `MemoryProfiler` tries to get a stable heap at the end of the
/// build.
// PORT: Java has no equals() (identity); structural equality is used here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryProfileStableHeapParameters {
    /// Pairs of (number of times to GC, time to sleep between GCs).
    pub gc_specs: Vec<(i32, Duration)>,
}

// Java: com/google/devtools/build/lib/profiler/MemoryProfiler.java:217 MemoryProfiler.MemoryProfileStableHeapParameters.toString
impl OptionScalar for MemoryProfileStableHeapParameters {
    fn java_to_string(&self) -> String {
        // MoreObjects.toStringHelper(this).add("gcSpecs", gcSpecs); Pair.toString is "(a, b)".
        format!(
            "MemoryProfileStableHeapParameters{{gcSpecs={}}}",
            blaze_options::value::java_list_to_string(self.gc_specs.iter().map(|(n, d)| format!(
                "({}, {})",
                n,
                blaze_options::value::java_duration_to_string(*d)
            )))
        )
    }
}
option_field_type!(
    MemoryProfileStableHeapParameters,
    "com.google.devtools.build.lib.profiler.MemoryProfiler$MemoryProfileStableHeapParameters",
    raw = MemoryProfileStableHeapParameters { gc_specs: vec![] }
);

// Java: com/google/devtools/build/lib/profiler/MemoryProfiler.java:169 MemoryProfiler.MemoryProfileStableHeapParameters.Converter
/// Converter for `MemoryProfileStableHeapParameters` option.
#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryProfileStableHeapParametersConverter;

impl MemoryProfileStableHeapParametersConverter {
    pub fn convert_parameters(
        &self,
        input: &str,
    ) -> Result<MemoryProfileStableHeapParameters, OptionsParsingException> {
        // Splitter.on(',').splitToList keeps empty pieces.
        let values: Vec<&str> = input.split(',').collect();
        if !values.len().is_multiple_of(2) {
            return Err(OptionsParsingException::new(
                "Expected even number of comma-separated integer values",
            ));
        }
        let parse_error = || {
            OptionsParsingException::new(
                "Expected even number of comma-separated integer values, could not parse integer in list",
            )
        };
        let mut gc_specs = Vec::with_capacity(values.len() / 2);
        for pair in values.chunks(2) {
            let num_times_to_do_gc = java_parse_int(pair[0]).ok_or_else(parse_error)?;
            let num_seconds_to_sleep_between_gcs =
                java_parse_int(pair[1]).ok_or_else(parse_error)?;
            if num_times_to_do_gc <= 0 {
                return Err(OptionsParsingException::new(
                    "Number of times to GC must be positive",
                ));
            }
            if num_seconds_to_sleep_between_gcs < 0 {
                return Err(OptionsParsingException::new(
                    "Number of seconds to sleep between GC's must be non-negative",
                ));
            }
            gc_specs.push((
                num_times_to_do_gc,
                Duration::from_secs(num_seconds_to_sleep_between_gcs as u64),
            ));
        }
        Ok(MemoryProfileStableHeapParameters { gc_specs })
    }
}

impl Converter for MemoryProfileStableHeapParametersConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_parameters(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        "integers, separated by a comma expected in pairs".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_pairs() {
        let c = MemoryProfileStableHeapParametersConverter;
        let p = c.convert_parameters("1,0").unwrap();
        assert_eq!(
            OptionScalar::java_to_string(&p),
            "MemoryProfileStableHeapParameters{gcSpecs=[(1, PT0S)]}"
        );
        assert_eq!(
            c.convert_parameters("1").unwrap_err().get_message(),
            "Expected even number of comma-separated integer values"
        );
        assert_eq!(
            c.convert_parameters("0,1").unwrap_err().get_message(),
            "Number of times to GC must be positive"
        );
        assert_eq!(
            c.convert_parameters("a,1").unwrap_err().get_message(),
            "Expected even number of comma-separated integer values, could not parse integer in list"
        );
    }
}
