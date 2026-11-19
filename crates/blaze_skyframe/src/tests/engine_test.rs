//! Tests of the blocking engine that have no Java test: the scenarios of
//! docs/reviews/2026-11-18-skyframe.md, with the results `ParallelEvaluator` gives for them
//! (derived by hand from `Evaluate.run`, `bubbleErrorUp` and `constructResult`; each test says
//! how).

use super::*;
use blaze_events::Postable;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn catastrophic(msg: &str) -> ComputeError {
    SkyFunctionException::catastrophic(SomeErrorException(msg.into()), Transience::Persistent)
        .into()
}

fn persistent(msg: impl Into<String>) -> ComputeError {
    SkyFunctionException::persistent(SomeErrorException(msg.into())).into()
}

/// The keys done in the graph (committed), without `ErrorTransienceValue.KEY`.
///
/// Nodes still in flight when an evaluation ends stay in the graph, not done, until the next
/// evaluation deletes them (AbstractInMemoryMemoizingEvaluator.java:134); only done nodes are
/// what an evaluation committed.
fn graph_keys(ev: &Evaluator) -> HashSet<SkyKey> {
    ev.get_all_keys_for_testing()
        .into_iter()
        .filter(|key| *key != *ERROR_TRANSIENCE_KEY && ev.committed_entry(key).is_some())
        .collect()
}

// Review S1-1, scenario A. Java: "cat" commits its catastrophic error and throws the
// SchedulerException; bubbleErrorUp re-runs "top", which rethrows a catastrophic exception:
// bubbleErrorInfo = {top: fromChildErrors(top, {fromException(top's exception)})}, so
// errorMap = {top} and the catastrophe is set (ParallelEvaluator.java:433-447, 575).
#[test]
fn keep_going_catastrophic_rethrow_reports_the_bubbling_root() {
    for p in [1, 4] {
        let tester = new_tester();
        tester
            .set_builder("cat", |_, _| Err(catastrophic("bad")))
            .set_builder("top", |_, env| {
                match env.get_value_or_throw::<T, SomeErrorException>(T("cat"))? {
                    Ok(_) => Ok(None),
                    Err(e) => Err(catastrophic(&format!("top got: {e}"))),
                }
            });
        let ev = tester_evaluator(tester);
        let r = eval(&ev, true, p, &[k("top")]);
        assert_eq!(
            exception_message(singleton_error(&r, &k("top"))),
            "top got: bad"
        );
        assert!(r.get_error(&k("top")).unwrap().is_catastrophic());
        assert_eq!(r.get_catastrophe().unwrap().to_string(), "top got: bad");
        // Java commits nothing above the catastrophic node.
        assert_eq!(graph_keys(&ev), HashSet::from([k("cat")]));
    }
}

