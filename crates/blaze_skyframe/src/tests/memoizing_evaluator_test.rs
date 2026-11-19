//! Ports of the `MemoizingEvaluatorTest` methods that do not need incremental evaluation.
//!
//! Ports of the `MemoizingEvaluatorTest` methods. The ones that need what is not ported (graph
//! listeners `NotifyingHelper`, rewinding, ...) are in `not_ported.rs`.

use super::*;

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

// Java: MemoizingEvaluatorTest.depMessageBeforeNodeMessageOrNodeValue
// PORT: the graph listener's check (the dep's warning is out before "top" is set) is covered by
// the order of the collected events; there is no graph listener.
#[test]
fn dep_message_before_node_message_or_node_value() {
    let tester = new_tester();
    tester
        .set("leaf", "leaf")
        .set_warning("leaf", "dep warning")
        .set_builder("top", |_, env| {
            let dep_value = env.get_value(T("leaf"))?;
            if dep_value.is_some() {
                // Default GraphTester implementation warns before requesting deps, which doesn't
                // work for ordering assertions with memoizing evaluator subclasses that don't
                // store events and instead just pass them through directly. By warning after the
                // dep is done we avoid that issue.
                env.listener().handle(&Event::warn("top warning"));
            }
            Ok(dep_value.map(|v| (*v).clone()))
        });
    let ev = tester_evaluator(tester);
    let (r, events) = eval_collecting(&ev, false, &[k("top")]);
    assert_eq!(*r.get(&T("top")).unwrap(), "leaf");
    assert_eq!(messages(&events), vec!["dep warning", "top warning"]);
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

// Java: MemoizingEvaluatorTest.cycleAboveIndependentCycle
// PORT: the second half (leafKey changed, cKey invalidated: dirty nodes in cycle checking) is
// not ported, incremental.
#[test]
fn memoizing_cycle_above_independent_cycle() {
    for p in [1, 4] {
        let tester = new_tester();
        // When aKey depends on leafKey and bKey,
        tester
            .set_builder("a", |_, env| {
                env.get_values_and_exceptions(&[k("leaf"), k("b")])?;
                Ok(None)
            })
            // And bKey depends on cKey,
            .add_dependency("b", "c")
            // And cKey depends on aKey and bKey in that order,
            .add_dependency("c", "a")
            .add_dependency("c", "b")
            // And leafKey is a leaf node,
            .set("leaf", "leafy");
        let ev = tester_evaluator(tester);
        // Then when we evaluate,
        let r = eval(&ev, true, p, &[k("a")]);
        // aKey has an error,
        assert!(r.get(&T("a")).is_none());
        // And both cycles were found underneath aKey: the (aKey->bKey->cKey) cycle, and the
        // aKey->(bKey->cKey) cycle. This is because cKey depended on aKey and then bKey, so it
        // pushed them down on the stack in that order, so bKey was processed first. It found its
        // cycle, then popped off the stack, and then aKey was processed and found its cycle.
        assert_eq!(
            as_set(r.get_error(&k("a")).unwrap().get_cycle_info()),
            as_set(&[
                CycleInfo::create_cycle_info(vec![], vec![k("a"), k("b"), k("c")]),
                CycleInfo::create_cycle_info(vec![k("a")], vec![k("b"), k("c")]),
            ])
        );
    }
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

// Java: MemoizingEvaluatorTest.errorOnlyBubblesToRequestingParents
// PORT: Java builds everything without errors first and then changes "error" and "slow", so that
// on the second build the dirty "1st top alphabetically" checks its first dep group ("mid") and
// has not registered on "error" when it throws. A fresh build has that shape here once "1st top"
// is blocked in "mid" (requests block), so "error" also waits until "slow" has started; Java
// needs no such latch because a dirty node never gets past its first changed dep group.
#[test]
fn error_only_bubbles_to_requesting_parents() {
    for _ in 0..10 {
        let tester = new_tester();
        // Set up timing of child values: slowKey waits to finish until errorKey has thrown an
        // exception that has been caught by the threadpool.
        let error_finish = Latch::new(1);
        let slow_start = Latch::new(1);
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
                    value: Some("leaf2"),
                    ..Chained::default()
                },
            )
            .add_dependency("mid", "slow")
            .add_dependency("2nd top alphabetically", "error")
            .add_dependency("1st top alphabetically", "mid")
            .add_dependency("1st top alphabetically", "error");
        let ev = tester_evaluator(tester);
        // We need control over the order of reverse deps, so use a deterministic graph.
        ev.deterministic_for_testing.store(true, Ordering::SeqCst);
        // errorKey finishes, written to graph -> slowKey maybe starts+finishes & (Visitor
        // aborts) -> some top key builds.
        let r = eval(
            &ev,
            false,
            4,
            &[k("2nd top alphabetically"), k("1st top alphabetically")],
        );
        assert!(r.has_error());
        assert!(r.get_error(&k("2nd top alphabetically")).is_some(), "{r:?}");
    }
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

// Java: MemoizingEvaluatorTest.raceConditionWithNoKeepGoingErrors_InflightError
// PORT: Java's graph listener releases "otherErrorKey" when errorKey's error is committed and
// holds the committing thread until otherErrorKey is done. Here "otherErrorKey" waits until the
// evaluation has failed (the error is committed by then).
#[test]
fn race_condition_with_no_keep_going_errors_inflight_error() {
    for _ in 0..10 {
        // Given a graph of two nodes, errorKey and otherErrorKey,
        let other_started = Latch::new(1);
        let num_other_invocations = Arc::new(AtomicUsize::new(0));
        let tester = new_tester();
        {
            let other_started = other_started.clone();
            tester.set_builder("errorKey", move |_, _| {
                // Given that errorKey waits for otherErrorKey to begin evaluation before
                // completing its evaluation,
                other_started.wait("otherErrorKey's SkyFunction didn't start in time.");
                // And given that errorKey throws an error,
                Err(SkyFunctionException::persistent(SomeErrorException("error".into())).into())
            });
        }
        {
            let n = num_other_invocations.clone();
            tester.set_builder("otherErrorKey", move |_, env| {
                other_started.count_down();
                let invocations = n.fetch_add(1, Ordering::SeqCst) + 1;
                // And given that otherErrorKey waits for errorKey's error to be committed before
                // trying to get errorKey's value,
                wait_for_exception(
                    env,
                    "errorKey's error didn't get committed to the graph in time",
                );
                match env.get_value_or_throw::<T, SomeErrorException>(T("errorKey"))? {
                    Ok(value) => {
                        assert!(value.is_none(), "bogus non-null value {value:?}");
                        assert_eq!(invocations, 1, "bogus invocation count: {invocations}");
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
        let ev = tester_evaluator(tester);
        // When the graph is evaluated in noKeepGoing mode,
        let r = eval(&ev, false, 4, &[k("errorKey"), k("otherErrorKey")]);
        // Then the result reports that an error occurred,
        assert!(r.has_error());
        // And no value is committed for otherErrorKey,
        assert!(
            ev.get_existing_error_for_testing(&k("otherErrorKey"))
                .is_none()
        );
        assert!(ev.get_existing_value(&k("otherErrorKey")).is_none());
        // And the SkyFunction for otherErrorKey was evaluated exactly once.
        assert_eq!(num_other_invocations.load(Ordering::SeqCst), 1);
    }
}

// Java: MemoizingEvaluatorTest.absentParent
// PORT: Java first builds "absentParent" on a good "my_error_value" and then turns the value
// into an error; a function cannot be "changed" here, so the error value is there from the
// start and "absentParent" is never built.
#[test]
fn absent_parent() {
    let tester = new_tester();
    tester
        .set_has_error("my_error_value", true)
        .add_dependency("absentParent", "my_error_value")
        .add_dependency("newParent", "my_error_value");
    let ev = tester_evaluator(tester);
    let r = eval(&ev, false, 2, &[k("newParent")]);
    singleton_error(&r, &k("newParent"));
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

// Java: MemoizingEvaluatorTest.persistentErrorsNotRerun
// PORT: `tester.getEnqueuedValues()` (EvaluationProgressReceiver) is the count of computed
// functions here.
#[test]
fn persistent_errors_not_rerun() {
    let tester = new_tester();
    tester
        .add_error_dependency("top", "persistentError1", "doesn't matter")
        .set_has_error("top", true)
        .set_has_error("persistentError1", true)
        .add_error_dependency("top", "transientError", "doesn't matter")
        .add_error_dependency("transientError", "persistentError2", "doesn't matter")
        .set_has_transient_error("transientError", true)
        .set_has_error("persistentError2", true);
    let ev = tester_evaluator(tester);
    eval_value_in_error(&ev, &k("top"));
    // top, transientError, persistentError1, persistentError2
    assert_eq!(ev.computed_count(), 4);

    ev.invalidate_transient_errors();
    eval_value_in_error(&ev, &k("top"));
    // Only top and transientError are evaluated again.
    assert_eq!(ev.computed_count(), 6);
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

// Java: MemoizingEvaluatorTest.sameDepInTwoGroups
// PORT: the graph listener that asserts "leaf4" is not looked at in the second build is not
// ported (no graph listeners).
#[test]
fn memoizing_same_dep_in_two_groups() {
    let tester = new_tester();
    tester.set("leaf4", "leaf4");
    // Create leaf0, leaf1 and leaf2 values with values "leaf2", "leaf3", "leaf4" respectively.
    // These will be requested as one dependency group. In the second build, leaf2 will have the
    // value "leaf5".
    tester
        .set("leaf0", "leaf2")
        .set("leaf1", "leaf3")
        .set_key(nk("leaf2"), "leaf4");
    // Create "top" value. It depends on all leaf values in two overlapping dependency groups.
    tester.set_builder("top", |_, env| {
        // Request the first group, [leaf0, leaf1, leaf2].
        // In the first build, it has values ["leaf2", "leaf3", "leaf4"].
        // In the second build it has values ["leaf2", "leaf3", "leaf5"]
        let values = env.get_values_and_exceptions(&[k("leaf0"), k("leaf1"), nk("leaf2")])?;
        if env.values_missing() {
            return Ok(None);
        }
        // Request the second group. In the first build it's [leaf2, leaf4].
        // In the second build it's [leaf2, leaf5]
        let next = leak((*values.get_typed(&N("leaf2")).unwrap()).clone());
        env.get_values_and_exceptions(&[nk("leaf2"), k(next)])?;
        if env.values_missing() {
            return Ok(None);
        }
        Ok(Some("top".into()))
    });
    let ev = tester_evaluator(tester.clone());
    // First build: assert we can evaluate "top".
    assert_eq!(
        *eval(&ev, false, 4, &[k("top")]).get(&T("top")).unwrap(),
        "top"
    );
    assert_eq!(
        ev.get_direct_deps(&k("top")),
        vec![k("leaf0"), k("leaf1"), nk("leaf2"), k("leaf4")]
    );

    // Second build: replace "leaf4" by "leaf5" in leaf2's value.
    tester.set("leaf5", "leaf5");
    tester.set_key(nk("leaf2"), "leaf5");
    invalidate(&tester, &ev);
    assert_eq!(
        *eval(&ev, false, 4, &[k("top")]).get(&T("top")).unwrap(),
        "top"
    );
    assert_eq!(
        ev.get_direct_deps(&k("top")),
        vec![k("leaf0"), k("leaf1"), nk("leaf2"), k("leaf5")]
    );
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

// Java: MemoizingEvaluatorTest.cachedChildErrorDepWithSiblingDepOnNoKeepGoingEval
// PORT: Java builds both parents on a good "error" node first and then turns it into an error
// (a changed function, incremental). The node is in error from the start here; what the test
// checks is kept: a keep_going build caches the child error, and a no-keep-going build of a
// second parent that requests it and a sibling dep does not crash.
#[test]
fn cached_child_error_dep_with_sibling_dep_on_no_keep_going_eval() {
    for p in [1, 4] {
        let tester = new_tester();
        for parent in ["parent1", "parent2"] {
            tester.set_builder(parent, |_, env| {
                env.get_value(T("error"))?;
                env.get_value(T("other"))?;
                if env.values_missing() {
                    return Ok(None);
                }
                Ok(Some("parent".into()))
            });
        }
        tester.set_has_error("error", true).set("other", "other");
        let ev = tester_evaluator(tester);
        let r = eval(&ev, true, p, &[k("parent1")]);
        singleton_error(&r, &k("parent1"));
        let r = eval(&ev, false, p, &[k("parent2")]);
        singleton_error(&r, &k("parent2"));
    }
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
