//! Ports of `ParallelEvaluatorTest` methods (the ones not already in `tests.rs`).
//!
//! PORT: where a Java test counts two runs of a function in a keep_going evaluation (the run that
//! returns null and the restart), the function runs once here: dependency requests block. Where
//! Java counts the normal run and the error-bubbling run, so does this port.

use super::*;
use blaze_events::{ExtendedEventHandler, Postable};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn ctx_p(
    handler: Arc<StoredEventHandler>,
    keep_going: bool,
    parallelism: usize,
) -> EvaluationContext {
    EvaluationContext {
        event_handler: handler,
        ..ctx(keep_going, parallelism)
    }
}

fn catastrophic(msg: &str, transience: Transience) -> ComputeError {
    SkyFunctionException::catastrophic(SomeErrorException(msg.into()), transience).into()
}

// Java: ParallelEvaluatorTest.smoke
#[test]
fn smoke() {
    for_each_mode(
        |t| {
            t.set("a", "a")
                .set("b", "b")
                .add_dependency("ab", "a")
                .add_dependency("ab", "b");
        },
        |ev, p| {
            let events = Arc::new(StoredEventHandler::new());
            let r = ev
                .evaluate(&[k("ab")], &ctx_p(events.clone(), false, p))
                .unwrap();
            assert_eq!(*r.get(&T("ab")).unwrap(), "ab");
            assert!(events.get_events().is_empty());
            assert!(events.get_posts().is_empty());
        },
    );
}

/// Java: ParallelEvaluatorTest.runInterruptionTest. `Thread.interrupt()` of the thread that calls
/// `eval` is `Evaluator::interrupt`.
fn run_interruption_test(
    builder: impl Fn(&Latch, &T, &mut Environment<'_>) -> ComputeResultT + Send + Sync + 'static,
) {
    let thread_started = Latch::new(1);
    let started = thread_started.clone();
    let tester = new_tester();
    tester.with("a", |_| {});
    let builder = Arc::new(builder);
    let ev = EvaluatorBuilder::new()
        .function(
            T::FUNCTION,
            move |key: &SkyKey, env: &mut Environment<'_>| {
                let t = key.downcast::<T>().expect("T");
                Ok(builder(&started, t, env)?.map(|v| Arc::new(v) as SkyValue))
            },
        )
        .build();
    std::thread::scope(|scope| {
        let t = scope.spawn(|| ev.evaluate(&[k("a")], &ctx(false, 2)));
        // Start the thread and wait for a semaphore. This ensures that the thread was really
        // started.
        thread_started.wait("evaluation did not start");
        // Interrupt the thread. The evaluation must acknowledge the interruption.
        ev.interrupt();
        let r = t.join().expect("no crash");
        assert!(
            matches!(r, Err(Interrupted)),
            "evaluation should have been interrupted"
        );
    });
}

// Java: ParallelEvaluatorTest.interruptedFunction
#[test]
fn interrupted_function() {
    run_interruption_test(|thread_started, _, env| {
        // Signal the waiting test thread that the evaluator thread has really started.
        thread_started.count_down();
        // Simulate a SkyFunction that runs for 10 seconds. The main thread should interrupt it
        // shortly after it got started.
        let _ = env;
        java_sleep(Duration::from_secs(10))?;
        panic!("SkyFunction should have been interrupted");
    });
}

