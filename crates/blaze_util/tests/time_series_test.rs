//! Port of `src/test/java/com/google/devtools/build/lib/profiler/TimeSeriesTest.java`.

use std::sync::{Arc, Barrier};
use std::time::Duration;

use blaze_util::profiler::TimeSeries;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn assert_close(values: &[f64], expected: &[f64]) {
    assert_eq!(values.len(), expected.len(), "{values:?} vs {expected:?}");
    for (v, e) in values.iter().zip(expected) {
        assert!((v - e).abs() <= 1.0e-10, "{values:?} vs {expected:?}");
    }
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/TimeSeriesTest.java:30 TimeSeriesTest.testAddRange
#[test]
fn test_add_range() {
    let time_series = TimeSeries::new(ms(42), ms(100));
    time_series.add_range(ms(42), ms(142));
    time_series.add_range(ms(442), ms(542));
    assert_close(&time_series.to_double_array(5), &[1.0, 0.0, 0.0, 0.0, 1.0]);
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/TimeSeriesTest.java:39 TimeSeriesTest.testAddRangeWithValue
#[test]
fn test_add_range_with_value() {
    let time_series = TimeSeries::new(ms(42), ms(100));
    time_series.add_range_value(ms(42), ms(242), 3.0);
    time_series.add_range_value(ms(442), ms(542), 0.5);
    assert_close(&time_series.to_double_array(5), &[3.0, 3.0, 0.0, 0.0, 0.5]);
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/TimeSeriesTest.java:48 TimeSeriesTest.testAddRangeOverlappingWithValue
#[test]
fn test_add_range_overlapping_with_value() {
    let time_series = TimeSeries::new(ms(42), ms(100));
    time_series.add_range_value(ms(42), ms(242), 3.0);
    time_series.add_range_value(ms(142), ms(442), 0.5);
    assert_close(&time_series.to_double_array(5), &[3.0, 3.5, 0.5, 0.5, 0.0]);
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/TimeSeriesTest.java:57 TimeSeriesTest.testAddRangeFractions
#[test]
fn test_add_range_fractions() {
    let time_series = TimeSeries::new(ms(42), ms(100));
    time_series.add_range(ms(92), ms(267));
    assert_close(&time_series.to_double_array(5), &[0.5, 1.0, 0.25, 0.0, 0.0]);
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/TimeSeriesTest.java:65 TimeSeriesTest.testAddRangeWithValueFractions
#[test]
fn test_add_range_with_value_fractions() {
    let time_series = TimeSeries::new(ms(42), ms(100));
    time_series.add_range_value(ms(92), ms(267), 3.0);
    assert_close(&time_series.to_double_array(5), &[1.5, 3.0, 0.75, 0.0, 0.0]);
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/TimeSeriesTest.java:73 TimeSeriesTest.testResize
#[test]
fn test_resize() {
    let time_series = TimeSeries::new(Duration::ZERO, ms(100));
    time_series.add_range_value(Duration::ZERO, ms(100 * 100 + 1), 42.0);
    let mut expected = vec![42.0; 101];
    expected[100] = 0.42;
    assert_close(&time_series.to_double_array(101), &expected);
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/TimeSeriesTest.java:84 TimeSeriesTest.testParallelism
#[test]
fn test_parallelism() {
    // Define two threads. One is writing 1 on odd places, and another writes 2 on even places.
    let time_series = Arc::new(TimeSeries::new(Duration::ZERO, ms(100)));
    let latch = Arc::new(Barrier::new(2));
    let (ts1, l1) = (time_series.clone(), latch.clone());
    let thread1 = std::thread::spawn(move || {
        l1.wait();
        for i in 0..50u64 {
            ts1.add_range_value(ms(2 * i * 100), ms((2 * i + 1) * 100), 1.0);
        }
    });
    let (ts2, l2) = (time_series.clone(), latch.clone());
    let thread2 = std::thread::spawn(move || {
        l2.wait();
        for i in 0..50u64 {
            ts2.add_range_value(ms((2 * i + 1) * 100), ms((2 * i + 2) * 100), 2.0);
        }
    });
    let expected: Vec<f64> = (0..100)
        .map(|i| if i % 2 == 0 { 1.0 } else { 2.0 })
        .collect();
    thread1.join().unwrap();
    thread2.join().unwrap();
    assert_close(&time_series.to_double_array(100), &expected);
}
