//! Port of `lib/runtime/SpawnStats.java`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering as AtomicOrdering};

use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::actions::SpawnResult;

// Java: com/google/devtools/build/lib/runtime/SpawnStats.java:36 SpawnStats.REPORT_FIRST
const REPORT_FIRST: [&str; 2] = ["disk cache hit", "remote cache hit"];

/// Collects results from SpawnResult.
// Java: com/google/devtools/build/lib/runtime/SpawnStats.java:35 SpawnStats
#[derive(Debug, Default)]
pub struct SpawnStats {
    /// `ConcurrentHashMultiset<String> runners`: runner name to count. A count of zero means the
    /// element is absent.
    runners: Mutex<HashMap<String, i32>>,
    runner_exec_kinds: Mutex<HashMap<String, String>>,
    total_wall_time_millis: AtomicI64,
    // Counts all actions, where an action can run an arbitrary number of spawns (including zero).
    all_actions_count: AtomicI32,
    // Counts internal actions, such as SymlinkTree actions, which are recognized by having zero
    // spawns.
    non_internal_actions_count: AtomicI32,
    action_cache_hit_count: AtomicI32,
}

impl SpawnStats {
    /// `new SpawnStats()`.
    pub fn new() -> SpawnStats {
        SpawnStats::default()
    }

    /// `countActionResult(ActionResult)`: the spawn results of one action.
    // Java: com/google/devtools/build/lib/runtime/SpawnStats.java:49 SpawnStats.countActionResult
    pub fn count_action_result(&self, spawn_results: &[SpawnResult]) {
        // This method is usually not called for internal actions with ActionResult#EMPTY, but
        // just in case, we double-check here.
        if !spawn_results.is_empty() {
            self.non_internal_actions_count
                .fetch_add(1, AtomicOrdering::SeqCst);
        }
        for r in spawn_results {
            self.store_runner_exec_kind(r);
            *self
                .runners
                .lock()
                .entry(r.get_runner_name().to_string())
                .or_insert(0) += 1;
            self.total_wall_time_millis.fetch_add(
                r.get_metrics().execution_wall_time_in_ms as i64,
                AtomicOrdering::SeqCst,
            );
        }
    }

    // Java: com/google/devtools/build/lib/runtime/SpawnStats.java:62 SpawnStats.storeRunnerExecKind
    fn store_runner_exec_kind(&self, r: &SpawnResult) {
        let name = r.get_runner_name().to_string();
        let exec_kind = r.get_metrics().exec_kind().to_string();
        self.runner_exec_kinds.lock().insert(name, exec_kind);
    }

    /// `incrementActionCount()`.
    // Java: com/google/devtools/build/lib/runtime/SpawnStats.java:68 SpawnStats.incrementActionCount
    pub fn increment_action_count(&self) {
        self.all_actions_count.fetch_add(1, AtomicOrdering::SeqCst);
    }

    /// `getTotalWallTimeMillis()`.
    // Java: com/google/devtools/build/lib/runtime/SpawnStats.java:72 SpawnStats.getTotalWallTimeMillis
    pub fn get_total_wall_time_millis(&self) -> i64 {
        self.total_wall_time_millis.load(AtomicOrdering::SeqCst)
    }

    /// `recordActionCacheStats(actionCacheStatistics)`: records `getHits()`.
    // Java: com/google/devtools/build/lib/runtime/SpawnStats.java:76 SpawnStats.recordActionCacheStats
    pub fn record_action_cache_stats(&self, hits: i32) {
        self.action_cache_hit_count
            .store(hits, AtomicOrdering::SeqCst);
    }

    /// Returns a human-readable summary of spawns counted.
    // Java: com/google/devtools/build/lib/runtime/SpawnStats.java:83 SpawnStats.getSummary
    pub fn get_summary(&self) -> IndexMap<String, i32> {
        self.get_summary_with(&REPORT_FIRST)
    }