/// Java: `Thread.sleep(duration)`: throws `InterruptedException` (and clears the status) when
/// the thread is interrupted.
fn java_sleep(duration: Duration) -> Result<(), Interrupted> {
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        if blaze_shell::thread_interrupt::interrupted() {
            return Err(Interrupted);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

/// An evaluator whose `T` function runs `builder`.
fn evaluator_with(
    builder: impl Fn(&T, &mut Environment<'_>) -> ComputeResultT + Send + Sync + 'static,
) -> Evaluator {
    EvaluatorBuilder::new()
        .function(
            T::FUNCTION,
            move |key: &SkyKey, env: &mut Environment<'_>| {
                let t = key.downcast::<T>().expect("T");
                Ok(builder(t, env)?.map(|v| Arc::new(v) as SkyValue))
            },
        )
        .build()
}

// PORT: Blaze only. Java: AbstractQueueVisitor.awaitTermination(interruptWorkers = true) after a
// fail-fast error: the threads still running functions are interrupted, so blocking work (a spawn
// waiting for its child) stops instead of running to its end.
#[test]
fn in_flight_function_is_interrupted_when_another_fails_fast() {
    let slow_started = Latch::new(1);
    let started = slow_started.clone();
    let slow_was_interrupted = Arc::new(AtomicBool::new(false));
    let was_interrupted = slow_was_interrupted.clone();
    let ev = evaluator_with(move |t, _env| match t.0 {
        "slow" => {
            started.count_down();
            if java_sleep(Duration::from_secs(30)).is_err() {
                was_interrupted.store(true, Ordering::SeqCst);
                return Err(Interrupted.into());
            }
            Ok(Some("slow".into()))
        }
        _ => {
            started.wait("slow did not start");
            Err(SkyFunctionException::persistent(SomeErrorException("bad".into())).into())
        }
    });
    let begin = std::time::Instant::now();
    let r = ev.evaluate(&[k("slow"), k("bad")], &ctx(false, 2)).unwrap();
    assert!(begin.elapsed() < Duration::from_secs(20));
    assert!(slow_was_interrupted.load(Ordering::SeqCst));
    assert!(r.has_error());
    assert_eq!(exception_message(r.get_error(&k("bad")).unwrap()), "bad");
    assert!(r.get(&T("slow")).is_none());
}

// PORT: Blaze only. With keep_going an ordinary error stops nothing.
#[test]
fn in_flight_function_is_not_interrupted_by_an_error_with_keep_going() {
    let ev = evaluator_with(move |t, _env| match t.0 {
        "slow" => {
            java_sleep(Duration::from_millis(300))?;
            Ok(Some("slow".into()))
        }
        _ => Err(SkyFunctionException::persistent(SomeErrorException("bad".into())).into()),
    });
    let r = ev.evaluate(&[k("slow"), k("bad")], &ctx(true, 2)).unwrap();
    assert_eq!(*r.get(&T("slow")).unwrap(), "slow");
    assert!(r.get_error(&k("bad")).is_some());
}

// PORT: Blaze only. The interrupt flag a signal handler sets (`Evaluator::interrupt_flag`) is
// noticed by the evaluating thread, which interrupts the running functions.
#[test]
fn setting_the_interrupt_flag_interrupts_in_flight_functions() {
    let slow_started = Latch::new(1);
    let started = slow_started.clone();
    let ev = evaluator_with(move |_, _env| {
        started.count_down();
        java_sleep(Duration::from_secs(30))?;
        Ok(Some("slow".into()))
    });
    std::thread::scope(|scope| {
        let begin = std::time::Instant::now();
        let t = scope.spawn(|| ev.evaluate(&[k("slow")], &ctx(false, 2)));
        slow_started.wait("evaluation did not start");
        ev.interrupt_flag().store(true, Ordering::SeqCst);
        assert!(matches!(t.join().unwrap(), Err(Interrupted)));
        assert!(begin.elapsed() < Duration::from_secs(20));
    });
}

// PORT: Blaze only. A function that computes a dependency inline is one job per function on
// the same thread; the job of the outer function stays registered (and is interrupted) while
// the inner one ran.
#[test]
fn a_function_waiting_on_an_inline_dependency_is_still_interrupted() {
    let leaf_done = Latch::new(1);
    let done = leaf_done.clone();
    let ev = evaluator_with(move |t, env| match t.0 {
        "parent" => {
            let Some(v) = env.get_value(T("leaf"))? else {
                return Ok(None);
            };
            done.count_down();
            java_sleep(Duration::from_secs(30))?;
            Ok(Some(format!("parent of {v}")))
        }
        _ => Ok(Some("leaf".into())),
    });
    std::thread::scope(|scope| {
        let begin = std::time::Instant::now();
        let t = scope.spawn(|| ev.evaluate(&[k("parent")], &ctx(false, 1)));
        leaf_done.wait("parent did not resume");
        ev.interrupt();
        assert!(matches!(t.join().unwrap(), Err(Interrupted)));
        assert!(begin.elapsed() < Duration::from_secs(20));
    });
}

// Java: ParallelEvaluatorTest.interruptedEvaluatorThread
#[test]
fn interrupted_evaluator_thread() {
    let value_id_counter = AtomicUsize::new(0);
    run_interruption_test(move |thread_started, _, env| {
        // Signal the waiting test thread that the Evaluator thread has really started.
        thread_started.count_down();
        // Keep the evaluator busy until the test's thread gets scheduled and can interrupt the
        // Evaluator's thread.
        std::thread::sleep(Duration::from_millis(1));
        let n = value_id_counter.fetch_add(1, Ordering::SeqCst);
        env.get_value(T(leak(format!("a{n}"))))?;
        // This method never throws InterruptedException itself, therefore it's the responsibility
        // of the Evaluator to detect the interrupt and avoid calling subsequent SkyFunctions.
        Ok(None)
    });
}

// Java: ParallelEvaluatorTest.unrecoverableError
#[test]
fn unrecoverable_error() {
    let ev = EvaluatorBuilder::new()
        .function(
            T::FUNCTION,
            |_: &SkyKey, _: &mut Environment<'_>| -> ComputeResult {
                panic!("CustomRuntimeException")
            },
        )
        .build();
    let re = catch_unwind(AssertUnwindSafe(|| ev.evaluate(&[k("a")], &ctx(false, 2))))
        .expect_err("RuntimeException");
    let message = re.downcast_ref::<String>().expect("message");
    assert!(
        message.contains("Unrecoverable error while evaluating node 'a'"),
        "{message}"
    );
    // Java: hasCauseThat().isInstanceOf(CustomRuntimeException.class)
    assert!(
        message.ends_with("\nCaused by: CustomRuntimeException"),
        "{message}"
    );
}

/// Java: ParallelEvaluatorTest.ExamplePost
#[derive(Debug)]
struct ExamplePost {
    store_for_replay: bool,
}
impl Postable for ExamplePost {
    fn store_for_replay(&self) -> bool {
        self.store_for_replay
    }
}

/// Java: ParallelEvaluatorTest.SkyframeEventType
#[derive(Clone, Copy, Debug, PartialEq)]
enum SkyframeEventType {
    Event,
    Post,
}

#[derive(Clone)]
enum TestReportable {
    Event(Event),
    Post(Arc<dyn Postable>),
}

impl TestReportable {
    fn report_to(&self, handler: &dyn ExtendedEventHandler) {
        match self {
            TestReportable::Event(e) => handler.handle(e),
            TestReportable::Post(p) => handler.post(p.clone()),
        }
    }
    /// `withTag`: posts have no tag.
    fn with_tag(&self, tag: &str) -> TestReportable {
        match self {
            TestReportable::Event(e) => TestReportable::Event(e.with_tag(Some(tag))),
            TestReportable::Post(p) => TestReportable::Post(p.clone()),
        }
    }
}

impl SkyframeEventType {
    fn create_unstored(self) -> TestReportable {
        match self {
            SkyframeEventType::Event => TestReportable::Event(Event::progress("analyzing")),
            SkyframeEventType::Post => TestReportable::Post(Arc::new(ExamplePost {
                store_for_replay: false,
            })),
        }
    }
    fn create_stored(self) -> TestReportable {
        match self {
            SkyframeEventType::Event => TestReportable::Event(Event::error("broken")),
            SkyframeEventType::Post => TestReportable::Post(Arc::new(ExamplePost {
                store_for_replay: true,
            })),
        }
    }
    /// `assertThat(eventType.getResults(reportedEvents)).containsExactly(expected...)`.
    fn assert_results(self, reported: &StoredEventHandler, expected: &[&TestReportable]) {
        match self {
            SkyframeEventType::Event => {
                let want: Vec<Event> = expected
                    .iter()
                    .map(|r| match r {
                        TestReportable::Event(e) => e.clone(),
                        TestReportable::Post(_) => unreachable!(),
                    })
                    .collect();
                assert_eq!(reported.get_events(), want);
            }
            SkyframeEventType::Post => {
                let got = reported.get_posts();
                assert_eq!(got.len(), expected.len(), "{got:?}");
                for (g, w) in got.iter().zip(expected) {
                    match w {
                        TestReportable::Post(p) => assert!(Arc::ptr_eq(g, p)),
                        TestReportable::Event(_) => unreachable!(),
                    }
                }
            }
        }
    }
}

// Java: ParallelEvaluatorTest.noEventStorage_unstoredEvent_reportedImmediately_notReplayed
#[test]
fn no_event_storage_unstored_event_reported_immediately_not_replayed() {
    for event_type in [SkyframeEventType::Event, SkyframeEventType::Post] {
        let reported_events = Arc::new(StoredEventHandler::new());
        let evaluated = Arc::new(AtomicBool::new(false));
        let unstored_event = event_type.create_unstored();
        let tester = new_tester();
        {
            let (evaluated, unstored_event, reported_events) = (
                evaluated.clone(),
                unstored_event.clone(),
                reported_events.clone(),
            );
            tester.set_builder("key", move |_, env| {
                evaluated.store(true, Ordering::SeqCst);
                unstored_event.report_to(env.listener());
                event_type.assert_results(&reported_events, &[&unstored_event]);
                Ok(Some("value".into()))
            });
        }
        let ev = tester_evaluator(tester);
        ev.evaluate(&[k("key")], &ctx_p(reported_events.clone(), false, 2))
            .unwrap();
        assert!(evaluated.load(Ordering::SeqCst));
        event_type.assert_results(&reported_events, &[&unstored_event]);

        reported_events.clear();
        evaluated.store(false, Ordering::SeqCst);
        ev.evaluate(&[k("key")], &ctx_p(reported_events.clone(), false, 2))
            .unwrap();
        assert!(!evaluated.load(Ordering::SeqCst));
        event_type.assert_results(&reported_events, &[]);
    }
}

// Java: ParallelEvaluatorTest.noEventStorage_storedEvent_reportedAfterSkyFunctionCompletes_notReplayed
#[test]
fn no_event_storage_stored_event_reported_after_sky_function_completes_not_replayed() {
    for event_type in [SkyframeEventType::Event, SkyframeEventType::Post] {
        let reported_events = Arc::new(StoredEventHandler::new());
        let tag = "this is the tag";
        let evaluated_mid = Arc::new(AtomicBool::new(false));
        let stored_event = event_type.create_stored();
        let tagged_event = stored_event.with_tag(tag);
        let tester = new_tester();
        {
            let (tagged_event, reported_events) = (tagged_event.clone(), reported_events.clone());
            tester.set_builder("top", move |_, env| {
                if env.get_value(T("mid"))?.is_none() {
                    return Ok(None);
                }
                event_type.assert_results(&reported_events, &[&tagged_event]);
                Ok(Some("topValue".into()))
            });
        }
        {
            let (evaluated_mid, stored_event, reported_events) = (
                evaluated_mid.clone(),
                stored_event.clone(),
                reported_events.clone(),
            );
            tester
                .set_builder("mid", move |_, env| {
                    evaluated_mid.store(true, Ordering::SeqCst);
                    stored_event.report_to(env.listener());
                    event_type.assert_results(&reported_events, &[]);
                    if env.get_value(T("bottom"))?.is_none() {
                        return Ok(None);
                    }
                    Ok(Some("midValue".into()))
                })
                .with("mid", |s| s.tag = Some(tag.to_string()))
                .set("bottom", "depValue");
        }
        let ev = EvaluatorBuilder::new()
            .client(tester.clone())
            .function(T::FUNCTION, TaggingTestFunction(tester))
            .build();
        ev.evaluate(&[k("top")], &ctx_p(reported_events.clone(), false, 2))
            .unwrap();
        assert!(evaluated_mid.load(Ordering::SeqCst));
        event_type.assert_results(&reported_events, &[&tagged_event]);

        reported_events.clear();
        evaluated_mid.store(false, Ordering::SeqCst);
        ev.evaluate(&[k("top")], &ctx_p(reported_events.clone(), false, 2))
            .unwrap();
        assert!(!evaluated_mid.load(Ordering::SeqCst));
        event_type.assert_results(&reported_events, &[]);
    }
}

// Java: ParallelEvaluatorTest.shouldCreateErrorValueWithRootCause
#[test]
fn should_create_error_value_with_root_cause() {
    for_each_mode(
        |t| {
            t.set("a", "a")
                .add_dependency("parent", "a")
                .add_dependency("parent", "error")
                .set_has_error("error", true);
        },
        |ev, _| {
            eval_value_in_error(ev, &k("parent"));
        },
    );
}

// Java: ParallelEvaluatorTest.shouldBuildOneTarget
#[test]
fn should_build_one_target() {
    for_each_mode(
        |t| {
            t.set("a", "a")
                .set("b", "b")
                .add_dependency("parent", "error")
                .add_dependency("parent", "a")
                .set_has_error("error", true)
                .add_dependency("ab", "a")
                .add_dependency("ab", "b");
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("parent"), k("ab")]);
            assert!(r.get_error(&k("parent")).is_some());
            assert_eq!(*r.get(&T("ab")).unwrap(), "ab");
        },
    );
}

