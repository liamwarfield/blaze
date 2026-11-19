//! Ports of `CounterSeriesTask.java`, `TimeSeries.java` and `CounterSeriesTraceData.java`.

use std::io::{self, Write};
use std::time::Duration;

use parking_lot::Mutex;

use super::json_writer::JsonWriter;
use super::trace_data::TraceData;

macro_rules! colors {
    ($($name:ident = $value:expr),* $(,)?) => {
        /// The revered color for rendering the bar chart.
        // Java: com/google/devtools/build/lib/profiler/CounterSeriesTask.java:31 CounterSeriesTask.Color
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum CounterSeriesTaskColor {
            $($name),*
        }

        impl CounterSeriesTaskColor {
            pub fn value(self) -> &'static str {
                match self {
                    $(CounterSeriesTaskColor::$name => $value),*
                }
            }
        }
    };
}

colors! {
    // Pick acceptable counter colors manually, unfortunately we have to pick from these
    // weird reserved names from
    // https://github.com/catapult-project/catapult/blob/master/tracing/tracing/base/color_scheme.html
    THREAD_STATE_UNINTERRUPTIBLE = "thread_state_uninterruptible",
    THREAD_STATE_IOWAIT = "thread_state_iowait",
    THREAD_STATE_RUNNING = "thread_state_running",
    THREAD_STATE_RUNNABLE = "thread_state_runnable",
    THREAD_STATE_SLEEPING = "thread_state_sleeping",
    THREAD_STATE_UNKNOWN = "thread_state_unknown",
    BACKGROUND_MEMORY_DUMP = "background_memory_dump",
    LIGHT_MEMORY_DUMP = "light_memory_dump",
    DETAILED_MEMORY_DUMP = "detailed_memory_dump",
    VSYNC_HIGHLIGHT_COLOR = "vsync_highlight_color",
    GENERIC_WORK = "generic_work",
    GOOD = "good",
    BAD = "bad",
    TERRIBLE = "terrible",
    BLACK = "black",
    GREY = "grey",
    WHITE = "white",
    YELLOW = "yellow",
    OLIVE = "olive",
    RAIL_RESPONSE = "rail_response",
    RAIL_ANIMATION = "rail_animation",
    RAIL_IDLE = "rail_idle",
    RAIL_LOAD = "rail_load",
    STARTUP = "startup",
    HEAP_DUMP_STACK_FRAME = "heap_dump_stack_frame",
    HEAP_DUMP_OBJECT_TYPE = "heap_dump_object_type",
    HEAP_DUMP_CHILD_NODE_ARROW = "heap_dump_child_node_arrow",
    CQ_BUILD_RUNNING = "cq_build_running",
    CQ_BUILD_PASSED = "cq_build_passed",
    CQ_BUILD_FAILED = "cq_build_failed",
    CQ_BUILD_ABANDONED = "cq_build_abandoned",
    CQ_BUILD_ATTEMPT_RUNNIG = "cq_build_attempt_runnig",
    CQ_BUILD_ATTEMPT_PASSED = "cq_build_attempt_passed",
    CQ_BUILD_ATTEMPT_FAILED = "cq_build_attempt_failed",
}

/// Describes counter series to be logged into profile.
///
/// `lane_name`: series with the same lane name should be stacked when displaying.
/// `color`: if `None`, the profile viewer will pick a color automatically.
// Java: com/google/devtools/build/lib/profiler/CounterSeriesTask.java:29 CounterSeriesTask
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CounterSeriesTask {
    lane_name: String,
    series_name: String,
    color: Option<CounterSeriesTaskColor>,
}

impl CounterSeriesTask {
    pub fn new(
        lane_name: impl Into<String>,
        series_name: impl Into<String>,
        color: Option<CounterSeriesTaskColor>,
    ) -> Self {
        CounterSeriesTask {
            lane_name: lane_name.into(),
            series_name: series_name.into(),
            color,
        }
    }

    pub fn lane_name(&self) -> &str {
        &self.lane_name
    }

    pub fn series_name(&self) -> &str {
        &self.series_name
    }

    pub fn color(&self) -> Option<CounterSeriesTaskColor> {
        self.color
    }
}

