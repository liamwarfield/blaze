//! The build tests of `src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java`
//! (the ones that run builds through `BuildIntegrationTestCase`), on the in-process runtime.
//! The unit tests of the class (`testCreateWorkerPoolMetrics` and Blaze's own) stay in
//! `crates/blaze_bep/tests/metrics_collector_test.rs`.
//!
//! PORT: Java's `BuildMetricsEventListener` module registers itself on each command's event bus
//! in `beforeCommand`; here a `beforeCommand` hook of the fixture registers a subscriber that
//! keeps the last `BuildMetricsEvent`.

use std::sync::Arc;

use blaze_bep::metrics::metrics_collector::BuildMetricsEvent;
use blaze_buildtool_testing::BuildIntegrationTestCase;
use blaze_protos::build_event_stream::BuildMetrics;
use blaze_protos::build_event_stream::build_metrics::artifact_metrics::FilesMetric;
use blaze_protos::build_event_stream::build_metrics::{
    ArtifactMetrics, BuildGraphMetrics, CumulativeMetrics,
};
use parking_lot::Mutex;

/// Java `BuildMetricsEventListener`: keeps the last `BuildMetricsEvent` of a command.
// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:57 MetricsCollectorTest.BuildMetricsEventListener
#[derive(Default)]
struct BuildMetricsEventListener {
    event: Mutex<Option<BuildMetrics>>,
    /// Events seen (to tell an unregistered listener from a missing BuildMetricsEvent).
    seen: std::sync::atomic::AtomicUsize,
}

impl blaze_events::EventBus for BuildMetricsEventListener {
    fn post(&self, event: Arc<dyn blaze_events::Postable>) {
        self.seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let event: Arc<dyn std::any::Any + Send + Sync> = event;
        if let Ok(e) = event.downcast::<BuildMetricsEvent>() {
            *self.event.lock() = Some(e.get_build_metrics().clone());
        }
    }
}

struct Fixture {
    t: BuildIntegrationTestCase,
    listener: Arc<BuildMetricsEventListener>,
}

impl Fixture {
    // Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:77 MetricsCollectorTest.getRuntimeBuilder
    // Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:84 MetricsCollectorTest.writeTrivialFooTarget
    fn new() -> Fixture {
        let t = BuildIntegrationTestCase::new();
        let listener = Arc::new(BuildMetricsEventListener::default());
        let registered = listener.clone();
        t.on_before_command(move |env| {
            env.get_event_bus().register(registered.clone());
        });
        t.write(
            "foo/BUILD",
            &[
                "genrule(",
                "    name = \"foo\",",
                "    outs = [\"out\"],",
                "    cmd = \"touch $@\",",
                ")",
            ],
        );
        Fixture { t, listener }
    }

    /// `buildTarget(targets...)`; `Err` when the build fails (Java throws).
    fn build_target(&mut self, targets: &[&str]) -> Result<(), ()> {
        if self.t.build_target(targets).is_success() {
            Ok(())
        } else {
            Err(())
        }
    }

    /// `buildMetricsEventListener.event.getBuildMetrics()`.
    fn metrics(&self) -> BuildMetrics {
        self.listener.event.lock().clone().unwrap_or_else(|| {
            panic!(
                "no BuildMetricsEvent among the {} events of the command",
                self.listener.seen.load(std::sync::atomic::Ordering::SeqCst)
            )
        })
    }

    /// `createSymlink(target, link)` (relative to the workspace).
    fn create_symlink(&self, target: &str, link: &str) {
        std::os::unix::fs::symlink(target, self.t.get_workspace().join(link)).unwrap();
    }
}

fn files_metric(size_in_bytes: i64, count: i32) -> FilesMetric {
    FilesMetric {
        size_in_bytes,
        count,
    }
}

/// `ignoringFieldAbsence()`: an absent message is its default instance.
fn artifact_metrics(m: &BuildMetrics) -> ArtifactMetrics {
    let a = m.artifact_metrics.clone().unwrap_or_default();
    ArtifactMetrics {
        source_artifacts_read: Some(a.source_artifacts_read.unwrap_or_default()),
        output_artifacts_seen: Some(a.output_artifacts_seen.unwrap_or_default()),
        output_artifacts_from_action_cache: Some(
            a.output_artifacts_from_action_cache.unwrap_or_default(),
        ),
        top_level_artifacts: Some(a.top_level_artifacts.unwrap_or_default()),
    }
}

/// `ArtifactMetrics.newBuilder()...build()` with every absent field at its default
/// (`ignoringFieldAbsence`).
fn expected_artifacts(
    source_artifacts_read: Option<FilesMetric>,
    output_artifacts_seen: Option<FilesMetric>,
    output_artifacts_from_action_cache: Option<FilesMetric>,
    top_level_artifacts: Option<FilesMetric>,
) -> ArtifactMetrics {
    ArtifactMetrics {
        source_artifacts_read: Some(source_artifacts_read.unwrap_or_default()),
        output_artifacts_seen: Some(output_artifacts_seen.unwrap_or_default()),
        output_artifacts_from_action_cache: Some(
            output_artifacts_from_action_cache.unwrap_or_default(),
        ),
        top_level_artifacts: Some(top_level_artifacts.unwrap_or_default()),
    }
}

