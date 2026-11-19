//! Ports from `lib/metrics`: `BuildMetricsEvent` and the `MetricsCollector` that builds it, and
//! the critical path (`lib/metrics/criticalpath`).

pub mod aggregated_spawn_metrics;
pub mod criticalpath;
pub mod metrics_collector;
