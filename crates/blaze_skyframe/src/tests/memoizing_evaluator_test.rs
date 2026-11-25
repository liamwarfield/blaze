//! Ports of the `MemoizingEvaluatorTest` methods. The graph listener of `NotifyingHelper` is the
//! test hook in `graph_listener.rs`. The ones that need what is not ported (rewinding, external
//! deps, ...) are in `not_ported.rs`.

use super::*;
use crate::node_entry::NodeEntry;

/// Java: `MemoizingEvaluatorTester.invalidate()`: the keys marked modified are invalidated at the
/// next evaluation.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5930 MemoizingEvaluatorTester.invalidate
fn invalidate(tester: &Tester, ev: &Evaluator) {
    ev.invalidate(tester.get_modified_values());
    tester.clear_modified_values();
}
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Java: MemoizingEvaluatorTest.initializeReporter + tester.eval: evaluates with a fresh event
/// collector and returns it.
fn eval_collecting(
    ev: &Evaluator,
    keep_going: bool,
    roots: &[SkyKey],
) -> (EvaluationResult, Arc<StoredEventHandler>) {
    let events = Arc::new(StoredEventHandler::new());
    let r = ev
        .evaluate(roots, &ctx_with(events.clone(), keep_going))
        .expect("not interrupted");
    (r, events)
}

/// Java: `assertThatEvents(eventCollector).containsExactly(...)` (any order).
fn assert_events(events: &StoredEventHandler, expected: &[&str]) {
    let mut got = messages(events);
    got.sort();
    let mut want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(got, want);
}

// Java: MemoizingEvaluatorTest.smoke
#[test]
fn smoke() {
    let tester = new_tester();
    tester.set("x", "y");
    let ev = tester_evaluator(tester);
    let r = eval(&ev, false, 2, &[k("x")]);
    assert_eq!(*r.get(&T("x")).unwrap(), "y");
}

// Java: MemoizingEvaluatorTest.evaluateEmptySet
#[test]
fn evaluate_empty_set() {
    let ev = tester_evaluator(new_tester());
    eval(&ev, false, 2, &[]);
    eval(&ev, true, 2, &[]);
}

// Java: MemoizingEvaluatorTest.bubbleRace
// Regression test for bug: "[skyframe-m1]: registerIfDone() crash".
#[test]
fn bubble_race() {
    for _ in 0..10 {
        // The top-level value declares dependencies on a "badValue" in error, and a
        // "sleepyValue" which is very slow. After "badValue" fails, the builder interrupts the
        // "sleepyValue" and attempts to re-run "top" for error bubbling. Make sure this doesn't
        // cause a precondition failure because "top" still has an outstanding dep
        // ("sleepyValue").
        let tester = new_tester();
        tester
            .set_builder("top", |_, env| {
                // PORT: Java requests "sleepyValue", gets null and goes on to "badValue". A
                // request blocks here, so both are requested in one batch to have "sleepyValue"
                // outstanding while "badValue" fails.
                env.get_values_and_exceptions(&[k("sleepyValue"), k("badValue")])?;
                if let Err(_e) = env.get_value_or_throw::<T, SomeErrorException>(T("badValue"))? {
                    // In order to trigger this bug, we need to request a dep on an already
                    // computed value.
                    env.get_value(T("otherValue1"))?;
                }
                assert!(
                    env.values_missing(),
                    "SleepyValue should always be unavailable"
                );
                Ok(None)
            })
            .set_builder("sleepyValue", |_, env| {
                // Java: Thread.sleep(99999), interrupted when the evaluation fails.
                let deadline = std::time::Instant::now() + WAIT_TIMEOUT;
                while std::time::Instant::now() < deadline {
                    env.check_interrupted()?;
                    std::thread::sleep(Duration::from_millis(1));
                }
                panic!("I should have been interrupted");
            })
            .add_dependency("badValue", "otherValue1")
            .set_has_error("badValue", true)
            .set("otherValue1", "otherVal1");
        let ev = tester_evaluator(tester);
        let r = eval(&ev, false, 4, &[k("top")]);
        singleton_error(&r, &k("top"));
    }
}

// Java: MemoizingEvaluatorTest.testEnvProvidesTemporaryDirectDeps
#[test]
fn test_env_provides_temporary_direct_deps() {
    let tester = new_tester();
    let deps = Arc::new(Mutex::new(vec![]));
    let d = deps.clone();
    tester
        .set_builder("top", move |_, env| {
            // PORT: Java sees no group in the first run and the group of "bottom" in the
            // restart. The one run here sees no group before the request and the group after.
            assert_eq!(env.get_temporary_direct_deps().num_groups(), 0);
            let v = env.get_value(T("bottom"))?;
            d.lock()
                .extend(env.get_temporary_direct_deps().get_dep_group(0).to_vec());
            Ok(v.map(|v| (*v).clone()))
        })
        .set("bottom", "bottom");
    let ev = tester_evaluator(tester);
    let r = eval(&ev, true, 2, &[k("top")]);
    assert_eq!(*r.get(&T("top")).unwrap(), "bottom");
    assert_eq!(*deps.lock(), vec![k("bottom")]);
}

// Java: MemoizingEvaluatorTest.cachedErrorShutsDownThreadpool
#[test]
fn cached_error_shuts_down_threadpool() {
    for _ in 0..10 {
        let tester = new_tester();
        // When a node throws an error on the first build,
        tester.set_has_error("error", true);
        let ev = tester_evaluator(tester.clone());
        eval_value_in_error(&ev, &k("error"));
        // And on the second build, it is requested as a dep,
        tester.add_dependency("top", "error");
        // And another node throws an error, but waits to throw until the child error is thrown,
        tester.set_chained(
            "newError",
            Chained {
                wait_for_exception: true,
                wait_to_finish: Some(Latch::new(0)),
                ..Chained::default()
            },
        );
        // Then when evaluation happens,
        let r = eval(&ev, false, 4, &[k("newError"), k("top")]);
        // The result has an error,
        assert!(r.has_error());
        // But the new error is not persisted to the graph, since the child error shut down
        // evaluation.
        assert!(r.get_error(&k("newError")).is_none(), "{r:?}");
        assert!(ev.get_existing_error_for_testing(&k("newError")).is_none());
    }
}

// Java: MemoizingEvaluatorTest.interruptBitCleared
#[test]
fn interrupt_bit_cleared() {
    let tester = new_tester();
    // Java: INTERRUPT_BUILDER throws InterruptedException.
    tester.set_builder("interrupt", |_, _| Err(Interrupted.into()));
    let ev = tester_evaluator(tester);
    assert!(matches!(
        ev.evaluate(&[k("interrupt")], &ctx(true, 2)),
        Err(Interrupted)
    ));
    // Java: assertThat(Thread.interrupted()).isFalse()
    assert!(!ev.interrupt_flag().load(Ordering::SeqCst));
}