// Java: ParallelEvaluatorTest.incrementalCycleWithCatastropheAndFailedBubbleUp
#[test]
fn incremental_cycle_with_catastrophe_and_failed_bubble_up() {
    for p in [1, 4] {
        let tester = new_tester();
        tester
            .set_builder("top", |_, env| {
                env.get_value(T("cycle"))?;
                Ok(if env.values_missing() {
                    None
                } else {
                    Some("top".into())
                })
            })
            .set_builder("cycle", |_, env| {
                env.get_values_and_exceptions(&[k("cycle"), k("catastrophe")])?;
                assert!(env.values_missing());
                Ok(None)
            })
            .set_builder("catastrophe", |_, _| {
                Err(catastrophic("catastrophe", Transience::Transient))
            });
        let ev = tester_evaluator(tester);
        // Java: DeterministicHelper.DeterministicProcessableGraph. "cycle" comes alphabetically
        // before "top", so bubbling goes from "cycle" into "cycle" again.
        ev.deterministic_for_testing.store(true, Ordering::SeqCst);
        let r = eval(&ev, true, p, &[k("top")]);
        assert!(r.has_error());
        assert_eq!(
            r.get_error(&k("top")).unwrap().get_cycle_info(),
            &[CycleInfo::create_cycle_info(
                vec![k("top")],
                vec![k("cycle")]
            )]
        );
    }
}

// Java: ParallelEvaluatorTest.parentFailureDoesntAffectChild
#[test]
fn parent_failure_doesnt_affect_child() {
    for_each_mode(
        |t| {
            t.set_has_error("parent", true)
                .set("child", "onions")
                .add_dependency("parent", "child");
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("parent"), k("child")]);
            // Child is guaranteed to complete successfully before parent can run (and fail),
            // since parent depends on it.
            assert_eq!(*r.get(&T("child")).unwrap(), "onions");
            assert!(r.get_error(&k("parent")).is_some());
        },
    );
}

// Java: ParallelEvaluatorTest.newParentOfErrorShouldHaveError
#[test]
fn new_parent_of_error_should_have_error() {
    for_each_mode(
        |t| {
            t.set_has_error("error", true)
                .add_dependency("parent", "error");
        },
        |ev, _| {
            eval_value_in_error(ev, &k("error"));
            eval_value_in_error(ev, &k("parent"));
        },
    );
}

// Java: ParallelEvaluatorTest.errorTwoLevelsDeep
#[test]
fn error_two_levels_deep() {
    for_each_mode(
        |t| {
            t.set_has_error("error", true)
                .add_dependency("mid", "error")
                .add_dependency("parent", "mid");
        },
        |ev, _| {
            eval_value_in_error(ev, &k("parent"));
        },
    );
}

// Java: ParallelEvaluatorTest.valueNotUsedInFailFastErrorRecovery
#[test]
fn value_not_used_in_fail_fast_error_recovery() {
    for_each_mode(
        |t| {
            t.add_dependency("top", "midRecovery")
                .add_error_dependency("midRecovery", "bad", "i recovered")
                .set_has_error("bad", true);
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("midRecovery")]);
            assert!(r.error_map().is_empty());
            assert!(!r.has_error());
            assert_eq!(*r.get(&T("midRecovery")).unwrap(), "i recovered");

            let r = eval(ev, false, p, &[k("top")]);
            assert!(r.has_error());
            assert!(r.key_names().is_empty());
            assert_eq!(r.error_map().len(), 1);
            assert!(r.get_error(&k("top")).unwrap().get_exception().is_some());
        },
    );
}

// Java: ParallelEvaluatorTest.multipleRootCauses
#[test]
fn multiple_root_causes() {
    for_each_mode(
        |t| {
            t.set_has_error("error", true)
                .set_has_error("error2", true)
                .set_has_error("error3", true)
                .add_dependency("mid", "error")
                .add_dependency("mid", "error2")
                .add_dependency("parent", "mid")
                .add_dependency("parent", "error2")
                .add_dependency("parent", "error3");
        },
        |ev, _| {
            eval_value_in_error(ev, &k("parent"));
        },
    );
}

// Java: ParallelEvaluatorTest.rootCauseWithNoKeepGoing
#[test]
fn root_cause_with_no_keep_going() {
    for_each_mode(
        |t| {
            t.set_has_error("error", true)
                .add_dependency("mid", "error")
                .add_dependency("parent", "mid");
        },
        |ev, p| {
            let r = eval(ev, false, p, &[k("parent")]);
            singleton_error(&r, &k("parent"));
        },
    );
}

