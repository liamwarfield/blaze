//! Port of `src/test/java/com/google/devtools/build/lib/profiler/SingleStatRecorderTest.java`.

use blaze_util::profiler::stat_recorder::{SingleStatRecorder, StatRecorder};

// Java: src/test/java/com/google/devtools/build/lib/profiler/SingleStatRecorderTest.java:27 SingleStatRecorderTest.meanAndStandardDeviation
#[test]
fn mean_and_standard_deviation() {
    let recorder = SingleStatRecorder::new(None, 2);
    recorder.add_stat(43, "null");
    recorder.add_stat(49, "null");
    recorder.add_stat(51, "null");
    recorder.add_stat(57, "null");
    let metrics = recorder.snapshot();
    assert!((metrics.get_avg() - 50.0).abs() <= 0.01);
    assert!((metrics.get_std_dev() - 5.0).abs() <= 0.01);
}