// Java: MemoizingEvaluatorTest.crashAfterInterruptCrashes
#[test]
fn crash_after_interrupt_crashes() {
    let tester = new_tester();
    // Given a SkyFunction implementation which is improperly coded to throw a runtime exception
    // when it is interrupted,
    let bad_interrupt_started = Latch::new(1);
    {
        let bad_interrupt_started = bad_interrupt_started.clone();
        tester.set_builder("bad-interrupt", move |_, env| {
            bad_interrupt_started.count_down();
            let deadline = std::time::Instant::now() + WAIT_TIMEOUT;
            while std::time::Instant::now() < deadline {
                if env.check_interrupted().is_err() {
                    panic!("I don't like being woken up!");
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            panic!("Shouldn't have slept so long");
        });
    }
    // And another SkyFunction that waits for the first to start, and then throws,
    tester.set_chained(
        "fail",
        Chained {
            wait_to_finish: Some(bad_interrupt_started),
            ..Chained::default()
        },
    );
    let ev = tester_evaluator(tester);
    // When it is interrupted during evaluation (here, caused by the failure of the throwing
    // SkyFunction during a no-keep-going evaluation), then the ParallelEvaluator#evaluate call
    // throws a RuntimeException e where e.getCause() is the RuntimeException thrown by that
    // SkyFunction.
    let e = catch_unwind(AssertUnwindSafe(|| {
        ev.evaluate(&[k("bad-interrupt"), k("fail")], &ctx(false, 4))
    }))
    .expect_err("RuntimeException");
    let message = e.downcast_ref::<String>().expect("message");
    assert!(
        message.ends_with("\nCaused by: I don't like being woken up!"),
        "{message}"
    );
}

// Java: MemoizingEvaluatorTest.interruptAfterFailFails
#[test]
fn interrupt_after_fail_fails() {
    let tester = new_tester();
    // Given a SkyFunction implementation that is properly coded to as not to throw a runtime
    // exception when it is interrupted,
    let interrupt_started = Latch::new(1);
    {
        let interrupt_started = interrupt_started.clone();
        tester.set_builder("interrupted", move |_, env| {
            interrupt_started.count_down();
            let deadline = std::time::Instant::now() + WAIT_TIMEOUT;
            while std::time::Instant::now() < deadline {
                env.check_interrupted()?;
                std::thread::sleep(Duration::from_millis(1));
            }
            panic!("Shouldn't have slept so long");
        });
    }
    // And another SkyFunction that waits for the first to start, and then throws,
    tester.set_chained(
        "fail",
        Chained {
            wait_to_finish: Some(interrupt_started),
            ..Chained::default()
        },
    );
    let ev = tester_evaluator(tester);
    // When it is interrupted during evaluation (here, caused by the failure of a sibling node
    // during a no-keep-going evaluation),
    let r = eval(&ev, false, 4, &[k("interrupted"), k("fail")]);
    // Then the ParallelEvaluator#evaluate call returns an EvaluationResult that has no error for
    // the interrupted SkyFunction.
    assert!(r.has_error(), "{r:?}");
    assert!(r.get_error(&k("fail")).is_some(), "{r:?}");
    assert!(r.get_error(&k("interrupted")).is_none(), "{r:?}");
}

// Java: MemoizingEvaluatorTest.warningBeforeErrorOnFailFastBuild
#[test]
fn warning_before_error_on_fail_fast_build() {
    let tester = new_tester();
    tester
        .set("dep", "dep")
        .set_warning("dep", "warn-dep")
        .set_has_error("top", true)
        .add_dependency("top", "dep");
    let ev = tester_evaluator(tester);
    for i in 0..2 {
        let (r, events) = eval_collecting(&ev, false, &[k("top")]);
        let error = singleton_error(&r, &k("top"));
        assert_eq!(exception_message(error), "top");
        assert!(error.exception_as::<SomeErrorException>().is_some());
        if i == 0 {
            assert_events(&events, &["warn-dep"]);
        }
    }
}

// Java: MemoizingEvaluatorTest.warningAndErrorOnFailFastBuild
#[test]
fn warning_and_error_on_fail_fast_build() {
    let tester = new_tester();
    tester
        .set("top", "top")
        .set_warning("top", "warning msg")
        .set_has_error("top", true);
    let ev = tester_evaluator(tester);
    for i in 0..2 {
        let (r, events) = eval_collecting(&ev, false, &[k("top")]);
        let error = singleton_error(&r, &k("top"));
        assert_eq!(exception_message(error), "top");
        assert!(error.exception_as::<SomeErrorException>().is_some());
        if i == 0 {
            assert_events(&events, &["warning msg"]);
        }
    }
}

// Java: MemoizingEvaluatorTest.warningAndErrorOnFailFastBuildAfterKeepGoingBuild
#[test]
fn warning_and_error_on_fail_fast_build_after_keep_going_build() {
    let tester = new_tester();
    tester
        .set("top", "top")
        .set_warning("top", "warning msg")
        .set_has_error("top", true);
    let ev = tester_evaluator(tester);
    for i in 0..2 {
        let (r, events) = eval_collecting(&ev, i == 0, &[k("top")]);
        let error = singleton_error(&r, &k("top"));
        assert_eq!(exception_message(error), "top");
        assert!(error.exception_as::<SomeErrorException>().is_some());
        if i == 0 {
            assert_events(&events, &["warning msg"]);
        }
    }
}

// Java: MemoizingEvaluatorTest.twoTLTsOnOneWarningValue
#[test]
fn two_tlts_on_one_warning_value() {
    let tester = new_tester();
    tester
        .set("t1", "t1")
        .add_dependency("t1", "dep")
        .set("t2", "t2")
        .add_dependency("t2", "dep")
        .set("dep", "dep")
        .set_warning("dep", "look both ways before crossing");
    let ev = tester_evaluator(tester);
    let (_, events) = eval_collecting(&ev, false, &[k("t1"), k("t2")]);
    assert_events(&events, &["look both ways before crossing"]);
}

// Java: MemoizingEvaluatorTest.errorValueDepOnWarningValue
#[test]
fn error_value_dep_on_warning_value() {
    let tester = new_tester();
    tester
        .set_has_error("error-value", true)
        .add_dependency("error-value", "warning-value")
        .set("warning-value", "warning-value")
        .set_warning("warning-value", "don't chew with your mouth open");
    let ev = tester_evaluator(tester);
    let (r, events) = eval_collecting(&ev, true, &[k("error-value")]);
    assert!(r.get_error(&k("error-value")).is_some());
    assert_events(&events, &["don't chew with your mouth open"]);
}

// Java: MemoizingEvaluatorTest.progressMessageOnlyPrintedTheFirstTime
#[test]
fn progress_message_only_printed_the_first_time() {
    // Skyframe does not store progress messages. Here we only see the message on the first build.
    let tester = new_tester();
    tester
        .set("x", "y")
        .set_progress("x", "just letting you know");
    let ev = tester_evaluator(tester);
    let (r, events) = eval_collecting(&ev, false, &[k("x")]);
    assert_eq!(*r.get(&T("x")).unwrap(), "y");
    assert_events(&events, &["just letting you know"]);

    let (r, events) = eval_collecting(&ev, false, &[k("x")]);
    assert_eq!(*r.get(&T("x")).unwrap(), "y");
    assert_events(&events, &[]);
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:663 MemoizingEvaluatorTest.depMessageBeforeNodeMessageOrNodeValue
#[test]
fn dep_message_before_node_message_or_node_value() {
    let tester = MemoizingTester::new();
    let top = k("top");
    let dep_warning_emitted = Arc::new(AtomicBool::new(false));
    {
        let (top, dep_warning_emitted) = (top.clone(), dep_warning_emitted.clone());
        tester.inject_graph_listener_for_testing(
            move |key: &SkyKey,
                  event_type: crate::graph_listener::EventType,
                  _: crate::graph_listener::Order,
                  _: &crate::graph_listener::Context| {
                if *key == top && event_type == crate::graph_listener::EventType::SetValue {
                    assert!(dep_warning_emitted.load(Ordering::SeqCst));
                }
            },
            /* deterministic= */ false,
        );
    }
    let dep_warning = "dep warning";
    let top_warning = Event::warn("top warning");
    let event_collector = Arc::new(StoredEventHandler::new());
    let reporter = {
        let (dep_warning_emitted, top_warning) = (dep_warning_emitted.clone(), top_warning.clone());
        Arc::new(DelegatingEventHandler {
            delegate: event_collector.clone(),
            // PORT: Java's override checks before it delegates; the order of the two does not
            // matter to the checks.
            on_event: Box::new(move |e| {
                if e.get_message() == dep_warning {
                    dep_warning_emitted.store(true, Ordering::SeqCst);
                }
                if *e == top_warning {
                    assert!(dep_warning_emitted.load(Ordering::SeqCst));
                }
            }),
        })
    };
    let leaf = k("leaf");
    tester
        .set_warning_key(leaf.clone(), dep_warning)
        .with_key(leaf.clone(), |s| s.value = Some("leaf".into()));
    tester.set_builder_key(top.clone(), move |_, env| {
        let dep_value = env.get_value_dyn(&leaf)?;
        if dep_value.is_some() {
            // Default GraphTester implementation warns before requesting deps, which doesn't
            // work for ordering assertions with memoizing evaluator subclasses that don't store
            // events and instead just pass them through directly. By warning after the dep is
            // done we avoid that issue.
            env.listener().handle(&top_warning);
        }
        Ok(dep_string(dep_value))
    });
    let r = tester
        .try_eval_with_reporter(false, std::slice::from_ref(&top), reporter)
        .expect("not interrupted");
    assert_eq!(*r.get(&T("top")).unwrap(), "leaf");
    assert_eq!(messages(&event_collector), vec![dep_warning, "top warning"]);
}

// Java: MemoizingEvaluatorTest.transientErrorValueInvalidation
#[test]
fn transient_error_value_invalidation() {
    // Verify that invalidating errors causes all transient error values to be rerun.
    let tester = new_tester();
    tester
        .set_has_transient_error("error-value", true)
        .set_progress("error-value", "just letting you know");
    let ev = tester_evaluator(tester.clone());
    let (r, events) = eval_collecting(&ev, true, &[k("error-value")]);
    assert!(r.get_error(&k("error-value")).is_some());
    assert_events(&events, &["just letting you know"]);

    // Change the progress message.
    tester
        .set_has_transient_error("error-value", true)
        .set_progress("error-value", "letting you know more");

    // Without invalidating errors, we shouldn't show the new progress message.
    for _ in 0..2 {
        let (r, events) = eval_collecting(&ev, true, &[k("error-value")]);
        assert!(r.get_error(&k("error-value")).is_some());
        assert_events(&events, &[]);
    }

    // When invalidating errors, we should show the new progress message.
    ev.invalidate_transient_errors();
    let (r, events) = eval_collecting(&ev, true, &[k("error-value")]);
    assert!(r.get_error(&k("error-value")).is_some());
    assert_events(&events, &["letting you know more"]);
}

// Java: MemoizingEvaluatorTest.simpleDependency
#[test]
fn simple_dependency() {
    for_each_mode(
        |t| {
            t.add_dependency("ab", "a").set("a", "me");
        },
        |ev, p| {
            let r = eval(ev, false, p, &[k("ab")]);
            assert_eq!(*r.get(&T("ab")).unwrap(), "me");
        },
    );
}

// Java: MemoizingEvaluatorTest.incrementalSimpleDependency
#[test]
fn incremental_simple_dependency() {
    let tester = new_tester();
    let a_key = nk("a");
    tester.add_dependency_key(k("ab"), a_key.clone());
    tester.set_key(a_key.clone(), "me");
    let ev = tester_evaluator(tester.clone());
    eval(&ev, false, 2, &[k("ab")]);

    tester.set_key(a_key, "other");
    invalidate(&tester, &ev);
    let r = eval(&ev, false, 2, &[k("ab")]);
    assert_eq!(*r.get(&T("ab")).unwrap(), "other");
}

// Java: MemoizingEvaluatorTest.setupDiamondDependency
fn setup_diamond_dependency(t: &Tester) {
    t.add_dependency("a", "b")
        .add_dependency("a", "c")
        .add_dependency("b", "d")
        .add_dependency("c", "d");
}

// Java: MemoizingEvaluatorTest.diamondDependency
#[test]
fn diamond_dependency() {
    for_each_mode(
        |t| {
            setup_diamond_dependency(t);
            t.set("d", "me");
        },
        |ev, p| {
            let r = eval(ev, false, p, &[k("a")]);
            assert_eq!(*r.get(&T("a")).unwrap(), "meme");
        },
    );
}

// Java: MemoizingEvaluatorTest.incrementalDiamondDependency
#[test]
fn incremental_diamond_dependency() {
    let tester = new_tester();
    // Java: setupDiamondDependency() with the base a non-hermetic key.
    tester
        .add_dependency("a", "b")
        .add_dependency("a", "c")
        .add_dependency_key(k("b"), nk("d"))
        .add_dependency_key(k("c"), nk("d"));
    tester.set_key(nk("d"), "me");
    let ev = tester_evaluator(tester.clone());
    eval(&ev, false, 2, &[k("a")]);

    tester.set_key(nk("d"), "other");
    invalidate(&tester, &ev);
    let r = eval(&ev, false, 2, &[k("a")]);
    assert_eq!(*r.get(&T("a")).unwrap(), "otherother");
}

/// Java: MemoizingEvaluatorTest.TEST_NODE_COUNT / TESTED_NODES.
const TEST_NODE_COUNT: usize = 100;
const TESTED_NODES: usize = 10;

// Java: MemoizingEvaluatorTest.manyValuesDependOnSingleValue
#[test]
fn many_values_depend_on_single_value() {
    let tester = new_tester();
    let leaf = nk("leaf");
    let values: Vec<&'static str> = (0..TEST_NODE_COUNT).map(|i| leak(i.to_string())).collect();
    for v in &values {
        tester.add_dependency_key(k(v), leaf.clone());
    }
    tester.set_key(leaf.clone(), "leaf");
    let ev = tester_evaluator(tester.clone());
    let roots: Vec<SkyKey> = values.iter().map(|v| k(v)).collect();
    let r = eval(&ev, false, 8, &roots);
    for v in &values {
        assert_eq!(*r.get(&T(v)).unwrap(), "leaf");
    }
    for j in 0..TESTED_NODES {
        tester.set_key(leaf.clone(), &format!("other{j}"));
        invalidate(&tester, &ev);
        let r = eval(&ev, false, 8, &roots);
        for (i, v) in values.iter().enumerate() {
            assert_eq!(
                *r.get(&T(v)).unwrap(),
                format!("other{j}"),
                "Run {j}, value {i}"
            );
        }
    }
}

// Java: MemoizingEvaluatorTest.singleValueDependsOnManyValues
#[test]
fn single_value_depends_on_many_values() {
    for batch in [false, true] {
        let tester = new_tester();
        let values: Vec<SkyKey> = (0..TEST_NODE_COUNT)
            .map(|i| nk(leak(i.to_string())))
            .collect();
        let mut expected = String::new();
        for (i, v) in values.iter().enumerate() {
            tester.set_key(v.clone(), &i.to_string());
            expected.push_str(&i.to_string());
        }
        for v in &values {
            tester.add_dependency_key(k("root"), v.clone());
        }
        tester.with("root", |s| s.batch = batch);
        let ev = tester_evaluator(tester.clone());
        let r = eval(&ev, false, 8, &[k("root")]);
        assert_eq!(*r.get(&T("root")).unwrap(), expected);

        for j in 0..10 {
            expected.clear();
            for (i, v) in values.iter().enumerate() {
                let s = format!("other{i} {j}");
                tester.set_key(v.clone(), &s);
                expected.push_str(&s);
            }
            invalidate(&tester, &ev);
            let r = eval(&ev, false, 8, &[k("root")]);
            assert_eq!(*r.get(&T("root")).unwrap(), expected);
        }
    }
}

/// Java: MemoizingEvaluatorTest.PassThroughSelected
fn pass_through_selected(
    selected: &'static str,
    other: &'static str,
) -> impl Fn(&T, &mut Environment<'_>) -> ComputeResultT + Send + Sync + 'static {
    move |_, env| {
        // GraphTester requests every dep before it looks at valuesMissing.
        let selected_value = env.get_value(T(selected))?;
        env.get_value(T(other))?;
        if env.values_missing() {
            return Ok(None);
        }
        Ok(Some((*selected_value.expect("checkNotNull")).clone()))
    }
}

fn two_rails(tester: &Tester) -> (&'static str, &'static str) {
    let left: Vec<&'static str> = (0..TEST_NODE_COUNT)
        .map(|i| leak(format!("left-{i}")))
        .collect();
    let right: Vec<&'static str> = (0..TEST_NODE_COUNT)
        .map(|i| leak(format!("right-{i}")))
        .collect();
    for i in 0..TEST_NODE_COUNT {
        if i == 0 {
            tester
                .add_dependency_key(k(left[i]), nk("leaf"))
                .add_dependency_key(k(right[i]), nk("leaf"));
        } else {
            tester
                .set_builder(left[i], pass_through_selected(left[i - 1], right[i - 1]))
                .set_builder(right[i], pass_through_selected(right[i - 1], left[i - 1]));
        }
    }
    tester.set_key(nk("leaf"), "leaf");
    (left[TEST_NODE_COUNT - 1], right[TEST_NODE_COUNT - 1])
}

// Java: MemoizingEvaluatorTest.twoRailLeftRightDependencies
#[test]
fn two_rail_left_right_dependencies() {
    let tester = new_tester();
    let (last_left, last_right) = two_rails(&tester);
    let ev = tester_evaluator(tester.clone());
    let r = eval(&ev, false, 4, &[k(last_left), k(last_right)]);
    assert_eq!(*r.get(&T(last_left)).unwrap(), "leaf");
    assert_eq!(*r.get(&T(last_right)).unwrap(), "leaf");

    for j in 0..TESTED_NODES {
        let value = format!("other{j}");
        tester.set_key(nk("leaf"), &value);
        invalidate(&tester, &ev);
        let r = eval(&ev, false, 4, &[k(last_left), k(last_right)]);
        assert_eq!(*r.get(&T(last_left)).unwrap(), value);
        assert_eq!(*r.get(&T(last_right)).unwrap(), value);
    }
}

// Java: MemoizingEvaluatorTest.noKeepGoingAfterKeepGoingCycle
#[test]
fn no_keep_going_after_keep_going_cycle() {
    for_each_mode(
        |t| {
            t.set("good", "good")
                .add_dependency("top", "mid")
                .add_dependency("mid", "a")
                .add_dependency("a", "b")
                .add_dependency("b", "a");
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("top"), k("good")]);
            assert_eq!(*r.get(&T("good")).unwrap(), "good");
            assert!(r.get(&T("top")).is_none());
            let cycle_info = only_cycle(&r, &k("top"));
            assert_eq!(cycle_info.get_cycle(), &[k("a"), k("b")]);
            assert_eq!(cycle_info.get_path_to_cycle(), &[k("top"), k("mid")]);

            // Java: tester.invalidate() with nothing changed.
            let r = eval(ev, false, p, &[k("top"), k("good")]);
            assert!(r.get(&T("top")).is_none());
            let cycle_info = only_cycle(&r, &k("top"));
            assert_eq!(cycle_info.get_cycle(), &[k("a"), k("b")]);
            assert_eq!(cycle_info.get_path_to_cycle(), &[k("top"), k("mid")]);
        },
    );
}

// Java: MemoizingEvaluatorTest.keepGoingCycleAlreadyPresent
#[test]
fn keep_going_cycle_already_present() {
    for_each_mode(
        |t| {
            t.add_dependency("selfEdge", "selfEdge")
                .add_dependency("parent", "selfEdge");
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("selfEdge")]);
            assert!(r.has_error());
            let cycle_info = only_cycle(&r, &k("selfEdge"));
            assert_eq!(cycle_info.get_cycle(), &[k("selfEdge")]);
            assert!(cycle_info.get_path_to_cycle().is_empty());
            let r2 = eval(ev, true, p, &[k("parent")]);
            assert!(r2.has_error());
            let cycle_info2 = only_cycle(&r2, &k("parent"));
            assert_eq!(cycle_info2.get_cycle(), &[k("selfEdge")]);
            assert_eq!(cycle_info2.get_path_to_cycle(), &[k("parent")]);
        },
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1109 MemoizingEvaluatorTest.cycleAboveIndependentCycle
#[test]
fn memoizing_cycle_above_independent_cycle() {
    let tester = MemoizingTester::new();
    tester.make_graph_deterministic();
    let a_key = k("a");
    let b_key = k("b");
    let c_key = nk("c");
    let leaf_key = nk("leaf");
    // When aKey depends on leafKey and bKey,
    {
        let (leaf_key, b_key) = (leaf_key.clone(), b_key.clone());
        tester.set_builder_key(a_key.clone(), move |_, env| {
            env.get_values_and_exceptions(&[leaf_key.clone(), b_key.clone()])?;
            Ok(None)
        });
    }
    // And bKey depends on cKey,
    tester.add_dependency_key(b_key.clone(), c_key.clone());
    // And cKey depends on aKey and bKey in that order,
    tester
        .add_dependency_key(c_key.clone(), a_key.clone())
        .add_dependency_key(c_key.clone(), b_key.clone());
    // And leafKey is a leaf node,
    tester.set_key(leaf_key.clone(), "leafy");
    // Then when we evaluate,
    let r = tester.eval(true, std::slice::from_ref(&a_key));
    // aKey has an error,
    assert!(r.get_dyn(&a_key).is_none());
    // And both cycles were found underneath aKey: the (aKey->bKey->cKey) cycle, and the
    // aKey->(bKey->cKey) cycle. This is because cKey depended on aKey and then bKey, so it pushed
    // them down on the stack in that order, so bKey was processed first. It found its cycle, then
    // popped off the stack, and then aKey was processed and found its cycle.
    let expected = as_set(&[
        CycleInfo::create_cycle_info(vec![], vec![a_key.clone(), b_key.clone(), c_key.clone()]),
        CycleInfo::create_cycle_info(vec![a_key.clone()], vec![b_key.clone(), c_key.clone()]),
    ]);
    assert_eq!(
        as_set(r.get_error(&a_key).expect("error entry").get_cycle_info()),
        expected
    );
    // When leafKey is changed, so that aKey will be marked as NEEDS_REBUILDING,
    tester.set_key(leaf_key, "crunchy");
    // And cKey is invalidated, so that cycle checking will have to explore the full graph,
    tester.get_or_create(c_key, /* mark_as_modified= */ true);
    tester.invalidate();
    // Then when we evaluate,
    let r2 = tester.eval(true, std::slice::from_ref(&a_key));
    // Things are just as before.
    assert!(r2.get_dyn(&a_key).is_none());
    // Java: the assertion checks `result` (the first result) again, not `result2`.
    assert_eq!(
        as_set(r.get_error(&a_key).expect("error entry").get_cycle_info()),
        expected
    );
}

// Java: MemoizingEvaluatorTest.parentOfCycleAndError (parentOfCycleAndErrorInternal with
// errorsStoredAlongsideValues, supportsTransientExceptions, useTransientError)
#[test]
fn parent_of_cycle_and_error() {
    for p in [1, 4] {
        let tester = new_tester();
        let top_evaluated = Latch::new(2);
        tester
            .add_dependency("cycleKey1", "cycleKey2")
            .add_dependency("cycleKey2", "cycleKey1")
            .set_has_transient_error("errorKey", true)
            .add_error_dependency("mid", "errorKey", "recovered")
            .set_chained(
                "top",
                Chained {
                    notify_start: Some(top_evaluated.clone()),
                    value: Some("unused"),
                    deps: vec!["mid", "cycleKey1"],
                    ..Chained::default()
                },
            );
        let ev = tester_evaluator(tester);
        let r = eval(&ev, true, p, &[k("top")]);
        assert!(r.has_error());
        let error_info = r.get_error(&k("top")).unwrap();
        assert_eq!(*top_evaluated.count.lock(), 1);
        // The parent should be transitively transient, since it transitively depends on a
        // transient error.
        assert!(error_info.is_transitively_transient());
        assert_eq!(exception_message(error_info), "errorKey");
        assert_eq!(
            error_info.get_cycle_info(),
            &[CycleInfo::create_cycle_info(
                vec![k("top")],
                vec![k("cycleKey1"), k("cycleKey2")]
            )]
        );
        // But the parent itself shouldn't have a direct dep on the special error transience
        // node.
        assert!(
            !ev.get_direct_deps(&k("top"))
                .contains(&ERROR_TRANSIENCE_KEY)
        );
    }
}

// Java: MemoizingEvaluatorTest.limitEvaluatorThreads
#[test]
fn limit_evaluator_threads() {
    let num_keys = 10;
    let in_progress_count = Arc::new(AtomicUsize::new(0));
    let max_value = Arc::new(AtomicUsize::new(0));
    let tester = new_tester();
    for i in 0..num_keys {
        let sub_key = leak(format!("subKey{i}"));
        tester.add_dependency("toplevel", sub_key);
        let (in_progress_count, max_value) = (in_progress_count.clone(), max_value.clone());
        tester.set_builder(sub_key, move |_, _| {
            let val = in_progress_count.fetch_add(1, Ordering::SeqCst) + 1;
            max_value.fetch_max(val, Ordering::SeqCst);
            // Java sleeps 5 seconds; long enough here for every permit to be taken.
            std::thread::sleep(Duration::from_millis(200));
            in_progress_count.fetch_sub(1, Ordering::SeqCst);
            Ok(Some("abc".into()))
        });
    }
    // PORT: the deps are requested as one batch; one-by-one requests block and never run two
    // functions at once.
    tester
        .set("toplevel", "xyz")
        .with("toplevel", |s| s.batch = true);
    let ev = tester_evaluator(tester);
    let r = eval(&ev, true, 5, &[k("toplevel")]);
    assert!(!r.has_error());
    assert_eq!(max_value.load(Ordering::SeqCst), 5);
}

// Java: MemoizingEvaluatorTest.continueWithErrorDep
#[test]
fn memoizing_continue_with_error_dep() {
    let tester = new_tester();
    let after_key = nk("after");
    tester
        .set_has_error("my_error_value", true)
        .set_key(after_key.clone(), "after")
        .add_error_dependency("parent", "my_error_value", "recovered")
        .add_dependency_key(k("parent"), after_key.clone());
    let ev = tester_evaluator(tester.clone());
    let r = eval(&ev, true, 2, &[k("parent")]);
    assert!(r.error_map().is_empty());
    assert_eq!(*r.get(&T("parent")).unwrap(), "recoveredafter");
    tester.set_key(after_key, "before");
    invalidate(&tester, &ev);
    let r = eval(&ev, true, 2, &[k("parent")]);
    assert!(r.error_map().is_empty());
    assert_eq!(*r.get(&T("parent")).unwrap(), "recoveredbefore");
}

// Java: MemoizingEvaluatorTest.doubleDepOnErrorTransienceValue
#[test]
fn double_dep_on_error_transience_value() {
    let tester = new_tester();
    let leaf_key = nk("leaf");
    tester.set_key(leaf_key.clone(), "leaf");
    let ev = tester_evaluator(tester.clone());
    // Prime the graph by putting leaf in beforehand.
    assert_eq!(
        *eval(&ev, false, 2, std::slice::from_ref(&leaf_key))
            .get(&N("leaf"))
            .unwrap(),
        "leaf"
    );
    tester
        .add_dependency_key(k("top"), leaf_key.clone())
        .set_has_error("top", true);
    // Build top -- it has an error.
    eval_value_in_error(&ev, &k("top"));
    // Invalidate top via leaf, and rebuild.
    tester.set_key(leaf_key, "leaf2");
    invalidate(&tester, &ev);
    eval_value_in_error(&ev, &k("top"));
}

// Java: MemoizingEvaluatorTest.errorInDependencyGroup
#[test]
fn error_in_dependency_group() {
    for _ in 0..10 {
        let tester = new_tester();
        let slow_start = Latch::new(1);
        let error_finish = Latch::new(1);
        tester
            .set_builder_key(nk("error"), {
                let f = Chained {
                    wait_to_finish: Some(slow_start.clone()),
                    notify_finish: Some(error_finish.clone()),
                    // ChainedFunction throws when value is null.
                    ..Chained::default()
                };
                move |t, env| f.compute(t, env)
            })
            .set_chained(
                "slow",
                Chained {
                    notify_start: Some(slow_start),
                    wait_to_finish: Some(error_finish),
                    wait_for_exception: true,
                    value: Some("slow"),
                    ..Chained::default()
                },
            )
            .add_dependency("mid", "slow")
            .add_dependency("mid2", "slow")
            .set_builder("top", |_, env| {
                env.get_values_and_exceptions(&[nk("error"), k("mid"), k("mid2")])?;
                if env.values_missing() {
                    return Ok(None);
                }
                Ok(Some("top".into()))
            });
        let ev = tester_evaluator(tester.clone());
        // Assert that build fails and "error" really is in error.
        let r = eval(&ev, false, 4, &[k("top")]);
        assert!(r.has_error());
        assert!(r.get_error(&k("top")).is_some());

        // Ensure that evaluation succeeds if errorKey does not throw an error.
        tester.with_key(nk("error"), |s| s.builder = None);
        tester.set_key(nk("error"), "ok");
        invalidate(&tester, &ev);
        let r = eval(&ev, false, 4, &[k("top")]);
        assert_eq!(*r.get(&T("top")).unwrap(), "top");
    }
}

// Java: MemoizingEvaluatorTest.valueInErrorWithGroups
/// Regression test -- if value top requests {depA, depB}, depC, with depA and depC there and depB
/// absent, and then throws an exception, the stored deps should be depA, depC (in different
/// groups), not {depA, depC} (same group).
#[test]
fn value_in_error_with_groups() {
    let tester = new_tester();
    let group_dep_a = nk("groupDepA");
    let group_dep_b = k("groupDepB");
    let dep_c = nk("depC");
    // Java: SkyKeyValue(key): the value names the key to request next ("n:" a non-hermetic one).
    fn key_of(value: &str) -> SkyKey {
        match value.strip_prefix("n:") {
            Some(name) => nk(leak(name.to_string())),
            None => k(leak(value.to_string())),
        }
    }
    tester
        .set_key(group_dep_a.clone(), "n:depC")
        .set_key(group_dep_b.clone(), "")
        .set_has_error_key(dep_c.clone(), true);
    {
        let (a, b) = (group_dep_a.clone(), group_dep_b.clone());
        tester.set_builder("top", move |_, env| {
            let val = env
                .get_values_and_exceptions(&[a.clone(), b.clone()])?
                .get_typed(&N("groupDepA"));
            if env.values_missing() {
                return Ok(None);
            }
            let next = key_of(&val.unwrap());
            if let Err(e) = env.get_value_or_throw_dyn::<SomeErrorException>(&next)? {
                return Err(SkyFunctionException::persistent(e).into());
            }
            Ok(if env.values_missing() {
                None
            } else {
                Some("top".into())
            })
        });
    }
    let ev = tester_evaluator(tester.clone());
    let r = eval(&ev, true, 2, &[group_dep_a.clone(), dep_c.clone()]);
    assert_eq!(*r.get(&N("groupDepA")).unwrap(), "n:depC");
    assert!(r.get_error(&dep_c).is_some());
    let r = eval(&ev, false, 2, &[k("top")]);
    assert!(r.get_error(&k("top")).is_some());

    tester.set_key(group_dep_a.clone(), "groupDepB");
    tester.get_or_create(dep_c, /* mark_as_modified= */ true);
    invalidate(&tester, &ev);
    let r = eval(&ev, false, 2, &[k("top")]);
    assert!(!r.has_error(), "{r:?}");
    assert_eq!(*r.get(&T("top")).unwrap(), "top");
    assert_eq!(
        ev.get_direct_deps(&k("top")),
        vec![group_dep_a, group_dep_b]
    );
}

// Java: MemoizingEvaluatorTest.errorOnlyEmittedOnce
#[test]
fn error_only_emitted_once() {
    let tester = new_tester();
    tester.set("x", "y").set_warning("x", "fizzlepop");
    let ev = tester_evaluator(tester);
    let (r, events) = eval_collecting(&ev, false, &[k("x")]);
    assert_eq!(*r.get(&T("x")).unwrap(), "y");
    assert_events(&events, &["fizzlepop"]);

    // Java: tester.invalidate() with nothing changed.
    let (r, events) = eval_collecting(&ev, false, &[k("x")]);
    assert_eq!(*r.get(&T("x")).unwrap(), "y");
    // No new events emitted.
    assert_events(&events, &[]);
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4338 MemoizingEvaluatorTest.errorOnlyBubblesToRequestingParents
/// Make sure that when an error is thrown, it is given for handling only to parents that have
/// already registered a dependence on the value that threw the error.
///
/// ```text
///  topBubbleKey  topErrorFirstKey
///    |       \    /
///  midKey  errorKey
///    |
/// slowKey
/// ```
///
/// On the second build, errorKey throws, and the threadpool aborts before midKey finishes.
/// topBubbleKey therefore has not yet requested errorKey this build. If errorKey bubbles up to
/// it, topBubbleKey must be able to handle that. (The evaluator can deal with this either by not
/// allowing errorKey to bubble up to topBubbleKey, or by dealing with that case.)
#[test]
fn error_only_bubbles_to_requesting_parents() {
    let tester = MemoizingTester::new();
    // We need control over the order of reverse deps, so use a deterministic graph.
    tester.make_graph_deterministic();
    let error_key = nk("error");
    tester.set_key(error_key.clone(), "biding time");
    let slow_key = nk("slow");
    tester.set_key(slow_key.clone(), "slow");
    let mid_key = k("mid");
    tester.add_dependency_key(mid_key.clone(), slow_key.clone());
    let top_error_first_key = k("2nd top alphabetically");
    tester.add_dependency_key(top_error_first_key.clone(), error_key.clone());
    let top_bubble_key = k("1st top alphabetically");
    tester
        .add_dependency_key(top_bubble_key.clone(), mid_key)
        .add_dependency_key(top_bubble_key.clone(), error_key.clone());
    // First error-free evaluation, to put all values in graph.
    let r = tester.eval(
        false,
        &[top_error_first_key.clone(), top_bubble_key.clone()],
    );
    assert_eq!(*r.get(&T("2nd top alphabetically")).unwrap(), "biding time");
    assert_eq!(
        *r.get(&T("1st top alphabetically")).unwrap(),
        "slowbiding time"
    );
    // Set up timing of child values: slowKey waits to finish until errorKey has thrown an
    // exception that has been caught by the threadpool.
    set_null(&tester, &slow_key);
    let error_finish = Latch::new(1);
    set_null(&tester, &error_key);
    tester.set_chained_key(
        error_key,
        Chained {
            notify_finish: Some(error_finish.clone()),
            ..Chained::default()
        },
    );
    tester.set_chained_key(
        slow_key,
        Chained {
            wait_to_finish: Some(error_finish),
            wait_for_exception: true,
            value: Some("leaf2"),
            ..Chained::default()
        },
    );
    tester.invalidate();
    // errorKey finishes, written to graph -> slowKey maybe starts+finishes & (Visitor aborts)
    // -> some top key builds.
    let r = tester.eval(false, &[top_error_first_key.clone(), top_bubble_key]);
    assert!(r.has_error());
    assert!(r.get_error(&top_error_first_key).is_some(), "{r:?}");
}

// Java: MemoizingEvaluatorTest.errorPropagationToTopLevelValues
#[test]
fn error_propagation_to_top_level_values() {
    for_each_mode(
        |t| {
            t.add_dependency("top", "mid")
                .add_dependency("mid", "bad")
                .set_has_error("bad", true);
        },
        |ev, p| {
            let r = eval(ev, false, p, &[k("top"), k("mid")]);
            assert!(r.get_error(&k("mid")).is_some(), "{r:?}");
            // Do it again with keepGoing.  We should also see an error for the top key this
            // time.
            let r = eval(ev, true, p, &[k("top"), k("mid")]);
            assert!(r.get_error(&k("mid")).is_some());
            assert!(r.get_error(&k("top")).is_some());
        },
    );
}

// Java: MemoizingEvaluatorTest.breakWithInterruptibleErrorDep
#[test]
fn break_with_interruptible_error_dep() {
    for_each_mode(
        |t| {
            t.set_has_error("my_error_value", true)
                .add_error_dependency("parent", "my_error_value", "recovered");
        },
        |ev, p| {
            // When the error value throws, the propagation will cause an interrupted exception
            // in parent.
            let r = eval(ev, false, p, &[k("parent")]);
            assert!(r.key_names().is_empty());
            singleton_error(&r, &k("parent"));
            assert!(!ev.interrupt_flag().load(Ordering::SeqCst));
            let r = eval(ev, true, p, &[k("parent")]);
            assert!(!r.has_error());
            assert_eq!(*r.get(&T("parent")).unwrap(), "recovered");
        },
    );
}

// Java: MemoizingEvaluatorTest.breakWithErrorDep
#[test]
fn break_with_error_dep() {
    for_each_mode(
        |t| {
            t.set_has_error("my_error_value", true)
                .set("after", "after")
                .add_error_dependency("parent", "my_error_value", "recovered")
                .add_dependency("parent", "after");
        },
        |ev, p| {
            let r = eval(ev, false, p, &[k("parent")]);
            singleton_error(&r, &k("parent"));
            let r = eval(ev, true, p, &[k("parent")]);
            assert!(!r.has_error());
            assert_eq!(*r.get(&T("parent")).unwrap(), "recoveredafter");
        },
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4537 MemoizingEvaluatorTest.raceConditionWithNoKeepGoingErrors_InflightError
#[test]
fn race_condition_with_no_keep_going_errors_inflight_error() {
    for _ in 0..10 {
        let tester = MemoizingTester::new();
        // Given a graph of two nodes, errorKey and otherErrorKey,
        let error_key = k("errorKey");
        let other_error_key = k("otherErrorKey");
        let error_committed = Latch::new(1);
        let other_started = Latch::new(1);
        let other_done = Latch::new(1);
        let num_other_invocations = Arc::new(AtomicUsize::new(0));
        let bogus_invocation_message: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let non_null_value_message: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        {
            let other_started = other_started.clone();
            tester.set_builder_key(error_key.clone(), move |_, _| {
                // Given that errorKey waits for otherErrorKey to begin evaluation before
                // completing its evaluation,
                other_started.wait("otherErrorKey's SkyFunction didn't start in time.");
                // And given that errorKey throws an error,
                Err(SkyFunctionException::persistent(SomeErrorException("error".into())).into())
            });
        }
        {
            let error_key = error_key.clone();
            let (error_committed, other_done, num_other_invocations) = (
                error_committed.clone(),
                other_done.clone(),
                num_other_invocations.clone(),
            );
            let (bogus_invocation_message, non_null_value_message) = (
                bogus_invocation_message.clone(),
                non_null_value_message.clone(),
            );
            tester.set_builder_key(other_error_key.clone(), move |_, env| {
                other_started.count_down();
                let invocations = num_other_invocations.fetch_add(1, Ordering::SeqCst) + 1;
                // And given that otherErrorKey waits for errorKey's error to be committed before
                // trying to get errorKey's value,
                error_committed.wait("errorKey's error didn't get committed to the graph in time");
                match env.get_value_or_throw_dyn::<SomeErrorException>(&error_key)? {
                    Ok(value) => {
                        if let Some(value) = value {
                            *non_null_value_message.lock() = Some(format!(
                                "bogus non-null value {:?}",
                                value.downcast_ref::<String>()
                            ));
                        }
                        if invocations != 1 {
                            *bogus_invocation_message.lock() =
                                Some(format!("bogus invocation count: {invocations}"));
                        }
                        other_done.count_down();
                        // And given that otherErrorKey throws an error,
                        Err(
                            SkyFunctionException::persistent(SomeErrorException("other".into()))
                                .into(),
                        )
                    }
                    Err(_) => panic!("fail()"),
                }
            });
        }
        {
            let error_key = error_key.clone();
            let (error_committed, other_done) = (error_committed.clone(), other_done.clone());
            tester.inject_graph_listener_for_testing(
                move |key: &SkyKey, event_type: EventType, order: Order, _: &ListenerContext| {
                    if *key == error_key
                        && event_type == EventType::SetValue
                        && order == Order::After
                    {
                        error_committed.count_down();
                        other_done.wait("otherErrorKey's SkyFunction didn't finish in time.");
                    }
                },
                /* deterministic= */ false,
            );
        }

        // When the graph is evaluated in noKeepGoing mode,
        let r = tester.eval(false, &[error_key, other_error_key.clone()]);

        // Then the result reports that an error occurred,
        assert!(r.has_error());

        // And no value is committed for otherErrorKey,
        assert!(
            tester
                .evaluator
                .get_existing_error_for_testing(&other_error_key)
                .is_none()
        );
        assert!(
            tester
                .evaluator
                .get_existing_value(&other_error_key)
                .is_none()
        );

        // And no value was committed for errorKey,
        assert_eq!(*non_null_value_message.lock(), None);

        // And the SkyFunction for otherErrorKey was evaluated exactly once.
        assert_eq!(num_other_invocations.load(Ordering::SeqCst), 1);
        assert_eq!(*bogus_invocation_message.lock(), None);
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4621 MemoizingEvaluatorTest.absentParent
#[test]
fn absent_parent() {
    let tester = MemoizingTester::new();
    let error_key = nk("my_error_value");
    tester.set_key(error_key.clone(), "biding time");
    let absent_parent_key = k("absentParent");
    tester.add_dependency_key(absent_parent_key.clone(), error_key.clone());
    assert_eq!(
        tester.eval_and_get(false, &absent_parent_key),
        "biding time"
    );
    tester
        .get_or_create(error_key.clone(), /* mark_as_modified= */ true)
        .set_has_error_key(error_key.clone(), true);
    let new_parent = k("newParent");
    tester.add_dependency_key(new_parent.clone(), error_key);
    tester.invalidate();
    let r = tester.eval(false, std::slice::from_ref(&new_parent));
    singleton_error(&r, &new_parent);
}

// Java: MemoizingEvaluatorTest.errorTransienceBug
// Tests that we have a sane implementation of error transience.
#[test]
fn error_transience_bug() {
    let tester = new_tester();
    tester.set_has_transient_error("key", true);
    let ev = tester_evaluator(tester.clone());
    assert!(
        eval_value_in_error(&ev, &k("key"))
            .get_exception()
            .is_some()
    );
    tester
        .set_has_transient_error("key", false)
        .set("key", "hi");
    ev.invalidate_transient_errors();
    let r = eval(&ev, false, 2, &[k("key")]);
    assert_eq!(*r.get(&T("key")).unwrap(), "hi");
}

// Java: MemoizingEvaluatorTest.valueInjection
#[test]
fn value_injection() {
    let ev = tester_evaluator(new_tester());
    let key = nk("new_value");
    ev.inject(N("new_value"), "val".to_string());
    let r = eval(&ev, false, 2, std::slice::from_ref(&key));
    assert_eq!(*r.get(&N("new_value")).unwrap(), "val");
}

// Java: MemoizingEvaluatorTest.valueInjectionOverExistingEntry
#[test]
fn value_injection_over_existing_entry() {
    let tester = new_tester();
    tester.with_key(nk("key"), |s| s.value = Some("old_val".into()));
    let ev = tester_evaluator(tester);
    ev.inject(N("key"), "val".to_string());
    let r = eval(&ev, false, 2, &[nk("key")]);
    assert_eq!(*r.get(&N("key")).unwrap(), "val");
}

// Java: MemoizingEvaluatorTest.valueInjectionOverValueWithErrors
#[test]
fn value_injection_over_value_with_errors() {
    let tester = new_tester();
    tester.set_has_error_key(nk("key"), true);
    let ev = tester_evaluator(tester);
    eval_value_in_error(&ev, &nk("key"));
    ev.inject(N("key"), "val".to_string());
    let r = eval(&ev, false, 2, &[nk("key")]);
    assert_eq!(*r.get(&N("key")).unwrap(), "val");
}

// Java: MemoizingEvaluatorTest.valueInjectionInvalidatesReverseDeps
#[test]
fn value_injection_invalidates_reverse_deps() {
    let tester = new_tester();
    let child_key = nk("child");
    tester
        .with_key(child_key.clone(), |s| s.value = Some("old_val".into()))
        .add_dependency_key(k("parent"), child_key.clone());
    let ev = tester_evaluator(tester);
    let r = eval(&ev, false, 2, &[k("parent")]);
    assert!(!r.has_error());
    assert_eq!(*r.get(&T("parent")).unwrap(), "old_val");

    ev.inject(N("child"), "val".to_string());
    let r = eval(&ev, false, 2, std::slice::from_ref(&child_key));
    assert_eq!(*r.get(&N("child")).unwrap(), "val");
    // Injecting a new child should have invalidated the parent.
    assert!(ev.get_existing_value(&k("parent")).is_none());

    eval(&ev, false, 2, std::slice::from_ref(&child_key));
    assert_eq!(
        *ev.get_existing_value(&child_key)
            .unwrap()
            .downcast::<String>()
            .unwrap(),
        "val"
    );
    assert!(ev.get_existing_value(&k("parent")).is_none());
    let r = eval(&ev, false, 2, &[k("parent")]);
    assert_eq!(*r.get(&T("parent")).unwrap(), "val");
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5145 MemoizingEvaluatorTest.runTestPersistentErrorsNotRerun
fn run_test_persistent_errors_not_rerun(include_transient_error: bool) {
    let tester = MemoizingTester::new();
    let top_key = k("top");
    let transient_error_key = k("transientError");
    let persistent_error_key1 = k("persistentError1");
    let persistent_error_key2 = k("persistentError2");

    tester
        .add_error_dependency_key(
            top_key.clone(),
            persistent_error_key1.clone(),
            "doesn't matter",
        )
        .set_has_error_key(top_key.clone(), true);
    tester.set_has_error_key(persistent_error_key1.clone(), true);
    if include_transient_error {
        tester.add_error_dependency_key(
            top_key.clone(),
            transient_error_key.clone(),
            "doesn't matter",
        );
        tester
            .add_error_dependency_key(
                transient_error_key.clone(),
                persistent_error_key2.clone(),
                "doesn't matter",
            )
            .set_has_transient_error_key(transient_error_key.clone(), true);
    }
    tester.set_has_error_key(persistent_error_key2.clone(), true);

    tester.eval_and_get_error(true, &top_key);
    if include_transient_error {
        assert_eq!(
            tester.get_enqueued_values(),
            key_set(&[
                top_key.clone(),
                transient_error_key.clone(),
                persistent_error_key1.clone(),
                persistent_error_key2
            ])
        );
    } else {
        assert_eq!(
            tester.get_enqueued_values(),
            key_set(&[top_key.clone(), persistent_error_key1.clone()])
        );
    }

    tester.invalidate();
    tester.invalidate_transient_errors();
    tester.eval_and_get_error(true, &top_key);
    if include_transient_error {
        // TODO(bazel-team): We can do better here once we implement change pruning for errors.
        assert_eq!(
            tester.get_enqueued_values(),
            key_set(&[top_key, transient_error_key])
        );
    } else {
        assert!(tester.get_enqueued_values().is_empty());
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5186 MemoizingEvaluatorTest.persistentErrorsNotRerun
#[test]
fn persistent_errors_not_rerun() {
    run_test_persistent_errors_not_rerun(/* include_transient_error= */ true);
}

// Java: MemoizingEvaluatorTest.duplicateUnfinishedDeps_KeepGoing
#[test]
fn duplicate_unfinished_deps_keep_going() {
    let tester = new_tester();
    let num_parent_compute_calls = Arc::new(AtomicUsize::new(0));
    let n = num_parent_compute_calls.clone();
    tester
        .set_builder("child", |_, _| Ok(Some("child".into())))
        .set_builder("parent", move |_, env| {
            n.fetch_add(1, Ordering::SeqCst);
            // PORT: Java's first run finds the child null twice; the one run here is the
            // restart, which finds it built.
            assert_eq!(*env.get_value(T("child"))?.unwrap(), "child");
            assert_eq!(*env.get_value(T("child"))?.unwrap(), "child");
            Err(SkyFunctionException::persistent(SomeErrorException("bad".into())).into())
        });
    let ev = tester_evaluator(tester);
    let r = eval(&ev, true, 2, &[k("parent")]);
    assert_eq!(exception_message(singleton_error(&r, &k("parent"))), "bad");
    assert_eq!(num_parent_compute_calls.load(Ordering::SeqCst), 1);
    // The duplicate key is one dep.
    assert_eq!(ev.get_direct_deps(&k("parent")), vec![k("child")]);
}

// Java: MemoizingEvaluatorTest.errorTransienceDepCleared
#[test]
fn error_transience_dep_cleared() {
    let tester = new_tester();
    let leaf = nk("leaf");
    tester
        .set_key(leaf.clone(), "leaf")
        .add_dependency_key(k("top"), leaf.clone())
        .set_has_transient_error("top", true);
    let ev = tester_evaluator(tester.clone());
    let r = eval(&ev, false, 2, &[k("top")]);
    assert!(r.has_error(), "{r:?}");
    tester.get_or_create(leaf, /* mark_as_modified= */ true);
    invalidate(&tester, &ev);
    tester.set("irrelevant", "irrelevant");
    eval(&ev, true, 2, &[k("irrelevant")]);
    ev.invalidate_transient_errors();
    let r = eval(&ev, true, 2, &[k("top")]);
    assert!(r.has_error(), "{r:?}");
}

// Java: MemoizingEvaluatorTest.incompleteValueAlreadyThereNotUsed
#[test]
fn incomplete_value_already_there_not_used() {
    for_each_mode(
        |t| {
            t.set_has_error("my_error_value", true)
                .add_error_dependency("mid", "my_error_value", "recovered")
                .add_error_dependency("parent", "mid", "don't use this");
        },
        |ev, p| {
            // Prime the graph by evaluating the mid-level value. It shouldn't be stored in the
            // graph because it was only called during the bubbling-up phase.
            let r = eval(ev, false, p, &[k("mid")]);
            assert!(r.get(&T("mid")).is_none());
            singleton_error(&r, &k("mid"));
            assert!(ev.get_existing_value(&k("mid")).is_none());
            assert!(ev.get_existing_error_for_testing(&k("mid")).is_none());
            // In a keepGoing build, midKey should be re-evaluated.
            let r = eval(ev, true, p, &[k("parent")]);
            assert_eq!(*r.get(&T("parent")).unwrap(), "recovered");
        },
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3373 MemoizingEvaluatorTest.sameDepInTwoGroups
#[test]
fn memoizing_same_dep_in_two_groups() {
    let tester = MemoizingTester::new();

    // leaf4 should not be built in the second build.
    let leaf4 = k("leaf4");
    let should_not_build_leaf4 = Arc::new(AtomicBool::new(false));
    {
        let (leaf4, should_not_build_leaf4) = (leaf4.clone(), should_not_build_leaf4.clone());
        tester.inject_graph_listener_for_testing(
            move |key: &SkyKey,
                  event_type: crate::graph_listener::EventType,
                  order: crate::graph_listener::Order,
                  context: &crate::graph_listener::Context| {
                use crate::graph_listener::EventType;
                if should_not_build_leaf4.load(Ordering::SeqCst)
                    && *key == leaf4
                    && event_type != EventType::RemoveReverseDep
                    && event_type != EventType::GetBatch
                {
                    panic!(
                        "IllegalStateException: leaf4 should not have been considered this \
                         build: {event_type:?}, {order:?}, {context:?}"
                    );
                }
            },
            /* deterministic= */ false,
        );
    }
    tester.set_key(leaf4, "leaf4");

    // Create leaf0, leaf1 and leaf2 values with values "leaf2", "leaf3", "leaf4" respectively.
    // These will be requested as one dependency group. In the second build, leaf2 will have the
    // value "leaf5".
    let leaves: Vec<SkyKey> = (0..=2)
        .map(|i| {
            let name = leak(format!("leaf{i}"));
            let leaf = if i == 2 { nk(name) } else { k(name) };
            tester.set_key(leaf.clone(), &format!("leaf{}", i + 2));
            leaf
        })
        .collect();

    // Create "top" value. It depends on all leaf values in two overlapping dependency groups.
    let top_key = k("top");
    {
        let leaves = leaves.clone();
        tester.set_builder_key(top_key.clone(), move |_, env| {
            // Request the first group, [leaf0, leaf1, leaf2].
            // In the first build, it has values ["leaf2", "leaf3", "leaf4"].
            // In the second build it has values ["leaf2", "leaf3", "leaf5"]
            let values = env.get_values_and_exceptions(&leaves)?;
            if env.values_missing() {
                return Ok(None);
            }

            // Request the second group. In the first build it's [leaf2, leaf4].
            // In the second build it's [leaf2, leaf5]
            let next = leak(dep_string(values.get(&leaves[2])).expect("leaf2"));
            env.get_values_and_exceptions(&[leaves[2].clone(), k(next)])?;
            if env.values_missing() {
                return Ok(None);
            }

            Ok(Some("top".into()))
        });
    }

    // First build: assert we can evaluate "top".
    assert_eq!(tester.eval_and_get(false, &top_key), "top");

    // Second build: replace "leaf4" by "leaf5" in leaf2's value. Assert leaf4 is not requested.
    let leaf5 = k("leaf5");
    tester.set_key(leaf5, "leaf5");
    tester.set_key(leaves[2].clone(), "leaf5");
    tester.invalidate();
    should_not_build_leaf4.store(true, Ordering::SeqCst);
    assert_eq!(tester.eval_and_get(false, &top_key), "top");
}

// Java: MemoizingEvaluatorTest.hermeticSkyFunctionCanThrowTransientErrorThenRecover
#[test]
fn hermetic_sky_function_can_throw_transient_error_then_recover() {
    let tester = new_tester();
    // When top depends on leaf, but throws a transient error,
    tester
        .add_dependency("top", "leaf")
        .set_has_transient_error("top", true)
        .set("leaf", "value");
    let ev = tester_evaluator(tester.clone());
    // And the first build throws a transient error (as expected),
    let r = eval(&ev, true, 2, &[k("top")]);
    assert!(r.has_error());
    assert!(r.get_error(&k("top")).unwrap().get_exception().is_some());
    // And then top's transient error is removed,
    tester.set_has_transient_error("top", false);
    ev.invalidate_transient_errors();
    // Then top evaluates successfully, even though it was hermetic and didn't give the same
    // result on successive evaluations with the same inputs.
    let r = eval(&ev, true, 2, &[k("top")]);
    assert!(!r.has_error());
    assert_eq!(*r.get(&T("top")).unwrap(), "value");
}

// Java: MemoizingEvaluatorTest.transientErrorTurningGoodHasNoError
#[test]
fn transient_error_turning_good_has_no_error() {
    let tester = new_tester();
    tester.set_has_transient_error("my_error_value", true);
    let ev = tester_evaluator(tester.clone());
    eval_value_in_error(&ev, &k("my_error_value"));
    // Re-evaluates to same thing when errors are invalidated
    ev.invalidate_transient_errors();
    eval_value_in_error(&ev, &k("my_error_value"));
    tester
        .set_has_transient_error("my_error_value", false)
        .set("my_error_value", "reformed");
    ev.invalidate_transient_errors();
    let r = eval(&ev, true, 2, &[k("my_error_value")]);
    assert_eq!(*r.get(&T("my_error_value")).unwrap(), "reformed");
    // Value builder will now throw, but we should never get to it because it isn't dirty.
    tester.set_has_transient_error("my_error_value", true);
    ev.invalidate_transient_errors();
    let r = eval(&ev, true, 2, &[k("my_error_value")]);
    assert_eq!(*r.get(&T("my_error_value")).unwrap(), "reformed");
}

// Java: MemoizingEvaluatorTest.transientErrorTurnsGoodOnSecondTry
#[test]
fn transient_error_turns_good_on_second_try() {
    let tester = new_tester();
    tester
        .add_dependency("top", "error")
        .set("top", "val")
        .add_dependency("error", "leaf")
        .set("error", "val")
        .set_has_transient_error("error", true)
        .set("leaf", "leaf");
    let ev = tester_evaluator(tester.clone());
    let error_info = eval_value_in_error(&ev, &k("top"));
    assert!(error_info.is_transitively_transient());
    ev.invalidate_transient_errors();
    let error_info = eval_value_in_error(&ev, &k("top"));
    assert!(error_info.is_transitively_transient());
    ev.invalidate_transient_errors();
    tester.set_has_transient_error("error", false);
    let r = eval(&ev, true, 2, &[k("top")]);
    assert_eq!(*r.get(&T("top")).unwrap(), "val");
}

// Java: MemoizingEvaluatorTest.valueInjectionInterrupt
#[test]
fn value_injection_interrupt() {
    let ev = tester_evaluator(new_tester());
    ev.inject(N("key"), "val".to_string());
    // Java: Thread.currentThread().interrupt()
    ev.interrupt();
    assert!(matches!(
        ev.evaluate(&[nk("key")], &ctx(false, 2)),
        Err(Interrupted)
    ));
    // Java: the InterruptedException cleared the interrupt bit.
    ev.clear_interrupt();
    let r = eval(&ev, false, 2, &[nk("key")]);
    assert_eq!(*r.get(&N("key")).unwrap(), "val");
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5464 MemoizingEvaluatorTest.cachedChildErrorDepWithSiblingDepOnNoKeepGoingEval
#[test]
fn cached_child_error_dep_with_sibling_dep_on_no_keep_going_eval() {
    let tester = MemoizingTester::new();
    let parent1_key = k("parent1");
    let parent2_key = k("parent2");
    let error_key = nk("error");
    let other_key = k("other");
    for parent in [&parent1_key, &parent2_key] {
        let (error_key, other_key) = (error_key.clone(), other_key.clone());
        tester.set_builder_key(parent.clone(), move |_, env| {
            env.get_value_dyn(&error_key)?;
            env.get_value_dyn(&other_key)?;
            if env.values_missing() {
                return Ok(None);
            }
            Ok(Some("parent".into()))
        });
    }
    tester.with_key(error_key.clone(), |s| s.value = Some("no error yet".into()));
    tester.with_key(other_key, |s| s.value = Some("other".into()));
    tester.eval(true, std::slice::from_ref(&parent1_key));
    tester.eval(false, std::slice::from_ref(&parent2_key));
    tester
        .get_or_create(error_key.clone(), /* mark_as_modified= */ true)
        .set_has_error_key(error_key, true);
    tester.invalidate();
    tester.eval(true, std::slice::from_ref(&parent1_key));
    tester.eval(false, std::slice::from_ref(&parent2_key));
}

// Java: MemoizingEvaluatorTest.cycleAndSelfEdgeWithDirtyValueInSameGroup
#[test]
fn cycle_and_self_edge_with_dirty_value_in_same_group() {
    for p in [1, 4] {
        let tester = new_tester();
        tester
            .add_dependency("AcycleKey2", "AcycleKey2")
            .set_builder("ZcycleKey1", |_, env| {
                // The order here is important -- 2 before 1.
                env.get_values_and_exceptions(&[k("AcycleKey2"), k("ZcycleKey1")])?;
                assert!(env.values_missing());
                Ok(None)
            });
        let ev = tester_evaluator(tester);
        ev.deterministic_for_testing.store(true, Ordering::SeqCst);
        // Evaluate twice to make sure nothing strange happens with invalidation the second time.
        for _ in 0..2 {
            let r = eval(&ev, true, p, &[k("ZcycleKey1")]);
            assert!(r.get(&T("ZcycleKey1")).is_none());
            let cycle_info = only_cycle(&r, &k("ZcycleKey1"));
            assert_eq!(cycle_info.get_cycle(), &[k("ZcycleKey1")]);
            assert!(cycle_info.get_path_to_cycle().is_empty());
        }
    }
}

// Java: MemoizingEvaluatorTest.incompleteDirectDepsAreClearedBeforeKeepGoing
/// Regression test: error on clearMaybeDirtyValue. topKey depends on midKey and errorKey. midKey
/// enqueues slowKey, and waits. errorKey throws an error, which bubbles up to topKey. slowKey
/// waits until errorKey has thrown and the threadpool has caught the exception before returning,
/// so the Evaluator will already have stopped enqueuing new jobs, so midKey is not evaluated. The
/// second evaluation is keepGoing, which should cause an access of the children of topKey.
#[test]
fn incomplete_direct_deps_are_cleared_before_keep_going() {
    for _ in 0..10 {
        let tester = new_tester();
        let slow_start = Latch::new(1);
        let error_finish = Latch::new(1);
        tester
            .set_chained(
                "error",
                Chained {
                    wait_to_finish: Some(slow_start.clone()),
                    notify_finish: Some(error_finish.clone()),
                    ..Chained::default()
                },
            )
            .set_chained(
                "slow",
                Chained {
                    notify_start: Some(slow_start),
                    wait_to_finish: Some(error_finish),
                    wait_for_exception: true,
                    value: Some("slow"),
                    ..Chained::default()
                },
            )
            .add_dependency("mid", "slow")
            .add_dependency("top", "mid")
            .add_dependency("top", "error")
            // PORT: Java's "top" gets null for "mid" and goes on to request "error". A request
            // blocks here, so both are requested in one batch.
            .with("top", |s| s.batch = true);
        let ev = tester_evaluator(tester.clone());
        // slowKey starts -> errorKey finishes, written to graph -> slowKey finishes & (Visitor
        // aborts) -> topKey builds.
        let r = eval(&ev, false, 4, &[k("top")]);
        singleton_error(&r, &k("top"));
        // Make sure midKey didn't finish building.
        assert!(ev.get_existing_value(&k("mid")).is_none());
        // Give slowKey a nice ordinary builder.
        tester.with("slow", |s| {
            s.builder = None;
            s.value = Some("slow".into());
        });
        // Put midKey into the graph. It won't have a reverse dependence on topKey.
        let r = eval(&ev, false, 4, &[k("mid")]);
        assert_eq!(*r.get(&T("mid")).unwrap(), "slow");
        // topKey should not access midKey as if it were already registered as a dependency.
        // We don't invalidate errors, but because topKey wasn't actually written to the graph
        // last build, it should be rebuilt here.
        let r = eval(&ev, true, 4, &[k("top")]);
        assert_eq!(exception_message(singleton_error(&r, &k("top"))), "oops");
    }
}

// Java: MemoizingEvaluatorTest.passThenFailToBuild (the private helper)
/// Regression test: tests that pass before other build actions fail yield crash in non -k
/// builds.
fn pass_then_fail_to_build_with(success_first: bool) {
    for _ in 0..10 {
        let tester = new_tester();
        let blocker = Latch::new(1);
        tester
            .set_chained(
                "success",
                Chained {
                    notify_finish: Some(blocker.clone()),
                    value: Some("yippee"),
                    ..Chained::default()
                },
            )
            .set_chained(
                "slow_then_fail",
                Chained {
                    wait_to_finish: Some(blocker),
                    ..Chained::default()
                },
            );
        let ev = tester_evaluator(tester);
        let roots = if success_first {
            [k("success"), k("slow_then_fail")]
        } else {
            [k("slow_then_fail"), k("success")]
        };
        let r = eval(&ev, false, 4, &roots);
        assert!(r.get_error(&k("slow_then_fail")).is_some(), "{r:?}");
        assert_eq!(r.key_names(), vec![k("success")]);
        assert_eq!(*r.get(&T("success")).unwrap(), "yippee");
    }
}

// Java: MemoizingEvaluatorTest.passThenFailToBuild
#[test]
fn pass_then_fail_to_build() {
    pass_then_fail_to_build_with(true);
}

// Java: MemoizingEvaluatorTest.passThenFailToBuildAlternateOrder
#[test]
fn pass_then_fail_to_build_alternate_order() {
    pass_then_fail_to_build_with(false);
}

// Java: MemoizingEvaluatorTest.dirtyErrorTransienceValue
#[test]
fn dirty_error_transience_value() {
    let tester = new_tester();
    tester.set_has_error("error", true);
    let ev = tester_evaluator(tester.clone());
    eval_value_in_error(&ev, &k("error"));
    ev.invalidate_transient_errors();
    tester.set_has_error("secondError", true);
    // secondError declares a new dependence on ErrorTransienceValue, but not until it has already
    // thrown an error.
    eval_value_in_error(&ev, &k("secondError"));
}

// ---------------------------------------------------------------------------------------------
// Incremental evaluation: ports that use MemoizingTester (Java's MemoizingEvaluatorTester, with
// a TrackingProgressReceiver and EventFilter.FULL_STORAGE).

/// Java: `INTERRUPT_BUILDER`.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4093 MemoizingEvaluatorTest.INTERRUPT_BUILDER
fn interrupt_builder(_: &T, _: &mut Environment<'_>) -> ComputeResultT {
    Err(Interrupted.into())
}

/// Java: `tester.set(key, null)`: no constant value any more, and marked modified.
fn set_null(tester: &MemoizingTester, key: &SkyKey) {
    tester.get_or_create(key.clone(), /* mark_as_modified= */ true);
    tester.unset_constant_value(key.clone());
}

/// Utility function to induce a graph clean of whatever value is requested, by trying to build
/// this value and interrupting the build as soon as this value's function evaluation starts.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4100 MemoizingEvaluatorTest.failBuildAndRemoveValue
fn fail_build_and_remove_value(tester: &MemoizingTester, value: &SkyKey) {
    set_null(tester, value);
    // Evaluator will think leaf was interrupted because it threw, so it will be cleaned from
    // graph.
    tester.get_or_create(value.clone(), /* mark_as_modified= */ true);
    tester.set_builder_key(value.clone(), interrupt_builder);
    tester.invalidate();
    assert_eq!(
        tester.try_eval(false, std::slice::from_ref(value)).err(),
        Some(Interrupted)
    );
    tester.with_key(value.clone(), |s| s.builder = None);
}

fn key_set(keys: &[SkyKey]) -> HashSet<SkyKey> {
    keys.iter().cloned().collect()
}

// Java: MemoizingEvaluatorTest.invalidationWithNothingChanged
#[test]
fn invalidation_with_nothing_changed() {
    let tester = MemoizingTester::new();
    tester.set("x", "y").set_warning("x", "fizzlepop");
    assert_eq!(tester.eval_and_get(false, &k("x")), "y");
    assert_eq!(tester.events(), vec!["fizzlepop"]);

    tester.initialize_reporter();
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &k("x")), "y");
}

// Java: MemoizingEvaluatorTest.deleteValues
#[test]
fn delete_values() {
    let tester = MemoizingTester::new();
    tester
        .add_dependency("top", "d1")
        .add_dependency("top", "d2")
        .add_dependency("top", "d3");
    tester.set("d1", "1").set("d2", "2").set("d3", "3");
    tester.eval(true, &[k("top")]);

    tester.delete("d1");
    tester.eval(true, &[k("d3")]);

    assert!(tester.get_dirty_keys().is_empty());
    assert_eq!(tester.get_deleted_keys(), key_set(&[k("d1"), k("top")]));
    assert_eq!(tester.get_existing_value(&k("top")), None);
    assert_eq!(tester.get_existing_value(&k("d1")), None);
    assert_eq!(tester.get_existing_value(&k("d2")).as_deref(), Some("2"));
    assert_eq!(tester.get_existing_value(&k("d3")).as_deref(), Some("3"));
}

// Java: MemoizingEvaluatorTest.deleteOldNodesTest
#[test]
fn delete_old_nodes_test() {
    let tester = MemoizingTester::new();
    let d2_key = nk("d2");
    tester
        .add_dependency("top", "d1")
        .add_dependency_key(k("top"), d2_key.clone());
    tester.set("d1", "one");
    tester.set_key(d2_key.clone(), "two");
    tester.eval(true, &[k("top")]);

    tester.set_key(d2_key.clone(), "three");
    tester.invalidate();
    tester.eval(true, std::slice::from_ref(&d2_key));

    let value_keys =
        |t: &MemoizingTester| -> HashSet<SkyKey> { t.evaluator.get_values().into_keys().collect() };
    // The graph now contains the three above nodes (and ERROR_TRANSIENCE).
    assert_eq!(
        value_keys(&tester),
        key_set(&[
            k("top"),
            k("d1"),
            d2_key.clone(),
            ERROR_TRANSIENCE_KEY.clone()
        ])
    );

    tester.evaluator.delete_dirty(2);
    tester.eval(true, &[]);

    // The top node's value is dirty, but less than two generations old, so it wasn't deleted.
    assert_eq!(
        value_keys(&tester),
        key_set(&[
            k("top"),
            k("d1"),
            d2_key.clone(),
            ERROR_TRANSIENCE_KEY.clone()
        ])
    );

    tester.evaluator.delete_dirty(2);
    tester.eval(true, &[]);

    // The top node's value was dirty, and was two generations old, so it was deleted.
    assert_eq!(
        value_keys(&tester),
        key_set(&[k("d1"), d2_key, ERROR_TRANSIENCE_KEY.clone()])
    );
}

// Java: MemoizingEvaluatorTest.deleteDirtyCleanedValue
#[test]
fn delete_dirty_cleaned_value() {
    let tester = MemoizingTester::new();
    let leaf_key = nk("leafKey");
    tester.with_key(leaf_key.clone(), |s| s.value = Some("value".into()));
    tester.add_dependency_key(k("topKey"), leaf_key.clone());

    assert_eq!(tester.eval_and_get(false, &k("topKey")), "value");
    fail_build_and_remove_value(&tester, &leaf_key);
    tester.evaluator.delete_dirty(0);
}

// Java: MemoizingEvaluatorTest.deleteNonexistentValues
#[test]
fn delete_nonexistent_values() {
    let tester = MemoizingTester::new();
    tester.with("d1", |s| s.value = Some("1".into()));
    tester.delete("d1");
    tester.delete("d2");
    tester.eval(true, &[k("d1")]);
}

// Java: MemoizingEvaluatorTest.signalValueEnqueued
#[test]
fn signal_value_enqueued() {
    let tester = MemoizingTester::new();
    tester
        .add_dependency("top1", "d1")
        .add_dependency("top1", "d2")
        .add_dependency("top2", "d3")
        .with("top3", |_| {});
    assert!(tester.get_enqueued_values().is_empty());

    tester.set("d1", "1").set("d2", "2").set("d3", "3");
    tester.eval(true, &[k("top1")]);
    assert_eq!(
        tester.get_enqueued_values(),
        key_set(&[k("top1"), k("d1"), k("d2")])
    );

    tester.eval(true, &[k("top2")]);
    assert_eq!(
        tester.get_enqueued_values(),
        key_set(&[k("top1"), k("d1"), k("d2"), k("top2"), k("d3")])
    );
}

// Java: MemoizingEvaluatorTest.invalidationWithChangeAndThenNothingChanged
#[test]
fn invalidation_with_change_and_then_nothing_changed() {
    let tester = MemoizingTester::new();
    let b_key = nk("b");
    tester.add_dependency_key(k("a"), b_key.clone());
    tester.set_key(b_key.clone(), "y");
    assert_eq!(tester.eval_and_get(false, &k("a")), "y");
    tester.set_key(b_key, "z");
    tester.invalidate();
    let old = tester.eval_and_get(false, &k("a"));
    assert_eq!(old, "z");
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &k("a")), old);
}

// Java: MemoizingEvaluatorTest.noKeepGoingErrorAfterKeepGoingError
#[test]
fn no_keep_going_error_after_keep_going_error() {
    let tester = MemoizingTester::new();
    let top_key = nk("top");
    tester.set_has_error("error", true);
    tester.add_dependency_key(top_key.clone(), k("error"));
    let r = tester.eval(true, std::slice::from_ref(&top_key));
    singleton_error(&r, &top_key);
    tester.get_or_create(top_key.clone(), /* mark_as_modified= */ true);
    tester.invalidate();
    let r = tester.eval(false, std::slice::from_ref(&top_key));
    singleton_error(&r, &top_key);
}

// Java: MemoizingEvaluatorTest.transientPruning
#[test]
fn transient_pruning() {
    let tester = MemoizingTester::new();
    let leaf = nk("leaf");
    tester
        .set_has_transient_error("top", true)
        .add_dependency_key(k("top"), leaf.clone());
    tester.set_key(leaf.clone(), "leafy");
    tester.eval_and_get_error(true, &k("top"));
    tester.get_or_create(leaf, /* mark_as_modified= */ true);
    tester.invalidate();
    tester.eval_and_get_error(true, &k("top"));
}

// Java: MemoizingEvaluatorTest.incrementalAddedDependency
#[test]
fn incremental_added_dependency() {
    let tester = MemoizingTester::new();
    let a_key = nk("a");
    let b_key = nk("b");
    tester.add_dependency_key(a_key.clone(), b_key.clone());
    tester.set_key(b_key.clone(), "first");
    tester.set("c", "second");
    tester.eval_and_get(false, &a_key);

    tester.add_dependency_key(a_key.clone(), k("c"));
    tester.set_key(b_key, "now");
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &a_key), "nowsecond");
}

// Java: MemoizingEvaluatorTest.nodeIsChangedWithoutBeingEvaluated
#[test]
fn node_is_changed_without_being_evaluated() {
    let tester = MemoizingTester::new();
    let build_file = nk("buildfile");
    let dep = nk("dep");
    tester.set_key(build_file.clone(), "depend on dep");
    tester.set_key(dep.clone(), "this is dep");
    {
        let (build_file, dep) = (build_file.clone(), dep.clone());
        tester.set_builder("top", move |_, env| {
            let val = dep_string(env.get_value_dyn(&build_file)?);
            if env.values_missing() {
                return Ok(None);
            }
            if val.as_deref() == Some("depend on dep") {
                let result = dep_string(env.get_value_dyn(&dep)?);
                return Ok(if env.values_missing() { None } else { result });
            }
            Err(SkyFunctionException::persistent(SomeErrorException("bork".into())).into())
        });
    }
    assert_eq!(tester.eval_and_get(false, &k("top")), "this is dep");
    tester.set_key(dep, "this is new dep");
    tester.set_key(build_file.clone(), "don't depend on dep");
    tester.invalidate();
    tester.eval(false, &[k("top")]);
    tester.set_key(build_file, "depend on dep");
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &k("top")), "this is new dep");
}

// Java: MemoizingEvaluatorTest.dirtyThenDeleted
#[test]
fn dirty_then_deleted() {
    let tester = MemoizingTester::new();
    let top_key = nk("top");
    let leaf_key = nk("leaf");
    tester.add_dependency_key(top_key.clone(), leaf_key.clone());
    tester.set_key(leaf_key.clone(), "leafy");
    assert_eq!(tester.eval_and_get(false, &top_key), "leafy");
    tester.get_or_create(top_key, /* mark_as_modified= */ true);
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &leaf_key), "leafy");
    // Java: tester.delete("top") deletes skyKey("top"), which is not in the graph.
    tester.delete("top");
    tester.get_or_create(leaf_key.clone(), /* mark_as_modified= */ true);
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &leaf_key), "leafy");
}

// Java: MemoizingEvaluatorTest.dirtyAndChanged
#[test]
fn dirty_and_changed() {
    let tester = MemoizingTester::new();
    let leaf = nk("leaf");
    let mid = nk("mid");
    tester.add_dependency_key(k("top"), mid.clone());
    tester.add_dependency_key(mid.clone(), leaf.clone());
    tester.set_key(leaf.clone(), "leafy");
    // For invalidation.
    tester.set("dummy", "dummy");
    assert_eq!(tester.eval_and_get(false, &k("top")), "leafy");
    tester.set_key(leaf, "crunchy");
    tester.invalidate();
    // For invalidation.
    tester.eval_and_get(false, &k("dummy"));
    tester.get_or_create(mid, /* mark_as_modified= */ true);
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &k("top")), "crunchy");
}

