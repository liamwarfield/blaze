//! Port of `src/test/java/com/google/devtools/build/lib/runtime/SpawnStatsTest.java`.

use blaze_exec::actions::{ExecKind, SpawnMetrics, SpawnResult, SpawnResultBuilder, Status};
use blaze_exec::runtime::spawn_stats::SpawnStats;

fn result(runner_name: &str) -> SpawnResult {
    SpawnResultBuilder::new()
        .set_status(Status::Success)
        .set_runner_name(runner_name)
        .build()
}

/// `stats.countActionResult(ActionResult.create(spawns)); stats.incrementActionCount();`
fn action(stats: &SpawnStats, spawns: &[SpawnResult]) {
    stats.count_action_result(spawns);
    stats.increment_action_count();
}

fn summary(stats: &SpawnStats) -> String {
    SpawnStats::convert_summary_to_string(&stats.get_summary())
}

#[test]
fn empty_set() {
    let stats = SpawnStats::new();
    assert_eq!(summary(&stats), "0 processes.");
}

#[test]
fn one() {
    let stats = SpawnStats::new();
    action(&stats, &[result("foo")]);
    assert_eq!(summary(&stats), "1 process: 1 foo.");
}

#[test]
fn one_remote() {
    let stats = SpawnStats::new();
    action(&stats, &[result("remote cache hit")]);
    assert_eq!(summary(&stats), "1 process: 1 remote cache hit.");
}

#[test]
fn two() {
    let stats = SpawnStats::new();
    for _ in 0..2 {
        action(&stats, &[result("foo")]);
    }
    assert_eq!(summary(&stats), "2 processes: 2 foo.");
}

#[test]
fn order() {
    let stats = SpawnStats::new();
    action(&stats, &[result("a")]);
    for _ in 0..2 {
        action(&stats, &[result("b")]);
    }
    for _ in 0..3 {
        action(&stats, &[result("c")]);
    }
    assert_eq!(summary(&stats), "6 processes: 1 a, 2 b, 3 c.");
}

#[test]
fn reverse_order() {
    let stats = SpawnStats::new();
    for _ in 0..3 {
        action(&stats, &[result("a")]);
    }
    for _ in 0..2 {
        action(&stats, &[result("b")]);
    }
    action(&stats, &[result("c")]);
    assert_eq!(summary(&stats), "6 processes: 3 a, 2 b, 1 c.");
}

#[test]
fn cache_first() {
    let stats = SpawnStats::new();
    for _ in 0..3 {
        action(&stats, &[result("a")]);
    }
    for _ in 0..2 {
        action(&stats, &[result("b")]);
    }
    action(&stats, &[result("remote cache hit")]);
    action(&stats, &[result("c")]);
    assert_eq!(
        summary(&stats),
        "7 processes: 1 remote cache hit, 3 a, 2 b, 1 c."
    );
}

#[test]
fn action_one_spawn() {
    let stats = SpawnStats::new();
    action(&stats, &[result("abc")]);
    assert_eq!(summary(&stats), "1 process: 1 abc.");
}

#[test]
fn action_many_spawn() {
    // One action with multiple spawns - should count as 1 action with 3 spawns
    let stats = SpawnStats::new();
    action(&stats, &[result("abc"), result("abc"), result("abc")]);
    assert_eq!(summary(&stats), "1 process: 3 abc.");
}

#[test]
fn action_many_spawn_mixed() {
    // One action with multiple spawns of different runners
    let stats = SpawnStats::new();
    action(&stats, &[result("abc"), result("abc"), result("cde")]);
    assert_eq!(summary(&stats), "1 process: 2 abc, 1 cde.");
}

#[test]
fn action_many_actions_mixed() {
    // Five actions:
    // Action 1: 1 spawn (abc)
    // Action 2: 2 spawns (abc, abc)
    // Action 3: 3 spawns (abc, abc, cde)
    // Action 4: 3 spawns (abc, abc, cde)
    // Action 5: 3 spawns (abc, abc, cde)
    let stats = SpawnStats::new();
    action(&stats, &[result("abc")]);
    action(&stats, &[result("abc"), result("abc")]);
    let spawns = [result("abc"), result("abc"), result("cde")];
    action(&stats, &spawns);
    action(&stats, &spawns);
    action(&stats, &spawns);
    assert_eq!(summary(&stats), "5 processes: 9 abc, 3 cde.");
}

#[test]
fn only_internal() {
    let stats = SpawnStats::new();
    stats.increment_action_count();
    assert_eq!(summary(&stats), "1 process: 1 internal.");
}

#[test]
fn order_cache_internal_rest() {
    let stats = SpawnStats::new();
    for _ in 0..3 {
        action(&stats, &[result("a")]);
    }
    for _ in 0..2 {
        action(&stats, &[result("b")]);
    }
    action(&stats, &[result("remote cache hit")]);
    action(&stats, &[result("c")]);
    for _ in 0..2 {
        action(&stats, &[result("z")]);
    }
    // Add 2 internal actions (no spawns)
    for _ in 0..2 {
        stats.increment_action_count();
    }
    assert_eq!(
        summary(&stats),
        "11 processes: 1 remote cache hit, 2 internal, 3 a, 2 b, 1 c, 2 z."
    );
}

#[test]
fn get_exec_kind_defined() {
    let stats = SpawnStats::new();
    let r_c = SpawnResultBuilder::new()
        .set_status(Status::Success)
        .set_spawn_metrics(SpawnMetrics::for_exec(ExecKind::Other))
        .set_runner_name("fgh")
        .build();
    stats.count_action_result(&[r_c]);
    assert_eq!(
        stats.get_exec_kind_for("fgh"),
        Some(ExecKind::Other.to_string())
    );
}

#[test]
fn get_exec_kind_not_defined() {
    let stats = SpawnStats::new();
    let _unused = stats.get_summary();
    assert_eq!(stats.get_exec_kind_for("total"), None);
    assert_eq!(stats.get_exec_kind_for("internal"), None);
}

#[test]
fn internal_count_with_multiple_spawns_per_action() {
    let stats = SpawnStats::new();
    // Action 1: 3 spawns (counts as 1 non-internal action)
    action(&stats, &[result("abc"), result("abc"), result("abc")]);
    // Action 2: 2 spawns (counts as 1 non-internal action)
    action(&stats, &[result("cde"), result("cde")]);
    // Action 3: internal action (no spawns)
    stats.increment_action_count();
    assert_eq!(summary(&stats), "3 processes: 1 internal, 3 abc, 2 cde.");
}