// Java: ParallelEvaluatorTest.errorBubblesToParentsOfTopLevelValue
#[test]
fn error_bubbles_to_parents_of_top_level_value() {
    for _ in 0..20 {
        let tester = new_tester();
        // Java: a graph listener releases the latch when "parent" has added itself as a reverse
        // dep of "error". Here "parent" counts it down just before it requests "error"; the
        // request registers the reverse dep before "error" can finish only if "error" has not
        // committed yet, so "error" also waits until "parent" is blocked on it or computing it.
        let latch = Latch::new(1);
        let l = latch.clone();
        tester
            .set_builder("error", move |_, _| {
                l.wait("parent did not request error");
                // Let "parent" reach its request (it either claims "error" itself, and then this
                // function runs under it, or it finds "error" in progress and registers).
                std::thread::sleep(Duration::from_millis(20));
                Err(SkyFunctionException::persistent(SomeErrorException("oops".into())).into())
            })
            .set_builder("parent", move |_, env| {
                latch.count_down();
                let v = env.get_value(T("error"))?;
                Ok(v.map(|v| (*v).clone()))
            });
        let ev = tester_evaluator(tester);
        let r = eval(&ev, false, 4, &[k("parent"), k("error")]);
        assert_eq!(r.error_map().len(), 2, "{r:?}");
    }
}

// Java: ParallelEvaluatorTest.manyUnprocessedValuesInCycle
#[test]
fn many_unprocessed_values_in_cycle() {
    let (last_self, first_self, mid_self) = ("zlastSelf", "afirstSelf", "midSelf9");
    for_each_mode(
        |t| {
            // We add firstSelf first so that it is processed last in cycle detection (LIFO),
            // meaning that none of the dep values have to be cleared from firstSelf.
            t.add_dependency(first_self, first_self);
            for i in 0..100 {
                let first_dep = leak(format!("first{i}"));
                let mid_dep = leak(format!("midSelf{i}dep"));
                let last_dep = leak(format!("last{i}"));
                t.add_dependency(first_self, first_dep)
                    .add_dependency(mid_self, mid_dep)
                    .add_dependency(last_self, last_dep);
                if i == 90 {
                    // Most of the deps will be cleared from midSelf.
                    t.add_dependency(mid_self, mid_self);
                }
                t.add_dependency(first_dep, first_dep)
                    .add_dependency(mid_dep, mid_dep)
                    .add_dependency(last_dep, last_dep);
            }
            // All the deps will be cleared from lastSelf.
            t.add_dependency(last_self, last_self);
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k(last_self), k(first_self), k(mid_self)]);
            assert!(r.key_names().is_empty(), "{r:?}");
            let keys: HashSet<SkyKey> = r.error_map().iter().map(|(key, _)| key.clone()).collect();
            assert_eq!(
                keys,
                HashSet::from([k(last_self), k(first_self), k(mid_self)])
            );

            // Check lastSelfKey.
            let cycle_info = only_cycle(&r, &k(last_self));
            assert_eq!(cycle_info.get_cycle(), &[k(last_self)]);
            assert!(cycle_info.get_path_to_cycle().is_empty());

            // Check firstSelfKey. It should not have discovered its own self-edge, because there
            // were too many other values before it in the queue.
            assert_many_cycles(r.get_error(&k(first_self)).unwrap(), &k(first_self), false);

            // Check midSelfKey. It should have discovered its own self-edge.
            assert_many_cycles(r.get_error(&k(mid_self)).unwrap(), &k(mid_self), true);
        },
    );
}

/// Checks that errorInfo has many self-edge cycles, and that one of them is a self-edge of
/// topKey, if `self_edge` is true.
// Java: ParallelEvaluatorTest.assertManyCycles
fn assert_many_cycles(error_info: &ErrorInfo, top_key: &SkyKey, self_edge: bool) {
    assert!(error_info.get_cycle_info().len() > 1);
    assert!(error_info.get_cycle_info().len() < 50);
    let mut found_self_edge = false;
    for cycle in error_info.get_cycle_info() {
        assert_eq!(cycle.get_cycle().len(), 1); // Self-edge.
        if !cycle.get_path_to_cycle().is_empty() {
            assert_eq!(cycle.get_path_to_cycle(), std::slice::from_ref(top_key));
        } else {
            assert_eq!(cycle.get_cycle(), std::slice::from_ref(top_key));
            found_self_edge = true;
        }
    }
    assert_eq!(found_self_edge, self_edge, "{error_info:?}, {top_key:?}");
}

// Java: ParallelEvaluatorTest.errorValueStoredWithKeepGoing
#[test]
fn error_value_stored_with_keep_going() {
    let tester = new_tester();
    tester.set_has_error("my_error_value", true);
    let ev = tester_evaluator(tester.clone());
    let r = eval(&ev, true, 2, &[k("my_error_value")]);
    singleton_error(&r, &k("my_error_value"));
    // Update value. But builder won't rebuild it.
    tester
        .set_has_error("my_error_value", false)
        .set("my_error_value", "no error?");
    let r = eval(&ev, true, 2, &[k("my_error_value")]);
    singleton_error(&r, &k("my_error_value"));
}

// Java: ParallelEvaluatorTest.continueWithErrorDep
#[test]
fn continue_with_error_dep() {
    for_each_mode(
        |t| {
            t.set_has_error("my_error_value", true)
                .set("after", "after")
                .add_error_dependency("parent", "my_error_value", "recovered")
                .add_dependency("parent", "after");
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("parent")]);
            assert!(r.error_map().is_empty());
            assert_eq!(*r.get(&T("parent")).unwrap(), "recoveredafter");
            let r = eval(ev, false, p, &[k("parent")]);
            singleton_error(&r, &k("parent"));
        },
    );
}

// Java: ParallelEvaluatorTest.transformErrorDep
#[test]
fn transform_error_dep() {
    for keep_going in [false, true] {
        for_each_mode(
            |t| {
                t.set_has_error("my_error_value", true)
                    .add_error_dependency("parent", "my_error_value", "recovered")
                    .set_has_error("parent", true);
            },
            |ev, p| {
                let r = eval(ev, keep_going, p, &[k("parent")]);
                let error = singleton_error(&r, &k("parent"));
                // The parent recovered from the child's error and threw its own (in error
                // bubbling without keep_going).
                assert_eq!(exception_message(error), "parent");
            },
        );
    }
}

// Java: ParallelEvaluatorTest.transformErrorDepOneLevelDownKeepGoing
#[test]
fn transform_error_dep_one_level_down_keep_going() {
    for_each_mode(
        |t| {
            t.set_has_error("my_error_value", true)
                .set("after", "after")
                .add_error_dependency("parent", "my_error_value", "recovered")
                .set("parent", "parent value")
                .add_dependency("top", "parent")
                .add_dependency("top", "after");
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("top")]);
            assert_eq!(r.key_names(), vec![k("top")]);
            assert_eq!(*r.get(&T("top")).unwrap(), "parent valueafter");
            assert!(r.error_map().is_empty());
        },
    );
}

// Java: ParallelEvaluatorTest.transformErrorDepOneLevelDownNoKeepGoing
#[test]
fn transform_error_dep_one_level_down_no_keep_going() {
    for_each_mode(
        |t| {
            t.set_has_error("my_error_value", true)
                .set("after", "after")
                .add_error_dependency("parent", "my_error_value", "recovered")
                .set("parent", "parent value")
                .add_dependency("top", "parent")
                .add_dependency("top", "after");
        },
        |ev, p| {
            let r = eval(ev, false, p, &[k("top")]);
            singleton_error(&r, &k("top"));
        },
    );
}