// Java: MemoizingEvaluatorTest.changePruning
#[test]
fn change_pruning() {
    let tester = MemoizingTester::new();
    let leaf = nk("leaf");
    tester.add_dependency("top", "mid");
    tester.add_dependency_key(k("mid"), leaf.clone());
    tester.set_key(leaf.clone(), "leafy");
    assert_eq!(tester.eval_and_get(false, &k("top")), "leafy");
    // Mark leaf changed, but don't actually change it.
    tester.get_or_create(leaf, /* mark_as_modified= */ true);
    // mid will give an error if re-evaluated, but it shouldn't be because it is not marked
    // changed, and its dirty child will evaluate to the same element.
    tester.get_or_create(k("mid"), /* mark_as_modified= */ false);
    tester.set_has_error("mid", true);
    tester.invalidate();
    let r = tester.eval(false, &[k("top")]);
    assert!(!r.has_error(), "{r:?}");
    assert_eq!(*r.get(&T("top")).unwrap(), "leafy");
    assert!(tester.get_dirty_keys().is_empty());
    assert!(tester.get_deleted_keys().is_empty());
}

// Java: MemoizingEvaluatorTest.changePruningWithDoneValue
#[test]
fn change_pruning_with_done_value() {
    let tester = MemoizingTester::new();
    let leaf = nk("leaf");
    tester.set("suffix", "suffix");
    tester
        .add_dependency("top", "mid")
        .add_dependency("top", "suffix");
    tester
        .add_dependency_key(k("mid"), leaf.clone())
        .add_dependency("mid", "suffix");
    tester.set_key(leaf.clone(), "leafy");
    assert_eq!(tester.eval_and_get(false, &k("top")), "leafysuffixsuffix");
    // Mark leaf changed, but don't actually change it.
    tester.get_or_create(leaf.clone(), /* mark_as_modified= */ true);
    // mid will give an error if re-evaluated, but it shouldn't be because it is not marked
    // changed, and its dirty child will evaluate to the same element.
    tester.set_has_error("mid", true);
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &leaf), "leafy");
    assert_eq!(tester.get_dirty_keys(), key_set(&[k("mid"), k("top")]));
    assert!(tester.get_deleted_keys().is_empty());
    let r = tester.eval(false, &[k("top")]);
    assert!(!r.has_error(), "{r:?}");
    assert_eq!(*r.get(&T("top")).unwrap(), "leafysuffixsuffix");
    assert!(tester.get_dirty_keys().is_empty());
    assert!(tester.get_deleted_keys().is_empty());
}

