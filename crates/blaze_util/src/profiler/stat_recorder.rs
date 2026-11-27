//! Ports of `StatRecorder.java`, `SingleStatRecorder.java`, `PredicateBasedStatRecorder.java`
//! and `MetricData.java`.

use std::fmt;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};

/// An object that can record time statistics about an object.
// Java: com/google/devtools/build/lib/profiler/StatRecorder.java:22 StatRecorder
pub trait StatRecorder: Send + Sync + fmt::Display {
    /// Add a new time statistic for the object `obj` (its `toString()`).
    fn add_stat(&self, duration: i32, obj: &str);

    /// True if it has not recorded any statistic.
    fn is_empty(&self) -> bool;
}

/// A predicate over the description of a recorded object.
pub type StringPredicate = fn(&str) -> bool;

/// A collection of heuristics for VFS kind of stats in order to detect the filesystem type.
// Java: com/google/devtools/build/lib/profiler/StatRecorder.java:31 StatRecorder.VfsHeuristics
pub fn vfs_type_heuristics() -> Vec<(&'static str, StringPredicate)> {
    vec![
        ("blaze-out", |s: &str| s.contains("/blaze-out/")),
        ("source", |_s: &str| true),
    ]
}

/// A stat recorder that can record time histograms, count of calls, average time, Std.
/// Deviation and max time.
// Java: com/google/devtools/build/lib/profiler/SingleStatRecorder.java:30 SingleStatRecorder
pub struct SingleStatRecorder {
    buckets: i32,
    description: Option<String>,
    histogram: Vec<AtomicI32>,
    sum: AtomicI64,
    sum_squared: AtomicI64,
    max: AtomicI64,
}

impl SingleStatRecorder {
    // Java: com/google/devtools/build/lib/profiler/SingleStatRecorder.java:39 SingleStatRecorder.<init>
    pub fn new(description: Option<String>, buckets: i32) -> Self {
        crate::check_argument!(
            buckets > 1,
            "At least two buckets (one for bellow start and onefor above start) are required"
        );
        SingleStatRecorder {
            buckets,
            description,
            histogram: (0..buckets).map(|_| AtomicI32::new(0)).collect(),
            sum: AtomicI64::new(0),
            sum_squared: AtomicI64::new(0),
            max: AtomicI64::new(-1),
        }
    }

    /// Create an snapshot of the stats recorded up to now.
    // Java: com/google/devtools/build/lib/profiler/SingleStatRecorder.java:48 SingleStatRecorder.snapshot
    pub fn snapshot(&self) -> MetricData {
        let mut result = Vec::new();
        let get = |i: usize| self.histogram[i].load(Ordering::Relaxed);
        result.push(HistogramElement {
            lower: 0,
            upper: Some(1),
            count: get(0),
        });
        let mut from: i32 = 1;
        for i in 1..self.histogram.len() - 1 {
            let to = from.wrapping_shl(1);
            result.push(HistogramElement {
                lower: from,
                upper: Some(to),
                count: get(i),
            });
            from = to;
        }
        result.push(HistogramElement {
            lower: from,
            upper: None,
            count: get(self.histogram.len() - 1),
        });
        let mut n: i32 = 0;
        for i in 0..self.histogram.len() {
            n = n.wrapping_add(get(i));
        }
        let sum = self.sum.load(Ordering::Relaxed);
        let stddev = if n == 1 {
            0.0
        } else {
            (((self.sum_squared.load(Ordering::Relaxed) as f64)
                - (sum as f64) * (sum as f64) / n as f64)
                / n as f64)
                .sqrt()
        };
        MetricData {
            description: self.description.clone(),
            histogram: result,
            count: n,
            avg: sum as f64 / n as f64,
            std_dev: stddev,
            max: self.max.load(Ordering::Relaxed) as i32,
        }
    }
}

impl StatRecorder for SingleStatRecorder {
    // Java: com/google/devtools/build/lib/profiler/SingleStatRecorder.java:76 SingleStatRecorder.addStat
    fn add_stat(&self, duration: i32, _obj: &str) {
        let bits = 32 - duration.leading_zeros() as i32;
        let histogram_bucket = bits.min(self.buckets - 1);
        self.sum.fetch_add(duration as i64, Ordering::Relaxed);
        // Java: `duration * duration` is an int product (wraps).
        self.sum_squared
            .fetch_add(duration.wrapping_mul(duration) as i64, Ordering::Relaxed);
        self.max.fetch_max(duration as i64, Ordering::Relaxed);
        self.histogram[histogram_bucket as usize].fetch_add(1, Ordering::Relaxed);
    }

    // Java: com/google/devtools/build/lib/profiler/SingleStatRecorder.java:84 SingleStatRecorder.isEmpty
    fn is_empty(&self) -> bool {
        self.snapshot().get_count() == 0
    }
}

impl fmt::Display for SingleStatRecorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.snapshot())
    }
}

/// A stat recorder that is able to look at the kind of object added and delegate to the
/// appropriate [`StatRecorder`] based on a predicate.
///
/// Note that the predicates are evaluated in order and delegated only to the first one. That
/// means that the most specific and cheapest predicates should be passed first.
// Java: com/google/devtools/build/lib/profiler/PredicateBasedStatRecorder.java:28 PredicateBasedStatRecorder
pub struct PredicateBasedStatRecorder {
    predicates: Vec<StringPredicate>,
    recorders: Vec<Box<dyn StatRecorder>>,
}