/// Converts a set of ranges into a graph by counting the number of ranges that are active at any
/// point in time. Time is split into equal-sized buckets, and we compute one value per bucket. If
/// a range partially overlaps a bucket, then the bucket is incremented by the fraction of
/// overlap.
// Java: com/google/devtools/build/lib/profiler/TimeSeries.java:28 TimeSeries
pub struct TimeSeries {
    /// `startTime`, in signed nanoseconds (Java's `Duration` is signed).
    start_time: i128,
    bucket_size_millis: i64,
    data: Mutex<Vec<f64>>,
}

const INITIAL_SIZE: usize = 100;

/// A Java `Duration` (signed nanoseconds); `toMillis` truncates toward negative infinity for
/// negative durations (`Math.floorDiv` on seconds plus nanos).
fn to_millis(nanos: i128) -> i64 {
    nanos.div_euclid(1_000_000) as i64
}

impl TimeSeries {
    // Java: com/google/devtools/build/lib/profiler/TimeSeries.java:36 TimeSeries.<init>
    pub fn new(start_time: Duration, bucket_duration: Duration) -> Self {
        Self::new_nanos(start_time.as_nanos() as i64, bucket_duration)
    }

    /// `new TimeSeries(Duration.ofNanos(startTimeNanos), bucketDuration)`.
    pub fn new_nanos(start_time_nanos: i64, bucket_duration: Duration) -> Self {
        TimeSeries {
            start_time: start_time_nanos as i128,
            bucket_size_millis: bucket_duration.as_millis() as i64,
            data: Mutex::new(vec![0.0; INITIAL_SIZE]),
        }
    }

    // Java: com/google/devtools/build/lib/profiler/TimeSeries.java:41 TimeSeries.addRange
    pub fn add_range(&self, start_time: Duration, end_time: Duration) {
        self.add_range_value(start_time, end_time, 1.0);
    }

    /// Adds a new range to the time series, by increasing every affected bucket by value.
    // Java: com/google/devtools/build/lib/profiler/TimeSeries.java:46 TimeSeries.addRange
    pub fn add_range_value(&self, range_start: Duration, range_end: Duration, value: f64) {
        // Compute times relative to start and their positions in the data array.
        let start = self.start_time;
        let range_start = range_start.as_nanos() as i128 - start;
        let range_end = range_end.as_nanos() as i128 - start;
        self.add_relative_range(range_start, range_end, value);
    }

    /// `addRange` with times relative to the series' start, in signed nanoseconds.
    fn add_relative_range(&self, range_start: i128, range_end: i128, value: f64) {
        let bucket = self.bucket_size_millis;
        let mut start_position = (to_millis(range_start) / bucket) as i32;
        let mut end_position = (to_millis(range_end) / bucket) as i32;

        // Assume we add the following range R:
        // ----------------------------------
        // |     |ssRRR|RRRRR|Reeee|      |
        // ----------------------------------
        // we cannot just add value to each affected bucket but have to correct the values for the
        // first and last bucket by calculating the size of 's' and 'e'.
        let mut missing_start_fraction = (to_millis(
            range_start - (bucket as i128 * start_position as i128) * 1_000_000,
        ) as f64)
            / bucket as f64;
        let mut missing_end_fraction = ((bucket * (end_position as i64 + 1)
            - to_millis(range_end)) as f64)
            / bucket as f64;

        if start_position < 0 {
            start_position = 0;
            missing_start_fraction = 0.0;
        }
        if end_position < start_position {
            end_position = start_position;
            missing_end_fraction = 0.0;
        }

        let mut data = self.data.lock();
        // Resize data array if necessary so it can at least fit endPosition.
        if end_position as usize >= data.len() {
            let new_len = (end_position as usize + 1).max(2 * data.len());
            data.resize(new_len, 0.0);
        }

        // Do the actual update.
        for i in start_position..=end_position {
            let mut fraction = 1.0;
            if i == start_position {
                fraction -= missing_start_fraction;
            }
            if i == end_position {
                fraction -= missing_end_fraction;
            }
            data[i as usize] += fraction * value;
        }
    }

    /// `addRange(Duration, Duration, double)` for ranges that may start before the series
    /// (Java's `Duration` is signed): times are signed nanoseconds on the series' clock.
    pub fn add_range_nanos(&self, range_start_nanos: i64, range_end_nanos: i64, value: f64) {
        let start = self.start_time;
        self.add_relative_range(
            range_start_nanos as i128 - start,
            range_end_nanos as i128 - start,
            value,
        );
    }