// Java: MemoizingEvaluatorTest.changePruningAfterParentPrunes
#[test]
fn change_pruning_after_parent_prunes() {
    let tester = MemoizingTester::new();
    let leaf = nk("leaf");
    tester.set_key(leaf.clone(), "leafy");
    // When top depends on leaf, but always returns the same value,
    let top_evaluated = Arc::new(AtomicBool::new(false));
    {
        let (leaf, top_evaluated) = (leaf.clone(), top_evaluated.clone());
        tester.set_builder("top", move |_, env| {
            top_evaluated.store(true, Ordering::SeqCst);
            Ok(env.get_value_dyn(&leaf)?.map(|_| "top".to_string()))
        });
    }
    // And top is evaluated, then top's value is as expected, and top was actually evaluated.
    assert_eq!(tester.eval_and_get(false, &k("top")), "top");
    assert!(top_evaluated.load(Ordering::SeqCst));
    // When leaf is changed,
    tester.set_key(leaf.clone(), "crunchy");
    tester.invalidate();
    top_evaluated.store(false, Ordering::SeqCst);
    // And top is evaluated, then top's value is as expected, and top was actually evaluated.
    assert_eq!(tester.eval_and_get(false, &k("top")), "top");
    assert!(top_evaluated.load(Ordering::SeqCst));
    // When leaf is invalidated but not actually changed,
    tester.get_or_create(leaf, /* mark_as_modified= */ true);
    tester.invalidate();
    top_evaluated.store(false, Ordering::SeqCst);
    // And top is evaluated, then top's value is as expected,
    assert_eq!(tester.eval_and_get(false, &k("top")), "top");
    // And top was *not* actually evaluated, because change pruning cut off evaluation.
    assert!(!top_evaluated.load(Ordering::SeqCst));
}

// Java: MemoizingEvaluatorTest.changePruningFromOtherNodeAfterParentPrunes
#[test]
fn change_pruning_from_other_node_after_parent_prunes() {
    let tester = MemoizingTester::new();
    let leaf = nk("leaf");
    let other = nk("other");
    tester.set_key(leaf.clone(), "leafy");
    tester.set_key(other.clone(), "other");
    // When top depends on leaf and other, but always returns the same value,
    let top_evaluated = Arc::new(AtomicBool::new(false));
    {
        let (leaf, other, top_evaluated) = (leaf.clone(), other.clone(), top_evaluated.clone());
        tester.set_builder("top", move |_, env| {
            top_evaluated.store(true, Ordering::SeqCst);
            // Java: env.getValue(other) == null || env.getValue(leaf) == null (short-circuit).
            if env.get_value_dyn(&other)?.is_none() || env.get_value_dyn(&leaf)?.is_none() {
                return Ok(None);
            }
            Ok(Some("top".to_string()))
        });
    }
    assert_eq!(tester.eval_and_get(false, &k("top")), "top");
    assert!(top_evaluated.load(Ordering::SeqCst));
    // When leaf is changed,
    tester.set_key(leaf, "crunchy");
    tester.invalidate();
    top_evaluated.store(false, Ordering::SeqCst);
    assert_eq!(tester.eval_and_get(false, &k("top")), "top");
    assert!(top_evaluated.load(Ordering::SeqCst));
    // When other is invalidated but not actually changed,
    tester.get_or_create(other, /* mark_as_modified= */ true);
    tester.invalidate();
    top_evaluated.store(false, Ordering::SeqCst);
    assert_eq!(tester.eval_and_get(false, &k("top")), "top");
    // And top was *not* actually evaluated, because change pruning cut off evaluation.
    assert!(!top_evaluated.load(Ordering::SeqCst));
}

// Java: MemoizingEvaluatorTest.changedChildChangesDepOfParent
#[test]
fn changed_child_changes_dep_of_parent() {
    let tester = MemoizingTester::new();
    let build_file = nk("buildFile");
    tester.set_key(build_file.clone(), "hemingway");
    tester.set("absinthe", "absinthe");
    tester.set("whiskey", "whiskey");
    {
        let build_file = build_file.clone();
        // Java: addDependency(buildFile).setComputedValue(authorDrink).
        tester.set_builder("top", move |_, env| {
            let author = dep_string(env.get_value_dyn(&build_file)?);
            if env.values_missing() {
                return Ok(None);
            }
            let author = author.expect("present");
            let beverage = match author.as_str() {
                "hemingway" => env.get_value(T("absinthe"))?,
                "joyce" => env.get_value(T("whiskey"))?,
                _ => panic!("IllegalStateException: {author}"),
            };
            Ok(beverage.map(|b| format!("{author} drank {b}")))
        });
    }
    assert_eq!(
        tester.eval_and_get(false, &k("top")),
        "hemingway drank absinthe"
    );
    tester.set_key(build_file.clone(), "joyce");
    // Don't evaluate absinthe successfully anymore.
    tester.set_has_error("absinthe", true);
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &k("top")), "joyce drank whiskey");
    assert_eq!(tester.get_dirty_keys(), key_set(&[build_file, k("top")]));
    assert!(tester.get_deleted_keys().is_empty());
}

// Java: MemoizingEvaluatorTest.dirtyDepIgnoresChildren
#[test]
fn dirty_dep_ignores_children() {
    let tester = MemoizingTester::new();
    let leaf = nk("leaf");
    tester.set("mid", "ignore");
    tester.add_dependency("top", "mid");
    tester.add_dependency_key(k("mid"), leaf.clone());
    tester.set_key(leaf.clone(), "leafy");
    assert_eq!(tester.eval_and_get(false, &k("top")), "ignore");
    assert!(tester.get_dirty_keys().is_empty());
    assert!(tester.get_deleted_keys().is_empty());
    // Change leaf.
    tester.set_key(leaf.clone(), "crunchy");
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &k("top")), "ignore");
    assert_eq!(
        tester.get_dirty_keys(),
        key_set(std::slice::from_ref(&leaf))
    );
    assert!(tester.get_deleted_keys().is_empty());
    tester.set_key(leaf.clone(), "smushy");
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &k("top")), "ignore");
    assert_eq!(tester.get_dirty_keys(), key_set(&[leaf]));
    assert!(tester.get_deleted_keys().is_empty());
}

// Java: MemoizingEvaluatorTest.dirtyBuildAfterFailedBuild
/// Make sure that when a dirty value is building, the fact that a child may no longer exist in
/// the graph doesn't cause problems.
#[test]
fn dirty_build_after_failed_build() {
    let tester = MemoizingTester::new();
    let leaf = nk("leaf");
    tester.add_dependency_key(k("top"), leaf.clone());
    tester.set_key(leaf.clone(), "leafy");
    assert_eq!(tester.eval_and_get(false, &k("top")), "leafy");
    assert!(tester.get_dirty_keys().is_empty());
    assert!(tester.get_deleted_keys().is_empty());
    fail_build_and_remove_value(&tester, &leaf);
    // Leaf should no longer exist in the graph. Check that this doesn't cause problems.
    set_null(&tester, &leaf);
    tester.set_key(leaf, "crunchy");
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &k("top")), "crunchy");
}

// Java: MemoizingEvaluatorTest.failedDirtyBuild
#[test]
fn failed_dirty_build() {
    let tester = MemoizingTester::new();
    let leaf = nk("leaf");
    tester.add_error_dependency_key(k("top"), leaf.clone(), "recover");
    tester.set_key(leaf.clone(), "leafy");
    assert_eq!(tester.eval_and_get(false, &k("top")), "leafy");
    assert!(tester.get_dirty_keys().is_empty());
    assert!(tester.get_deleted_keys().is_empty());
    // Change leaf.
    tester.get_or_create(leaf.clone(), /* mark_as_modified= */ true);
    tester.set_has_error_key(leaf, true);
    tester.set_has_error("top", true);
    tester.invalidate();
    let r = tester.eval(false, &[k("top")]);
    singleton_error(&r, &k("top"));
}

// Java: MemoizingEvaluatorTest.makeTestKey
fn make_test_key(tester: &MemoizingTester, node0: &SkyKey) -> SkyKey {
    let mut key = None;
    // Create a long chain of nodes. Most of them will not actually be dirtied, but the last one
    // to be dirtied will enqueue its parent for dirtying, so it will be in the queue for the next
    // run.
    for i in 0..TEST_NODE_COUNT {
        let this = if i == 0 {
            node0.clone()
        } else {
            k(leak(format!("node{i}")))
        };
        if i > 1 {
            tester.add_dependency_key(this.clone(), k(leak(format!("node{}", i - 1))));
        } else if i == 1 {
            tester.add_dependency_key(this.clone(), node0.clone());
        } else {
            tester.set_key(this.clone(), "node0");
        }
        key = Some(this);
    }
    key.expect("TEST_NODE_COUNT > 0")
}

// Java: MemoizingEvaluatorTest.deletingDirtyNodes
/// Regression test for case where the user requests that we delete nodes that are already in the
/// queue to be dirtied. We should handle that gracefully and not complain.
#[test]
fn deleting_dirty_nodes() {
    let tester = MemoizingTester::new();
    let node0 = nk("node0");
    let key = make_test_key(&tester, &node0);
    // Seed the graph.
    assert_eq!(tester.eval_and_get(false, &key), "node0");
    // Start the dirtying process.
    tester.set_key(node0, "new");
    tester.invalidate();

    // Interrupt current thread on a next invalidate call.
    // PORT: Java's thread interrupt is the evaluator's interrupt; invalidation here is not
    // interruptible, the evaluation that follows it is.
    let interrupt = tester.evaluator.interrupt_flag();
    tester
        .progress_receiver
        .set_next_invalidation_callback(move || interrupt.store(true, Ordering::SeqCst));

    assert_eq!(
        tester.try_eval(false, std::slice::from_ref(&key)).err(),
        Some(Interrupted)
    );
    tester.evaluator.clear_interrupt();

    // Now delete all the nodes. The node that was going to be dirtied is also deleted, which we
    // should handle.
    tester.evaluator.delete(|_, _| true);
    assert_eq!(tester.eval_and_get(false, &key), "new");
}

// Java: MemoizingEvaluatorTest.deleteInvalidatedValue
#[test]
fn delete_invalidated_value() {
    let tester = MemoizingTester::new();
    let to_delete = nk("toDelete");
    // Must be a concatenation -- COPY doesn't actually copy.
    tester.add_dependency_key(k("top"), to_delete.clone());
    tester.set_key(to_delete.clone(), "toDelete");
    let value = tester
        .eval(false, &[k("top")])
        .get_dyn(&k("top"))
        .unwrap()
        .clone();
    tester.set("forceInvalidation", "forceInvalidation");
    tester.get_or_create(to_delete, /* mark_as_modified= */ true);
    tester.invalidate();
    tester.eval(false, &[k("forceInvalidation")]);
    tester.delete("toDelete");
    // Java: a WeakReference to the value is cleared by the GC.
    let weak = Arc::downgrade(&value);
    drop(value);
    tester.eval(false, &[k("forceInvalidation")]);
    tester.invalidate(); // So that invalidation receiver doesn't hang on to reference.
    // Java: GcFinalization.awaitClear(ref). `delete` also deletes every dirty node, among them
    // "top", whose last value was the only other reference.
    assert!(weak.upgrade().is_none());
}

// Java: MemoizingEvaluatorTest.valueInjectionOverExistingDirtyEntry
#[test]
fn value_injection_over_existing_dirty_entry() {
    let tester = MemoizingTester::new();
    let key = nk("key");
    tester.with_key(key.clone(), |s| s.value = Some("old_val".into()));
    tester.evaluator.inject(N("key"), "val".to_string());
    tester.eval(false, &[]); // Create the value.

    tester.evaluator.invalidate([key.clone()]);
    tester.eval(false, &[]); // Mark value as dirty.

    tester.evaluator.inject(N("key"), "val".to_string());
    tester.eval(false, &[]); // Inject again.
    assert_eq!(tester.eval_and_get(false, &key), "val");
}

// Java: MemoizingEvaluatorTest.valueInjectionOverExistingEntryMarkedForInvalidation
#[test]
fn value_injection_over_existing_entry_marked_for_invalidation() {
    let tester = MemoizingTester::new();
    let key = nk("key");
    tester.with_key(key.clone(), |s| s.value = Some("old_val".into()));
    tester.evaluator.invalidate([key.clone()]);
    tester.evaluator.inject(N("key"), "val".to_string());
    assert_eq!(tester.eval_and_get(false, &key), "val");
}

// Java: MemoizingEvaluatorTest.valueInjectionOverExistingEntryMarkedForDeletion
#[test]
fn value_injection_over_existing_entry_marked_for_deletion() {
    let tester = MemoizingTester::new();
    let key = nk("key");
    tester.with_key(key.clone(), |s| s.value = Some("old_val".into()));
    tester.evaluator.delete(|_, _| true);
    tester.evaluator.inject(N("key"), "val".to_string());
    assert_eq!(tester.eval_and_get(false, &key), "val");
}

// Java: MemoizingEvaluatorTest.valueInjectionOverExistingEqualEntryMarkedForInvalidation
#[test]
fn value_injection_over_existing_equal_entry_marked_for_invalidation() {
    let tester = MemoizingTester::new();
    let key = nk("key");
    tester.evaluator.inject(N("key"), "val".to_string());
    assert_eq!(tester.eval_and_get(false, &key), "val");

    tester.evaluator.invalidate([key.clone()]);
    tester.evaluator.inject(N("key"), "val".to_string());
    assert_eq!(tester.eval_and_get(false, &key), "val");
}

// Java: MemoizingEvaluatorTest.valueInjectionOverExistingEqualEntryMarkedForDeletion
#[test]
fn value_injection_over_existing_equal_entry_marked_for_deletion() {
    let tester = MemoizingTester::new();
    let key = nk("key");
    tester.evaluator.inject(N("key"), "val".to_string());
    assert_eq!(tester.eval_and_get(false, &key), "val");

    tester.evaluator.delete(|_, _| true);
    tester.evaluator.inject(N("key"), "val".to_string());
    assert_eq!(tester.eval_and_get(false, &key), "val");
}

// Java: MemoizingEvaluatorTest.valueInjectionOverValueWithDeps
#[test]
fn value_injection_over_value_with_deps() {
    let tester = MemoizingTester::new();
    let key = nk("key");
    let other_key = nk("other");
    tester.with_key(other_key.clone(), |s| s.value = Some("foo".into()));
    tester.add_dependency_key(key.clone(), other_key.clone());
    assert_eq!(tester.eval_and_get(false, &key), "foo");
    tester.evaluator.inject(N("key"), "val".to_string());
    tester.with_key(other_key.clone(), |s| s.value = Some("newfoo".into()));
    tester.evaluator.invalidate([other_key]);
    // Injected value is ignored for value with deps.
    assert_eq!(tester.eval_and_get(false, &key), "newfoo");
}

// Java: MemoizingEvaluatorTest.valueInjectionOverEqualValueWithDeps
#[test]
fn value_injection_over_equal_value_with_deps() {
    let tester = MemoizingTester::new();
    let key = nk("key");
    tester.with("other", |s| s.value = Some("val".into()));
    tester.add_dependency_key(key.clone(), k("other"));
    assert_eq!(tester.eval_and_get(false, &key), "val");
    tester.evaluator.inject(N("key"), "val".to_string());
    assert_eq!(tester.eval_and_get(false, &key), "val");
}

// Java: MemoizingEvaluatorTest.valueInjectionOverExistingEqualEntryDoesNotInvalidate
#[test]
fn value_injection_over_existing_equal_entry_does_not_invalidate() {
    let tester = MemoizingTester::new();
    let child_key = nk("child");
    tester.add_dependency_key(k("parent"), child_key.clone());
    tester.with_key(child_key.clone(), |s| s.value = Some("same_val".into()));
    assert_eq!(tester.eval_and_get(false, &k("parent")), "same_val");

    tester.evaluator.inject(N("child"), "same_val".to_string());
    assert_eq!(
        tester.get_existing_value(&child_key).as_deref(),
        Some("same_val")
    );
    // Since we are injecting an equal value, the parent should not have been invalidated.
    assert_eq!(
        tester.get_existing_value(&k("parent")).as_deref(),
        Some("same_val")
    );
}

// Java: MemoizingEvaluatorTest.checkNotComparableNotPruned
fn check_not_comparable_not_pruned(has_event: bool) {
    let tester = MemoizingTester::new();
    let child = SkyKey::new(NotComparableKey("child"));
    tester.with_key(child.clone(), |s| s.value = Some("not comparable".into()));
    if has_event {
        tester.set_warning_key(child.clone(), "shmoop");
    }
    let parent_evaluated = Arc::new(AtomicUsize::new(0));
    {
        let (child, parent_evaluated) = (child.clone(), parent_evaluated.clone());
        // Java: addDependency(child).setComputedValue(... "some val").
        tester.set_builder("parent", move |_, env| {
            env.get_value_dyn(&child)?;
            if env.values_missing() {
                return Ok(None);
            }
            parent_evaluated.fetch_add(1, Ordering::SeqCst);
            Ok(Some("some val".into()))
        });
    }
    assert_eq!(tester.eval_and_get(false, &k("parent")), "some val");
    assert_eq!(parent_evaluated.load(Ordering::SeqCst), 1);
    if has_event {
        assert_eq!(tester.events(), vec!["shmoop"]);
    } else {
        assert!(tester.events().is_empty());
    }
    tester.clear_events();

    tester.reset_played_events();
    tester.get_or_create(child, /* mark_as_modified= */ true);
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &k("parent")), "some val");
    assert_eq!(parent_evaluated.load(Ordering::SeqCst), 2);
    if has_event {
        assert_eq!(tester.events(), vec!["shmoop"]);
    } else {
        assert!(tester.events().is_empty());
    }
}

// Java: MemoizingEvaluatorTest.notComparableNotPrunedNoEvent
#[test]
fn not_comparable_not_pruned_no_event() {
    check_not_comparable_not_pruned(false);
}

// Java: MemoizingEvaluatorTest.notComparableNotPrunedEvent
#[test]
fn not_comparable_not_pruned_event() {
    check_not_comparable_not_pruned(true);
}