/// `comparingExpectedFieldsOnly()` on `ArtifactMetrics`: the fields the expected message has
/// (a submessage is present when it was set).
fn assert_artifacts_expected_fields_only(actual: &ArtifactMetrics, expected: &ArtifactMetrics) {
    let pairs = [
        (
            "source_artifacts_read",
            &actual.source_artifacts_read,
            &expected.source_artifacts_read,
        ),
        (
            "output_artifacts_seen",
            &actual.output_artifacts_seen,
            &expected.output_artifacts_seen,
        ),
        (
            "output_artifacts_from_action_cache",
            &actual.output_artifacts_from_action_cache,
            &expected.output_artifacts_from_action_cache,
        ),
        (
            "top_level_artifacts",
            &actual.top_level_artifacts,
            &expected.top_level_artifacts,
        ),
    ];
    for (name, a, e) in pairs {
        let Some(e) = e else { continue };
        let a = a.clone().unwrap_or_default();
        // FilesMetric's own fields: proto3 scalars are present when not zero.
        if e.size_in_bytes != 0 {
            assert_eq!(a.size_in_bytes, e.size_in_bytes, "{name}.size_in_bytes");
        }
        if e.count != 0 {
            assert_eq!(a.count, e.count, "{name}.count");
        }
    }
}

/// `comparingExpectedFieldsOnly()` on `BuildGraphMetrics`: proto3 scalar fields are present in
/// the expected message only when not zero, so only those are compared.
fn assert_graph_expected_fields_only(actual: &BuildGraphMetrics, expected: &BuildGraphMetrics) {
    let fields = [
        (
            "action_lookup_value_count",
            actual.action_lookup_value_count,
            expected.action_lookup_value_count,
        ),
        (
            "action_lookup_value_count_not_including_aspects",
            actual.action_lookup_value_count_not_including_aspects,
            expected.action_lookup_value_count_not_including_aspects,
        ),
        ("action_count", actual.action_count, expected.action_count),
        (
            "action_count_not_including_aspects",
            actual.action_count_not_including_aspects,
            expected.action_count_not_including_aspects,
        ),
        (
            "input_file_configured_target_count",
            actual.input_file_configured_target_count,
            expected.input_file_configured_target_count,
        ),
        (
            "output_file_configured_target_count",
            actual.output_file_configured_target_count,
            expected.output_file_configured_target_count,
        ),
        (
            "other_configured_target_count",
            actual.other_configured_target_count,
            expected.other_configured_target_count,
        ),
        (
            "output_artifact_count",
            actual.output_artifact_count,
            expected.output_artifact_count,
        ),
        (
            "post_invocation_skyframe_node_count",
            actual.post_invocation_skyframe_node_count,
            expected.post_invocation_skyframe_node_count,
        ),
    ];
    for (name, a, e) in fields {
        if e != 0 {
            assert_eq!(a, e, "{name} (actual {actual:?})");
        }
    }
}