    // Java: com/google/devtools/build/lib/profiler/TimeSeries.java:92 TimeSeries.toDoubleArray
    pub fn to_double_array(&self, len: usize) -> Vec<f64> {
        let data = self.data.lock();
        let mut out = vec![0.0; len];
        let n = len.min(data.len());
        out[..n].copy_from_slice(&data[..n]);
        out
    }
}

/// Used to inject a whole counter series (or even multiple) in one go in the JSON trace profile;
/// these could be used to represent CPU or memory usages over the course of an invocation (as
/// opposed to individual tasks such as executing an action).
// Java: com/google/devtools/build/lib/profiler/CounterSeriesTraceData.java:31 CounterSeriesTraceData
pub struct CounterSeriesTraceData {
    counter_series_map: Vec<(CounterSeriesTask, Vec<f64>)>,
    /// `profileStart` (a signed `Duration` on the profiler's clock), in nanoseconds.
    profile_start: i64,
    bucket_duration: Duration,
    len: i32,
    thread_id: i64,
    display_name: Option<String>,
    color_name: Option<&'static str>,
}

impl CounterSeriesTraceData {
    pub const PROCESS_ID: i64 = 1;

    /// If multiple series are passed: - they will be rendered as stacked chart; - we assume they
    /// all have the same length; - display name and color are picked from the first profile task
    /// in the map. However, colors the remaining series are picked arbitrarily by the Trace
    /// renderer.
    // Java: com/google/devtools/build/lib/profiler/CounterSeriesTraceData.java:47 CounterSeriesTraceData.<init>
    pub fn new(
        counter_series_map: Vec<(CounterSeriesTask, Vec<f64>)>,
        profile_start: i64,
        bucket_duration: Duration,
    ) -> Self {
        let mut len: i32 = -1;
        let mut display_name = None;
        let mut color_name = None;
        for (task, values) in &counter_series_map {
            if len == -1 {
                len = values.len() as i32;
                display_name = Some(task.lane_name().to_string());
                if let Some(color) = task.color() {
                    color_name = Some(color.value());
                }
            } else {
                // Check that second and subsequent series have the same length as the first.
                crate::check_state!(len == values.len() as i32);
            }
        }
        CounterSeriesTraceData {
            counter_series_map,
            profile_start,
            bucket_duration,
            len,
            thread_id: super::thread_metadata::current_thread_id(),
            display_name,
            color_name,
        }
    }
}

impl TraceData for CounterSeriesTraceData {
    // Java: com/google/devtools/build/lib/profiler/CounterSeriesTraceData.java:75 CounterSeriesTraceData.writeTraceData
    fn write_trace_data(
        &self,
        json_writer: &mut JsonWriter<&mut dyn Write>,
        profile_start_time_nanos: i64,
    ) -> io::Result<()> {
        // See
        // https://docs.google.com/document/d/1CvAClvFfyA5R-PhYUmn5OOQtYMH4h6I0nSsKchNAySU/preview#heading=h.msg3086636uq
        // for how counter series are represented in the Chrome Trace Event format.
        let mut recorded = false;
        for i in 0..self.len.max(0) {
            let time_nanos =
                self.profile_start + self.bucket_duration.as_nanos() as i64 * i as i64;
            json_writer.set_indent("  ");
            json_writer.begin_object()?;
            json_writer.set_indent("");
            json_writer
                .name("name")?
                .value_str(self.display_name.as_deref().unwrap_or(""))?;
            json_writer.name("pid")?.value_long(Self::PROCESS_ID)?;
            json_writer.name("tid")?.value_long(self.thread_id)?;
            if let Some(color_name) = self.color_name {
                json_writer.name("cname")?.value_str(color_name)?;
            }
            json_writer.name("ph")?.value_str("C")?;
            json_writer
                .name("ts")?
                .value_long(nanos_to_micros(time_nanos - profile_start_time_nanos))?;
            json_writer.name("args")?;

            json_writer.begin_object()?;
            for (task, values) in &self.counter_series_map {
                let value = values[i as usize];
                // Skip counts equal to zero. They will show up as a thin line in the profile.
                // Once we record the profile task we need to post it until the end.
                if value.abs() > 0.00001 || recorded {
                    json_writer.name(task.series_name())?.value_double(value)?;
                    recorded = true;
                }
            }
            json_writer.end_object()?;

            json_writer.end_object()?;
        }
        Ok(())
    }
}

/// `TimeUnit.NANOSECONDS.toMicros(d)`: truncates toward zero.
pub(crate) fn nanos_to_micros(nanos: i64) -> i64 {
    nanos / 1000
}