// Java: ParallelEvaluatorTest.errorDepDoesntStopOtherDep
#[test]
fn error_dep_doesnt_stop_other_dep() {
    let tester = new_tester();
    tester.set_has_error("error", true);
    let ev = tester_evaluator(tester.clone());
    let r1 = eval(&ev, true, 2, &[k("error")]);
    assert!(r1.has_error());
    assert!(r1.get_error(&k("error")).unwrap().get_exception().is_some());
    tester.set("other", "other");
    let top_exception: SkyError = Arc::new(SomeErrorException("top exception".into()));
    let num_computes = Arc::new(AtomicUsize::new(0));
    {
        let (top_exception, num_computes) = (top_exception.clone(), num_computes.clone());
        tester.set_builder("top", move |_, env| {
            let values = env.get_values_and_exceptions(&[k("error"), k("other")])?;
            // PORT: Java's first run finds "other" not done (valuesMissing) and the restart
            // finds everything done. The one run here is that restart.
            assert_eq!(num_computes.fetch_add(1, Ordering::SeqCst), 0);
            assert!(!env.values_missing());
            match values.get_or_throw::<SomeErrorException>(&k("error")) {
                Ok(_) => panic!("Should have thrown"),
                Err(_) => Err(SkyFunctionException::from_cause(
                    top_exception.clone(),
                    Transience::Persistent,
                )
                .into()),
            }
        });
    }
    let r2 = eval(&ev, true, 2, &[k("top")]);
    assert!(r2.has_error());
    assert!(Arc::ptr_eq(
        r2.get_error(&k("top")).unwrap().get_exception().unwrap(),
        &top_exception
    ));
    assert_eq!(num_computes.load(Ordering::SeqCst), 1);
}

// Java: ParallelEvaluatorTest.cycleWithMultipleUnfinishedChildren
// PORT: the isCycleNodePartialReevaluation=true variant is not ported (partial reevaluation).
#[test]
fn cycle_with_multiple_unfinished_children() {
    for_each_mode(
        |t| {
            t.add_dependency("top", "mid")
                // selfEdge* come before cycleKey, so cycleKey's path will be checked first
                // (LIFO), and the cycle with mid will be detected before the selfEdge* cycles
                // are.
                .add_dependency("mid", "selfEdge1")
                .add_dependency("mid", "selfEdge2")
                .add_dependency("mid", "cycle")
                .add_dependency("cycle", "mid")
                .add_dependency("selfEdge1", "selfEdge1")
                .add_dependency("selfEdge2", "selfEdge2");
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("top")]);
            let cycle_info = only_cycle(&r, &k("top"));
            assert_eq!(r.error_map().len(), 1);
            assert_eq!(cycle_info.get_path_to_cycle(), &[k("top")]);
            assert_eq!(cycle_info.get_cycle(), &[k("mid"), k("cycle")]);
        },
    );
}

/// The graph of the "value in cycle depends on error" regression tests: top -> mid ->
/// {error, cycle}, cycle -> mid. "mid" asks for both deps in one batch: with one request at a
/// time "cycle" would only start after "error" has thrown (requests block), and the Java tests
/// need "cycle" to have declared its dep on "mid" before that.
fn cycle_and_error(tester: &Tester, error_waits_for: Arc<Latch>, cycle_finish: Arc<Latch>) {
    tester
        .add_dependency("top", "mid")
        .add_dependency("mid", "error")
        .add_dependency("mid", "cycle")
        .with("mid", |s| s.batch = true)
        // We need to ensure that cycle value has finished its work, and we have recorded
        // dependencies.
        .set_chained(
            "cycle",
            Chained {
                notify_finish: Some(cycle_finish),
                value: Some(""),
                deps: vec!["mid"],
                ..Chained::default()
            },
        )
        .set_chained(
            "error",
            Chained {
                wait_to_finish: Some(error_waits_for),
                ..Chained::default()
            },
        );
}

// Java: ParallelEvaluatorTest.cycleAndErrorInBubbleUp
// PORT: the isCycleNodePartialReevaluation=true variants are not ported (partial reevaluation).
#[test]
fn cycle_and_error_in_bubble_up() {
    for keep_going in [false, true] {
        for _ in 0..10 {
            let tester = new_tester();
            let cycle_finish = Latch::new(1);
            cycle_and_error(&tester, cycle_finish.clone(), cycle_finish);
            let ev = tester_evaluator(tester);
            // Java: DeterministicHelper.DeterministicProcessableGraph: "mid" bubbles up to
            // "cycle" before "top".
            ev.deterministic_for_testing.store(true, Ordering::SeqCst);
            let r = eval(&ev, keep_going, 4, &[k("top")]);
            assert_eq!(
                singleton_error(&r, &k("top")).get_cycle_info(),
                &[CycleInfo::create_cycle_info(
                    vec![k("top")],
                    vec![k("mid"), k("cycle")]
                )],
                "{r:?}"
            );
        }
    }
}

// Java: ParallelEvaluatorTest.cycleAndErrorAndOtherInBubbleUp
#[test]
fn cycle_and_error_and_other_in_bubble_up() {
    for _ in 0..10 {
        let tester = new_tester();
        let top_start_and_cycle_finish = Latch::new(2);
        cycle_and_error(
            &tester,
            // error waits until otherTop starts and cycle finishes, to make sure otherTop will
            // request its dep before the threadpool shuts down.
            top_start_and_cycle_finish.clone(),
            top_start_and_cycle_finish.clone(),
        );
        // In nokeep_going mode, otherTop will wait until the threadpool has received an
        // exception, then request its own dep. This guarantees that there is a value that is not
        // finished when cycle detection happens.
        tester.set_chained(
            "otherTop",
            Chained {
                notify_start: Some(top_start_and_cycle_finish),
                wait_to_finish: Some(Latch::new(0)),
                wait_for_exception: true,
                value: Some("never returned"),
                deps: vec!["dep that never builds"],
                ..Chained::default()
            },
        );
        let ev = tester_evaluator(tester);
        ev.deterministic_for_testing.store(true, Ordering::SeqCst);
        let r = eval(&ev, false, 4, &[k("top"), k("otherTop")]);
        assert_eq!(r.error_map().len(), 1, "{r:?}");
        let cycle_info = only_cycle(&r, &k("top"));
        assert_eq!(cycle_info.get_path_to_cycle(), &[k("top")]);
        assert_eq!(cycle_info.get_cycle(), &[k("mid"), k("cycle")]);
    }
}