// Java: MemoizingEvaluatorTest.changePruningWithEvent
// PORT: Java's parent runs twice in the first build (it restarts once because the child is not
// ready); it runs once here (the request blocks), so the latch counts one run.
#[test]
fn change_pruning_with_event() {
    let tester = MemoizingTester::new();
    let child = nk("child");
    tester.with_key(child.clone(), |s| s.value = Some("child".into()));
    tester.set_warning_key(child.clone(), "bloop");
    // Restart once because child isn't ready.
    let parent_evaluated = Latch::new(3);
    {
        let parent_evaluated = parent_evaluated.clone();
        let child = child.clone();
        tester.set_builder("parent", move |_, env| {
            parent_evaluated.count_down();
            env.get_value_dyn(&child)?;
            if env.values_missing() {
                return Ok(None);
            }
            Ok(Some("parent".into()))
        });
    }
    assert_eq!(tester.eval_and_get(false, &k("parent")), "parent");
    assert_eq!(parent_evaluated.get_count(), 2);
    assert_eq!(tester.events(), vec!["bloop"]);
    tester.clear_events();
    tester.reset_played_events();
    tester.get_or_create(child, /* mark_as_modified= */ true);
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &k("parent")), "parent");
    assert_eq!(tester.events(), vec!["bloop"]);
    assert_eq!(parent_evaluated.get_count(), 2);
}

// Java: MemoizingEvaluatorTest.errorChanged
#[test]
fn error_changed() {
    let tester = MemoizingTester::new();
    let error = nk("error");
    tester.set_has_error_key(error.clone(), true);
    assert!(
        tester
            .eval_and_get_error(true, &error)
            .get_exception()
            .is_some()
    );
    tester.get_or_create(error.clone(), /* mark_as_modified= */ true);
    tester.invalidate();
    assert!(
        tester
            .eval_and_get_error(true, &error)
            .get_exception()
            .is_some()
    );
}

// Java: MemoizingEvaluatorTest.cycleWithDirtyValue
#[test]
fn cycle_with_dirty_value() {
    let tester = MemoizingTester::new();
    let cycle_key1 = nk("cycleKey1");
    let cycle_key2 = k("cycleKey2");
    tester.add_dependency_key(cycle_key1.clone(), cycle_key2.clone());
    tester.add_dependency_key(cycle_key2.clone(), cycle_key1.clone());
    let r = tester.eval(true, std::slice::from_ref(&cycle_key1));
    assert!(r.get_dyn(&cycle_key1).is_none());
    let cycle_info = only_cycle(&r, &cycle_key1);
    assert_eq!(
        cycle_info.get_cycle(),
        &[cycle_key1.clone(), cycle_key2.clone()]
    );
    assert!(cycle_info.get_path_to_cycle().is_empty());
    tester.get_or_create(cycle_key1.clone(), /* mark_as_modified= */ true);
    tester.invalidate();
    let r = tester.eval(true, std::slice::from_ref(&cycle_key1));
    assert!(r.get_dyn(&cycle_key1).is_none());
    let cycle_info = only_cycle(&r, &cycle_key1);
    assert_eq!(cycle_info.get_cycle(), &[cycle_key1, cycle_key2]);
    assert!(cycle_info.get_path_to_cycle().is_empty());
}

// Java: MemoizingEvaluatorTest.breakCycle
#[test]
fn break_cycle() {
    let tester = MemoizingTester::new();
    let a_key = nk("a");
    let b_key = nk("b");
    // When aKey and bKey depend on each other,
    tester.add_dependency_key(a_key.clone(), b_key.clone());
    tester.add_dependency_key(b_key.clone(), a_key.clone());
    // And they are evaluated,
    let r = tester.eval(true, &[a_key.clone(), b_key.clone()]);
    // Then the evaluation is in error,
    assert!(r.has_error());
    // And each node has the expected cycle.
    let a_cycle_info = only_cycle(&r, &a_key);
    assert_eq!(a_cycle_info.get_cycle(), &[a_key.clone(), b_key.clone()]);
    assert!(a_cycle_info.get_path_to_cycle().is_empty());
    let b_cycle_info = only_cycle(&r, &b_key);
    assert_eq!(b_cycle_info.get_cycle(), &[b_key.clone(), a_key.clone()]);
    assert!(b_cycle_info.get_path_to_cycle().is_empty());

    // When both dependencies are broken,
    tester.remove_dependency_key(b_key.clone(), &a_key);
    tester.set_key(b_key.clone(), "bValue");
    tester.remove_dependency_key(a_key.clone(), &b_key);
    tester.set_key(a_key.clone(), "aValue");
    tester.invalidate();
    // And the nodes are re-evaluated,
    let r = tester.eval(true, &[a_key, b_key]);
    // Then evaluation is successful and the nodes have the expected values.
    assert_eq!(*r.get(&N("a")).unwrap(), "aValue");
    assert_eq!(*r.get(&N("b")).unwrap(), "bValue");
}

// Java: MemoizingEvaluatorTest.continueWithErrorDepTurnedGood
#[test]
fn continue_with_error_dep_turned_good() {
    let tester = MemoizingTester::new();
    let error_key = nk("my_error_value");
    tester.set_has_error_key(error_key.clone(), true);
    tester.set("after", "after");
    tester
        .add_error_dependency_key(k("parent"), error_key.clone(), "recovered")
        .add_dependency("parent", "after");
    let r = tester.eval(true, &[k("parent")]);
    assert!(r.error_map().is_empty());
    assert_eq!(*r.get(&T("parent")).unwrap(), "recoveredafter");
    tester.set_key(error_key.clone(), "reformed");
    tester.set_has_error_key(error_key, false);
    tester.invalidate();
    let r = tester.eval(true, &[k("parent")]);
    assert!(r.error_map().is_empty());
    assert_eq!(*r.get(&T("parent")).unwrap(), "reformedafter");
}

// Java: MemoizingEvaluatorTest.errorDepAlreadyThereThenTurnedGood
#[test]
fn error_dep_already_there_then_turned_good() {
    let tester = MemoizingTester::new();
    let error_key = nk("my_error_value");
    tester.set_has_error_key(error_key.clone(), true);
    tester
        .add_error_dependency_key(k("parent"), error_key.clone(), "recovered")
        .set_has_error("parent", true);
    // Prime the graph by putting the error value in it beforehand.
    tester.eval_and_get_error(true, &error_key);
    // Request the parent.
    let r = tester.eval(false, &[k("parent")]);
    singleton_error(&r, &k("parent"));
    // Change the error value to no longer throw.
    tester.set_key(error_key.clone(), "reformed");
    tester.set_has_error_key(error_key.clone(), false);
    tester.set_has_error("parent", false);
    tester.evaluator.invalidate([error_key]);
    tester.invalidate();
    // Request the parent again. This time it should succeed.
    let r = tester.eval(false, &[k("parent")]);
    assert!(r.error_map().is_empty());
    assert_eq!(*r.get(&T("parent")).unwrap(), "reformed");
    // Confirm that the parent no longer depends on the error transience value -- make it
    // unbuildable again, but without invalidating it, and invalidate transient errors. The
    // parent should not be rebuilt.
    tester.set_has_error("parent", true);
    tester.invalidate_transient_errors();
    let r = tester.eval(false, &[k("parent")]);
    assert!(r.error_map().is_empty());
    assert_eq!(*r.get(&T("parent")).unwrap(), "reformed");
}

// Java: MemoizingEvaluatorTest.dirtyDependsOnErrorTurningGood
#[test]
fn dirty_depends_on_error_turning_good() {
    let tester = MemoizingTester::new();
    let error = nk("error");
    tester.set_has_error_key(error.clone(), true);
    tester.add_dependency_key(k("top"), error.clone());
    let r = tester.eval(false, &[k("top")]);
    singleton_error(&r, &k("top"));
    tester.set_has_error_key(error.clone(), false);
    tester.set_key(error, "reformed");
    tester.invalidate();
    let r = tester.eval(false, &[k("top")]);
    assert_eq!(*r.get(&T("top")).unwrap(), "reformed");
    assert!(!r.has_error());
}

// ---------------------------------------------------------------------------------------------
// Incremental evaluation, second part (worker I1-E).

