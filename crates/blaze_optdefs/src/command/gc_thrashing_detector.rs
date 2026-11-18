//! `GcThrashingDetector.Limit` from `com/google/devtools/build/lib/runtime/GcThrashingDetector.java`.

use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/GcThrashingDetector.java:64 GcThrashingDetector.Limit
/// A period and the number of full GCs allowed in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GcThrashingDetectorLimit {
    pub period: Duration,
    pub count: i32,
}

impl GcThrashingDetectorLimit {
    // Java: com/google/devtools/build/lib/runtime/GcThrashingDetector.java:72 GcThrashingDetector.Limit.of
    pub fn of(period: Duration, count: i32) -> Self {
        // Java's compact constructor (checkArgument: IllegalArgumentException).
        blaze_util::check_argument!(
            !period.is_zero(),
            "period must be positive: {}",
            blaze_options::value::java_duration_to_string(period)
        );
        blaze_util::check_argument!(count > 0, "count must be positive: {}", count);
        GcThrashingDetectorLimit { period, count }
    }
}

// Java record toString.
impl OptionScalar for GcThrashingDetectorLimit {
    fn java_to_string(&self) -> String {
        format!(
            "Limit[period={}, count={}]",
            blaze_options::value::java_duration_to_string(self.period),
            self.count
        )
    }
}
option_field_type!(
    GcThrashingDetectorLimit,
    "com.google.devtools.build.lib.runtime.GcThrashingDetector$Limit",
    raw = GcThrashingDetectorLimit {
        period: Duration::ZERO,
        count: 0,
    }
);