// Java: ParallelEvaluatorTest.cycleAndErrorAndError
// PORT: the isCycleNodePartialReevaluation=true variants are not ported (partial reevaluation).
#[test]
fn cycle_and_error_and_error() {
    for keep_going in [false, true] {
        for _ in 0..10 {
            let tester = new_tester();
            let top_start_and_cycle_finish = Latch::new(2);
            cycle_and_error(
                &tester,
                top_start_and_cycle_finish.clone(),
                top_start_and_cycle_finish.clone(),
            );
            // In nokeep_going mode, otherTop will wait until the threadpool has received an
            // exception, then throw its own exception. This guarantees that its exception will
            // not be the one bubbling up, but that there is a top-level value with an exception
            // by the time the bubbling up starts.
            tester.set_chained(
                "otherTop",
                Chained {
                    notify_start: Some(top_start_and_cycle_finish),
                    wait_to_finish: Some(Latch::new(0)),
                    wait_for_exception: !keep_going,
                    ..Chained::default()
                },
            );
            let ev = tester_evaluator(tester);
            ev.deterministic_for_testing.store(true, Ordering::SeqCst);
            let r = eval(&ev, keep_going, 4, &[k("top"), k("otherTop")]);
            if keep_going {
                assert_eq!(r.error_map().len(), 2, "{r:?}");
            }
            assert_eq!(
                r.get_error(&k("top"))
                    .expect("top in error")
                    .get_cycle_info(),
                &[CycleInfo::create_cycle_info(
                    vec![k("top")],
                    vec![k("mid"), k("cycle")]
                )],
                "{r:?}"
            );
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ChildKey(&'static str);
impl fmt::Debug for ChildKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "child:{}", self.0)
    }
}
impl Key for ChildKey {
    type Value = String;
    const FUNCTION: SkyFunctionName = SkyFunctionName("child");
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ParentKey(&'static str);
impl fmt::Debug for ParentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parent:{}", self.0)
    }
}
impl Key for ParentKey {
    type Value = String;
    const FUNCTION: SkyFunctionName = SkyFunctionName("parent");
}

// Java: ParallelEvaluatorTest.testFunctionCrashTrace
#[test]
fn test_function_crash_trace() {
    let ev = EvaluatorBuilder::new()
        .typed_function::<ChildKey, _>(|_, _| panic!("I WANT A PONY!!!"))
        .typed_function::<ParentKey, _>(|_, env| {
            if env.get_value(ChildKey("billy the kid"))?.is_none() {
                return Ok(None);
            }
            panic!("IllegalStateException"); // Should never get here.
        })
        .build();
    let e = catch_unwind(AssertUnwindSafe(|| {
        ev.evaluate(&[SkyKey::new(ParentKey("octodad"))], &ctx(false, 2))
    }))
    .expect_err("RuntimeException");
    let message = e.downcast_ref::<String>().expect("message");
    // Java: e.getMessage(), then e.getCause().getMessage().
    assert_eq!(
        message,
        "Unrecoverable error while evaluating node 'child:billy the kid' (requested by nodes \
         'parent:octodad')\nCaused by: I WANT A PONY!!!"
    );
}

#[derive(Debug, Clone, PartialEq)]
struct SomeOtherErrorException(String);
impl fmt::Display for SomeOtherErrorException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SomeOtherErrorException {}

// Java: ParallelEvaluatorTest.unexpectedErrorDep
#[test]
fn unexpected_error_dep() {
    for keep_going in [false, true] {
        for_each_mode(
            |t| {
                t.set_builder("my_error_value", |_, _| {
                    Err(SkyFunctionException::persistent(SomeOtherErrorException(
                        "error exception".into(),
                    ))
                    .into())
                })
                .add_error_dependency("top", "my_error_value", "recovered");
            },
            |ev, p| {
                let r = eval(ev, keep_going, p, &[k("top")]);
                singleton_error(&r, &k("top"));
            },
        );
    }
}

// Java: ParallelEvaluatorTest.unexpectedErrorDepOneLevelDown
#[test]
fn unexpected_error_dep_one_level_down() {
    for keep_going in [false, true] {
        for p in [1, 4] {
            let tester = new_tester();
            tester
                .set_builder("my_error_value", |_, _| {
                    Err(SkyFunctionException::persistent(SomeErrorException(
                        "error exception".into(),
                    ))
                    .into())
                })
                .add_dependency("parent", "my_error_value")
                .set_builder("top", move |_, env| {
                    match env.get_value_or_throw::<T, SomeErrorException>(T("parent"))? {
                        Ok(None) => return Ok(None),
                        Ok(Some(_)) => {}
                        Err(e) => assert_eq!(e, SomeErrorException("error exception".into())),
                    }
                    if keep_going {
                        Ok(Some("top".into()))
                    } else {
                        Err(SkyFunctionException::persistent(SomeErrorException(
                            "top exception".into(),
                        ))
                        .into())
                    }
                })
                .add_error_dependency("top", "my_error_value", "recovered");
            let ev = tester_evaluator(tester);
            let r = eval(&ev, keep_going, p, &[k("top")]);
            if !keep_going {
                singleton_error(&r, &k("top"));
            } else {
                assert!(!r.has_error(), "{r:?}");
                assert_eq!(*r.get(&T("top")).unwrap(), "top");
            }
        }
    }
}

// Java: ParallelEvaluatorTest.sameDepInTwoGroups
#[test]
fn same_dep_in_two_groups() {
    for same_first in [false, true] {
        for two_calls in [false, true] {
            for p in [1, 4] {
                let tester = new_tester();
                let leaves: Vec<SkyKey> = vec![k("leaf1"), k("leaf2"), k("leaf3")];
                tester
                    .set("leaf1", "leaf1")
                    .set("leaf2", "leaf2")
                    .set("leaf3", "leaf3")
                    .set("leaf4", "leaf4");
                tester.set_builder("top", move |_, env| {
                    env.get_values_and_exceptions(&leaves)?;
                    if two_calls && env.values_missing() {
                        return Ok(None);
                    }
                    let first = if same_first { k("leaf1") } else { k("leaf4") };
                    let second = if same_first { k("leaf4") } else { k("leaf3") };
                    env.get_values_and_exceptions(&[first, second])?;
                    if env.values_missing() {
                        return Ok(None);
                    }
                    Ok(Some("top".into()))
                });
                let ev = tester_evaluator(tester);
                eval(&ev, false, p, &[k("top")]);
                let r = eval(&ev, false, p, &[k("top")]);
                assert_eq!(*r.get(&T("top")).unwrap(), "top");
                // A key is recorded in the first group that requested it only.
                let deps = ev.get_direct_deps(&k("top"));
                assert_eq!(deps, vec![k("leaf1"), k("leaf2"), k("leaf3"), k("leaf4")]);
            }
        }
    }
}

// Java: ParallelEvaluatorTest.getValueOrThrowWithErrors
#[test]
fn get_value_or_throw_with_errors() {
    for keep_going in [false, true] {
        for p in [1, 4] {
            let tester = new_tester();
            let deps: Vec<SkyKey> = vec![k("child1"), k("child2"), k("child3")];
            tester
                .set_builder("errorChild", |_, _| {
                    Err(
                        SkyFunctionException::persistent(SomeErrorException("child error".into()))
                            .into(),
                    )
                })
                .set("child1", "child1")
                .set("child2", "child2")
                .set("child3", "child3")
                .set_builder("parent", move |_, env| {
                    match env.get_value_or_throw::<T, SomeErrorException>(T("errorChild"))? {
                        Ok(None) => return Ok(None),
                        Ok(Some(_)) => {}
                        Err(_) => {
                            // Recover from the child error.
                        }
                    }
                    env.get_values_and_exceptions(&deps)?;
                    if env.values_missing() {
                        return Ok(None);
                    }
                    Err(
                        SkyFunctionException::persistent(SomeErrorException("parent error".into()))
                            .into(),
                    )
                });
            let ev = tester_evaluator(tester);
            let r = eval(&ev, keep_going, p, &[k("parent")]);
            assert!(r.has_error());
            assert_eq!(
                exception_message(r.get_first_error().unwrap()),
                if keep_going {
                    "parent error"
                } else {
                    "child error"
                }
            );
        }
    }
}

// Java: ParallelEvaluatorTest.getValuesAndExceptions
#[test]
fn get_values_and_exceptions() {
    for p in [1, 4] {
        let tester = new_tester();
        let top_exception: SkyError = Arc::new(SomeErrorException("top exception".into()));
        let num_computes = Arc::new(AtomicUsize::new(0));
        tester
            .set("other", "other")
            .set("another", "another")
            .set_has_error("errorExpected", true);
        {
            let (top_exception, num_computes) = (top_exception.clone(), num_computes.clone());
            tester.set_builder("top", move |_, env| {
                let dep_keys = [k("other"), k("another"), k("errorExpected")];
                let skyframe_lookup_result = env.get_values_and_exceptions(&dep_keys)?;
                // PORT: Java's first run (numComputes == 1) finds nothing done and returns null;
                // the one run here is Java's second.
                assert_eq!(num_computes.fetch_add(1, Ordering::SeqCst), 0);
                let value1 = skyframe_lookup_result.get(&k("other"));
                assert!(value1.is_some());
                assert!(!env.values_missing());
                match skyframe_lookup_result.get_or_throw::<SomeErrorException>(&k("another")) {
                    Ok(value2) => {
                        assert!(value2.is_some());
                        assert!(!env.values_missing());
                    }
                    Err(e) => panic!("Should not have thrown {e}"),
                }
                match skyframe_lookup_result.get_or_throw::<SomeErrorException>(&k("errorExpected"))
                {
                    Ok(_) => panic!("Should throw"),
                    Err(_) => assert!(!env.values_missing()),
                }
                Err(
                    SkyFunctionException::from_cause(top_exception.clone(), Transience::Persistent)
                        .into(),
                )
            });
        }
        let ev = tester_evaluator(tester);
        let r = eval(&ev, true, p, &[k("top")]);
        assert!(r.has_error());
        assert!(Arc::ptr_eq(
            r.get_error(&k("top")).unwrap().get_exception().unwrap(),
            &top_exception
        ));
        assert_eq!(num_computes.load(Ordering::SeqCst), 1);
    }
}

// Java: ParallelEvaluatorTest.getValuesAndExceptionsWithErrors
#[test]
fn get_values_and_exceptions_with_errors() {
    for p in [1, 4] {
        let tester = new_tester();
        let child_exn: SkyError = Arc::new(SomeErrorException("child error".into()));
        let num_computes = Arc::new(AtomicUsize::new(0));
        {
            let child_exn = child_exn.clone();
            tester.set_builder("error", move |_, _| {
                Err(
                    SkyFunctionException::from_cause(child_exn.clone(), Transience::Persistent)
                        .into(),
                )
            });
        }
        {
            let num_computes = num_computes.clone();
            tester.set_builder("parent", move |_, env| {
                match env
                    .get_values_and_exceptions(&[k("error")])?
                    .get_or_throw::<SomeOtherErrorException>(&k("error"))
                {
                    Ok(value) => assert!(value.is_none()),
                    Err(e) => panic!("Should not have thrown {e}"),
                }
                num_computes.fetch_add(1, Ordering::SeqCst);
                assert!(env.values_missing());
                Ok(None)
            });
        }
        let ev = tester_evaluator(tester);
        let r = eval(&ev, true, p, &[k("parent")]);
        assert!(r.has_error());
        assert!(Arc::ptr_eq(
            r.get_error(&k("parent")).unwrap().get_exception().unwrap(),
            &child_exn
        ));
        // PORT: 2 in Java (the run before "error" is done, and the restart).
        assert_eq!(num_computes.load(Ordering::SeqCst), 1);
    }
}

// Java: ParallelEvaluatorTest.duplicateCycles
#[test]
fn duplicate_cycles() {
    for_each_mode(
        |t| {
            t.add_dependency("loop1", "loop2")
                .add_dependency("loop2", "loop1")
                .add_dependency("parent1", "loop1")
                .add_dependency("parent2", "loop2")
                .add_dependency("grandparent", "parent1")
                .add_dependency("grandparent", "parent2");
        },
        |ev, _| {
            let error_info = eval_value_in_error(ev, &k("grandparent"));
            let cycles: Vec<Vec<SkyKey>> = error_info
                .get_cycle_info()
                .iter()
                .map(|c| c.get_cycle().to_vec())
                .collect();
            // Skyframe doesn't automatically dedupe cycles that are the same except for entry
            // point.
            assert_eq!(cycles.len(), 2);
            // Java: CycleDeduper: cycles that are rotations of one another are the same.
            let mut unique: HashSet<Vec<String>> = HashSet::new();
            for cycle in &cycles {
                let mut names: Vec<String> = cycle.iter().map(|key| format!("{key:?}")).collect();
                let min = (0..names.len()).min_by_key(|&i| names[i].clone()).unwrap();
                names.rotate_left(min);
                unique.insert(names);
            }
            assert_eq!(unique.len(), 1);
        },
    );
}

// Java: ParallelEvaluatorTest.runDepOnErrorHaltsNoKeepGoingBuildEagerly
#[test]
fn run_dep_on_error_halts_no_keep_going_build_eagerly() {
    for child_error_cached in [false, true] {
        for handle_child_error in [false, true] {
            for p in [1, 4] {
                let tester = new_tester();
                tester.set_has_error("child", true);
                // The parent should be built exactly twice: once during normal evaluation and
                // once during error bubbling.
                let num_parent_invocations = Arc::new(AtomicUsize::new(0));
                let n = num_parent_invocations.clone();
                tester.set_builder("parent", move |_, env| {
                    let invocations = n.fetch_add(1, Ordering::SeqCst) + 1;
                    if handle_child_error {
                        match env.get_value_or_throw::<T, SomeErrorException>(T("child"))? {
                            Ok(value) => {
                                // On the first invocation, either the child error should already
                                // be cached and not propagated, or it should be computed freshly
                                // and not propagated. On the second build (error bubbling), the
                                // child error should be propagated.
                                assert!(value.is_none(), "bogus non-null value {value:?}");
                                assert_eq!(
                                    invocations, 1,
                                    "parent incorrectly re-computed during normal evaluation"
                                );
                                assert!(
                                    !env.in_error_bubbling(),
                                    "child error not propagated during error bubbling"
                                );
                                Ok(value.map(|v| (*v).clone()))
                            }
                            Err(_) => {
                                assert!(
                                    env.in_error_bubbling(),
                                    "child error propagated during normal evaluation"
                                );
                                assert_eq!(invocations, 2);
                                Ok(None)
                            }
                        }
                    } else if invocations == 1 {
                        assert!(
                            !env.in_error_bubbling(),
                            "parent's first computation should be during normal evaluation"
                        );
                        Ok(env.get_value(T("child"))?.map(|v| (*v).clone()))
                    } else {
                        assert_eq!(invocations, 2);
                        assert!(
                            env.in_error_bubbling(),
                            "parent incorrectly re-computed during normal evaluation"
                        );
                        Ok(env.get_value(T("child"))?.map(|v| (*v).clone()))
                    }
                });
                let ev = tester_evaluator(tester);
                if child_error_cached {
                    // Ensure that the child is already in the graph.
                    eval_value_in_error(&ev, &k("child"));
                }
                let r = eval(&ev, false, p, &[k("parent")]);
                assert_eq!(num_parent_invocations.load(Ordering::SeqCst), 2);
                assert!(r.get_error(&k("parent")).is_some());
            }
        }
    }
}

// Java: ParallelEvaluatorTest.raceConditionWithNoKeepGoingErrors_FutureError
// PORT: Java holds the commit of "errorKey" (a graph listener on SET_VALUE) until "otherParentKey"
// was signalled, to show the evaluator does not restart it. There is no restart to race with
// here; "otherKey" waits until the evaluation has failed instead, which keeps the assertions:
// each parent runs once in normal evaluation, and only "errorParentKey" is re-run, in error
// bubbling.
#[test]
fn race_condition_with_no_keep_going_errors_future_error() {
    for _ in 0..10 {
        let other_started = Latch::new(1);
        let num_other_parent_invocations = Arc::new(AtomicUsize::new(0));
        let num_error_parent_invocations = Arc::new(AtomicUsize::new(0));
        let tester = new_tester();
        {
            let n = num_other_parent_invocations.clone();
            tester.set_builder("otherParentKey", move |_, env| {
                let invocations = n.fetch_add(1, Ordering::SeqCst) + 1;
                assert_eq!(invocations, 1, "otherParentKey should not be restarted");
                Ok(env.get_value(T("otherKey"))?.map(|v| (*v).clone()))
            });
        }
        {
            let other_started = other_started.clone();
            tester.set_builder("otherKey", move |_, env| {
                other_started.count_down();
                wait_for_exception(env, "error didn't get committed to the graph in time");
                Ok(Some("other".into()))
            });
        }
        tester.set_builder("errorKey", move |_, _| {
            other_started.wait("other didn't start in time");
            Err(SkyFunctionException::persistent(SomeErrorException("error".into())).into())
        });
        {
            let n = num_error_parent_invocations.clone();
            tester.set_builder("errorParentKey", move |_, env| {
                let invocations = n.fetch_add(1, Ordering::SeqCst) + 1;
                match env.get_value_or_throw::<T, SomeErrorException>(T("errorKey"))? {
                    Ok(value) => {
                        assert!(value.is_none(), "bogus non-null value {value:?}");
                        if invocations == 1 {
                            Ok(None)
                        } else {
                            assert!(!env.in_error_bubbling());
                            panic!("RACE CONDITION: errorParentKey was restarted!");
                        }
                    }
                    Err(_) => {
                        assert!(
                            env.in_error_bubbling(),
                            "child error propagated during normal evaluation"
                        );
                        assert_eq!(invocations, 2);
                        Ok(None)
                    }
                }
            });
        }
        let ev = tester_evaluator(tester);
        let r = eval(&ev, false, 4, &[k("otherParentKey"), k("errorParentKey")]);
        assert!(r.has_error());
        assert!(r.get_error(&k("errorParentKey")).is_some());
        assert_eq!(num_other_parent_invocations.load(Ordering::SeqCst), 1);
        assert_eq!(num_error_parent_invocations.load(Ordering::SeqCst), 2);
    }
}

// Java: ParallelEvaluatorTest.cachedErrorsFromKeepGoingUsedOnNoKeepGoing
#[test]
fn cached_errors_from_keep_going_used_on_no_keep_going() {
    for_each_mode(
        |t| {
            t.add_dependency("parent1", "error")
                .set("parent1", "parent1")
                .add_dependency("parent2", "error")
                .set("parent2", "parent2")
                .set_has_error("error", true);
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("parent1")]);
            singleton_error(&r, &k("parent1"));
            let r = eval(ev, false, p, &[k("parent2")]);
            singleton_error(&r, &k("parent2"));
        },
    );
}