impl PredicateBasedStatRecorder {
    // Java: com/google/devtools/build/lib/profiler/PredicateBasedStatRecorder.java:34 PredicateBasedStatRecorder.<init>
    pub fn new(stats: Vec<(Box<dyn StatRecorder>, StringPredicate)>) -> Self {
        let mut predicates = Vec::with_capacity(stats.len());
        let mut recorders = Vec::with_capacity(stats.len());
        for (recorder, predicate) in stats {
            predicates.push(predicate);
            recorders.push(recorder);
        }
        PredicateBasedStatRecorder {
            predicates,
            recorders,
        }
    }
}

impl StatRecorder for PredicateBasedStatRecorder {
    // Java: com/google/devtools/build/lib/profiler/PredicateBasedStatRecorder.java:45 PredicateBasedStatRecorder.addStat
    fn add_stat(&self, duration: i32, obj: &str) {
        for (i, predicate) in self.predicates.iter().enumerate() {
            if predicate(obj) {
                self.recorders[i].add_stat(duration, obj);
                return;
            }
        }
    }

    // Java: com/google/devtools/build/lib/profiler/PredicateBasedStatRecorder.java:56 PredicateBasedStatRecorder.isEmpty
    fn is_empty(&self) -> bool {
        self.recorders.iter().all(|r| r.is_empty())
    }
}

impl fmt::Display for PredicateBasedStatRecorder {
    // Java: com/google/devtools/build/lib/profiler/PredicateBasedStatRecorder.java:66 PredicateBasedStatRecorder.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for recorder in &self.recorders {
            if recorder.is_empty() {
                continue;
            }
            writeln!(f, "{recorder}")?;
        }
        Ok(())
    }
}

/// An histogram element that contains the range that applies to and the number of elements.
// Java: com/google/devtools/build/lib/profiler/MetricData.java:103 MetricData.HistogramElement
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistogramElement {
    lower: i32,
    /// `None` for `Range.atLeast(lower)`.
    upper: Option<i32>,
    count: i32,
}

impl HistogramElement {
    pub fn get_count(&self) -> i32 {
        self.count
    }
}

impl fmt::Display for HistogramElement {
    // Java: com/google/devtools/build/lib/profiler/MetricData.java:122 MetricData.HistogramElement.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let range = format!(
            "[{}..{} ms]",
            self.lower,
            match self.upper {
                Some(u) => u.to_string(),
                None => "\u{221e}".to_string(), // infinite symbol
            }
        );
        write!(f, "{:<15}:{:>10}", range, self.count)
    }
}

/// Metric data for `description` object. Contains count, average, standard deviation, max and
/// histogram.
// Java: com/google/devtools/build/lib/profiler/MetricData.java:28 MetricData
#[derive(Clone, Debug, PartialEq)]
pub struct MetricData {
    description: Option<String>,
    histogram: Vec<HistogramElement>,
    count: i32,
    avg: f64,
    std_dev: f64,
    max: i32,
}

impl MetricData {
    // Java: com/google/devtools/build/lib/profiler/MetricData.java:46 MetricData.getDescription
    pub fn get_description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    // Java: com/google/devtools/build/lib/profiler/MetricData.java:50 MetricData.getHistogram
    pub fn get_histogram(&self) -> &[HistogramElement] {
        &self.histogram
    }

    // Java: com/google/devtools/build/lib/profiler/MetricData.java:54 MetricData.getCount
    pub fn get_count(&self) -> i32 {
        self.count
    }

    // Java: com/google/devtools/build/lib/profiler/MetricData.java:58 MetricData.getAvg
    pub fn get_avg(&self) -> f64 {
        self.avg
    }

    // Java: com/google/devtools/build/lib/profiler/MetricData.java:62 MetricData.getStdDev
    pub fn get_std_dev(&self) -> f64 {
        self.std_dev
    }

    // Java: com/google/devtools/build/lib/profiler/MetricData.java:66 MetricData.getMax
    pub fn get_max(&self) -> i32 {
        self.max
    }
}

/// `new DecimalFormat("0.###")` with US symbols: at most three fraction digits, rounded
/// half-even, trailing zeros dropped.
fn format_0_hash3(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "∞" } else { "-∞" }.to_string();
    }
    // PORT: Rust's `{:.3}` rounds half to even on the exact binary value, as DecimalFormat does.
    let s = format!("{value:.3}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    if s == "-0" { "-0".to_string() } else { s }
}

impl fmt::Display for MetricData {
    // Java: com/google/devtools/build/lib/profiler/MetricData.java:75 MetricData.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = self.description.as_deref().unwrap_or("null");
        if self.count == 0 {
            return write!(f, "'{description}'. Zero data recorded");
        }
        let histogram: Vec<String> = self
            .histogram
            .iter()
            .filter(|element| element.count > 0)
            .map(|e| e.to_string())
            .collect();
        write!(
            f,
            "'{}'.  Count: {} Avg: {} StdDev: {} Max: {} Histogram:\n  {}",
            description,
            self.count,
            format_0_hash3(self.avg),
            format_0_hash3(self.std_dev),
            self.max,
            histogram.join("\n  ")
        )
    }
}
