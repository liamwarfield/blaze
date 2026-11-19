//! Port of `src/test/java/com/google/devtools/build/lib/profiler/SystemNetworkStatsTest.java`.

use blaze_util::profiler::network_metrics_collector::get_net_io_counters;

// Java: src/test/java/com/google/devtools/build/lib/profiler/SystemNetworkStatsTest.java:29 SystemNetworkStatsTest.getNetIoCounters_doesNotCrash
#[test]
fn get_net_io_counters_does_not_crash() {
    get_net_io_counters().unwrap();
}