// Java: ParallelEvaluatorTest.cachedTopLevelErrorsShouldHaltNoKeepGoingBuildEarly
#[test]
fn cached_top_level_errors_should_halt_no_keep_going_build_early() {
    let tester = new_tester();
    tester.set_has_error("error", true);
    let ev = tester_evaluator(tester.clone());
    let r = eval(&ev, true, 2, &[k("error")]);
    singleton_error(&r, &k("error"));
    tester.set_builder("rogue", |_, _| {
        // This SkyFunction could do an arbitrarily bad computation, e.g. loop-forever. So we want
        // to make sure that it is never run when we want to fail-fast anyway.
        panic!("eval call should have already terminated");
    });
    let r = eval(&ev, false, 2, &[k("error"), k("rogue")]);
    assert_eq!(r.error_map().len(), 1);
    assert!(r.get_error(&k("error")).is_some());
    assert!(r.get_error(&k("rogue")).is_none());
}

// Java: ParallelEvaluatorTest.runUnhandledTransitiveErrors
#[test]
fn run_unhandled_transitive_errors() {
    for keep_going in [false, true] {
        for explicitly_propagate_error in [false, true] {
            for p in [1, 4] {
                let tester = new_tester();
                let error_propagated = Arc::new(AtomicBool::new(false));
                let ep = error_propagated.clone();
                tester
                    .set_builder("grandparent", move |_, env| {
                        match env.get_value_or_throw::<T, SomeErrorException>(T("parent"))? {
                            Ok(v) => Ok(v.map(|v| (*v).clone())),
                            Err(e) => {
                                ep.store(true, Ordering::SeqCst);
                                Err(SkyFunctionException::persistent(e).into())
                            }
                        }
                    })
                    .set_builder("parent", move |_, env| {
                        if explicitly_propagate_error {
                            match env.get_value_or_throw::<T, SomeErrorException>(T("child"))? {
                                Ok(v) => Ok(v.map(|v| (*v).clone())),
                                Err(e) => Err(SkyFunctionException::persistent(e).into()),
                            }
                        } else {
                            Ok(env.get_value(T("child"))?.map(|v| (*v).clone()))
                        }
                    })
                    .set_has_error("child", true);
                let ev = tester_evaluator(tester);
                let r = eval(&ev, keep_going, p, &[k("grandparent")]);
                assert!(error_propagated.load(Ordering::SeqCst));
                singleton_error(&r, &k("grandparent"));
            }
        }
    }
}