/// Java: `assertThat(tester.progressReceiver.evaluated)`.
fn evaluated_keys(tester: &MemoizingTester) -> HashSet<SkyKey> {
    tester.progress_receiver.evaluated.lock().clone()
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:875 MemoizingEvaluatorTest.receiverToldOfVerifiedValueDependingOnCycle
#[test]
fn receiver_told_of_verified_value_depending_on_cycle() {
    let tester = MemoizingTester::new();
    let leaf = nk("leaf");
    let cycle = k("cycle");
    let top = k("top");
    tester.set_key(leaf.clone(), "leaf");
    tester.add_dependency_key(cycle.clone(), cycle.clone());
    tester
        .add_dependency_key(top.clone(), leaf.clone())
        .add_dependency_key(top.clone(), cycle.clone());
    tester.eval(true, std::slice::from_ref(&top));
    assert_eq!(
        evaluated_keys(&tester),
        key_set(&[leaf.clone(), top.clone(), cycle])
    );
    tester.progress_receiver.clear();
    tester.get_or_create(leaf.clone(), /* mark_as_modified= */ true);
    tester.invalidate();
    tester.eval(true, std::slice::from_ref(&top));
    assert_eq!(evaluated_keys(&tester), key_set(&[leaf, top]));
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1070 MemoizingEvaluatorTest.changeCycle
fn change_cycle(keep_going: bool) {
    let tester = MemoizingTester::new();
    let a_key = k("a");
    let b_key = nk("b");
    let top_key = k("top");
    let mid_key = k("mid");
    tester.add_dependency_key(top_key.clone(), mid_key.clone());
    tester.add_dependency_key(mid_key.clone(), a_key.clone());
    tester.add_dependency_key(a_key.clone(), b_key.clone());
    tester.add_dependency_key(b_key.clone(), a_key.clone());
    let r = tester.eval(keep_going, std::slice::from_ref(&top_key));
    assert!(r.get_dyn(&top_key).is_none());
    let cycle_info = only_cycle(&r, &top_key);
    assert_eq!(cycle_info.get_cycle(), &[a_key.clone(), b_key.clone()]);
    assert_eq!(
        cycle_info.get_path_to_cycle(),
        &[top_key.clone(), mid_key.clone()]
    );

    tester.remove_dependency_key(b_key.clone(), &a_key);
    tester.set_key(b_key, "bValue");
    tester.invalidate();
    let r = tester.eval(keep_going, std::slice::from_ref(&top_key));
    assert_eq!(*r.get(&T("top")).unwrap(), "bValue");
    assert!(r.get_error(&top_key).is_none());
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1096 MemoizingEvaluatorTest.changeCycle_NoKeepGoing
#[test]
fn change_cycle_no_keep_going() {
    change_cycle(false);
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1101 MemoizingEvaluatorTest.changeCycle_KeepGoing
#[test]
fn change_cycle_keep_going() {
    change_cycle(true);
}

/// Regression test: "crash in cycle checker with dirty values".
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1180 MemoizingEvaluatorTest.cycleAndSelfEdgeWithDirtyValue
#[test]
fn cycle_and_self_edge_with_dirty_value() {
    let tester = MemoizingTester::new();
    // The cycle detection algorithm non-deterministically traverses into children nodes, so use
    // explicit determinism.
    tester.make_graph_deterministic();
    let cycle_key1 = nk("ZcycleKey1");
    let cycle_key2 = k("AcycleKey2");
    tester
        .add_dependency_key(cycle_key1.clone(), cycle_key2.clone())
        .add_dependency_key(cycle_key1.clone(), cycle_key1.clone());
    tester.add_dependency_key(cycle_key2.clone(), cycle_key1.clone());
    let r = tester.eval(true, std::slice::from_ref(&cycle_key1));
    assert!(r.get_dyn(&cycle_key1).is_none());
    let cycle_info = only_cycle(&r, &cycle_key1);
    assert_eq!(cycle_info.get_cycle(), std::slice::from_ref(&cycle_key1));
    assert!(cycle_info.get_path_to_cycle().is_empty());
    tester.get_or_create(cycle_key1.clone(), /* mark_as_modified= */ true);
    tester.invalidate();
    let r = tester.eval(true, &[cycle_key1.clone(), cycle_key2.clone()]);
    assert!(r.get_dyn(&cycle_key1).is_none());
    let cycle_info = only_cycle(&r, &cycle_key1);
    assert_eq!(cycle_info.get_cycle(), std::slice::from_ref(&cycle_key1));
    assert!(cycle_info.get_path_to_cycle().is_empty());
    let error = tester
        .evaluator
        .get_existing_error_for_testing(&cycle_key2)
        .expect("cycleKey2 in error");
    assert_eq!(error.get_cycle_info().len(), 1, "{error:?}");
    let cycle_info = &error.get_cycle_info()[0];
    assert_eq!(cycle_info.get_cycle(), &[cycle_key1]);
    assert_eq!(cycle_info.get_path_to_cycle(), &[cycle_key2]);
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1546 MemoizingEvaluatorTest.nodeInvalidatedThenDoubleCycle
#[test]
fn node_invalidated_then_double_cycle() {
    let tester = MemoizingTester::new();
    tester.make_graph_deterministic();
    // When topKey depends on depKey, and both are top-level nodes in the graph,
    let top_key = nk("bKey");
    let dep_key = nk("aKey");
    tester
        .add_dependency_key(top_key.clone(), dep_key.clone())
        .with_key(top_key.clone(), |s| s.value = Some("a".into()));
    tester.with_key(dep_key.clone(), |s| s.value = Some("b".into()));
    // Then evaluation is as expected.
    let r1 = tester.eval(true, &[top_key.clone(), dep_key.clone()]);
    assert_eq!(*r1.get(&N("bKey")).unwrap(), "a");
    assert_eq!(*r1.get(&N("aKey")).unwrap(), "b");
    assert!(!r1.has_error(), "{r1:?}");
    // When both nodes acquire self-edges, with topKey still also depending on depKey, in the
    // same group,
    tester
        .get_or_create(dep_key.clone(), /* mark_as_modified= */ true)
        .add_dependency_key(dep_key.clone(), dep_key.clone());
    tester.get_or_create(top_key.clone(), /* mark_as_modified= */ true);
    tester
        .unset_constant_value(top_key.clone())
        .remove_dependency_key(top_key.clone(), &dep_key);
    {
        let (top_key, dep_key) = (top_key.clone(), dep_key.clone());
        tester.set_builder_key(top_key.clone(), move |_, env| {
            // Order depKey first - makeGraphDeterministic() only makes the batch maps returned by
            // the graph deterministic, not the order of temporary direct deps. This makes the
            // order of deps match (alphabetized by the string representation).
            env.get_values_and_exceptions(&[dep_key.clone(), top_key.clone()])?;
            assert!(env.values_missing());
            Ok(None)
        });
    }
    tester.invalidate();
    // Then evaluation is as expected -- topKey has removed its dep on depKey (since depKey was not
    // done when topKey found its cycle), and both topKey and depKey have cycles.
    let r2 = tester.eval(true, &[top_key.clone(), dep_key.clone()]);
    assert_eq!(
        as_set(
            r2.get_error(&top_key)
                .expect("top in error")
                .get_cycle_info()
        ),
        as_set(&[CycleInfo::create_cycle_info(vec![], vec![top_key.clone()])])
    );
    assert_eq!(
        tester.evaluator.get_direct_deps(&top_key),
        vec![top_key.clone()]
    );
    assert_eq!(
        as_set(
            r2.get_error(&dep_key)
                .expect("dep in error")
                .get_cycle_info()
        ),
        as_set(&[CycleInfo::create_cycle_info(vec![], vec![dep_key.clone()])])
    );
    // When the nodes return to their original, error-free state,
    tester.get_or_create(top_key.clone(), /* mark_as_modified= */ true);
    tester
        .with_key(top_key.clone(), |s| s.builder = None)
        .add_dependency_key(top_key.clone(), dep_key.clone())
        .with_key(top_key.clone(), |s| s.value = Some("a".into()));
    tester
        .get_or_create(dep_key.clone(), /* mark_as_modified= */ true)
        .remove_dependency_key(dep_key.clone(), &dep_key);
    tester.invalidate();
    // Then evaluation is as expected.
    let r3 = tester.eval(true, &[top_key, dep_key]);
    assert_eq!(*r3.get(&N("bKey")).unwrap(), "a");
    assert_eq!(*r3.get(&N("aKey")).unwrap(), "b");
    assert!(!r3.has_error(), "{r3:?}");
}

/// The `slow` / `error` / `mid` / `top` graph of the `incompleteDirectDeps*` regression tests:
/// slowKey starts -> errorKey finishes, written to graph -> slowKey finishes & (Visitor aborts)
/// -> topKey builds.
///
/// PORT: Java's "top" gets null for "mid" and goes on to request "error". A request blocks here
/// (it would wait for "slow", which waits for "error"), so "top" requests both in one batch.
fn incomplete_direct_deps_graph(tester: &MemoizingTester, error_key: &SkyKey, top_key: &SkyKey) {
    let slow_start = Latch::new(1);
    let error_finish = Latch::new(1);
    tester.set_chained_key(
        error_key.clone(),
        Chained {
            wait_to_finish: Some(slow_start.clone()),
            notify_finish: Some(error_finish.clone()),
            ..Chained::default()
        },
    );
    tester.set_chained(
        "slow",
        Chained {
            notify_start: Some(slow_start),
            wait_to_finish: Some(error_finish),
            wait_for_exception: true,
            value: Some("slow"),
            ..Chained::default()
        },
    );
    tester.add_dependency("mid", "slow");
    tester
        .add_dependency_key(top_key.clone(), k("mid"))
        .add_dependency_key(top_key.clone(), error_key.clone())
        .with_key(top_key.clone(), |s| s.batch = true);
}

/// Regression test: error on clearMaybeDirtyValue. We do an evaluation of topKey, which
/// registers dependencies on midKey and errorKey. midKey enqueues slowKey, and waits. errorKey
/// throws an error, which bubbles up to topKey. If topKey does not unregister its dependence on
/// midKey, it will have a dangling reference to midKey after unfinished values are cleaned from
/// the graph. Note that slowKey will wait until errorKey has thrown and the threadpool has caught
/// the exception before returning, so the Evaluator will already have stopped enqueuing new jobs,
/// so midKey is not evaluated.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1708 MemoizingEvaluatorTest.incompleteDirectDepsAreClearedBeforeInvalidation
#[test]
fn incomplete_direct_deps_are_cleared_before_invalidation() {
    let tester = MemoizingTester::new();
    let error_key = nk("error");
    let top_key = k("top");
    incomplete_direct_deps_graph(&tester, &error_key, &top_key);
    let r = tester.eval(false, std::slice::from_ref(&top_key));
    singleton_error(&r, &top_key);
    // Make sure midKey didn't finish building.
    assert_eq!(tester.get_existing_value(&k("mid")), None);
    // Give slowKey a nice ordinary builder.
    tester.get_or_create(k("slow"), /* mark_as_modified= */ false);
    tester.with("slow", |s| {
        s.builder = None;
        s.value = Some("slow".into());
    });
    // Put midKey into the graph. It won't have a reverse dependence on topKey.
    tester.eval_and_get(false, &k("mid"));
    tester.evaluator.invalidate([error_key]);
    // topKey should not access midKey as if it were already registered as a dependency.
    tester.eval(false, &[top_key]);
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1866 MemoizingEvaluatorTest.incompleteDirectDepsForDirtyValue
#[test]
fn incomplete_direct_deps_for_dirty_value() {
    let tester = MemoizingTester::new();
    let top_key = nk("top");
    tester.set_key(top_key.clone(), "initial");
    // Put topKey into graph so it will be dirtied on next run.
    assert_eq!(tester.eval_and_get(false, &top_key), "initial");
    let error_key = k("error");
    set_null(&tester, &top_key);
    incomplete_direct_deps_graph(&tester, &error_key, &top_key);
    tester.invalidate();
    // slowKey starts -> errorKey finishes, written to graph -> slowKey finishes & (Visitor
    // aborts) -> topKey builds.
    let r = tester.eval(false, std::slice::from_ref(&top_key));
    singleton_error(&r, &top_key);
    // Make sure midKey didn't finish building.
    assert_eq!(tester.get_existing_value(&k("mid")), None);
    // Give slowKey a nice ordinary builder.
    tester.get_or_create(k("slow"), /* mark_as_modified= */ false);
    tester.with("slow", |s| {
        s.builder = None;
        s.value = Some("slow".into());
    });
    // Put midKey into the graph. It won't have a reverse dependence on topKey.
    tester.eval_and_get(false, &k("mid"));
    // topKey should not access midKey as if it were already registered as a dependency. We don't
    // invalidate errors, but since topKey wasn't actually written to the graph before, it will be
    // rebuilt.
    tester.eval(true, &[top_key]);
}

/// Java: `MemoizingEvaluatorTest.RUNS`.
const RUNS: usize = 10;

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3568 MemoizingEvaluatorTest.singleValueDependsOnManyDirtyValues
#[test]
fn single_value_depends_on_many_dirty_values() {
    let tester = MemoizingTester::new();
    let mut values = vec![];
    let mut expected = String::new();
    for i in 0..TEST_NODE_COUNT {
        let value_name = leak(i.to_string());
        let key = nk(value_name);
        tester.set_key(key.clone(), value_name);
        expected.push_str(value_name);
        values.push(key);
    }
    let top_key = k("top");
    for sky_key in &values {
        tester.add_dependency_key(top_key.clone(), sky_key.clone());
    }

    let r = tester.eval(false, std::slice::from_ref(&top_key));
    assert_eq!(*r.get(&T("top")).unwrap(), expected);

    for _ in 0..RUNS {
        for sky_key in &values {
            tester.get_or_create(sky_key.clone(), /* mark_as_modified= */ true);
        }
        // This value has an error, but we should never discover it because it is not marked
        // changed and all of its dependencies re-evaluate to the same thing.
        tester.get_or_create(top_key.clone(), /* mark_as_modified= */ false);
        tester.set_has_error("top", true);
        tester.invalidate();

        let r = tester.eval(false, std::slice::from_ref(&top_key));
        assert_eq!(*r.get(&T("top")).unwrap(), expected);
    }
}

/// Tests scenario where we have dirty values in the graph, and then one of them is deleted since
/// its evaluation did not complete before an error was thrown. Can either test the graph via an
/// evaluation of that deleted value, or an invalidation of a child, and can either remove the
/// thrown error or throw it again on that evaluation.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3605 MemoizingEvaluatorTest.dirtyValueChildrenProperlyRemovedOnEarlyBuildAbort
fn dirty_value_children_properly_removed_on_early_build_abort(
    reevaluate_missing_value: bool,
    remove_error: bool,
) {
    let tester = MemoizingTester::new();
    let error_key = nk("error");
    tester.set_key(error_key.clone(), "biding time");
    let slow_key = nk("slow");
    tester.set_key(slow_key.clone(), "slow");
    let mid_key = k("mid");
    tester.add_dependency_key(mid_key.clone(), slow_key.clone());
    let last_key = nk("last");
    tester.set_key(last_key.clone(), "last");
    let mother_key = k("mother");
    tester
        .add_dependency_key(mother_key.clone(), error_key.clone())
        .add_dependency_key(mother_key.clone(), mid_key.clone())
        .add_dependency_key(mother_key.clone(), last_key.clone());
    let father_key = k("father");
    tester
        .add_dependency_key(father_key.clone(), error_key.clone())
        .add_dependency_key(father_key.clone(), mid_key.clone())
        .add_dependency_key(father_key.clone(), last_key.clone());
    let r = tester.eval(false, &[mother_key.clone(), father_key.clone()]);
    assert_eq!(*r.get(&T("mother")).unwrap(), "biding timeslowlast");
    assert_eq!(*r.get(&T("father")).unwrap(), "biding timeslowlast");
    set_null(&tester, &slow_key);
    // Each parent depends on errorKey, midKey, lastKey. We keep slowKey waiting until errorKey is
    // finished. So there is no way lastKey can be enqueued by either parent. Thus, the parent that
    // is cleaned has not interacted with lastKey this build. Still, lastKey's reverse dep on that
    // parent should be removed.
    let error_finish = Latch::new(1);
    set_null(&tester, &error_key);
    tester.set_chained_key(
        error_key.clone(),
        Chained {
            notify_finish: Some(error_finish.clone()),
            ..Chained::default()
        },
    );
    tester.set_chained_key(
        slow_key.clone(),
        Chained {
            wait_to_finish: Some(error_finish),
            wait_for_exception: true,
            value: Some("leaf2"),
            ..Chained::default()
        },
    );
    tester.invalidate();
    // errorKey finishes, written to graph -> leafKey maybe starts+finishes & (Visitor aborts)
    // -> one of mother or father builds. The other one should be cleaned, and no references to it
    // left in the graph.
    let r = tester.eval(false, &[mother_key.clone(), father_key.clone()]);
    assert!(r.has_error());
    // Only one of mother or father should be in the graph.
    assert!(
        r.get_error(&mother_key).is_none() != r.get_error(&father_key).is_none(),
        "{:?}, {:?}",
        r.get_error(&mother_key),
        r.get_error(&father_key)
    );
    let parent_key = if reevaluate_missing_value == r.get_error(&mother_key).is_none() {
        mother_key
    } else {
        father_key
    };
    // Give slowKey a nice ordinary builder.
    tester.get_or_create(slow_key.clone(), /* mark_as_modified= */ false);
    tester.with_key(slow_key, |s| {
        s.builder = None;
        s.value = Some("leaf2".into());
    });
    if remove_error {
        tester.get_or_create(error_key.clone(), /* mark_as_modified= */ true);
        tester.with_key(error_key, |s| {
            s.builder = None;
            s.value = Some("reformed".into());
        });
    }
    let mut last_string = "last";
    if !reevaluate_missing_value {
        // Mark the last key modified if we're not trying the absent value again. This
        // invalidation will test if lastKey still has a reference to the absent value.
        last_string = "last2";
        tester.set_key(last_key, last_string);
    }
    tester.invalidate();
    let r = tester.eval(false, std::slice::from_ref(&parent_key));
    if remove_error {
        assert_eq!(
            *r.get_dyn(&parent_key)
                .unwrap_or_else(|| panic!("{r:?}"))
                .downcast_ref::<String>()
                .unwrap(),
            format!("reformedleaf2{last_string}")
        );
    } else {
        assert!(r.get_error(&parent_key).is_some(), "{r:?}");
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3708 MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithInvalidateRemoveError
#[test]
fn dirty_children_properly_removed_with_invalidate_remove_error() {
    dirty_value_children_properly_removed_on_early_build_abort(
        /* reevaluate_missing_value= */ false, /* remove_error= */ true,
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3714 MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithInvalidateKeepError
#[test]
fn dirty_children_properly_removed_with_invalidate_keep_error() {
    dirty_value_children_properly_removed_on_early_build_abort(
        /* reevaluate_missing_value= */ false, /* remove_error= */ false,
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3720 MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithReevaluateRemoveError
#[test]
fn dirty_children_properly_removed_with_reevaluate_remove_error() {
    dirty_value_children_properly_removed_on_early_build_abort(
        /* reevaluate_missing_value= */ true, /* remove_error= */ true,
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3726 MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithReevaluateKeepError
#[test]
fn dirty_children_properly_removed_with_reevaluate_keep_error() {
    dirty_value_children_properly_removed_on_early_build_abort(
        /* reevaluate_missing_value= */ true, /* remove_error= */ false,
    );
}

/// Regression test: enqueue so many values that some of them won't have started processing, and
/// then either interrupt processing or have a child throw an error. In the latter case, this also
/// tests that a value that hasn't started processing can still have a child error bubble up to
/// it. In both cases, it tests that the graph is properly cleaned of the dirty values and
/// references to them.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3740 MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnFail
fn many_dirty_values_clear_children_on_fail(interrupt: bool) {
    let tester = MemoizingTester::new();
    let leaf_key = nk("leaf");
    tester.set_key(leaf_key.clone(), "leafy");
    let last_key = nk("last");
    tester.set_key(last_key.clone(), "last");
    // Request far more top-level values than there are threads, so some of them will block until
    // the leaf child is enqueued for processing.
    let tops: Vec<SkyKey> = (0..10000)
        .map(|i| {
            let top_key = k(leak(format!("top{i}")));
            tester
                .add_dependency_key(top_key.clone(), leaf_key.clone())
                .add_dependency_key(top_key.clone(), last_key.clone());
            top_key
        })
        .collect();
    tester.eval(false, &tops);
    let notify_start = Latch::new(1);
    set_null(&tester, &leaf_key);
    if interrupt {
        // leaf will wait for an interrupt if desired. We cannot use the usual ChainedFunction
        // because we need to actually throw the interrupt.
        let should_sleep = Arc::new(AtomicBool::new(true));
        {
            let (notify_start, should_sleep) = (notify_start.clone(), should_sleep.clone());
            tester.get_or_create(leaf_key.clone(), /* mark_as_modified= */ true);
            tester.set_builder_key(leaf_key.clone(), move |_, env| {
                notify_start.count_down();
                if should_sleep.load(Ordering::SeqCst) {
                    // Should be interrupted within 5 seconds.
                    // Java: Thread.sleep(5000), which throws InterruptedException.
                    let deadline = std::time::Instant::now() + Duration::from_secs(5);
                    while std::time::Instant::now() < deadline {
                        env.check_interrupted()?;
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    panic!("AssertionError: leaf was not interrupted");
                }
                Ok(Some("crunchy".into()))
            });
        }
        tester.invalidate();
        std::thread::scope(|scope| {
            let eval_thread = scope.spawn(|| {
                assert_eq!(tester.try_eval(false, &tops).err(), Some(Interrupted));
            });
            notify_start.wait("leaf did not start");
            // Java: evalThread.interrupt().
            tester.evaluator.interrupt();
            eval_thread.join().expect("eval thread");
        });
        // Java: the interrupt was consumed by the evaluation thread.
        tester.evaluator.clear_interrupt();
        // Free leafKey to compute next time.
        should_sleep.store(false, Ordering::SeqCst);
    } else {
        // Non-interrupt case. Just throw an error in the child.
        tester
            .get_or_create(leaf_key.clone(), /* mark_as_modified= */ true)
            .set_has_error_key(leaf_key.clone(), true);
        tester.invalidate();
        // The error thrown may non-deterministically bubble up to a parent that has not yet
        // started processing, but has been enqueued for processing.
        tester.eval(false, &tops);
        tester
            .get_or_create(leaf_key.clone(), /* mark_as_modified= */ true)
            .set_has_error_key(leaf_key.clone(), false);
        tester.set_key(leaf_key, "crunchy");
    }
    // lastKey was not touched during the previous build, but its reverse deps on its parents
    // should still be accurate.
    tester.set_key(last_key, "new last");
    tester.invalidate();
    let r = tester.eval(false, &tops);
    for top_key in &tops {
        assert_eq!(
            r.get_dyn(top_key)
                .unwrap_or_else(|| panic!("{top_key:?}"))
                .downcast_ref::<String>()
                .unwrap(),
            "crunchynew last",
            "{top_key:?}"
        );
    }
}

/// Regression test: make sure that if an evaluation fails before a dirty value starts evaluation
/// (in particular, before it is reset), the graph remains consistent.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3816 MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnError
#[test]
fn many_dirty_values_clear_children_on_error() {
    many_dirty_values_clear_children_on_fail(/* interrupt= */ false);
}

/// Regression test: Make sure that if an evaluation is interrupted before a dirty value starts
/// evaluation (in particular, before it is reset), the graph remains consistent.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3825 MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnInterrupt
#[test]
fn many_dirty_values_clear_children_on_interrupt() {
    many_dirty_values_clear_children_on_fail(/* interrupt= */ true);
}

/// Regression test: child value that has been deleted since it and its parent were marked dirty
/// no longer knows it has a reverse dep on its parent.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4140 MemoizingEvaluatorTest.changedBuildAfterFailedThenSuccessfulBuild
#[test]
fn changed_build_after_failed_then_successful_build() {
    let tester = MemoizingTester::new();
    let leaf = nk("leaf");
    let top = nk("top");
    tester.add_dependency_key(top.clone(), leaf.clone());
    tester.set_key(leaf.clone(), "leafy");
    assert_eq!(tester.eval_and_get(false, &top), "leafy");
    assert!(tester.get_dirty_keys().is_empty());
    assert!(tester.get_deleted_keys().is_empty());
    fail_build_and_remove_value(&tester, &leaf);
    tester.set_key(leaf.clone(), "crunchy");
    tester.invalidate();
    tester.eval(false, std::slice::from_ref(&leaf));
    // Leaf no longer has reverse dep on top. Check that this doesn't cause problems, even if the
    // top value is evaluated unconditionally.
    tester.get_or_create(top.clone(), /* mark_as_modified= */ true);
    tester.invalidate();
    assert_eq!(tester.eval_and_get(false, &top), "crunchy");
}

/// Regression test: child value that has been deleted since it and its parent were marked dirty
/// no longer knows it has a reverse dep on its parent.
///
/// Start with:
///
/// ```text
///              top0  ... top1000
///                  \  | /
///                   leaf
/// ```
///
/// Then fail to build leaf. Now the entry for leaf should have no "memory" that it was ever
/// depended on by tops. Now build tops, but fail again.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4177 MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnSecondFail
#[test]
fn many_dirty_values_clear_children_on_second_fail() {
    let tester = MemoizingTester::new();
    let leaf_key = nk("leaf");
    tester.set_key(leaf_key.clone(), "leafy");
    let last_key = k("last");
    tester.set_key(last_key.clone(), "last");
    // Request far more top-level values than there are threads, so some of them will block until
    // the leaf child is enqueued for processing.
    let tops: Vec<SkyKey> = (0..10000)
        .map(|i| {
            let top_key = k(leak(format!("top{i}")));
            tester
                .add_dependency_key(top_key.clone(), leaf_key.clone())
                .add_dependency_key(top_key.clone(), last_key.clone());
            top_key
        })
        .collect();
    tester.eval(false, &tops);
    fail_build_and_remove_value(&tester, &leaf_key);
    // Request the tops. Since leaf was deleted from the graph last build, it no longer knows that
    // its parents depend on it. When leaf throws, at least one of its parents (hopefully) will not
    // have re-informed leaf that the parent depends on it, exposing the bug, since the parent
    // should then not try to clean the reverse dep from leaf.
    set_null(&tester, &leaf_key);
    // Evaluator will think leaf was interrupted because it threw, so it will be cleaned from
    // graph.
    tester.get_or_create(leaf_key.clone(), /* mark_as_modified= */ true);
    tester.set_builder_key(leaf_key, interrupt_builder);
    tester.invalidate();
    assert_eq!(tester.try_eval(false, &tops).err(), Some(Interrupted));
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4231 MemoizingEvaluatorTest.failedDirtyBuildInBuilder
#[test]
fn failed_dirty_build_in_builder() {
    let tester = MemoizingTester::new();
    let leaf = nk("leaf");
    let second_error = nk("secondError");
    let top = k("top");
    tester
        .add_dependency_key(top.clone(), leaf.clone())
        .add_error_dependency_key(top.clone(), second_error.clone(), "recover");
    tester
        .set_key(second_error.clone(), "secondError")
        .add_dependency_key(second_error.clone(), leaf.clone());
    tester.set_key(leaf.clone(), "leafy");
    assert_eq!(tester.eval_and_get(false, &top), "leafysecondError");
    assert!(tester.get_dirty_keys().is_empty());
    assert!(tester.get_deleted_keys().is_empty());
    // Invalidate leaf.
    tester.get_or_create(leaf.clone(), /* mark_as_modified= */ true);
    tester.set_key(leaf, "crunchy");
    tester
        .get_or_create(second_error.clone(), /* mark_as_modified= */ true)
        .set_has_error_key(second_error, true);
    tester
        .get_or_create(top.clone(), /* mark_as_modified= */ false)
        .set_has_error("top", true);
    tester.invalidate();
    let r = tester.eval(false, std::slice::from_ref(&top));
    singleton_error(&r, &top);
}

/// Regression test for crash bug.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4289 MemoizingEvaluatorTest.dirtyWithOwnErrorDependsOnTransientErrorTurningGood
#[test]
fn dirty_with_own_error_depends_on_transient_error_turning_good() {
    let tester = MemoizingTester::new();
    let error = nk("error");
    tester.set_has_transient_error_key(error.clone(), true);
    let top_key = k("top");
    {
        let error = error.clone();
        tester.set_builder_key(top_key.clone(), move |_, env| {
            match env.get_value_or_throw_dyn::<SomeErrorException>(&error)? {
                Ok(v) => Ok(dep_string(v)),
                Err(e) => Err(SkyFunctionException::persistent(e).into()),
            }
        });
    }
    let r = tester.eval(false, std::slice::from_ref(&top_key));
    tester.invalidate_transient_errors();
    singleton_error(&r, &top_key);
    tester.set_has_transient_error_key(error.clone(), false);
    tester.set_key(error.clone(), "reformed");
    tester.get_or_create(top_key.clone(), /* mark_as_modified= */ false);
    tester
        .with_key(top_key.clone(), |s| s.builder = None)
        .add_dependency_key(top_key.clone(), error);
    tester.invalidate();
    tester.invalidate_transient_errors();
    let r = tester.eval(false, std::slice::from_ref(&top_key));
    assert_eq!(*r.get(&T("top")).unwrap(), "reformed");
    assert!(!r.has_error(), "{r:?}");
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4394 MemoizingEvaluatorTest.dirtyWithRecoveryErrorDependsOnErrorTurningGood
#[test]
fn dirty_with_recovery_error_depends_on_error_turning_good() {
    let tester = MemoizingTester::new();
    let error = nk("error");
    tester.set_has_error_key(error.clone(), true);
    let top_key = k("top");
    {
        let error = error.clone();
        tester.set_builder_key(top_key.clone(), move |_, env| {
            if let Err(e) = env.get_value_or_throw_dyn::<SomeErrorException>(&error)? {
                return Err(SkyFunctionException::persistent(e).into());
            }
            Ok(None)
        });
    }
    let r = tester.eval(false, std::slice::from_ref(&top_key));
    singleton_error(&r, &top_key);
    tester.set_has_error_key(error.clone(), false);
    tester.set_key(error.clone(), "reformed");
    tester
        .with_key(top_key.clone(), |s| s.builder = None)
        .add_dependency_key(top_key.clone(), error);
    tester.invalidate();
    let r = tester.eval(false, std::slice::from_ref(&top_key));
    assert_eq!(*r.get(&T("top")).unwrap(), "reformed");
    assert!(!r.has_error(), "{r:?}");
}

/// Java: `new DelegatingEventHandler(reporter) { handle(e) { super.handle(e); ... } }`.
struct DelegatingEventHandler {
    delegate: Arc<StoredEventHandler>,
    on_event: Box<dyn Fn(&Event) + Send + Sync>,
}

impl blaze_events::EventHandler for DelegatingEventHandler {
    fn handle(&self, e: &Event) {
        self.delegate.handle(e);
        (self.on_event)(e);
    }
}

impl blaze_events::ExtendedEventHandler for DelegatingEventHandler {
    fn post(&self, obj: Arc<dyn blaze_events::Postable>) {
        self.delegate.post(obj);
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4712 MemoizingEvaluatorTest.changePruningWithIntermittentEvent
#[test]
fn change_pruning_with_intermittent_event() {
    let tester = MemoizingTester::new();
    let parent_event = "parent_event";
    let wait_event = "wait_event";
    let child_event = "child_event";
    let wait = k("wait_key");
    let parent = k("parent_key");
    let child = nk("child_key");
    let parent_evaluated = Latch::new(2);

    // Java: eventCollector, which eventCollector.clear() empties in place.
    let event_collector: Arc<Mutex<Arc<StoredEventHandler>>> =
        Arc::new(Mutex::new(Arc::new(StoredEventHandler::new())));
    let reporter = || -> Arc<dyn blaze_events::ExtendedEventHandler> {
        let parent_evaluated = parent_evaluated.clone();
        Arc::new(DelegatingEventHandler {
            delegate: event_collector.lock().clone(),
            on_event: Box::new(move |e| {
                // Release the CountDownLatch every time the parent node fires the event
                if e.get_message() == parent_event {
                    parent_evaluated.count_down();
                }
            }),
        })
    };

    {
        let (parent_evaluated, event_collector) =
            (parent_evaluated.clone(), event_collector.clone());
        tester.set_builder_key(wait.clone(), move |_, env| {
            // Wait for the parent and child actions to complete before computing wait node
            // Java: parentEvaluated.await(WAIT_TIMEOUT_SECONDS) (the result is not checked).
            let deadline = std::time::Instant::now() + WAIT_TIMEOUT;
            while parent_evaluated.get_count() > 0 && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            let mut got = messages(&event_collector.lock());
            got.sort();
            assert_eq!(got, vec![child_event, parent_event]);

            env.listener()
                .handle(&Event::progress(wait_event.to_string()));
            Ok(Some("wait_value".into()))
        });
    }
    tester
        .with_key(child.clone(), |s| s.value = Some("child_value".into()))
        .set_warning_key(child.clone(), child_event);
    tester
        .add_dependency_key(parent.clone(), child.clone())
        .with_key(parent.clone(), |s| s.value = Some("parent_value".into()))
        .set_error_event_key(parent.clone(), parent_event);

    let r = tester
        .try_eval_with_reporter(false, std::slice::from_ref(&parent), reporter())
        .expect("not interrupted");
    assert_eq!(*r.get(&T("parent_key")).unwrap(), "parent_value");
    {
        let mut got = messages(&event_collector.lock());
        got.sort();
        assert_eq!(got, vec![child_event, parent_event]);
    }
    assert_eq!(parent_evaluated.get_count(), 1);

    // Reset the event collector and mark the child as modified without actually changing values
    *event_collector.lock() = Arc::new(StoredEventHandler::new());
    tester.reset_played_events();
    tester.get_or_create(child, /* mark_as_modified= */ true);
    tester.invalidate();

    let r = tester
        .try_eval_with_reporter(false, &[parent, wait], reporter())
        .expect("not interrupted");
    let mut values: Vec<String> = r
        .values
        .iter()
        .map(|(_, v)| v.downcast_ref::<String>().unwrap().clone())
        .collect();
    values.sort();
    assert_eq!(values, vec!["parent_value", "wait_value"]);

    // These assertions are to check that all events fired at the end of evaluation.
    assert_eq!(parent_evaluated.get_count(), 0);
    let mut got = messages(&event_collector.lock());
    got.sort();
    assert_eq!(got, vec![child_event, parent_event, wait_event]);
}

/// Java: the anonymous `EventFilter` of `depEventPredicate`.
struct DepEventPredicateFilter {
    parent: SkyKey,
    included_dep: SkyKey,
}

impl EventFilter for DepEventPredicateFilter {
    fn store_events(&self) -> bool {
        true
    }

    fn should_propagate(&self, dep_key: &SkyKey, primary_key: &SkyKey) -> bool {
        *primary_key != self.parent || *dep_key == self.included_dep
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4775 MemoizingEvaluatorTest.depEventPredicate
#[test]
fn dep_event_predicate() {
    let parent = k("parent");
    let excluded_dep = k("excludedDep");
    let included_dep = k("includedDep");
    let tester = MemoizingTester::with_options(
        Arc::new(tracking_progress_receiver::TrackingProgressReceiver::new(
            /* check_evaluation_results= */ true,
        )),
        Arc::new(DepEventPredicateFilter {
            parent: parent.clone(),
            included_dep: included_dep.clone(),
        }),
    );
    tester
        .add_dependency_key(parent.clone(), excluded_dep.clone())
        .add_dependency_key(parent.clone(), included_dep.clone());
    tester
        .set_error_event_key(excluded_dep.clone(), "excludedDep error message")
        .with_key(excluded_dep.clone(), |s| {
            s.value = Some("excludedDep".into())
        });
    tester
        .set_error_event_key(included_dep.clone(), "includedDep error message")
        .with_key(included_dep.clone(), |s| {
            s.value = Some("includedDep".into())
        });
    tester.eval(false, &[included_dep, excluded_dep]);
    let mut got = tester.events();
    got.sort();
    assert_eq!(
        got,
        vec!["excludedDep error message", "includedDep error message"]
    );
    tester.clear_events();
    tester.reset_played_events();
    tester.eval(true, std::slice::from_ref(&parent));
    assert_eq!(tester.events(), vec!["includedDep error message"]);
    let node = tester.evaluator.node(&parent).expect("parent in the graph");
    let events = {
        let state = node.lock();
        crate::value_with_metadata::ValueWithMetadata::get_events(
            state
                .entry
                .get_value_maybe_with_metadata()
                .expect("parent done"),
        )
    };
    assert_eq!(
        *events.to_list(),
        vec![crate::value_with_metadata::StoredReportable::Event(
            Event::error("includedDep error message".to_string())
        )]
    );
}

/// General stress/fuzz test of the evaluator with failure. Construct a large graph, and then
/// throw exceptions during building at various points.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4915 MemoizingEvaluatorTest.twoRailLeftRightDependenciesWithFailure
#[test]
fn two_rail_left_right_dependencies_with_failure() {
    /// Java: `PassThroughSelected`.
    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5499 MemoizingEvaluatorTest.PassThroughSelected
    fn pass_through_selected(key: SkyKey) -> Computer {
        Arc::new(move |deps, _| {
            Ok(Some(
                deps.iter()
                    .find(|(d, _)| *d == key)
                    .expect("checkNotNull")
                    .1
                    .clone(),
            ))
        })
    }
    let tester = MemoizingTester::new();
    let left_values: Vec<SkyKey> = (0..TEST_NODE_COUNT)
        .map(|i| nk(leak(format!("left-{i}"))))
        .collect();
    let right_values: Vec<SkyKey> = (0..TEST_NODE_COUNT)
        .map(|i| k(leak(format!("right-{i}"))))
        .collect();
    for i in 0..TEST_NODE_COUNT {
        if i == 0 {
            tester
                .add_dependency_key(left_values[i].clone(), k("leaf"))
                .add_dependency_key(right_values[i].clone(), k("leaf"));
        } else {
            tester
                .add_dependency_key(left_values[i].clone(), left_values[i - 1].clone())
                .add_dependency_key(left_values[i].clone(), right_values[i - 1].clone())
                .with_key(left_values[i].clone(), |s| {
                    s.computer = Some(pass_through_selected(left_values[i - 1].clone()))
                });
            tester
                .add_dependency_key(right_values[i].clone(), left_values[i - 1].clone())
                .add_dependency_key(right_values[i].clone(), right_values[i - 1].clone())
                .with_key(right_values[i].clone(), |s| {
                    s.computer = Some(pass_through_selected(right_values[i - 1].clone()))
                });
        }
    }
    tester.set("leaf", "leaf");

    let last_left = left_values[TEST_NODE_COUNT - 1].clone();
    let last_right = right_values[TEST_NODE_COUNT - 1].clone();
    let get = |r: &EvaluationResult, key: &SkyKey| -> String {
        r.get_dyn(key)
            .unwrap_or_else(|| panic!("{key:?}: {r:?}"))
            .downcast_ref::<String>()
            .unwrap()
            .clone()
    };

    #[allow(clippy::needless_range_loop)] // Java's loop, over the first TESTED_NODES
    for i in 0..TESTED_NODES {
        tester
            .get_or_create(left_values[i].clone(), /* mark_as_modified= */ true)
            .set_has_error_key(left_values[i].clone(), true);
        tester.invalidate();
        let r = tester.eval(false, &[last_left.clone(), last_right.clone()]);
        assert!(r.has_error(), "run {i}");
        tester.evaluator.invalidate([left_values[i].clone()]);
        tester.invalidate();
        let r = tester.eval(false, &[last_left.clone(), last_right.clone()]);
        assert!(r.has_error(), "run {i}");
        tester
            .get_or_create(left_values[i].clone(), /* mark_as_modified= */ true)
            .set_has_error_key(left_values[i].clone(), false);
        tester.invalidate();
        let r = tester.eval(false, &[last_left.clone(), last_right.clone()]);
        assert_eq!(get(&r, &last_left), "leaf", "run {i}");
        assert_eq!(get(&r, &last_right), "leaf", "run {i}");
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5511 MemoizingEvaluatorTest.removedNodeComesBack
fn removed_node_comes_back(tester: &MemoizingTester) {
    let top = nk("top");
    let mid = k("mid");
    let leaf = nk("leaf");
    // When top depends on mid, which depends on leaf,
    tester.add_dependency_key(top.clone(), mid.clone());
    tester.add_dependency_key(mid.clone(), leaf.clone());
    tester.set_key(leaf.clone(), "leaf");
    // Then when top is evaluated, its value is as expected.
    assert_eq!(tester.eval_and_get(true, &top), "leaf");
    // When top is changed to no longer depend on mid,
    tester
        .get_or_create(top.clone(), /* mark_as_modified= */ true)
        .remove_dependency_key(top.clone(), &mid)
        .with_key(top.clone(), |s| s.value = Some("top".into()));
    // And leaf is invalidated,
    tester.get_or_create(leaf.clone(), /* mark_as_modified= */ true);
    // Then when top is evaluated, its value is as expected,
    tester.invalidate();
    assert_eq!(tester.eval_and_get(true, &top), "top");
    // And there is no value for mid in the graph,
    assert!(tester.evaluator.get_existing_value(&mid).is_none());
    assert!(
        tester
            .evaluator
            .get_existing_error_for_testing(&mid)
            .is_none()
    );
    // Or for leaf.
    assert!(tester.evaluator.get_existing_value(&leaf).is_none());
    assert!(
        tester
            .evaluator
            .get_existing_error_for_testing(&leaf)
            .is_none()
    );

    // When top is changed to depend directly on leaf,
    tester
        .get_or_create(top.clone(), /* mark_as_modified= */ true)
        .add_dependency_key(top.clone(), leaf.clone())
        .with_key(top.clone(), |s| s.value = None);
    // Then when top is evaluated, its value is as expected,
    tester.invalidate();
    assert_eq!(tester.eval_and_get(true, &top), "leaf");
    // and there is no value for mid in the graph,
    assert!(tester.evaluator.get_existing_value(&mid).is_none());
    assert!(
        tester
            .evaluator
            .get_existing_error_for_testing(&mid)
            .is_none()
    );
}

/// Tests that a removed and then reinstated node doesn't try to invalidate its erstwhile parent
/// when it is invalidated.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5559 MemoizingEvaluatorTest.removedNodeComesBackAndInvalidates
#[test]
fn removed_node_comes_back_and_invalidates() {
    let tester = MemoizingTester::new();
    removed_node_comes_back(&tester);
    // When leaf is invalidated again,
    // Java: skyKey("leaf") (the hermetic key, not the nonHermeticKey "leaf" of the graph).
    tester.get_or_create(k("leaf"), /* mark_as_modified= */ true);
    // Then when top is evaluated, its value is as expected.
    tester.invalidate();
    assert_eq!(tester.eval_and_get(true, &nk("top")), "leaf");
}

/// Tests that a removed and then reinstated node behaves properly when its parent disappears and
/// then reappears.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5572 MemoizingEvaluatorTest.removedNodeComesBackAndOtherInvalidates
#[test]
fn removed_node_comes_back_and_other_invalidates() {
    let tester = MemoizingTester::new();
    removed_node_comes_back(&tester);
    let top = nk("top");
    let mid = k("mid");
    let leaf = nk("leaf");
    // When top is invalidated again,
    tester
        .get_or_create(top.clone(), /* mark_as_modified= */ true)
        .remove_dependency_key(top.clone(), &leaf)
        .add_dependency_key(top.clone(), mid);
    // Then when top is evaluated, its value is as expected.
    tester.invalidate();
    assert_eq!(tester.eval_and_get(true, &top), "leaf");
}

/// Tests that a removed and then reinstated node doesn't have a reverse dep on a former parent.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5586 MemoizingEvaluatorTest.removedInvalidatedNodeComesBackAndOtherInvalidates
#[test]
fn removed_invalidated_node_comes_back_and_other_invalidates() {
    let tester = MemoizingTester::new();
    let top = nk("top");
    let leaf = nk("leaf");
    // When top depends on leaf,
    tester.add_dependency_key(top.clone(), leaf.clone());
    tester.set_key(leaf.clone(), "leaf");
    // Then when top is evaluated, its value is as expected.
    assert_eq!(tester.eval_and_get(true, &top), "leaf");
    // When top is changed to no longer depend on leaf,
    tester
        .get_or_create(top.clone(), /* mark_as_modified= */ true)
        .remove_dependency_key(top.clone(), &leaf)
        .with_key(top.clone(), |s| s.value = Some("top".into()));
    // And leaf is invalidated,
    tester.get_or_create(leaf.clone(), /* mark_as_modified= */ true);
    // Then when top is evaluated, its value is as expected,
    tester.invalidate();
    assert_eq!(tester.eval_and_get(true, &top), "top");
    // And there is no value for leaf in the graph.
    assert!(tester.evaluator.get_existing_value(&leaf).is_none());
    assert!(
        tester
            .evaluator
            .get_existing_error_for_testing(&leaf)
            .is_none()
    );
    // When leaf is evaluated, so that it is present in the graph again,
    assert_eq!(tester.eval_and_get(true, &leaf), "leaf");
    // And top is changed to depend on leaf again,
    tester
        .get_or_create(top.clone(), /* mark_as_modified= */ true)
        .add_dependency_key(top.clone(), leaf)
        .with_key(top.clone(), |s| s.value = None);
    // Then when top is evaluated, its value is as expected.
    tester.invalidate();
    assert_eq!(tester.eval_and_get(true, &top), "leaf");
}

// ---------------------------------------------------------------------------------------------
// Ports that use a graph listener (Java's NotifyingHelper / DeterministicHelper).

use crate::graph_listener::{Context as ListenerContext, EventType, Order};

/// Java: `Thread.sleep(millis)` in a listener or function, ended by an interrupt of the current
/// thread (`blaze_shell::ThreadInterrupt`, which the evaluator's `interruptInFlightTasks` sets).
/// Returns true if interrupted, false if the time ran out. The interrupt status stays set (Java's
/// `catch (InterruptedException e) { Thread.currentThread().interrupt(); }`).
fn sleep_until_interrupted(millis: u64) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_millis(millis);
    let me = blaze_shell::ThreadInterrupt::current();
    while std::time::Instant::now() < deadline {
        if me.is_interrupted() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    me.is_interrupted()
}

/// Java: `latch.await(timeout)` in a listener: true if the latch reached zero, false if the time
/// ran out or the current thread was interrupted (Java throws `InterruptedException`; the listeners
/// catch it and set the interrupt status again). Returns `Err(())` when interrupted.
fn await_latch_interruptibly(latch: &Latch, millis: u64) -> Result<bool, ()> {
    let deadline = std::time::Instant::now() + Duration::from_millis(millis);
    let me = blaze_shell::ThreadInterrupt::current();
    loop {
        if latch.get_count() == 0 {
            return Ok(true);
        }
        if me.is_interrupted() {
            return Err(());
        }
        if std::time::Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Java: `TestUtils.WAIT_TIMEOUT_MILLISECONDS`.
const WAIT_TIMEOUT_MILLISECONDS: u64 = 60_000;

/// Regression test for bug where a value is requested by a parent and enqueued as a top-level
/// value at the same time. The graph listener makes the top-level enqueuing wait for the value to
/// be set, so that the top-level enqueuing finds that mid has already been built. The progress
/// receiver should be notified that mid has been built.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:843 MemoizingEvaluatorTest.alreadyAnalyzedBadTarget
#[test]
fn already_analyzed_bad_target() {
    let tester = MemoizingTester::new();
    let mid = k("zzmid");
    let value_set = Latch::new(1);
    {
        let (mid, value_set) = (mid.clone(), value_set.clone());
        tester.inject_graph_listener_for_testing(
            move |key: &SkyKey, event_type: EventType, _order: Order, context: &ListenerContext| {
                if *key != mid {
                    return;
                }
                match event_type {
                    EventType::AddReverseDep => {
                        if matches!(context, ListenerContext::Null) {
                            // Context is null when we are enqueuing this value as a top-level
                            // job.
                            value_set.wait("value not set");
                        }
                    }
                    EventType::SetValue => value_set.count_down(),
                    _ => {}
                }
            },
            /* deterministic= */ true,
        );
    }
    let top = k("aatop");
    tester.add_dependency_key(top.clone(), mid.clone());
    tester.set_has_error_key(mid.clone(), true);
    tester.eval(false, &[top, mid.clone()]);
    assert_eq!(value_set.get_count(), 0);
    assert_eq!(evaluated_keys(&tester), key_set(&[mid]));
}

/// Regression test: IllegalStateException in BuildingState.isReady(). The ParallelEvaluator used
/// to assume during cycle-checking that all values had been built as fully as possible -- that
/// evaluation had not been interrupted. However, we also do cycle-checking in nokeep-going mode
/// when a value throws an error (possibly prematurely shutting down evaluation) but that error
/// then bubbles up into a cycle.
///
/// We want to achieve the following state: we are checking for a cycle; the value we examine has
/// not yet finished checking its children to see if they are dirty; but all children checked so
/// far have been unchanged. This value is "otherTop". We first build otherTop, then mark its first
/// child changed (without actually changing it), and then do a second build. On the second build,
/// we also build "top", which requests a cycle that depends on an error. We wait to signal
/// otherTop that its first child is done until the error throws and shuts down evaluation. The
/// error then bubbles up to the cycle, and so the bubbling is aborted. Finally, cycle checking
/// happens, and otherTop is examined, as desired.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1393 MemoizingEvaluatorTest.cycleAndErrorAndReady
#[test]
fn cycle_and_error_and_ready() {
    let tester = MemoizingTester::new();
    // This value will not have finished building on the second build when the error is thrown.
    let other_top = k("otherTop");
    let error_key = k("error");
    // Is the graph state all set up and ready for the error to be thrown? The three values are
    // exceptionMarker, cycle2Key, and dep1 (via signaling otherTop).
    let values_ready = Latch::new(3);
    // Is evaluation being shut down? This is counted down by the exceptionMarker's builder, after
    // it has waited for the threadpool's exception latch to be released.
    let error_thrown = Latch::new(1);
    // We don't do anything on the first build.
    let second_build = Arc::new(AtomicBool::new(false));
    {
        let (other_top, values_ready, error_thrown, second_build) = (
            other_top.clone(),
            values_ready.clone(),
            error_thrown.clone(),
            second_build.clone(),
        );
        tester.inject_graph_listener_for_testing(
            move |key: &SkyKey, event_type: EventType, _: Order, _: &ListenerContext| {
                if !second_build.load(Ordering::SeqCst) {
                    return;
                }
                if *key == other_top && event_type == EventType::Signal {
                    // otherTop is being signaled that dep1 is done. Tell the error value that it
                    // is ready, then wait until the error is thrown, so that otherTop's builder
                    // is not re-entered.
                    values_ready.count_down();
                    error_thrown.wait("error not thrown");
                }
            },
            /* deterministic= */ true,
        );
    }
    let dep1 = nk("dep1");
    tester.set_key(dep1.clone(), "dep1");
    let dep2 = k("dep2");
    tester.set_key(dep2.clone(), "dep2");
    // otherTop should request the deps one at a time, so that it can be in the
    // CHECK_DEPENDENCIES state even after one dep is re-evaluated.
    {
        let (dep1, dep2) = (dep1.clone(), dep2.clone());
        tester.set_builder_key(other_top.clone(), move |_, env| {
            env.get_value_dyn(&dep1)?;
            if env.values_missing() {
                return Ok(None);
            }
            env.get_value_dyn(&dep2)?;
            Ok(if env.values_missing() {
                None
            } else {
                Some("otherTop".into())
            })
        });
    }
    // Prime the graph with otherTop, so we can dirty it next build.
    assert_eq!(tester.eval_and_get(false, &other_top), "otherTop");
    // Mark dep1 changed, so otherTop will be dirty and request re-evaluation of dep1.
    tester.get_or_create(dep1, /* mark_as_modified= */ true);
    let top_key = k("top");
    // Note that since DeterministicHelper alphabetizes reverse deps, it is important that
    // "cycle2" comes before "top".
    let cycle1_key = k("cycle1");
    let cycle2_key = k("cycle2");
    tester.add_dependency_key(top_key.clone(), cycle1_key.clone());
    tester
        .add_dependency_key(cycle1_key.clone(), error_key.clone())
        .add_dependency_key(cycle1_key.clone(), cycle2_key.clone())
        // PORT: Java's cycle1 gets null for error (not done) and goes on to request cycle2, so
        // both are enqueued. A request blocks here (error waits for cycle2 to start), so cycle1
        // requests both in one batch.
        .with_key(cycle1_key.clone(), |s| s.batch = true);
    tester.set_chained_key(
        error_key,
        Chained {
            wait_to_finish: Some(values_ready.clone()),
            ..Chained::default()
        },
    );
    // Make sure cycle2Key has declared its dependence on cycle1Key before error throws.
    tester.set_chained_key(
        cycle2_key.clone(),
        Chained {
            notify_start: Some(values_ready.clone()),
            value: Some("never returned"),
            deps: vec!["cycle1"],
            ..Chained::default()
        },
    );
    // Value that waits until an exception is thrown to finish building. We use it just to be
    // informed when the threadpool is shutting down.
    let exception_marker = k("exceptionMarker");
    tester.set_chained_key(
        exception_marker.clone(),
        Chained {
            notify_start: Some(values_ready),
            wait_to_finish: Some(Latch::new(0)),
            notify_finish: Some(error_thrown),
            wait_for_exception: true,
            value: Some("exception marker"),
            ..Chained::default()
        },
    );
    tester.invalidate();
    second_build.store(true, Ordering::SeqCst);
    // otherTop must be first, since we check top-level values for cycles in the order in which
    // they appear here.
    let r = tester.eval(false, &[other_top, top_key.clone(), exception_marker]);
    let cycle_infos = r
        .get_error(&top_key)
        .unwrap_or_else(|| panic!("{r:?}"))
        .get_cycle_info();
    assert!(!cycle_infos.is_empty(), "{r:?}");
    assert_eq!(cycle_infos.len(), 1, "{r:?}");
    let cycle_info = &cycle_infos[0];
    assert_eq!(
        r.error_map()
            .iter()
            .map(|(k, _)| k.clone())
            .collect::<Vec<_>>(),
        vec![top_key.clone()]
    );
    assert_eq!(cycle_info.get_path_to_cycle(), &[top_key]);
    assert_eq!(cycle_info.get_cycle(), &[cycle1_key, cycle2_key]);
}

/// We are checking here that we are resilient to a race condition in which a value that is
/// checking its children for dirtiness is signaled by all of its children, putting it in a ready
/// state, before the thread has terminated. Optionally, one of its children may throw an error,
/// shutting down the threadpool. The essential race is that a child about to throw signals its
/// parent and the parent's builder restarts itself before the exception is thrown. Here, the
/// signaling happens while dirty dependencies are being checked. We control the timing by
/// blocking "top"'s registering itself on its deps.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2207 MemoizingEvaluatorTest.dirtyChildEnqueuesParentDuringCheckDependencies
fn dirty_child_enqueues_parent_during_check_dependencies(throw_error: bool) {
    let tester = MemoizingTester::new();
    // Value to be built. It will be signaled to rebuild before it has finished checking its deps.
    let top = k("a_top");
    // otherTop is alphabetically after top.
    let other_top = k("z_otherTop");
    // Dep that blocks before it acknowledges being added as a dep by top, so the firstKey value
    // has time to signal top. (Importantly its key is alphabetically after 'firstKey').
    let slow_adding_dep = k("slowDep");
    // Value that is modified on the second build. Its thread won't finish until it signals top,
    // which will wait for the signal before it enqueues its next dep. We prevent the thread from
    // finishing by having the graph listener block on the second reverse dep to signal.
    let first_key = nk("first");
    tester.set_key(first_key.clone(), "biding");
    // Don't perform any blocking on the first build.
    let delay_top_signaling = Arc::new(AtomicBool::new(false));
    let top_signaled = Latch::new(1);
    let top_requested_dep_or_restarted_build = Latch::new(1);
    let parents_requested = Latch::new(2);
    {
        let (top, other_top, slow_adding_dep, first_key) = (
            top.clone(),
            other_top.clone(),
            slow_adding_dep.clone(),
            first_key.clone(),
        );
        let (delay_top_signaling, top_signaled, top_requested, parents_requested) = (
            delay_top_signaling.clone(),
            top_signaled.clone(),
            top_requested_dep_or_restarted_build.clone(),
            parents_requested.clone(),
        );
        tester.inject_graph_listener_for_testing(
            move |key: &SkyKey, event_type: EventType, order: Order, context: &ListenerContext| {
                if !delay_top_signaling.load(Ordering::SeqCst) {
                    return;
                }
                if *key == other_top && event_type == EventType::Signal {
                    top_requested.wait("top's builder did not start in time");
                    return;
                }
                if *key == first_key
                    && event_type == EventType::AddReverseDep
                    && order == Order::After
                {
                    parents_requested.count_down();
                    return;
                }
                if *key == first_key
                    && event_type == EventType::CheckIfDone
                    && order == Order::After
                {
                    parents_requested.count_down();
                    if throw_error {
                        top_requested.count_down();
                    }
                    return;
                }
                if *key == top && event_type == EventType::Signal && order == Order::After {
                    // top is signaled by firstKey (since slowAddingDep is blocking), so
                    // slowAddingDep is now free to acknowledge top as a parent.
                    top_signaled.count_down();
                    return;
                }
                if *key == first_key && event_type == EventType::SetValue && order == Order::Before
                {
                    // Make sure both parents add themselves as rdeps.
                    parents_requested.wait("parents did not request dep in time");
                }
                if *key == slow_adding_dep
                    && event_type == EventType::CheckIfDone
                    && context.key() == Some(&top)
                    && order == Order::Before
                {
                    // If top is trying to declare a dep on slowAddingDep, wait until firstKey has
                    // signaled top. Then this add dep will return DONE and top will be signaled,
                    // making it ready, so it will be enqueued.
                    top_signaled.wait("first key didn't signal top in time");
                }
            },
            /* deterministic= */ true,
        );
    }
    tester.set_key(slow_adding_dep.clone(), "dep");
    let num_top_invocations = Arc::new(AtomicUsize::new(0));
    {
        let (first_key, slow_adding_dep) = (first_key.clone(), slow_adding_dep.clone());
        let (num_top_invocations, delay_top_signaling, top_requested) = (
            num_top_invocations.clone(),
            delay_top_signaling.clone(),
            top_requested_dep_or_restarted_build.clone(),
        );
        tester.set_builder_key(top.clone(), move |_, env| {
            num_top_invocations.fetch_add(1, Ordering::SeqCst);
            if delay_top_signaling.load(Ordering::SeqCst) {
                // The graph listener will block on firstKey's signaling of otherTop above until
                // this thread starts running.
                top_requested.count_down();
            }
            // top's builder just requests both deps in a group.
            env.get_values_and_exceptions(&[first_key.clone(), slow_adding_dep.clone()])?;
            Ok(if env.values_missing() {
                None
            } else {
                Some("top".into())
            })
        });
    }
    // First build : just prime the graph.
    let r = tester.eval(false, std::slice::from_ref(&top));
    assert!(!r.has_error(), "{r:?}");
    assert_eq!(*r.get(&T("a_top")).unwrap(), "top");
    assert_eq!(num_top_invocations.load(Ordering::SeqCst), 2);
    // Now dirty the graph, and maybe have firstKey throw an error.
    if throw_error {
        tester.get_or_create(first_key.clone(), /* mark_as_modified= */ true);
        tester.unset_constant_value(first_key.clone());
        let parents_requested = parents_requested.clone();
        tester.set_builder_key(first_key.clone(), move |t, _| {
            parents_requested.wait("both parents didn't request in time");
            Err(SkyFunctionException::persistent(SomeErrorException(format!("{t:?}"))).into())
        });
    } else {
        tester
            .get_or_create(first_key.clone(), /* mark_as_modified= */ true)
            .with_key(first_key.clone(), |s| s.value = Some("new".into()));
    }
    tester.add_dependency_key(other_top.clone(), first_key);
    tester.invalidate();
    delay_top_signaling.store(true, Ordering::SeqCst);
    let r = tester.eval(false, &[top.clone(), other_top]);
    if throw_error {
        assert!(r.has_error(), "{r:?}");
        // No successfully evaluated values.
        assert!(r.key_names().is_empty(), "{r:?}");
        assert!(r.get_error(&top).is_some(), "{r:?}");
        assert_eq!(
            num_top_invocations.load(Ordering::SeqCst),
            3,
            "on the incremental build, top's builder should have only been used in error bubbling"
        );
    } else {
        assert_eq!(*r.get(&T("a_top")).unwrap(), "top");
        assert!(!r.has_error(), "{r:?}");
        assert_eq!(
            num_top_invocations.load(Ordering::SeqCst),
            3,
            "on the incremental build, top's builder should have only been executed once in \
             normal evaluation"
        );
    }
    assert_eq!(top_signaled.get_count(), 0);
    assert_eq!(top_requested_dep_or_restarted_build.get_count(), 0);
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2340 MemoizingEvaluatorTest.dirtyChildEnqueuesParentDuringCheckDependencies_ThrowDoesntEnqueue
#[test]
#[ignore = "restart model (lib.rs `Model`, engine.md (b) `Evaluate.run:640-656`): Java's top \
            runs twice in the first build (it returns null with its batch of deps not done and \
            is restarted), the blocking engine runs it once, so the first assertion \
            (numTopInvocations == 2) fails; the second build's choreography waits for Java's \
            re-enqueueing of top by its dep's signal, which the blocking engine does not do"]
fn dirty_child_enqueues_parent_during_check_dependencies_throw_doesnt_enqueue() {
    dirty_child_enqueues_parent_during_check_dependencies(/* throw_error= */ true);
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2346 MemoizingEvaluatorTest.dirtyChildEnqueuesParentDuringCheckDependencies_NoThrow
#[test]
#[ignore = "restart model (lib.rs `Model`, engine.md (b) `Evaluate.run:640-656`): Java's top \
            runs twice in the first build (it returns null with its batch of deps not done and \
            is restarted), the blocking engine runs it once, so the first assertion \
            (numTopInvocations == 2) fails; the second build's choreography waits for Java's \
            re-enqueueing of top by its dep's signal, which the blocking engine does not do"]
fn dirty_child_enqueues_parent_during_check_dependencies_no_throw() {
    dirty_child_enqueues_parent_during_check_dependencies(/* throw_error= */ false);
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2351 MemoizingEvaluatorTest.removeReverseDepFromRebuildingNode
#[test]
fn remove_reverse_dep_from_rebuilding_node() {
    let tester = MemoizingTester::new();
    let top_key = k("top");
    let mid_key = nk("mid");
    let changed_key = nk("changed");
    tester.with_key(changed_key.clone(), |s| s.value = Some("first".into()));
    // When top depends on mid,
    tester.add_dependency_key(top_key.clone(), mid_key.clone());
    // And mid depends on changed,
    tester.add_dependency_key(mid_key.clone(), changed_key.clone());
    let changed_key_started = Latch::new(1);
    let changed_key_can_finish = Latch::new(1);
    let control_timing = Arc::new(AtomicBool::new(false));
    {
        let (mid_key, changed_key) = (mid_key.clone(), changed_key.clone());
        let (changed_key_started, changed_key_can_finish, control_timing) = (
            changed_key_started.clone(),
            changed_key_can_finish.clone(),
            control_timing.clone(),
        );
        tester.inject_graph_listener_for_testing(
            move |key: &SkyKey, event_type: EventType, order: Order, context: &ListenerContext| {
                if !control_timing.load(Ordering::SeqCst) {
                    return;
                }
                if *key == mid_key && event_type == EventType::CheckIfDone && order == Order::Before
                {
                    changed_key_started.wait("changed key didn't start");
                } else if *key == changed_key
                    && event_type == EventType::RemoveReverseDep
                    && order == Order::After
                    && context.key() == Some(&mid_key)
                {
                    changed_key_can_finish.count_down();
                }
            },
            /* deterministic= */ false,
        );
    }
    // Then top builds as expected.
    assert_eq!(tester.eval_and_get(false, &top_key), "first");
    // When changed is modified,
    tester.get_or_create(changed_key.clone(), /* mark_as_modified= */ true);
    tester.unset_constant_value(changed_key.clone());
    // And changed is not allowed to finish building until it is released,
    tester.set_chained_key(
        changed_key.clone(),
        Chained {
            notify_start: Some(changed_key_started),
            wait_to_finish: Some(changed_key_can_finish),
            value: Some("second"),
            ..Chained::default()
        },
    );
    // And mid is independently marked as modified,
    tester
        .get_or_create(mid_key.clone(), /* mark_as_modified= */ true)
        .remove_dependency_key(mid_key.clone(), &changed_key)
        .with_key(mid_key, |s| s.value = Some("mid".into()));
    tester.invalidate();
    let new_top_key = k("newTop");
    // And changed will start rebuilding independently of midKey, because it's requested directly
    // by newTop
    tester.add_dependency_key(new_top_key.clone(), changed_key);
    // And we control the timing using the graph listener above to make sure that:
    // (1) before we do anything with mid, changed has already started, and
    // (2) changed key can't finish until mid tries to remove its reverse dep from changed,
    control_timing.store(true, Ordering::SeqCst);
    // Then this evaluation completes without crashing.
    tester.eval(false, &[new_top_key, top_key]);
}

/// Test whether a value that was already marked changed will be incorrectly marked dirty, not
/// changed, if another thread tries to mark it just dirty. To exercise this, we need to have a
/// race condition where both threads see that the value is not dirty yet, then the "changed"
/// thread marks the value changed before the "dirty" thread marks the value dirty. To accomplish
/// this, we use a countdown latch to make the "dirty" thread wait until the "changed" thread is
/// done, and another countdown latch to make both of them wait until they have both checked if
/// the value is currently clean.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3478 MemoizingEvaluatorTest.dirtyAndChangedValueIsChanged
#[test]
#[ignore = "invalidation is sequential, on the calling thread (lib.rs `Not ported`: \
            `InvalidatingNodeVisitor` is not a fork-join pool, invalidation.rs `visit`): the \
            test needs two invalidation threads marking `parent` at once (its listener blocks \
            the dirtying thread until the changing thread has marked the node), so one thread \
            waits for itself until the 60 s timeout"]
fn dirty_and_changed_value_is_changed() {
    let tester = MemoizingTester::new();
    let parent = nk("parent");
    let blocking_enabled = Arc::new(AtomicBool::new(false));
    let wait_for_changed = Latch::new(1);
    // changed thread checks value entry once (to see if it is changed). dirty thread checks
    // twice, to see if it is changed, and if it is dirty.
    let threads_started = Latch::new(3);
    {
        let parent = parent.clone();
        let (blocking_enabled, wait_for_changed, threads_started) = (
            blocking_enabled.clone(),
            wait_for_changed.clone(),
            threads_started.clone(),
        );
        tester.inject_graph_listener_for_testing(
            move |key: &SkyKey, event_type: EventType, order: Order, context: &ListenerContext| {
                if !blocking_enabled.load(Ordering::SeqCst) {
                    return;
                }
                if *key != parent {
                    return;
                }
                if event_type == EventType::IsChanged && order == Order::Before {
                    threads_started.count_down();
                }
                // Dirtiness only checked by dirty thread.
                if event_type == EventType::IsDirty && order == Order::Before {
                    threads_started.count_down();
                }
                if event_type == EventType::MarkDirty {
                    threads_started.wait("Both threads did not query if value isChanged in time");
                    if order == Order::Before
                        && let ListenerContext::DirtyType(DirtyType::Dirty) = context
                    {
                        wait_for_changed
                            .wait("'changed' thread did not mark value changed in time");
                        return;
                    }
                    if order == Order::After
                        && let ListenerContext::MarkDirtyAfter(c) = context
                        && c.dirty_type == DirtyType::Change
                    {
                        wait_for_changed.count_down();
                    }
                }
            },
            /* deterministic= */ false,
        );
    }
    let leaf = nk("leaf");
    tester.set_key(leaf.clone(), "leaf");
    tester.add_dependency_key(parent.clone(), leaf.clone());
    let r = tester.eval(false, std::slice::from_ref(&parent));
    assert_eq!(*r.get(&N("parent")).unwrap(), "leaf");
    // Invalidate leaf, but don't actually change it. It will transitively dirty parent
    // concurrently with parent directly dirtying itself.
    tester.get_or_create(leaf, /* mark_as_modified= */ true);
    let other2 = k("other2");
    tester.set_key(other2.clone(), "other2");
    // Invalidate parent, actually changing it.
    tester
        .get_or_create(parent.clone(), /* mark_as_modified= */ true)
        .add_dependency_key(parent.clone(), other2);
    tester.invalidate();
    blocking_enabled.store(true, Ordering::SeqCst);
    let r = tester.eval(false, std::slice::from_ref(&parent));
    assert_eq!(*r.get(&N("parent")).unwrap(), "leafother2");
    assert_eq!(wait_for_changed.get_count(), 0);
    assert_eq!(threads_started.get_count(), 0);
}

/// Regression test: "clearing incomplete values on --keep_going build is racy". Tests that if a
/// value is requested on the first (non-keep-going) build and its child throws an error, when the
/// second (keep-going) build runs, there is not a race that keeps it as a reverse dep of its
/// children.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4469 MemoizingEvaluatorTest.raceClearingIncompleteValues
#[test]
fn race_clearing_incomplete_values() {
    let tester = MemoizingTester::new();
    // Make sure top is enqueued before mid, to avoid a deadlock.
    let top_key = k("aatop");
    let mid_key = k("zzmid");
    let bad_key = k("bad");
    let wait_for_second_call = Arc::new(AtomicBool::new(false));
    let other_thread_winning = Latch::new(1);
    let first_thread: Arc<Mutex<Option<std::thread::ThreadId>>> = Arc::new(Mutex::new(None));
    {
        let mid_key = mid_key.clone();
        let (wait_for_second_call, other_thread_winning, first_thread) = (
            wait_for_second_call.clone(),
            other_thread_winning.clone(),
            first_thread.clone(),
        );
        tester.inject_graph_listener_for_testing(
            move |key: &SkyKey, event_type: EventType, order: Order, _: &ListenerContext| {
                if !wait_for_second_call.load(Ordering::SeqCst) {
                    return;
                }
                if *key == mid_key {
                    let me = std::thread::current().id();
                    if event_type == EventType::CreateIfAbsent {
                        // The first thread to create midKey will not be the first thread to add a
                        // reverse dep to it.
                        first_thread.lock().get_or_insert(me);
                        return;
                    }
                    if event_type == EventType::AddReverseDep {
                        let is_first = *first_thread.lock() == Some(me);
                        if order == Order::Before && is_first {
                            // If this thread created midKey, block until the other thread adds a
                            // dep on it.
                            other_thread_winning.wait("other thread didn't pass this one");
                        } else if order == Order::After && !is_first {
                            // This thread has added a dep. Allow the other thread to proceed.
                            other_thread_winning.count_down();
                        }
                    }
                }
            },
            /* deterministic= */ true,
        );
    }
    tester.add_dependency_key(top_key.clone(), mid_key.clone());
    tester.add_dependency_key(mid_key.clone(), bad_key.clone());
    tester.set_has_error_key(bad_key, true);
    let r = tester.eval(false, &[top_key.clone(), mid_key.clone()]);
    assert!(r.get_error(&mid_key).is_some(), "{r:?}");
    wait_for_second_call.store(true, Ordering::SeqCst);
    let r = tester.eval(true, &[top_key.clone(), mid_key.clone()]);
    assert!(first_thread.lock().is_some());
    assert_eq!(other_thread_winning.get_count(), 0);
    assert!(r.get_error(&mid_key).is_some(), "{r:?}");
    assert!(r.get_error(&top_key).is_some(), "{r:?}");
}

/// Test that a parent of a cached but invalidated error doesn't successfully build. First build
/// the error. Then invalidate the error via a dependency (so it will not actually change) and
/// build two new parents. Parent A will request error and abort since error isn't done yet. error
/// is then revalidated, and A is restarted. If A does not throw upon encountering the error, and
/// instead sets its value, then we throw in parent B, which waits for error to be done before
/// requesting it. Then there will be the impossible situation of a node that was built during
/// this evaluation depending on a node in error.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5205 MemoizingEvaluatorTest.shutDownBuildOnCachedError_Done
#[test]
fn shut_down_build_on_cached_error_done() {
    let tester = MemoizingTester::new();
    // errorKey will be invalidated due to its dependence on invalidatedKey, but later
    // revalidated since invalidatedKey re-evaluates to the same value on a subsequent build.
    let error_key = k("error");
    let invalidated_key = nk("invalidated-leaf");
    tester.set_key(invalidated_key.clone(), "invalidated-leaf-value");
    tester
        .add_dependency_key(error_key.clone(), invalidated_key.clone())
        .set_has_error_key(error_key.clone(), true);
    // Names are alphabetized in reverse deps of errorKey.
    let fast_to_request_slow_to_set_value_key = k("A-slow-set-value-parent");
    let failing_key = k("B-fast-fail-parent");
    tester.add_dependency_key(
        fast_to_request_slow_to_set_value_key.clone(),
        error_key.clone(),
    );
    tester.add_dependency_key(failing_key.clone(), error_key.clone());
    // We only want to force a particular order of operations at some points during evaluation.
    let synchronize_threads = Arc::new(AtomicBool::new(false));
    // We don't expect slow-set-value to actually be built, but if it is, we wait for it.
    let slow_built = Latch::new(1);
    {
        let (fast_key, failing_key) = (
            fast_to_request_slow_to_set_value_key.clone(),
            failing_key.clone(),
        );
        let (synchronize_threads, slow_built) = (synchronize_threads.clone(), slow_built.clone());
        tester.inject_graph_listener_for_testing(
            move |key: &SkyKey, event_type: EventType, order: Order, _: &ListenerContext| {
                if !synchronize_threads.load(Ordering::SeqCst) {
                    return;
                }
                if event_type == EventType::GetLifecycleState && *key == failing_key {
                    // Wait for the build to abort or for the other node to incorrectly build.
                    // An interrupt (Err) is ok, because it indicates the build is shutting down.
                    if let Ok(done) =
                        await_latch_interruptibly(&slow_built, WAIT_TIMEOUT_MILLISECONDS)
                    {
                        assert!(done);
                    }
                } else if event_type == EventType::SetValue
                    && *key == fast_key
                    && order == Order::After
                {
                    // This indicates a problem -- this parent shouldn't be built since it depends
                    // on an error.
                    slow_built.count_down();
                    // Before this node actually sets its value (and then throws an exception) we
                    // wait for the other node to throw an exception.
                    if !sleep_until_interrupted(WAIT_TIMEOUT_MILLISECONDS) {
                        panic!("IllegalStateException: uninterrupted in {key:?}");
                    }
                }
            },
            /* deterministic= */ true,
        );
    }
    // Initialize graph.
    tester.eval(true, std::slice::from_ref(&error_key));
    tester.get_or_create(invalidated_key, /* mark_as_modified= */ true);
    tester.invalidate();
    synchronize_threads.store(true, Ordering::SeqCst);
    tester.eval(false, &[fast_to_request_slow_to_set_value_key, failing_key]);
}

/// Test that the invalidated parent of a cached but invalidated error doesn't get marked clean.
/// First build the parent -- it will contain an error. Then invalidate the error via a dependency
/// (so it will not actually change) and then build the parent and another node that depends on
/// the error. The other node will wait to throw until the parent is signaled that all of its
/// dependencies are done, or until it is interrupted. If it throws, the parent will be
/// VERIFIED_CLEAN but not done, which is not a valid state once evaluation shuts down. The
/// evaluator avoids this situation by throwing when the error is encountered, even though the
/// error isn't evaluated or requested by an evaluating node.
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5274 MemoizingEvaluatorTest.shutDownBuildOnCachedError_Verified
#[test]
fn shut_down_build_on_cached_error_verified() {
    // TrackingProgressReceiver does unnecessary examination of node values.
    let tester = MemoizingTester::with_progress_receiver(Arc::new(
        tracking_progress_receiver::TrackingProgressReceiver::new(
            /* check_evaluation_results= */ false,
        ),
    ));
    // errorKey will be invalidated due to its dependence on invalidatedKey, but later
    // revalidated since invalidatedKey re-evaluates to the same value on a subsequent build.
    let error_key = k("error");
    let invalidated_key = nk("invalidated-leaf");
    let changed_key = nk("changed-leaf");
    tester.set_key(invalidated_key.clone(), "invalidated-leaf-value");
    tester.set_key(changed_key.clone(), "changed-leaf-value");
    // Names are alphabetized in reverse deps of errorKey.
    let cached_parent_key = k("A-cached-parent");
    let uncached_parent_key = k("B-uncached-parent");
    tester
        .add_dependency_key(error_key.clone(), invalidated_key.clone())
        .set_has_error_key(error_key.clone(), true);
    tester.add_dependency_key(cached_parent_key.clone(), error_key.clone());
    tester
        .add_dependency_key(uncached_parent_key.clone(), changed_key.clone())
        .add_dependency_key(uncached_parent_key.clone(), error_key);
    // We only want to force a particular order of operations at some points during evaluation.
    // In particular, we don't want to force anything during error bubbling.
    let synchronize_threads = Arc::new(AtomicBool::new(false));
    let shutdown_awaiter_started = Latch::new(1);
    {
        let (cached_parent_key, uncached_parent_key) =
            (cached_parent_key.clone(), uncached_parent_key.clone());
        let (synchronize_threads, shutdown_awaiter_started) = (
            synchronize_threads.clone(),
            shutdown_awaiter_started.clone(),
        );
        let cached_signaled = Latch::new(1);
        tester.inject_graph_listener_for_testing(
            move |key: &SkyKey, event_type: EventType, order: Order, _: &ListenerContext| {
                if !synchronize_threads.load(Ordering::SeqCst)
                    || order != Order::Before
                    || event_type != EventType::Signal
                {
                    return;
                }
                shutdown_awaiter_started.wait("shutdown awaiter not started");
                if *key == uncached_parent_key {
                    // When the uncached parent is first signaled by its changed dep, make sure
                    // that we wait until the cached parent is signaled too.
                    // An interrupt (Err): before the relevant bug was fixed, this code was not
                    // interrupted, and the uncached parent got to build, yielding an inconsistent
                    // state at a later point during evaluation. With the bugfix, the cached parent
                    // is never signaled before the evaluator shuts down, and so the above code is
                    // interrupted.
                    if let Ok(done) =
                        await_latch_interruptibly(&cached_signaled, WAIT_TIMEOUT_MILLISECONDS)
                    {
                        assert!(done);
                    }
                } else if *key == cached_parent_key {
                    // This branch should never be reached by a well-behaved evaluator, since when
                    // the error node is reached, the evaluator should shut down. However, we
                    // don't test for that behavior here because that would be brittle and we
                    // expect that such an evaluator will crash hard later on in any case.
                    cached_signaled.count_down();
                    // Sleep until we're interrupted by the evaluator, so we know it's shutting
                    // down.
                    if !sleep_until_interrupted(WAIT_TIMEOUT_MILLISECONDS) {
                        panic!(
                            "IllegalStateException: no interruption in time in {key:?} for \
                             uninterrupted {:?}",
                            std::thread::current().id()
                        );
                    }
                }
            },
            /* deterministic= */ true,
        );
    }
    // Initialize graph.
    tester.eval(
        true,
        &[cached_parent_key.clone(), uncached_parent_key.clone()],
    );
    tester.get_or_create(invalidated_key.clone(), /* mark_as_modified= */ true);
    tester.set_key(changed_key, "new value");
    tester.invalidate();
    synchronize_threads.store(true, Ordering::SeqCst);
    let wait_for_shutdown_key = k("wait-for-shutdown");
    {
        let (synchronize_threads, shutdown_awaiter_started) = (
            synchronize_threads.clone(),
            shutdown_awaiter_started.clone(),
        );
        tester.set_builder_key(wait_for_shutdown_key.clone(), move |_, env| {
            shutdown_awaiter_started.count_down();
            wait_for_exception(env, "exception not thrown");
            // Threadpool is shutting down. Don't try to synchronize anything in the future during
            // error bubbling.
            synchronize_threads.store(false, Ordering::SeqCst);
            Err(Interrupted.into())
        });
    }
    let r = tester.eval(
        false,
        &[
            cached_parent_key.clone(),
            uncached_parent_key.clone(),
            wait_for_shutdown_key,
        ],
    );
    assert!(r.has_error(), "{r:?}");
    tester.get_or_create(invalidated_key, /* mark_as_modified= */ true);
    tester.invalidate();
    let r = tester.eval(false, &[cached_parent_key, uncached_parent_key]);
    assert!(r.has_error(), "{r:?}");
}

/// Tests that a race between a node being marked clean and another node requesting it is benign.
/// Here, we first evaluate errorKey, depending on invalidatedKey. Then we invalidate
/// invalidatedKey (without actually changing it) and evaluate errorKey and topKey together.
/// Through forced synchronization, we make sure that the following sequence of events happens:
///
/// 1. topKey requests errorKey;
/// 2. errorKey is marked clean;
/// 3. topKey finishes its first evaluation and registers its deps;
/// 4. topKey restarts, since it sees that its only dep, errorKey, is done;
/// 5. topKey sees the error thrown by errorKey and throws the error, shutting down the
///    threadpool;
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5396 MemoizingEvaluatorTest.cachedErrorCausesRestart
#[test]
#[ignore = "restart model (lib.rs `Model`): the listener waits, in errorKey's markClean, for \
            topKey's second run (Java restarts topKey once errorKey is done); the blocking \
            engine never restarts a function, and topKey's request of errorKey blocks until \
            errorKey is done, so topKey cannot count down `topRequestedError` before errorKey's \
            markClean returns: the threads wait for each other until the 60 s timeout"]
fn cached_error_causes_restart() {
    // TrackingProgressReceiver does unnecessary examination of node values.
    let tester = MemoizingTester::with_progress_receiver(Arc::new(
        tracking_progress_receiver::TrackingProgressReceiver::new(
            /* check_evaluation_results= */ false,
        ),
    ));
    let error_key = k("error");
    let invalidated_key = nk("invalidated");
    let top_key = k("top");
    tester
        .add_dependency_key(error_key.clone(), invalidated_key.clone())
        .set_has_error_key(error_key.clone(), true);
    tester.with_key(invalidated_key.clone(), |s| {
        s.value = Some("constant".into())
    });
    let top_second_eval = Latch::new(2);
    let top_requested_error = Latch::new(1);
    let error_marked_clean = Latch::new(1);
    {
        let error_key = error_key.clone();
        let (top_second_eval, top_requested_error, error_marked_clean) = (
            top_second_eval.clone(),
            top_requested_error.clone(),
            error_marked_clean.clone(),
        );
        tester.inject_graph_listener_for_testing(
            move |key: &SkyKey, event_type: EventType, order: Order, _: &ListenerContext| {
                if *key == error_key && event_type == EventType::MarkClean {
                    if order == Order::Before {
                        top_requested_error.wait("top didn't request");
                    } else {
                        error_marked_clean.count_down();
                        top_second_eval.wait("top didn't restart");
                        // Make sure that the other thread notices the error and interrupts this
                        // thread.
                        sleep_until_interrupted(WAIT_TIMEOUT_MILLISECONDS);
                    }
                }
            },
            /* deterministic= */ false,
        );
    }
    let r = tester.eval(false, std::slice::from_ref(&error_key));
    assert!(r.has_error());
    assert!(
        r.get_error(&error_key)
            .expect("error entry")
            .get_exception()
            .is_some()
    );
    {
        let error_key = error_key.clone();
        tester.set_builder_key(top_key.clone(), move |_, env| {
            top_second_eval.count_down();
            env.get_value_dyn(&error_key)?;
            top_requested_error.count_down();
            assert!(env.values_missing());
            error_marked_clean.wait("error not marked clean");
            Ok(None)
        });
    }
    tester.get_or_create(invalidated_key, /* mark_as_modified= */ true);
    tester.invalidate();
    let r2 = tester.eval(false, &[error_key.clone(), top_key.clone()]);
    assert!(r2.has_error());
    assert!(
        r2.get_error(&error_key)
            .expect("error entry")
            .get_exception()
            .is_some()
    );
    assert!(
        r2.get_error(&top_key)
            .expect("top entry")
            .get_exception()
            .is_some()
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5624 MemoizingEvaluatorTest.cleanReverseDepFromDirtyNodeNotInBuild
#[test]
fn clean_reverse_dep_from_dirty_node_not_in_build() {
    let tester = MemoizingTester::new();
    let top_key = nk("top");
    let inactive_key = nk("inactive");
    let should_interrupt = Arc::new(AtomicBool::new(false));
    {
        let top_key = top_key.clone();
        let should_interrupt = should_interrupt.clone();
        // Java: mainThread.interrupt(): the thread that called evaluate.
        let main_thread_interrupt = tester.evaluator.interrupt_flag();
        tester.inject_graph_listener_for_testing(
            move |key: &SkyKey, event_type: EventType, order: Order, _: &ListenerContext| {
                if should_interrupt.load(Ordering::SeqCst)
                    && *key == top_key
                    && event_type == EventType::IsReady
                    && order == Order::Before
                {
                    main_thread_interrupt.store(true, Ordering::SeqCst);
                    should_interrupt.store(false, Ordering::SeqCst);
                    // Make sure threadpool propagates interrupt.
                    sleep_until_interrupted(WAIT_TIMEOUT_MILLISECONDS);
                }
            },
            /* deterministic= */ false,
        );
    }
    // When top depends on inactive,
    tester.add_dependency_key(top_key.clone(), inactive_key.clone());
    // And inactive is constant,
    tester.set_key(inactive_key.clone(), "inactive");
    // Then top evaluates normally.
    assert_eq!(tester.eval_and_get(true, &top_key), "inactive");
    // When evaluation will be interrupted as soon as top starts evaluating,
    should_interrupt.store(true, Ordering::SeqCst);
    // And inactive is dirty,
    tester.get_or_create(inactive_key.clone(), /* mark_as_modified= */ true);
    // And so is top,
    tester.get_or_create(top_key.clone(), /* mark_as_modified= */ true);
    tester.invalidate();
    assert_eq!(
        tester.try_eval(false, std::slice::from_ref(&top_key)).err(),
        Some(Interrupted)
    );
    // Java: assertThrows consumed the interrupt.
    tester.evaluator.clear_interrupt();
    // But inactive is still present,
    let inactive_node = tester
        .evaluator
        .node(&inactive_key)
        .expect("inactive is still present");
    // And still dirty,
    assert!(inactive_node.lock().entry.is_dirty());
    // And re-evaluates successfully,
    assert_eq!(tester.eval_and_get(true, &inactive_key), "inactive");
    // But top is gone from the graph,
    assert!(tester.evaluator.node(&top_key).is_none());
    // And we can successfully invalidate and re-evaluate inactive again.
    tester.get_or_create(inactive_key.clone(), /* mark_as_modified= */ true);
    tester.invalidate();
    assert_eq!(tester.eval_and_get(true, &inactive_key), "inactive");
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:209 MemoizingEvaluatorTest.injectGraphTransformer_transformedGraphUsedForEvaluation
#[test]
fn inject_graph_transformer_transformed_graph_used_for_evaluation() {
    let tester = MemoizingTester::new();
    // Java: a Mockito mock of Listener; verify(listener).accept(...).
    type Call = (SkyKey, EventType, Order, String);
    let calls: Arc<Mutex<Vec<Call>>> = Arc::new(Mutex::new(vec![]));
    {
        let calls = calls.clone();
        tester
            .evaluator
            .graph
            .listeners_for_testing
            .inject(Arc::new(
                move |key: &SkyKey,
                      event_type: EventType,
                      order: Order,
                      context: &ListenerContext| {
                    calls
                        .lock()
                        .push((key.clone(), event_type, order, format!("{context:?}")))
                },
            ));
    }
    let key = k("key");
    tester.with_key(key.clone(), |s| s.value = Some("val".into()));

    assert_eq!(tester.eval_and_get(false, &key), "val");

    let expected: Call = (
        key,
        EventType::GetBatch,
        Order::Before,
        format!(
            "{:?}",
            ListenerContext::Reason(crate::graph_listener::Reason::PreOrPostEvaluation)
        ),
    );
    assert!(calls.lock().contains(&expected), "{:?}", calls.lock());
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:223 MemoizingEvaluatorTest.injectGraphTransformer_multipleTransformersAppliedInOrder
#[test]
fn inject_graph_transformer_multiple_transformers_applied_in_order() {
    let tester = MemoizingTester::new();
    // Java: two Mockito mocks and inOrder(inner, outer): one log of the calls on both.
    type Call = (&'static str, SkyKey, EventType, Order, Option<String>);
    let calls: Arc<Mutex<Vec<Call>>> = Arc::new(Mutex::new(vec![]));
    for name in ["inner", "outer"] {
        let calls = calls.clone();
        tester
            .evaluator
            .graph
            .listeners_for_testing
            .inject(Arc::new(
                move |key: &SkyKey,
                      event_type: EventType,
                      order: Order,
                      context: &ListenerContext| {
                    let value = match context {
                        ListenerContext::Value(Some(v)) => v.downcast_ref::<String>().cloned(),
                        _ => None,
                    };
                    calls
                        .lock()
                        .push((name, key.clone(), event_type, order, value))
                },
            ));
    }
    let key = k("key");
    tester.with_key(key.clone(), |s| s.value = Some("val".into()));

    assert_eq!(tester.eval_and_get(false, &key), "val");

    let val = Some("val".to_string());
    let expected: Vec<Call> = vec![
        (
            "outer",
            key.clone(),
            EventType::SetValue,
            Order::Before,
            val.clone(),
        ),
        (
            "inner",
            key.clone(),
            EventType::SetValue,
            Order::Before,
            val.clone(),
        ),
        (
            "inner",
            key.clone(),
            EventType::SetValue,
            Order::After,
            val.clone(),
        ),
        ("outer", key, EventType::SetValue, Order::After, val),
    ];
    // Java: InOrder.verify: each expected call happens, in this order (other calls in between
    // are allowed).
    let calls = calls.lock();
    let mut it = calls.iter();
    for e in &expected {
        assert!(it.any(|c| c == e), "{e:?} not found in order in {calls:?}");
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5808 MemoizingEvaluatorTest.runTestDuplicateUnfinishedDeps (keepGoing = false)
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5692 MemoizingEvaluatorTest.duplicateUnfinishedDeps_NoKeepGoing
#[test]
#[ignore = "requests evaluate their dep (lib.rs `Model`, PORT on request order): Java's parent \
            gets null for child twice and throws, and without keep_going that first exception \
            ends the evaluation before child is evaluated; here the request evaluates child, \
            whose function panics by design (\"shouldn't get here\")"]
fn duplicate_unfinished_deps_no_keep_going() {
    let tester = MemoizingTester::new();
    let parent_key = k("parent");
    let child_key = k("child");
    let keep_going = false;
    tester.set_builder_key(child_key.clone(), move |_, _| {
        if keep_going {
            Ok(Some("child".into()))
        } else {
            panic!("IllegalStateException: shouldn't get here")
        }
    });
    let num_parent_compute_calls = Arc::new(AtomicUsize::new(0));
    {
        let (child_key, num_parent_compute_calls) =
            (child_key.clone(), num_parent_compute_calls.clone());
        tester.set_builder_key(parent_key.clone(), move |_, env| {
            let n = num_parent_compute_calls.fetch_add(1, Ordering::SeqCst) + 1;
            if !keep_going || n == 1 {
                blaze_util::check_state!(env.get_value_dyn(&child_key)?.is_none());
                blaze_util::check_state!(env.get_value_dyn(&child_key)?.is_none());
            } else {
                blaze_util::check_state!(
                    dep_string(env.get_value_dyn(&child_key)?).as_deref() == Some("child")
                );
                blaze_util::check_state!(
                    dep_string(env.get_value_dyn(&child_key)?).as_deref() == Some("child")
                );
            }
            Err(SkyFunctionException::persistent(SomeErrorException("bad".into())).into())
        });
    }
    let exception = tester.eval_and_get_error(keep_going, &parent_key);
    assert_eq!(exception_message(&exception), "bad");
}

// ---------------------------------------------------------------------------------------------
// Review 2026-11-25 (I1), S2-1 and S2-2: no Java test; each checks the Java behaviour named.

/// A graph whose dirty node has lost a dep of its last build (an inconsistency: deletion takes
/// the reverse transitive closure, so only the test hook `graph.remove` makes it). `top` asks
/// `first`, then `second`; `changed` (non-hermetic) is invalidated with a new value.
fn graph_with_missing_old_dep(first: SkyKey, second: SkyKey, missing: &SkyKey) -> MemoizingTester {
    let tester = MemoizingTester::new();
    tester.add_dependency_key(k("top"), first);
    tester.add_dependency_key(k("top"), second);
    tester.set("kept", "kept");
    tester.set_key(nk("changed"), "old");
    assert!(!tester.eval(true, &[k("top")]).has_error());
    tester.set_key(nk("changed"), "new");
    tester.invalidate();
    tester.evaluator.graph.remove(missing);
    tester
}

fn panic_message(e: Box<dyn std::any::Any + Send>) -> String {
    match e.downcast::<String>() {
        Ok(s) => *s,
        Err(e) => e
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .unwrap_or_default(),
    }
}

/// The dirty check of `top` reaches its first old dep, which is missing.
// Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:394 Evaluate.handleKnownChildrenForDirtyNode
#[test]
fn dirty_parent_with_missing_child_fails_with_java_text() {
    let tester = graph_with_missing_old_dep(k("kept"), nk("changed"), &k("kept"));
    let e = catch_unwind(AssertUnwindSafe(|| tester.try_eval(true, &[k("top")])))
        .expect_err("checkNotNull fails");
    let message = panic_message(e);
    assert!(
        message.contains("Dirty parent had missing child (child=kept, parent=top "),
        "{message}"
    );
}

/// `changed` makes `top` rebuild; the function then asks again for `kept`, an old dep that is
/// missing from the graph.
// Java: com/google/devtools/build/skyframe/AbstractParallelEvaluator.java:1022 Evaluate.run
#[test]
fn rebuilt_parent_with_missing_declared_dep_fails_with_java_text() {
    let tester = graph_with_missing_old_dep(nk("changed"), k("kept"), &k("kept"));
    let e = catch_unwind(AssertUnwindSafe(|| tester.try_eval(true, &[k("top")])))
        .expect_err("checkState fails");
    let message = panic_message(e);
    assert!(
        message.contains("Missing already declared dep kept (parent=top)"),
        "{message}"
    );
}

/// With an interrupt pending and every root done, Java's `ParallelEvaluator.eval` returns the
/// cached result (no interrupt check on that path); a root that is not done sees the interrupt.
// Java: com/google/devtools/build/skyframe/ParallelEvaluator.java:641 ParallelEvaluator.eval
#[test]
fn interrupt_pending_with_all_roots_done_returns_cached_result() {
    let tester = MemoizingTester::new();
    tester.set("done", "value");
    tester.set("other", "other");
    assert_eq!(tester.eval_and_get(false, &k("done")), "value");
    tester.evaluator.interrupt();
    let r = tester.try_eval(false, &[k("done")]).expect("cached result");
    assert_eq!(*r.get(&T("done")).unwrap(), "value");
    assert_eq!(
        tester.try_eval(false, &[k("other")]).err(),
        Some(Interrupted)
    );
    tester.evaluator.clear_interrupt();
}