fn graph(m: &BuildMetrics) -> BuildGraphMetrics {
    m.build_graph_metrics.clone().unwrap_or_default()
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:107 MetricsCollectorTest.testActionsCreated
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_actions_created() {
    let mut f = Fixture::new();
    f.build_target(&["//foo:foo"]).expect("build");
    let build_metrics = f.metrics();
    assert!(
        build_metrics
            .action_summary
            .unwrap_or_default()
            .actions_created
            > 0
    );
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:114 MetricsCollectorTest.testActionsCreatedIsZeroOnSecondBuild
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_actions_created_is_zero_on_second_build() {
    let mut f = Fixture::new();
    f.build_target(&["//foo:foo"]).expect("build");
    f.build_target(&["//foo:foo"]).expect("build");
    let build_metrics = f.metrics();
    assert_eq!(
        build_metrics
            .action_summary
            .unwrap_or_default()
            .actions_created,
        0
    );
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:122 MetricsCollectorTest.testActionsExecuted
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_actions_executed() {
    let mut f = Fixture::new();
    f.build_target(&["//foo:foo"]).expect("build");
    let build_metrics = f.metrics();
    assert!(
        build_metrics
            .action_summary
            .unwrap_or_default()
            .actions_executed
            > 0
    );
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:129 MetricsCollectorTest.testActionsExecutedIsZeroOnSecondBuild
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported; also: the real BazelWorkspaceStatusAction runs where Java's test runtime has DummyBuildInfoAction (it runs again on the second build)"]
fn test_actions_executed_is_zero_on_second_build() {
    let mut f = Fixture::new();
    f.build_target(&["//foo:foo"]).expect("build");
    f.build_target(&["//foo:foo"]).expect("build");
    let build_metrics = f.metrics();
    assert_eq!(
        build_metrics
            .action_summary
            .unwrap_or_default()
            .actions_executed,
        0
    );
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:137 MetricsCollectorTest.testActionsCreatedForIndividualMnemonics
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_actions_created_for_individual_mnemonics() {
    let mut f = Fixture::new();
    f.t.write(
        "bar/BUILD",
        &["load(\":bar.bzl\", \"bar\")", "bar(name = \"bar\")"],
    );
    f.t.write(
        "bar/bar.bzl",
        &[
            "def _impl(ctx):",
            "    output1 = ctx.actions.declare_file(ctx.attr.name + \".out1\")",
            "    output2 = ctx.actions.declare_file(ctx.attr.name + \".out2\")",
            "    ctx.actions.write(output1, \"foo\")",
            "    ctx.actions.write(output2, \"bar\")",
            "    # Note that we created 2 actions, but pass along only one of their outputs.",
            "    return [DefaultInfo(files = depset([output1]))]",
            "",
            "bar = rule(",
            "    implementation = _impl,",
            ")",
        ],
    );
    f.build_target(&["//bar:bar"]).expect("build");

    let build_metrics = f.metrics();
    let summary = build_metrics.action_summary.unwrap_or_default();
    let action_data = &summary.action_data;
    let file_write_actions: Vec<_> = action_data
        .iter()
        .filter(|a| a.mnemonic == "FileWrite")
        .collect();
    assert_eq!(file_write_actions.len(), 1);
    let file_write_action = file_write_actions[0];
    assert_eq!(file_write_action.actions_created, 2);
    assert_eq!(file_write_action.actions_executed, 1);

    let total_actions_created: i64 = action_data.iter().map(|a| a.actions_created).sum();
    let total_actions_executed: i64 = action_data.iter().map(|a| a.actions_executed).sum();
    assert_eq!(total_actions_created, 3);
    assert_eq!(total_actions_executed, 2);
    assert_eq!(total_actions_created, summary.actions_created);
    assert_eq!(total_actions_executed, summary.actions_executed);
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:182 MetricsCollectorTest.testAspectActionsCreatedNotOverCountedForAlias
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_aspect_actions_created_not_over_counted_for_alias() {
    let mut f = Fixture::new();
    f.t.write(
        "pkg/BUILD",
        &[
            "load(\":defs.bzl\", \"my_rule\")",
            "my_rule(name = \"foo\", dep = \"dep_alias_alias\")",
            "alias(name = \"dep_alias_alias\", actual = \":dep_alias\")",
            "alias(name = \"dep_alias\", actual = \":dep\")",
            "my_rule(name = \"dep\")",
        ],
    );
    f.t.write(
        "pkg/defs.bzl",
        &[
            "def _aspect_impl(target, ctx):",
            "    f = ctx.actions.declare_file(target.label.name + \".out\")",
            "    ctx.actions.run_shell(",
            "        outputs = [f],",
            "        command = \"touch $@\",",
            "        mnemonic = \"MyAspectAction\",",
            "    )",
            "    return [OutputGroupInfo(my_out = depset([f]))]",
            "",
            "my_aspect = aspect(implementation = _aspect_impl)",
            "",
            "def _impl(ctx):",
            "    pass",
            "",
            "my_rule = rule(",
            "    implementation = _impl,",
            "    attrs = {",
            "        \"dep\": attr.label(aspects = [my_aspect]),",
            "    },",
            ")",
        ],
    );
    f.build_target(&["//pkg:foo"]).expect("build");

    let build_metrics = f.metrics();
    let summary = build_metrics.action_summary.unwrap_or_default();
    let aspect_actions: Vec<_> = summary
        .action_data
        .iter()
        .filter(|a| a.mnemonic == "MyAspectAction")
        .collect();
    assert_eq!(aspect_actions.len(), 1);
    assert_eq!(aspect_actions[0].actions_created, 1);
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:230 MetricsCollectorTest.buildGraphAndArtifactMetrics
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported; also: the real BazelWorkspaceStatusAction runs where Java's test runtime has DummyBuildInfoAction (its outputs count as artifacts)"]
fn build_graph_and_artifact_metrics() {
    let mut f = Fixture::new();
    f.t.write(
        "a/BUILD",
        &["genrule(name = 'a', srcs = ['//b:b', '//b:c'], outs = ['a.out'], cmd = 'cat $(SRCS) > $@')"],
    );
    f.t.write(
        "b/BUILD",
        &[
            "genrule(name = 'b', srcs = ['b.in', 'c.in'], outs = ['b.out'], cmd = 'cat $(SRCS) > $@')",
            "genrule(name = 'c', srcs = ['c.in', 'c2.in'], outs = ['c.out'], cmd = 'cat $(SRCS) > $@')",
        ],
    );
    f.t.write("b/b.in", &["12345"]);
    f.t.write("b/c.in", &["12"]);
    f.create_symlink("c.in", "b/c2.in");
    f.t.write(
        "e/BUILD",
        &[
            "alias(",
            "    name = \"facade\",",
            "    actual = \":e.out\",",
            ")",
            "",
            "genrule(",
            "    name = \"e\",",
            "    srcs = [\"e.in\"],",
            "    outs = [\"e.out\"],",
            "    cmd = \"cat $(SRCS) > $@\",",
            ")",
        ],
    );
    f.t.write("e/e.in", &["abc"]);

    // Do one build of a target in a standalone package. Gets us a baseline for
    // analysis/execution.
    f.build_target(&["//e:facade"]).expect("build");
    // getCommandEnvironment().withMergedAnalysisAndExecutionSourceOfTruth(): true for the
    // fixture's options (`--experimental_merged_skyframe_analysis_execution` defaults to true,
    // and none of --nobuild, --explain, several package path entries, or a non-positive
    // --experimental_cpu_heavy_skykeys_thread_pool_size turns it off;
    // SkymeldModule.determineIfMergingAnalysisExecution).
    let skymeld_was_involved_for_baseline_build = true;
    let build_graph_metrics = graph(&f.metrics());
    let mut action_lookup_value_count = build_graph_metrics.action_lookup_value_count;
    // All these numbers should be big, but just want a basic check.
    assert!(action_lookup_value_count > 0);
    assert_eq!(
        build_graph_metrics.action_lookup_value_count_not_including_aspects,
        action_lookup_value_count
    );
    let action_count = build_graph_metrics.action_count;
    assert!(action_count > 0);
    assert_eq!(
        build_graph_metrics.action_count_not_including_aspects,
        action_count
    );
    assert_graph_expected_fields_only(
        &build_graph_metrics,
        &BuildGraphMetrics {
            output_file_configured_target_count: 1,
            other_configured_target_count: 1,
            ..Default::default()
        },
    );
    let output_artifact_count = build_graph_metrics.output_artifact_count;
    assert!(output_artifact_count > 0);
    let graph_size = build_graph_metrics.post_invocation_skyframe_node_count;
    assert!(graph_size > 0);
    let artifacts = artifact_metrics(&f.metrics());
    assert!(
        artifacts
            .source_artifacts_read
            .clone()
            .unwrap()
            .size_in_bytes
            > 0
    );
    assert_eq!(artifacts.output_artifacts_seen, Some(files_metric(4, 3)));
    assert_eq!(
        artifacts
            .output_artifacts_from_action_cache
            .clone()
            .unwrap()
            .count,
        0
    );
    assert_eq!(artifacts.top_level_artifacts, Some(files_metric(4, 1)));
    // Adjust for the "alias", "input" and "output" configured targets, which won't be in play
    // later.
    action_lookup_value_count -= 3;

    // Now do a build of a target with non-trivial transitive deps, and verify the metrics.
    // Blaze won't redo analysis of dependencies or re-read their sources.
    f.build_target(&["//a"]).expect("build");

    let build_graph_metrics = graph(&f.metrics());
    assert_graph_expected_fields_only(
        &build_graph_metrics,
        &BuildGraphMetrics {
            // Two dependencies and three source files for action lookup values.
            action_lookup_value_count: 5 + action_lookup_value_count,
            action_count: 2 + action_count,
            input_file_configured_target_count: 4,
            output_artifact_count: 2 + output_artifact_count,
            ..Default::default()
        },
    );

    let mut new_graph_size = build_graph_metrics.post_invocation_skyframe_node_count;
    assert!(new_graph_size > graph_size);

    assert_artifacts_expected_fields_only(
        &artifact_metrics(&f.metrics()),
        &ArtifactMetrics {
            // 2 distinct artifacts of 6 and 3 bytes, with a symlink to the 3-byte one.
            source_artifacts_read: Some(files_metric(12, 3)),
            // b outputs 9 bytes, c outputs 6, a outputs 15, 30 total.
            output_artifacts_seen: Some(files_metric(30, 3)),
            output_artifacts_from_action_cache: None,
            top_level_artifacts: Some(files_metric(15, 1)),
        },
    );

    // Do a null build. No useful analysis stats.
    f.build_target(&["//a"]).expect("build");
    if skymeld_was_involved_for_baseline_build {
        // The BuildDriverKey of //e:facade is gone.
        new_graph_size -= 1;
    }

    // For null build, we don't do any conflict checking. As the metrics are collected during
    // the traversal that's part of conflict checking, these analysis-related numbers are 0.
    assert_graph_expected_fields_only(
        &graph(&f.metrics()),
        &BuildGraphMetrics {
            post_invocation_skyframe_node_count: new_graph_size,
            ..Default::default()
        },
    );
    assert_eq!(
        artifact_metrics(&f.metrics()),
        expected_artifacts(None, None, None, None)
    );

    // Change a BUILD file and rebuild: no source artifacts read, but analysis stats present.
    f.t.write(
        "a/BUILD",
        &["genrule(name = 'a', srcs = ['//b:c', '//b:b'], outs = ['a.out'], cmd = 'cat $(SRCS) > $@')"],
    );
    f.build_target(&["//a"]).expect("build");

    assert_graph_expected_fields_only(
        &graph(&f.metrics()),
        &BuildGraphMetrics {
            action_lookup_value_count: 5 + action_lookup_value_count,
            action_lookup_value_count_not_including_aspects: 5 + action_lookup_value_count,
            action_count: 2 + action_count,
            action_count_not_including_aspects: 2 + action_count,
            input_file_configured_target_count: 4,
            output_artifact_count: 2 + output_artifact_count,
            // ArtifactNestedSet node for stale nested set is still in graph, since it is
            // technically still valid (even though nobody wants that nested set anymore).
            post_invocation_skyframe_node_count: new_graph_size + 1,
            ..Default::default()
        },
    );
    let single_file_metric = files_metric(15, 1);
    assert_eq!(
        artifact_metrics(&f.metrics()),
        expected_artifacts(
            None,
            Some(single_file_metric.clone()),
            None,
            Some(single_file_metric.clone())
        )
    );

    // Change BUILD file back, but don't do a full build.
    f.t.write(
        "a/BUILD",
        &["genrule(name = 'a', srcs = ['//b:c', '//b:b'], outs = ['a.out'], cmd = 'cat $(SRCS) > $@')"],
    );
    f.t.add_options(&["--nobuild"]); // this disables skymeld, because there is no execution phase
    f.build_target(&["//a"]).expect("build");
    assert_graph_expected_fields_only(
        &graph(&f.metrics()),
        &BuildGraphMetrics {
            action_lookup_value_count: 5 + action_lookup_value_count,
            action_lookup_value_count_not_including_aspects: 5 + action_lookup_value_count,
            action_count: 2 + action_count,
            action_count_not_including_aspects: 2 + action_count,
            input_file_configured_target_count: 4,
            output_artifact_count: 2 + output_artifact_count,
            post_invocation_skyframe_node_count: new_graph_size + 1,
            ..Default::default()
        },
    );
    assert_eq!(
        artifact_metrics(&f.metrics()),
        expected_artifacts(None, None, None, None)
    );

    // Null --nobuild.
    f.build_target(&["//a"]).expect("build");
    if skymeld_was_involved_for_baseline_build {
        // When doing --nobuild, which doesn't trigger skymeld, no new BuildDriverKey entry is put
        // in the graph while the old one is deleted.
        new_graph_size -= 1;
    }

    // Stale action execution have been GC'ed.
    assert_eq!(
        graph(&f.metrics()).post_invocation_skyframe_node_count,
        new_graph_size - 1
    );

    // Do a null full build. Back to baseline.
    f.t.add_options(&["--build"]);
    f.build_target(&["//a"]).expect("build");
    if skymeld_was_involved_for_baseline_build {
        // Extra BuildDriverKey
        new_graph_size += 1;
    }

    // We now have three copies of the ArtifactNestedSetKey, since the re-analysis didn't re-use
    // the old nested set.
    assert_eq!(
        graph(&f.metrics()).post_invocation_skyframe_node_count,
        new_graph_size + 2
    );
    assert_artifacts_expected_fields_only(
        &artifact_metrics(&f.metrics()),
        &ArtifactMetrics {
            source_artifacts_read: None,
            output_artifacts_seen: Some(single_file_metric.clone()),
            output_artifacts_from_action_cache: Some(single_file_metric.clone()),
            top_level_artifacts: Some(single_file_metric.clone()),
        },
    );

    // Change a source file. It and its symlink are both re-read.
    f.t.write("b/c.in", &["1234"]);
    f.build_target(&["//a"]).expect("build");
    assert_graph_expected_fields_only(
        &graph(&f.metrics()),
        &BuildGraphMetrics {
            // Analysis not re-triggered, even of the input file that was changed.
            input_file_configured_target_count: 0,
            post_invocation_skyframe_node_count: new_graph_size + 2,
            ..Default::default()
        },
    );
    assert_artifacts_expected_fields_only(
        &artifact_metrics(&f.metrics()),
        &ArtifactMetrics {
            source_artifacts_read: Some(files_metric(10, 2)),
            output_artifacts_seen: Some(files_metric(42, 3)),
            output_artifacts_from_action_cache: None,
            top_level_artifacts: Some(files_metric(21, 1)),
        },
    );

    // Validate RuleClass Aspect and SkyFunction data is not reported by default
    let bgm = graph(&f.metrics());
    assert!(bgm.rule_class.is_empty());
    assert!(bgm.aspect.is_empty());

    // Enable skyframe metrics via flag and verify they're reported.
    f.t.add_options(&["--experimental_record_skyframe_metrics=1"]);
    f.build_target(&["//a"]).expect("build");
    let bgm = graph(&f.metrics());

    let mut rule_classes: Vec<&str> = bgm.rule_class.iter().map(|r| r.key.as_str()).collect();
    rule_classes.sort();
    let mut expected = vec![
        "genrule",
        "constraint_setting",
        "constraint_value",
        "platform",
    ];
    expected.sort();
    assert_eq!(rule_classes, expected);
    assert!(bgm.aspect.is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:506 MetricsCollectorTest.treeArtifactAndTopLevelMetrics
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported; also: the real BazelWorkspaceStatusAction runs where Java's test runtime has DummyBuildInfoAction (its outputs count as artifacts)"]
fn tree_artifact_and_top_level_metrics() {
    let mut f = Fixture::new();
    f.t.write(
        "foo/tree_artifact_rule.bzl",
        &[
            "def _tree_artifact_files_impl(ctx):",
            "    directory = ctx.actions.declare_directory(ctx.attr.name + \"_artifact\")",
            "    ctx.actions.run_shell(",
            "        outputs = [directory],",
            "        command = \"cd %s && echo a > file1 && echo bcde > file2}\" % (directory.path),",
            "    )",
            "    return [DefaultInfo(files = depset([directory]))]",
            "",
            "def _several_outputs_impl(ctx):",
            "    file = ctx.actions.declare_file(ctx.attr.name + \"_file\")",
            "    ctx.actions.write(output = file, content = \"abc\")",
            "    return [",
            "        DefaultInfo(files = depset([file])),",
            "        OutputGroupInfo(dep_files = ctx.attr.dep[DefaultInfo].files),",
            "    ]",
            "",
            "my_tree = rule(implementation = _tree_artifact_files_impl)",
            "my_rule = rule(",
            "    implementation = _several_outputs_impl,",
            "    attrs = {\"dep\": attr.label()},",
            ")",
        ],
    );
    f.t.write(
        "foo/BUILD",
        &[
            "load(\"//foo:tree_artifact_rule.bzl\", \"my_rule\", \"my_tree\")",
            "",
            "my_tree(name = \"tree\")",
            "",
            "my_rule(",
            "    name = \"top\",",
            "    dep = \":tree\",",
            ")",
        ],
    );
    // Null build to populate silly things like fake build-info artifact.
    f.build_target(&[]).expect("build");
    f.t.add_options(&["--output_groups=+dep_files"]);
    f.build_target(&["//foo:top"]).expect("build");
    assert_eq!(
        artifact_metrics(&f.metrics()),
        expected_artifacts(
            None,
            Some(files_metric(10, 3)),
            None,
            Some(files_metric(10, 3))
        )
    );
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:566 MetricsCollectorTest.testTargetCounts
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_target_counts() {
    let mut f = Fixture::new();
    f.build_target(&["//foo:foo"]).expect("build");
    let target_metrics = f.metrics().target_metrics.unwrap_or_default();
    assert!(target_metrics.targets_configured > 0);
    assert!(target_metrics.targets_configured_not_including_aspects > 0);
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:575 MetricsCollectorTest.testTargetsCountsAreZeroOnSecondBuild
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_targets_counts_are_zero_on_second_build() {
    let mut f = Fixture::new();
    f.build_target(&["//foo:foo"]).expect("build");
    f.build_target(&["//foo:foo"]).expect("build");
    let target_metrics = f.metrics().target_metrics.unwrap_or_default();
    assert_eq!(target_metrics.targets_loaded, 0);
    assert_eq!(target_metrics.targets_configured, 0);
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:584 MetricsCollectorTest.aspectLoadedMetric
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn aspect_loaded_metric() {
    let mut f = Fixture::new();
    f.t.write(
        "foo/foo.bzl",
        &[
            "def _aspect_impl(target, ctx):",
            "    outfile = ctx.actions.declare_file(ctx.rule.attr.name + \"aspect.out\")",
            "    ctx.actions.run_shell(",
            "        outputs = [outfile],",
            "        command = 'echo \"1\" > ' + outfile.path,",
            "    )",
            "    return [OutputGroupInfo(files = [outfile])]",
            "",
            "def _impl(ctx):",
            "    return []",
            "",
            "rule_aspect = aspect(implementation = _aspect_impl, attr_aspects = [\"deps\"])",
            "",
            "aspected = rule(",
            "    implementation = _impl,",
            "    attrs = {\"deps\": attr.label_list(aspects = [rule_aspect])},",
            ")",
        ],
    );
    f.t.write(
        "foo/BUILD",
        &[
            "load(\"//foo:foo.bzl\", \"aspected\")",
            "",
            "aspected(",
            "    name = \"top\",",
            "    deps = [\":dep\"],",
            ")",
            "",
            "aspected(name = \"dep\")",
        ],
    );
    f.build_target(&["//foo:top"]).expect("build");
    let build_metrics = f.metrics();
    let target_metrics = build_metrics.target_metrics.clone().unwrap_or_default();
    let summary = build_metrics.action_summary.clone().unwrap_or_default();
    let bgm = graph(&build_metrics);
    // 2 additional: aspect and workspace status action.
    assert_eq!(
        target_metrics.targets_configured,
        2 + target_metrics.targets_configured_not_including_aspects
    );
    assert_eq!(
        summary.actions_created,
        2 + summary.actions_created_not_including_aspects
    );
    // Traversing the Skyframe graph doesn't hit the workspace status action.
    assert_eq!(
        bgm.action_lookup_value_count,
        1 + bgm.action_lookup_value_count_not_including_aspects
    );
    assert_eq!(bgm.action_count, 1 + bgm.action_count_not_including_aspects);

    // Analyzing a new target makes the aspect drop out of the target metric, but the build
    // graph metric still knows about it.
    f.t.write(
        "bar/BUILD",
        &["genrule(name = 'bar', outs = ['out'], cmd = 'touch $@')"],
    );
    f.build_target(&["//foo:top", "//bar:bar"]).expect("build");
    let build_metrics = f.metrics();
    let target_metrics = build_metrics.target_metrics.clone().unwrap_or_default();
    let summary = build_metrics.action_summary.clone().unwrap_or_default();
    let bgm = graph(&build_metrics);
    assert_eq!(
        target_metrics.targets_configured,
        target_metrics.targets_configured_not_including_aspects
    );
    assert_eq!(
        summary.actions_created,
        summary.actions_created_not_including_aspects
    );
    assert_eq!(
        bgm.action_lookup_value_count,
        1 + bgm.action_lookup_value_count_not_including_aspects
    );
    assert_eq!(bgm.action_count, 1 + bgm.action_count_not_including_aspects);
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:655 MetricsCollectorTest.testPackagesLoaded
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_packages_loaded() {
    let mut f = Fixture::new();
    f.build_target(&["//foo:foo"]).expect("build");
    assert!(
        f.metrics()
            .package_metrics
            .unwrap_or_default()
            .packages_loaded
            > 0
    );
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:662 MetricsCollectorTest.testPackagesLoadedIsZeroOnSecondBuild
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_packages_loaded_is_zero_on_second_build() {
    let mut f = Fixture::new();
    f.build_target(&["//foo:foo"]).expect("build");
    f.build_target(&["//foo:foo"]).expect("build");
    assert_eq!(
        f.metrics()
            .package_metrics
            .unwrap_or_default()
            .packages_loaded,
        0
    );
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:670 MetricsCollectorTest.testAnalysisTimeInMs
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_analysis_time_in_ms() {
    let mut f = Fixture::new();
    f.build_target(&["//foo:foo"]).expect("build");
    assert!(
        f.metrics()
            .timing_metrics
            .unwrap_or_default()
            .analysis_phase_time_in_ms
            > 0
    );
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:677 MetricsCollectorTest.testExecutionTimeInMs
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_execution_time_in_ms() {
    let mut f = Fixture::new();
    f.build_target(&["//foo:foo"]).expect("build");
    assert!(
        f.metrics()
            .timing_metrics
            .unwrap_or_default()
            .execution_phase_time_in_ms
            > 0
    );
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:684 MetricsCollectorTest.testActionsExecutionStartTimeInMs
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_actions_execution_start_time_in_ms() {
    let mut f = Fixture::new();
    f.build_target(&["//foo:foo"]).expect("build");
    let timing = f.metrics().timing_metrics.unwrap_or_default();
    let actions_execution_start_in_ms = timing.actions_execution_start_in_ms;
    assert!(actions_execution_start_in_ms > 0);
    assert!(actions_execution_start_in_ms < timing.wall_time_in_ms);
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:694 MetricsCollectorTest.testUsedHeapSizePostBuild
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_used_heap_size_post_build() {
    let mut f = Fixture::new();
    f.t.add_options(&["--memory_profile=/dev/null"]);

    // The options from above do not get added to the initial command environment, so it has to
    // be recreated here.
    f.build_target(&["//foo:foo"]).expect("build");
    assert!(
        f.metrics()
            .memory_metrics
            .unwrap_or_default()
            .used_heap_size_post_build
            > 0
    );
    // Note that we cannot test peak heap size here since the tiny builds that we do here don't
    // trigger a full GC.
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:709 MetricsCollectorTest.testUsedHeapSizePostBuildCollectionOffByDefault
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_used_heap_size_post_build_collection_off_by_default() {
    let mut f = Fixture::new();
    f.build_target(&["//foo:foo"]).expect("build");
    let memory = f.metrics().memory_metrics.unwrap_or_default();
    assert_eq!(memory.used_heap_size_post_build, 0);
    assert_eq!(memory.peak_post_gc_heap_size, 0);
    assert_eq!(memory.peak_post_gc_tenured_space_heap_size, 0);
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:718 MetricsCollectorTest.testWallTimePostBuild
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn test_wall_time_post_build() {
    let mut f = Fixture::new();
    f.build_target(&["//foo:foo"]).expect("build");
    assert!(
        f.metrics()
            .timing_metrics
            .unwrap_or_default()
            .wall_time_in_ms
            > 0
    );
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:725 MetricsCollectorTest.cumulativeMetrics
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported; also: MetricsModule's counters are a process global in Blaze (a module field in Java), so earlier fixtures of the test process count"]
fn cumulative_metrics() {
    let mut f = Fixture::new();
    let cumulative = |num_analyses, num_builds| {
        Some(CumulativeMetrics {
            num_analyses,
            num_builds,
        })
    };
    f.build_target(&["//foo:foo"]).expect("build");
    assert_eq!(f.metrics().cumulative_metrics, cumulative(1, 1));

    f.t.add_options(&["--nobuild"]);
    f.build_target(&["//foo:foo"]).expect("build");
    assert_eq!(f.metrics().cumulative_metrics, cumulative(2, 1));

    f.t.add_options(&["--build", "--noanalyze"]);
    f.build_target(&["//foo:foo"]).expect("build");
    assert_eq!(f.metrics().cumulative_metrics, cumulative(2, 1));

    f.t.write(
        "foo/BUILD",
        &[
            "genrule(",
            "    name = \"foo\",",
            "    srcs = [\"//noexist\"],",
            "    outs = [\"out\"],",
            "    cmd = \"touch $@\",",
            ")",
        ],
    );

    f.t.add_options(&["--analyze"]);
    // assertThrows(ViewCreationFailedException)
    assert!(f.build_target(&["//foo:foo"]).is_err());
    assert_eq!(f.metrics().cumulative_metrics, cumulative(3, 1));

    f.t.write(
        "foo/BUILD",
        &[
            "genrule(",
            "    name = \"foo\",",
            "    outs = [\"out\"],",
            "    cmd = \"/bin/false\",",
            ")",
        ],
    );

    // assertThrows(BuildFailedException)
    assert!(f.build_target(&["//foo:foo"]).is_err());
    assert_eq!(f.metrics().cumulative_metrics, cumulative(4, 2));
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:772 MetricsCollectorTest.testActionData
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported; also: the real BazelWorkspaceStatusAction runs where Java's test runtime has DummyBuildInfoAction"]
fn test_action_data() {
    let mut f = Fixture::new();
    f.t.write(
        "bar/BUILD",
        &[
            "genrule(",
            "    name = \"bar\",",
            "    srcs = [",
            "        \":dep1\",",
            "        \":dep2\",",
            "    ],",
            "    outs = [\"out\"],",
            "    cmd = \"touch $@\",",
            ")",
            "",
            "genrule(",
            "    name = \"dep1\",",
            "    outs = [\"out1\"],",
            "    cmd = \"touch $@\",",
            ")",
            "",
            "genrule(",
            "    name = \"dep2\",",
            "    outs = [\"out2\"],",
            "    cmd = \"touch $@\",",
            ")",
        ],
    );
    f.build_target(&["//bar"]).expect("build");
    let action_data_list = f.metrics().action_summary.unwrap_or_default().action_data;
    assert_eq!(action_data_list.len(), 2);
    assert_eq!(action_data_list[0].mnemonic, "Genrule");
    assert_eq!(action_data_list[0].actions_executed, 3);

    assert_eq!(action_data_list[1].mnemonic, "DummyBuildInfoAction");
    assert_eq!(action_data_list[1].actions_executed, 1);

    for action_data in &action_data_list {
        assert!(action_data.first_started_ms <= action_data.last_ended_ms);
    }
}

// Java: src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java:814 MetricsCollectorTest.skymeldNullIncrementalBuild_buildGraphMetricsNotCollected
#[test]
#[ignore = "blaze_runtime posts no BuildMetricsEvent for a build without a build event transport (MetricsOnlyCommandSupport; Java's MetricsCollector posts it for every build): S2, reported"]
fn skymeld_null_incremental_build_build_graph_metrics_not_collected() {
    let mut f = Fixture::new();
    f.t.write(
        "foo/BUILD",
        &[
            "genrule(",
            "    name = \"foo\",",
            "    outs = [\"out\"],",
            "    cmd = \"touch $@\",",
            ")",
            "",
            "genrule(",
            "    name = \"bar\",",
            "    outs = [\"out2\"],",
            "    cmd = \"touch $@\",",
            ")",
        ],
    );
    f.t.add_options(&["--experimental_merged_skyframe_analysis_execution"]);
    let expected = BuildGraphMetrics {
        action_lookup_value_count: 8,
        action_lookup_value_count_not_including_aspects: 8,
        action_count: 2,
        action_count_not_including_aspects: 2,
        input_file_configured_target_count: 1,
        output_artifact_count: 2,
        ..Default::default()
    };
    f.build_target(&["//foo:foo", "//foo:bar"]).expect("build");

    assert_graph_expected_fields_only(&graph(&f.metrics()), &expected);

    // Null build.
    f.build_target(&["//foo:foo", "//foo:bar"]).expect("build");

    // Every expected field is 0, so none is present (comparingExpectedFieldsOnly).
    let expected_null_build = BuildGraphMetrics::default();
    assert_graph_expected_fields_only(&graph(&f.metrics()), &expected_null_build);
}