// Java: ParallelEvaluatorTest.testDepOnCachedNodeThatItselfDependsOnBothCycleAndError
#[test]
fn test_dep_on_cached_node_that_itself_depends_on_both_cycle_and_error() {
    for p in [1, 4] {
        let tester = new_tester();
        tester
            .add_dependency("cycle", "cycle")
            .set_has_error("error", true)
            .add_dependency("child", "cycle")
            .add_dependency("child", "error");
        let ev = tester_evaluator(tester.clone());
        let r = eval(&ev, true, p, &[k("child")]);
        let error = r.get_error(&k("child")).unwrap();
        assert_eq!(
            error.get_cycle_info(),
            &[CycleInfo::create_cycle_info(
                vec![k("child")],
                vec![k("cycle")]
            )]
        );
        assert!(exception_message(error).contains("error"));

        tester.set_builder("parent", |_, env| {
            let _unused_lookup_result = env.get_values_and_exceptions(&[k("child")])?;
            // env.valuesMissing() should always return true given the graph structure staged
            // above, regardless of the cached state. (This test case specifically stages a fully
            // cached graph state and picks on the getValuesAndExceptions method, because that was
            // the situation of the bug for which this is a regression test.)
            //
            // If childKey is legit not yet in the graph, then of course env.valuesMissing()
            // should return true.
            //
            // If childKey is in the graph, then env.valuesMissing() should still return true
            // because childKey transitively depends on a cycle.
            assert!(env.values_missing());
            Ok(None)
        });
        let r = eval(&ev, true, p, &[k("parent")]);
        let error = r.get_error(&k("parent")).unwrap();
        assert_eq!(
            error.get_cycle_info(),
            &[CycleInfo::create_cycle_info(
                vec![k("parent"), k("child")],
                vec![k("cycle")]
            )]
        );
        assert!(exception_message(error).contains("error"));
    }
}