    /// Returns a human-readable summary of spawns counted. The counts of `report_first` are
    /// reported first and removed from the counts (`runners.setCount(s, 0)`).
    // Java: com/google/devtools/build/lib/runtime/SpawnStats.java:90 SpawnStats.getSummary
    pub fn get_summary_with(&self, report_first: &[&str]) -> IndexMap<String, i32> {
        let mut result = SummaryBuilder::default();
        let num_non_internal_actions = self.non_internal_actions_count.load(AtomicOrdering::SeqCst);
        let num_all_actions = self.all_actions_count.load(AtomicOrdering::SeqCst);
        result.put("total", num_all_actions);

        // First report cache results.
        let action_cache_hit_count = self.action_cache_hit_count.load(AtomicOrdering::SeqCst);
        if action_cache_hit_count > 0 {
            result.put("action cache hit", action_cache_hit_count);
        }
        let mut runners = self.runners.lock();
        for s in report_first {
            let count = runners.remove(*s).unwrap_or(0);
            if count > 0 {
                result.put(s, count);
            }
        }

        // Account for internal actions such as SymlinkTree.
        // This condition is always fulfilled if incrementActionCount is called for each action
        // for which countActionResult is called eventually.
        if num_non_internal_actions < num_all_actions {
            result.put("internal", num_all_actions - num_non_internal_actions);
        }

        // Sort the rest alphabetically
        let mut list: Vec<(String, i32)> = runners
            .iter()
            .filter(|(_, c)| **c > 0)
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        list.sort_by(|a, b| java_string_compare(&a.0, &b.0));

        for (element, count) in list {
            result.put(&element, count);
        }

        result.build_or_throw()
    }

    /// `getExecKindFor(runnerName)`.
    // Java: com/google/devtools/build/lib/runtime/SpawnStats.java:125 SpawnStats.getExecKindFor
    pub fn get_exec_kind_for(&self, runner_name: &str) -> Option<String> {
        self.runner_exec_kinds.lock().get(runner_name).cloned()
    }

    /// `convertSummaryToString(spawnSummary)`: e.g. `"53 processes: 10 internal, 43 linux-sandbox."`.
    // Java: com/google/devtools/build/lib/runtime/SpawnStats.java:129 SpawnStats.convertSummaryToString
    pub fn convert_summary_to_string(spawn_summary: &IndexMap<String, i32>) -> String {
        // This summary is misleading in a number of ways:
        // - The "processes" count is actually the number of actions, not spawns.
        // - Even if it were the number of spawns, the "* cache hit" runners do not correspond to
        //   processes executed, but rather to cache hits that avoided process execution.
        // - The total count does not include action cache hits, so the sum of the parts is greater
        //   than the total.
        let total = *spawn_summary
            .get("total")
            .expect("NullPointerException: total");
        if total == 0 {
            return "0 processes.".to_string();
        }

        let mut string_summary = String::new();
        string_summary.push_str(&total.to_string());
        string_summary.push_str(" process");
        if total > 1 {
            string_summary.push_str("es");
        }
        let mut separator = ": ";

        for (key, value) in spawn_summary {
            if key == "total" {
                continue;
            }
            string_summary.push_str(separator);
            separator = ", ";
            string_summary.push_str(&value.to_string());
            string_summary.push(' ');
            string_summary.push_str(key);
        }
        string_summary.push('.');
        string_summary
    }
}

/// `ImmutableMap.Builder<String, Integer>` with `buildOrThrow()`.
#[derive(Default)]
struct SummaryBuilder {
    entries: Vec<(String, i32)>,
}

impl SummaryBuilder {
    fn put(&mut self, key: &str, value: i32) {
        self.entries.push((key.to_string(), value));
    }

    fn build_or_throw(self) -> IndexMap<String, i32> {
        let mut map = IndexMap::new();
        for (k, v) in self.entries {
            if map.contains_key(&k) {
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {k}={v} and {k}={}",
                    map[&k]
                );
            }
            map.insert(k, v);
        }
        map
    }
}

/// Java `String.compareTo`: lexicographic over UTF-16 code units.
fn java_string_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}