// Review S1-1, scenario B. Java: bubbleErrorUp re-runs "mid" (throws a non-catastrophic
// exception: error = fromException(mid's exception)), then "top", which requests [mid, other]:
// "mid" is in bubbleErrorInfo, "other" never started (NULL_MARKER), so "top" returns null and
// gets fromChildErrors(top, {mid's thrown error}). maybeEnsureCatastrophe takes that exception.
#[test]
fn keep_going_catastrophe_wrapped_below_a_batch_reports_the_root() {
    for p in [1, 4] {
        let tester = new_tester();
        tester
            .set_builder("cat", |_, _| Err(catastrophic("bad")))
            .set_builder("mid", |_, env| {
                match env.get_value_or_throw::<T, SomeErrorException>(T("cat"))? {
                    Ok(_) => Ok(None),
                    Err(e) => Err(persistent(format!("mid got: {e}"))),
                }
            })
            .set_builder("other", |_, env| {
                // Runs (if at all) until the evaluation fails.
                loop {
                    env.check_interrupted()?;
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
            .set_builder("top", |_, env| {
                let r = env.get_values_and_exceptions(&[k("mid"), k("other")])?;
                let _ = r.get(&k("mid"));
                let _ = r.get(&k("other"));
                assert!(env.values_missing());
                Ok(None)
            });
        let ev = tester_evaluator(tester);
        let r = eval(&ev, true, p, &[k("top")]);
        let error = singleton_error(&r, &k("top"));
        assert_eq!(exception_message(error), "mid got: bad");
        assert!(!error.is_catastrophic());
        assert_eq!(r.get_catastrophe().unwrap().to_string(), "mid got: bad");
        assert_eq!(graph_keys(&ev), HashSet::from([k("cat")]));
    }
}

// Review S1-1: nodes that did not commit leave the graph also when `evaluate` panics.
#[test]
fn crash_leaves_no_uncommitted_node_in_the_graph() {
    let tester = new_tester();
    tester
        .set("leaf", "leaf")
        .set_builder("crash", |_, _| panic!("function crashed"))
        .add_dependency("mid", "leaf")
        .add_dependency("mid", "crash")
        .add_dependency("top", "mid");
    let ev = tester_evaluator(tester);
    let crash = catch_unwind(AssertUnwindSafe(|| ev.evaluate(&[k("top")], &ctx(true, 2))));
    assert!(crash.is_err());
    assert_eq!(graph_keys(&ev), HashSet::from([k("leaf")]));
    // And the evaluator is usable again.
    let r = eval(&ev, true, 2, &[k("leaf")]);
    assert_eq!(*r.get(&T("leaf")).unwrap(), "leaf");
}

#[derive(Debug)]
struct FailedPost(#[allow(dead_code)] &'static str); // read through Debug
impl Postable for FailedPost {
    fn store_for_replay(&self) -> bool {
        true
    }
}

// Review S1-2. Java: neither parent sees the exception in normal evaluation (the dep is null,
// SkyFunctionEnvironment.java:649, 788). bubbleErrorUp re-runs one parent per level
// (`Iterables.getFirst(reverseDeps)`, ParallelEvaluator.java:342) and reports that run's events
// (440-445): one error event and one post, whatever the number of in-flight parents.
#[test]
fn no_keep_going_reports_events_and_posts_of_one_chain_only() {
    for i in 0..60 {
        let p = if i % 3 == 0 { 2 } else { 8 };
        let parents_started = Latch::new(2);
        let tester = new_tester();
        {
            let parents_started = parents_started.clone();
            tester.set_builder("err", move |_, _| {
                // Throw once both parents are in flight: one computes "err" under it, the other
                // waits for it or asks just after it committed.
                parents_started.wait("parents did not start");
                if i % 2 == 0 {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(persistent("err"))
            });
        }
        for name in ["p1", "p2"] {
            let parents_started = parents_started.clone();
            tester.set_builder(name, move |t, env| {
                parents_started.count_down();
                match env.get_value_or_throw::<T, SomeErrorException>(T("err"))? {
                    Ok(None) => Ok(None),
                    Ok(Some(_)) => panic!("err has no value"),
                    Err(e) => {
                        assert!(env.in_error_bubbling());
                        env.listener()
                            .handle(&Event::error(format!("{t:?}: dep failed")));
                        env.listener().post(Arc::new(FailedPost(t.0)));
                        Err(persistent(format!("{t:?} wraps {e}")))
                    }
                }
            });
        }
        let ev = tester_evaluator(tester);
        let events = Arc::new(StoredEventHandler::new());
        let r = ev
            .evaluate(
                &[k("p1"), k("p2")],
                &EvaluationContext {
                    event_handler: events.clone(),
                    ..ctx(false, p)
                },
            )
            .unwrap();
        assert_eq!(r.error_map().len(), 1, "{r:?}");
        let (root, error) = &r.error_map()[0];
        let root = format!("{root:?}");
        assert_eq!(exception_message(error), format!("{root} wraps err"));
        assert_eq!(messages(&events), vec![format!("{root}: dep failed")]);
        let posts = events.get_posts();
        assert_eq!(posts.len(), 1, "{posts:?}");
        assert_eq!(format!("{:?}", posts[0]), format!("FailedPost({root:?})"));
    }
}

// Review S2-1. Java (ParallelEvaluator.java:343-349): bubbling goes err -> B -> A -> B (A's only
// in-progress reverse dep is B), finds B in bubbleErrorInfo and returns null; constructResult
// then runs the cycle detector: {A: CycleInfo([], [A, B])}, no exception.
#[test]
fn no_keep_going_error_below_a_cycle_through_the_root_reports_the_cycle() {
    for p in [1, 4] {
        for _ in 0..10 {
            let tester = new_tester();
            tester
                .add_dependency("A", "B")
                .set_builder("B", |_, env| {
                    env.get_values_and_exceptions(&[k("err"), k("A")])?;
                    assert!(env.values_missing());
                    Ok(None)
                })
                .set_has_error("err", true);
            let ev = tester_evaluator(tester);
            let r = eval(&ev, false, p, &[k("A")]);
            let error = singleton_error(&r, &k("A"));
            assert!(error.get_exception().is_none(), "{r:?}");
            assert_eq!(
                error.get_cycle_info(),
                &[CycleInfo::create_cycle_info(vec![], vec![k("A"), k("B")])]
            );
        }
    }
}

// Review S2-2. Java: SkyframeLookupResult is the environment; getOrThrow reads
// maybeGetValueFromErrorOrDeps, every dep requested so far by any call
// (SkyFunctionEnvironment.java:463-476, 667-680).
#[test]
fn lookup_result_reads_deps_of_earlier_and_later_requests() {
    for keep_going in [false, true] {
        let tester = new_tester();
        tester
            .set("a", "a")
            .set("b", "b")
            .set("c", "c")
            .set_builder("top", |_, env| {
                let a = env.get_value(T("a"))?;
                let r = env.get_values_and_exceptions(&[k("b")])?;
                let c = env.get_value(T("c"))?;
                if env.values_missing() {
                    return Ok(None);
                }
                assert_eq!(r.get_typed(&T("a")), a);
                assert_eq!(*r.get_typed(&T("b")).unwrap(), "b");
                assert_eq!(r.get_typed(&T("c")), c);
                assert!(!env.values_missing());
                // Never requested: Java sends a bug report, returns null and does not mark
                // values missing.
                assert!(r.get(&k("never requested")).is_none());
                assert!(!env.values_missing());
                Ok(Some("top".into()))
            });
        let ev = tester_evaluator(tester);
        let r = eval(&ev, keep_going, 2, &[k("top")]);
        assert_eq!(*r.get(&T("top")).unwrap(), "top");
    }
}

// Review S2-3: the depth guard cannot start its thread. The claimed key must be finished: the
// evaluation crashes (Java: OutOfMemoryError "unable to create native thread"), it does not
// hang.
#[test]
fn spawn_failure_of_the_depth_guard_crashes_instead_of_hanging() {
    let ev = chain_evaluator();
    // One spawn for the worker of the root; the depth guard's spawn fails.
    *ev.spawns_before_failure_for_testing.lock() = Some(1);
    let context = EvaluationContext {
        max_inline_depth: 8,
        ..ctx(true, 2)
    };
    let crash = catch_unwind(AssertUnwindSafe(|| {
        ev.evaluate(&[SkyKey::new(Chain(100, 0))], &context)
    }))
    .expect_err("crash");
    let message = crash.downcast_ref::<String>().expect("message");
    assert!(
        message.contains("unable to create evaluator thread for Chain(100, 8)"),
        "{message}"
    );
    assert_eq!(ev.live_threads(), 0);
    // Nothing stays claimed: the next evaluation computes the same keys.
    *ev.spawns_before_failure_for_testing.lock() = None;
    let r = ev
        .evaluate(&[SkyKey::new(Chain(100, 0))], &context)
        .unwrap();
    assert_eq!(*r.get(&Chain(100, 0)).unwrap(), 100);
}

// Review S2-3: a waiter on the key whose thread could not be started wakes up.
#[test]
fn spawn_failure_wakes_waiters_of_the_claimed_key() {
    for _ in 0..20 {
        let ev = chain_evaluator();
        // Two root workers start; every depth-guard spawn fails.
        *ev.spawns_before_failure_for_testing.lock() = Some(2);
        let context = EvaluationContext {
            max_inline_depth: 4,
            ..ctx(true, 2)
        };
        // Both roots run into Chain(40, 20..): one claims a key at depth 4, the other waits.
        let crash = catch_unwind(AssertUnwindSafe(|| {
            ev.evaluate(
                &[SkyKey::new(Chain(40, 0)), SkyKey::new(Chain(40, 2))],
                &context,
            )
        }));
        assert!(crash.is_err());
        assert_eq!(ev.live_threads(), 0);
    }
}

// Review S2-3: batch helpers that cannot start are not needed: the requester drains the batch.
#[test]
fn spawn_failure_of_batch_helpers_falls_back_to_the_requesting_thread() {
    let ev = fan_evaluator();
    *ev.spawns_before_failure_for_testing.lock() = Some(1);
    let r = ev
        .evaluate(&[SkyKey::new(Fan(4, 0))], &ctx(true, 8))
        .unwrap();
    assert!(!r.has_error());
    assert_eq!(ev.live_threads(), 0);
}

// Review S2-3: no root worker can start: crash.
#[test]
#[should_panic(expected = "unable to create evaluator thread")]
fn spawn_failure_of_every_root_worker_crashes() {
    let ev = evaluator();
    *ev.spawns_before_failure_for_testing.lock() = Some(0);
    let _ = ev.evaluate(&[SkyKey::new(Num(30))], &ctx(true, 4));
}

// Review S2-4, first corner. Java: a parent that does not throw gets
// fromChildErrors(parent, {error}) with `error` the leaf failure (ParallelEvaluator.java:298,
// 480-484), not the wrapper built for the node below. With a cycle in the leaf's error (a cached
// keep_going error reused without keep_going) the path to the cycle is [grandparent], not
// [grandparent, parent].
#[test]
fn bubbled_error_of_a_parent_that_does_not_throw_wraps_the_leaf_failure() {
    for p in [1, 4] {
        let tester = new_tester();
        tester
            .add_dependency("cyc", "cyc")
            .add_dependency("parent", "cyc")
            .add_dependency("grandparent", "parent");
        let ev = tester_evaluator(tester);
        let r = eval(&ev, true, p, &[k("cyc")]);
        assert_eq!(
            only_cycle(&r, &k("cyc")),
            CycleInfo::create_cycle_info(vec![], vec![k("cyc")])
        );
        let r = eval(&ev, false, p, &[k("grandparent")]);
        assert_eq!(
            singleton_error(&r, &k("grandparent")).get_cycle_info(),
            &[CycleInfo::create_cycle_info(
                vec![k("grandparent")],
                vec![k("cyc")]
            )]
        );
    }
}

// Review S2-4, second corner. Java (AbstractParallelEvaluator.java:487-546): without keep_going
// a function that throws is committed as the first error even if it requested a dep in error:
// the node is done in the graph, directly transient errors depend on ErrorTransienceValue.KEY,
// and the dep's transitive transience counts.
#[test]
fn no_keep_going_function_that_throws_with_a_dep_in_error_commits_its_own_error() {
    for p in [1, 4] {
        let tester = new_tester();
        tester
            .set_has_transient_error("bad", true)
            .set_builder("top", |_, env| {
                let v = env.get_value(T("bad"))?;
                assert!(v.is_none() && env.values_missing());
                assert!(!env.in_error_bubbling());
                Err(persistent("top's own"))
            });
        let ev = tester_evaluator(tester);
        // The dep's error is cached, so "top" is the first to fail in the second evaluation.
        eval_value_in_error(&ev, &k("bad"));
        let r = eval(&ev, false, p, &[k("top")]);
        let error = singleton_error(&r, &k("top"));
        assert_eq!(exception_message(error), "top's own");
        assert!(!error.is_directly_transient());
        assert!(error.is_transitively_transient());
        let stored = ev
            .get_existing_error_for_testing(&k("top"))
            .expect("committed");
        assert_eq!(exception_message(&stored), "top's own");
        // The undone dep is not recorded (removeUndoneNewlyRequestedDeps does not apply: the dep
        // is done, in error), so "top" depends on "bad".
        assert_eq!(ev.get_direct_deps(&k("top")), vec![k("bad")]);
    }
}

// Review, interleaving 7: a batch whose rest contains the requester's own key.
#[test]
fn batch_with_the_requesters_own_key_is_a_self_cycle() {
    for keep_going in [false, true] {
        for first in [false, true] {
            for _ in 0..10 {
                let tester = new_tester();
                tester.set("Y", "y").set_builder("X", move |_, env| {
                    let keys = if first {
                        [k("X"), k("Y")]
                    } else {
                        [k("Y"), k("X")]
                    };
                    env.get_values_and_exceptions(&keys)?;
                    assert!(env.values_missing());
                    Ok(None)
                });
                let ev = tester_evaluator(tester);
                let r = eval(&ev, keep_going, 8, &[k("X")]);
                assert_eq!(
                    only_cycle(&r, &k("X")),
                    CycleInfo::create_cycle_info(vec![], vec![k("X")])
                );
            }
        }
    }
}

// A state Java cannot reach: a function throws although a dep is missing, and the missing dep
// is below it in a cycle that has already asked for it. Java commits such an error without ever
// evaluating what is below (AbstractParallelEvaluator.java:487); the result is the error, and
// bubbling goes up, not into the cycle below.
#[test]
fn no_keep_going_function_that_throws_above_its_own_cycle_reports_its_error() {
    for deterministic in [false, true] {
        for with_top in [false, true] {
            for p in [1, 4] {
                let tester = new_tester();
                tester
                    .add_dependency("top", "P")
                    .add_dependency("X", "P")
                    .set_builder("P", |_, env| {
                        let v = env.get_value(T("X"))?;
                        assert!(v.is_none() && env.values_missing());
                        Err(persistent("P's own"))
                    });
                let ev = tester_evaluator(tester);
                ev.deterministic_for_testing
                    .store(deterministic, Ordering::SeqCst);
                let root = if with_top { k("top") } else { k("P") };
                let r = eval(&ev, false, p, std::slice::from_ref(&root));
                let error = singleton_error(&r, &root);
                assert_eq!(exception_message(error), "P's own");
                assert!(error.get_cycle_info().is_empty());
            }
        }
    }
}

// Java: SkyFunctionEnvironment.processDepValue sets the thread's interrupt bit when a dep in
// error is requested during error bubbling (624-630); before that the function runs freely.
#[test]
fn check_interrupted_fails_in_error_bubbling_once_a_dep_in_error_was_requested() {
    let tester = new_tester();
    let seen = Arc::new(Mutex::new(vec![]));
    let s = seen.clone();
    tester
        .set("ok", "ok")
        .set_has_error("bad", true)
        .set_builder("top", move |_, env| {
            let bubbling = env.in_error_bubbling();
            let before = env.check_interrupted().is_ok();
            env.get_value(T("ok"))?;
            let after_ok = env.check_interrupted().is_ok();
            env.get_value(T("bad"))?;
            let after_bad = env.check_interrupted().is_ok();
            s.lock().push((bubbling, before, after_ok, after_bad));
            env.check_interrupted()?;
            Ok(None)
        });
    let ev = tester_evaluator(tester);
    let r = eval(&ev, false, 1, &[k("top")]);
    singleton_error(&r, &k("top"));
    assert_eq!(
        *seen.lock(),
        vec![
            // Normal evaluation: "bad" throws, the evaluation is aborting (Java interrupts the
            // threads of a failed evaluation).
            (false, true, true, false),
            // Error bubbling.
            (true, true, true, false),
        ]
    );
}

// Java: SkyFunctionEnvironment.createForError passes parentEntry.getTemporaryDirectDeps().
#[test]
fn get_temporary_direct_deps_in_error_bubbling_are_the_deps_of_the_normal_run() {
    let tester = new_tester();
    let seen = Arc::new(Mutex::new(vec![]));
    let s = seen.clone();
    tester
        .set("ok", "ok")
        .set_has_error("bad", true)
        .set_builder("top", move |_, env| {
            if env.in_error_bubbling() {
                s.lock().push(env.get_temporary_direct_deps().clone());
            }
            env.get_values_and_exceptions(&[k("ok"), k("bad")])?;
            Ok(None)
        });
    let ev = tester_evaluator(tester);
    eval(&ev, false, 1, &[k("top")]);
    let mut expected = GroupedDeps::new();
    expected.append_group(vec![k("ok"), k("bad")]);
    assert_eq!(*seen.lock(), vec![expected]);
}

// Java: SkyFunctionEnvironment.lookupRequestedDep (495-498).
#[test]
#[should_panic(expected = "Error transience key cannot be in requested deps of top")]
fn requesting_the_error_transience_key_is_rejected() {
    let tester = new_tester();
    tester.set_builder("top", |_, env| {
        env.get_value_dyn(&ERROR_TRANSIENCE_KEY)?;
        Ok(None)
    });
    let ev = tester_evaluator(tester);
    let _ = ev.evaluate(&[k("top")], &ctx(true, 1));
}

// The engine API that is not ported fails loudly (AGENTS.md: never guess a value).
#[test]
#[should_panic(expected = "not ported yet: SkyFunctionEnvironment.registerDependencies")]
fn register_dependencies_is_not_ported() {
    let tester = new_tester();
    tester.set_builder("top", |_, env| {
        env.register_dependencies(&[k("a")]);
        Ok(None)
    });
    let ev = tester_evaluator(tester);
    let _ = ev.evaluate(&[k("top")], &ctx(true, 1));
}

// A state Java cannot reach: "Q" asks for "N", whose dep throws the first error, and then "Q"
// throws its own exception (not the first error: ignored). In Java "Q" would have thrown before
// "N" was evaluated and its exception would be the first error. The error still reaches the
// top-level node, through "Q", which throws again in its error-bubbling run.
#[test]
fn no_keep_going_requester_that_throws_after_its_dep_failed_is_still_bubbled_through() {
    for p in [1, 4] {
        let tester = new_tester();
        let q_runs = Arc::new(AtomicUsize::new(0));
        let n = q_runs.clone();
        tester
            .add_dependency("top", "Q")
            .set_builder("Q", move |_, env| {
                n.fetch_add(1, Ordering::SeqCst);
                let v = env.get_value(T("N"))?;
                assert!(v.is_none() && env.values_missing());
                Err(persistent("Q's own"))
            })
            .add_dependency("N", "err")
            .set_has_error("err", true);
        let ev = tester_evaluator(tester);
        let r = eval(&ev, false, p, &[k("top")]);
        assert_eq!(exception_message(singleton_error(&r, &k("top"))), "Q's own");
        assert_eq!(q_runs.load(Ordering::SeqCst), 2);
        assert_eq!(graph_keys(&ev), HashSet::from([k("err")]));
    }
}

// Java: a function that throws although a dep was not done registered nothing with that dep
// (`enqueueChild` runs only after the function returns null), so it is not on the bubbling path
// of that dep's error: it runs once and never sees the exception. (The Java test
// MemoizingEvaluatorTest.raceConditionWithNoKeepGoingErrors_InflightError holds for both orders
// of "request" and "commit"; this pins the order "request first".)
#[test]
fn no_keep_going_waiter_that_throws_is_not_rerun_in_error_bubbling() {
    for _ in 0..20 {
        let other_waiting = Latch::new(1);
        let other_runs = Arc::new(AtomicUsize::new(0));
        let tester = new_tester();
        {
            let other_waiting = other_waiting.clone();
            tester.set_builder("errorKey", move |_, _| {
                other_waiting.wait("otherErrorKey did not start");
                // Let "otherErrorKey" block on this node before it commits.
                std::thread::sleep(Duration::from_millis(10));
                Err(persistent("error"))
            });
        }
        {
            let n = other_runs.clone();
            tester.set_builder("otherErrorKey", move |_, env| {
                n.fetch_add(1, Ordering::SeqCst);
                other_waiting.count_down();
                match env.get_value_or_throw::<T, SomeErrorException>(T("errorKey"))? {
                    Ok(value) => {
                        assert!(value.is_none());
                        Err(persistent("other"))
                    }
                    Err(_) => panic!("the exception reached a function that is not on the path"),
                }
            });
        }
        let ev = tester_evaluator(tester);
        let r = eval(&ev, false, 4, &[k("errorKey"), k("otherErrorKey")]);
        assert_eq!(
            exception_message(singleton_error(&r, &k("errorKey"))),
            "error"
        );
        assert_eq!(other_runs.load(Ordering::SeqCst), 1);
    }
}

// Java: a node whose function returned null for a dep that was not done waits to be restarted
// when the dep is done; after the evaluation has failed the visitor accepts no new work
// (`preventNewEvaluations`), so the node never finishes even if the dep does. The function that
// was blocked in the request here must not commit either.
#[test]
fn requester_of_a_dep_that_finishes_after_the_failure_does_not_commit() {
    for _ in 0..20 {
        let dep_started = Latch::new(1);
        let tester = new_tester();
        {
            let dep_started = dep_started.clone();
            tester.set_builder("dep", move |_, env| {
                dep_started.count_down();
                wait_for_exception(env, "the evaluation did not fail");
                Ok(Some("dep".into()))
            });
        }
        tester.add_dependency("parent", "dep");
        tester.set_builder("fail", move |_, _| {
            dep_started.wait("dep did not start");
            Err(persistent("fail"))
        });
        let ev = tester_evaluator(tester);
        let r = eval(&ev, false, 4, &[k("parent"), k("fail")]);
        assert_eq!(exception_message(singleton_error(&r, &k("fail"))), "fail");
        // "dep" was running when the evaluation failed and committed; "parent" was waiting.
        assert_eq!(graph_keys(&ev), HashSet::from([k("dep"), k("fail")]));
        // The next evaluation builds "parent" on the committed "dep".
        let computed = ev.computed_count();
        let r = eval(&ev, false, 4, &[k("parent")]);
        assert_eq!(*r.get(&T("parent")).unwrap(), "dep");
        assert_eq!(ev.computed_count(), computed + 1);
    }
}

// Review round 2, NEW-1. Root "K" requests "Y"; "Y" requests "X" and throws its own exception
// when "X" is missing; "X" requests "K". Java (AbstractParallelEvaluator.java:487-546): "Y"
// throws in the run that requested "X", so its error is committed and "X" is never evaluated;
// bubbleErrorUp goes Y -> K, "K" has no reverse dep and is top-level:
// {K: fromChildErrors(K, {Y's error})}. Here the request evaluated "X", which registered with
// "K"; the walk must not go down into it.
#[test]
fn no_keep_going_walk_does_not_go_below_the_thrower_through_a_cycle() {
    for deterministic in [false, true] {
        for p in [1, 4] {
            let tester = new_tester();
            let k_runs = Arc::new(AtomicUsize::new(0));
            let x_runs = Arc::new(AtomicUsize::new(0));
            let (kn, xn) = (k_runs.clone(), x_runs.clone());
            tester
                .set_builder("K", move |_, env| {
                    kn.fetch_add(1, Ordering::SeqCst);
                    Ok(env.get_value(T("Y"))?.map(|v| (*v).clone()))
                })
                .set_builder("Y", |_, env| {
                    let v = env.get_value(T("X"))?;
                    assert!(v.is_none() && env.values_missing());
                    Err(persistent("Y's own"))
                })
                .set_builder("X", move |_, env| {
                    xn.fetch_add(1, Ordering::SeqCst);
                    Ok(env.get_value(T("K"))?.map(|v| (*v).clone()))
                });
            let ev = tester_evaluator(tester);
            ev.deterministic_for_testing
                .store(deterministic, Ordering::SeqCst);
            let r = eval(&ev, false, p, &[k("K")]);
            let error = singleton_error(&r, &k("K"));
            assert_eq!(exception_message(error), "Y's own");
            assert!(error.get_cycle_info().is_empty());
            // "K": the normal run and the error-bubbling run. "X" is not on the path.
            assert_eq!(k_runs.load(Ordering::SeqCst), 2);
            assert_eq!(x_runs.load(Ordering::SeqCst), 1);
            assert_eq!(graph_keys(&ev), HashSet::from([k("Y")]));
        }
    }
}

// NEW-1 with a second top-level key on the cycle. Java evaluates "X" as a top-level key: it
// requests "K" and registers with it, so bubbling goes Y -> K -> X, and "X", which has no
// reverse dep ("Y" threw, it never registered), is top-level: both report Y's error.
#[test]
fn no_keep_going_walk_reaches_a_top_level_key_below_the_thrower() {
    for _ in 0..20 {
        let tester = new_tester();
        tester
            .add_dependency("K", "Y")
            .set_builder("Y", |_, env| {
                let v = env.get_value(T("X"))?;
                assert!(v.is_none() && env.values_missing());
                Err(persistent("Y's own"))
            })
            .add_dependency("X", "K");
        let ev = tester_evaluator(tester);
        // One worker: "K" is evaluated first, "X" under "Y".
        let r = eval(&ev, false, 1, &[k("K"), k("X")]);
        assert_eq!(r.error_map().len(), 2, "{r:?}");
        for key in ["K", "X"] {
            let error = r.get_error(&k(key)).unwrap();
            assert_eq!(exception_message(error), "Y's own");
            assert!(error.get_cycle_info().is_empty());
        }
    }
}

// NEW-1: what the thrower's missing dep went on to request is below the thrower as well.
#[test]
fn no_keep_going_walk_skips_everything_only_the_throwers_missing_dep_requested() {
    for p in [1, 4] {
        let tester = new_tester();
        tester
            .add_dependency("top", "K")
            .add_dependency("K", "Y")
            .set_builder("Y", |_, env| {
                let v = env.get_value(T("X"))?;
                assert!(v.is_none() && env.values_missing());
                Err(persistent("Y's own"))
            })
            // X -> Z -> {K, top}: both registrations are from below "Y".
            .add_dependency("X", "Z")
            .add_dependency("Z", "K")
            .add_dependency("Z", "top");
        let ev = tester_evaluator(tester);
        ev.deterministic_for_testing.store(true, Ordering::SeqCst);
        let r = eval(&ev, false, p, &[k("top")]);
        let error = singleton_error(&r, &k("top"));
        assert_eq!(exception_message(error), "Y's own");
        assert!(error.get_cycle_info().is_empty());
    }
}

// Java: an InMemoryMemoizingEvaluator with keepEdges = false (EdgelessInMemoryGraphImpl, every
// evaluation at Version.constant()): values are memoized; an injection over a done node removes
// it first (AbstractInMemoryMemoizingEvaluator.pruneInjectedValues, "No incrementality"); a node
// that depended on it keeps its value, as nothing records the edge.
#[test]
fn keep_edges_false_memoizes_and_injects_without_edges() {
    let tester = new_tester();
    tester.add_dependency_key(k("top"), nk("leaf"));
    let ev = EvaluatorBuilder::new()
        .client(tester.clone())
        .function(T::FUNCTION, TestFunction)
        .function(N::FUNCTION, TestFunction)
        .keep_edges(false)
        .inject(N("leaf"), "leaf".to_string())
        .build();
    let r = eval(&ev, false, 2, &[k("top")]);
    assert_eq!(*r.get(&T("top")).unwrap(), "leaf");
    let computed = ev.computed_count();
    let r = eval(&ev, false, 2, &[k("top")]);
    assert_eq!(*r.get(&T("top")).unwrap(), "leaf");
    assert_eq!(ev.computed_count(), computed);

    ev.inject(N("leaf"), "other".to_string());
    let r = eval(&ev, false, 2, &[nk("leaf"), k("top")]);
    assert_eq!(*r.get(&N("leaf")).unwrap(), "other");
    assert_eq!(*r.get(&T("top")).unwrap(), "leaf");
    // Java: DelegatingWalkableGraph.getDirectDeps on a node that keeps no edges.
    let deps = catch_unwind(AssertUnwindSafe(|| ev.get_direct_deps(&k("top"))));
    assert!(deps.is_err());
}
