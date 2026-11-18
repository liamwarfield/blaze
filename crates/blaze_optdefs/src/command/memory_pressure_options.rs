//! Transcription of `com/google/devtools/build/lib/runtime/MemoryPressureOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/MemoryPressureOptions.java:34 MemoryPressureOptions
options! {
    /// Options for responding to memory pressure.
    pub struct MemoryPressureOptions {
        #[option(
            name = "skyframe_high_water_mark_threshold",
            default_value = "85",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "Flag for advanced configuration of Bazel's internal Skyframe engine. If Bazel detects",
                " its retained heap percentage usage is at least this threshold, it will drop",
                " unnecessary temporary Skyframe state. Tweaking this may let you mitigate wall",
                " time impact of GC thrashing, when the GC thrashing is (i) caused by the memory",
                " usage of this temporary state and (ii) more costly than reconstituting the state",
                " when it is needed.",
            ),
        )]
        pub skyframe_high_water_mark_memory_threshold: i32,

        #[option(
            name = "skyframe_high_water_mark_minor_gc_drops_per_invocation",
            default_value = "10",
            converter = NonNegativeIntegerConverter,
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "Flag for advanced configuration of Bazel's internal Skyframe engine. If Bazel detects",
                " its retained heap percentage usage exceeds the threshold set by",
                " --skyframe_high_water_mark_threshold, when a minor GC event occurs, it will drop",
                " unnecessary temporary Skyframe state, up to this many times per invocation.",
                " Defaults to 10. Zero means that minor GC events will never trigger drops. If the",
                " limit is reached, Skyframe state will no longer be dropped when a minor GC event",
                " occurs and that retained heap percentage threshold is exceeded.",
            ),
        )]
        pub skyframe_high_water_mark_minor_gc_drops_per_invocation: i32,

        #[option(
            name = "skyframe_high_water_mark_full_gc_drops_per_invocation",
            default_value = "10",
            converter = NonNegativeIntegerConverter,
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "Flag for advanced configuration of Bazel's internal Skyframe engine. If Bazel detects",
                " its retained heap percentage usage exceeds the threshold set by",
                " --skyframe_high_water_mark_threshold, when a full GC event occurs, it will drop",
                " unnecessary temporary Skyframe state, up to this many times per invocation.",
                " Defaults to 10. Zero means that full GC events will never trigger drops. If the",
                " limit is reached, Skyframe state will no longer be dropped when a full GC event",
                " occurs and that retained heap percentage threshold is exceeded.",
            ),
        )]
        pub skyframe_high_water_mark_full_gc_drops_per_invocation: i32,

        #[option(
            name = "gc_thrashing_limits",
            default_value = "1s:2,20s:3,1m:5",
            converter = GcThrashingLimitsConverter,
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "Limits which, if reached, cause GcThrashingDetector to crash Bazel with an OOM. Each",
                " limit is specified as <period>:<count> where period is a duration and count is a",
                " positive integer. If more than --gc_thrashing_threshold percent of tenured space",
                " (old gen heap) remains occupied after <count> consecutive full GCs within",
                " <period>, an OOM is triggered. Multiple limits can be specified separated by",
                " commas.",
            ),
        )]
        pub gc_thrashing_limits: Vec<GcThrashingDetectorLimit>,

        #[option(
            name = "gc_thrashing_threshold",
            old_name = "experimental_oom_more_eagerly_threshold",
            old_name_warning = false,
            default_value = "100",
            converter = PercentageConverter,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "The percent of tenured space occupied (0-100) above which GcThrashingDetector considers",
                " memory pressure events against its limits (--gc_thrashing_limits). If set to 100,",
                " GcThrashingDetector is disabled.",
            ),
        )]
        pub gc_thrashing_threshold: i32,

        #[option(
            name = "gc_churning_threshold",
            default_value = "100",
            converter = PercentageConverter,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "At any point after an invocation has been running for at least one minute, if Blaze has",
                " spent at least this percentage of the invocation's wall time doing full GCs,",
                " Blaze will give up and fail with an OOM. A value of 100 effectively means to",
                " never give up for this reason.",
            ),
        )]
        pub gc_churning_threshold: i32,

        #[option(
            name = "gc_churning_threshold_if_multiple_top_level_targets",
            default_value = "-1",
            converter = OptionalPercentageConverter,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "If set to a value in [0, 100] and this is a command that takes top-level targets (e.g.",
                " build but not query) and there are multiple such top-level targets, overrides",
                " --gc_churning_threshold. Useful to configure more aggressive OOMing behavior",
                " (i.e. a lower value than --gc_churning_threshold) when they are multiple",
                " top-level targets so that the invoker of Bazel can split and retry while still",
                " having less aggressive behavior when there is a single top-level target.",
            ),
        )]
        pub gc_churning_threshold_if_multiple_top_level_targets: OptionalInt,

        #[option(
            name = "jvm_heap_histogram_internal_object_pattern",
            default_value = "jdk\\.internal\\.vm\\.Filler.+",
            converter = RegexPatternConverter,
            documentation_category = LOGGING,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Regex for overriding the matching logic for JDK21+ JVM heap memory",
                " collection. We are relying on volatile internal G1 GC implemenation",
                " details to get a clean memory metric, this option allows us to adapt",
                " to changes in that internal implementation without having to wait",
                " for a binary release.  Passed to JDK Matcher.find()",
            ),
        )]
        pub jvm_heap_histogram_internal_object_pattern: RegexPatternOption,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const MEMORY_PRESSURE_OPTIONS_OPTION_NAMES: &[&str] = &[
        "skyframe_high_water_mark_threshold",
        "skyframe_high_water_mark_minor_gc_drops_per_invocation",
        "skyframe_high_water_mark_full_gc_drops_per_invocation",
        "gc_thrashing_limits",
        "gc_thrashing_threshold",
        "gc_churning_threshold",
        "gc_churning_threshold_if_multiple_top_level_targets",
        "jvm_heap_histogram_internal_object_pattern",
    ];

    #[test]
    fn memory_pressure_options_option_names() {
        super::super::test_util::check_option_names(
            MemoryPressureOptions::class(),
            MEMORY_PRESSURE_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn memory_pressure_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<MemoryPressureOptions>(
            SKIP_ROUND_TRIP,
        );
    }
}

// ---- hand-written below this line (preserved by the generator) ----

// Java: com/google/devtools/build/lib/runtime/MemoryPressureOptions.java:156 MemoryPressureOptions.NonNegativeIntegerConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct NonNegativeIntegerConverter;

impl Converter for NonNegativeIntegerConverter {
    fn convert(&self, input: Option<&str>, c: Option<&ConversionContext>) -> ConvResult {
        RangeConverter::new(0, i32::MAX).convert(input, c)
    }
    fn get_type_description(&self) -> String {
        RangeConverter::new(0, i32::MAX).get_type_description()
    }
}

// Java: com/google/devtools/build/lib/runtime/MemoryPressureOptions.java:162 MemoryPressureOptions.GcThrashingLimitsConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct GcThrashingLimitsConverter;

impl GcThrashingLimitsConverter {
    // Java: com/google/devtools/build/lib/runtime/MemoryPressureOptions.java:170 MemoryPressureOptions.GcThrashingLimitsConverter.convert
    pub fn convert_limits(
        &self,
        input: &str,
    ) -> Result<Vec<GcThrashingDetectorLimit>, OptionsParsingException> {
        let positive_int_converter = RangeConverter::new(1, i32::MAX);
        let mut result = Vec::new();
        for part in CommaSeparatedOptionListConverter.convert_list(input)? {
            let Some(colon_index) = part.find(':') else {
                return Err(OptionsParsingException::new(format!(
                    "Expected <period>:<count>, got {}",
                    part
                )));
            };
            let period = DurationConverter
                .convert(Some(&part[..colon_index]), None)?
                .expect("DurationConverter never returns null")
                .get::<Duration>();
            let count = positive_int_converter.convert_int(&part[colon_index + 1..])?;
            result.push(GcThrashingDetectorLimit::of(period, count));
        }
        Ok(result)
    }
}

impl Converter for GcThrashingLimitsConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::list_of(self.convert_limits(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        "comma separated pairs of <period>:<count>".into()
    }
}

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, parse_err};
    use super::*;

    #[test]
    fn gc_thrashing_limits() {
        let d = MemoryPressureOptions::default();
        assert_eq!(
            d.gc_thrashing_limits,
            vec![
                GcThrashingDetectorLimit::of(Duration::from_secs(1), 2),
                GcThrashingDetectorLimit::of(Duration::from_secs(20), 3),
                GcThrashingDetectorLimit::of(Duration::from_secs(60), 5),
            ]
        );
        assert_eq!(
            parse_err::<MemoryPressureOptions>(&["--gc_thrashing_limits=5"]),
            "While parsing option --gc_thrashing_limits=5: Expected <period>:<count>, got 5"
        );
        assert!(
            parse::<MemoryPressureOptions>(&["--gc_thrashing_limits="])
                .gc_thrashing_limits
                .is_empty()
        );
        let o = parse::<MemoryPressureOptions>(&["--experimental_oom_more_eagerly_threshold=90"]);
        assert_eq!(o.gc_thrashing_threshold, 90);
        assert_eq!(
            d.gc_churning_threshold_if_multiple_top_level_targets,
            OptionalInt(None)
        );
    }
}
