//! Ports of the `MemoizingEvaluatorTest` methods about `SkyFunction.Reset` and rewinding
//! (`resetSupported()`), run on the in-memory evaluator, which keeps edges
//! (`incrementalitySupported()`).

use super::*;
use crate::graph_inconsistency_receiver::{GraphInconsistencyReceiver, Inconsistency};
use crate::reset::Reset;
use crate::walkable_graph::WalkableGraph;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// A key of these tests; its value is a `String` (Java's `StringValue`).
#[derive(Clone, PartialEq, Eq, Hash)]
struct R(&'static str);

impl fmt::Debug for R {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Key for R {
    type Value = String;
    const FUNCTION: SkyFunctionName = SkyFunctionName("REWIND_TEST");
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
}

fn r(name: &'static str) -> SkyKey {
    SkyKey::new(R(name))
}

type RBuilder = Arc<dyn Fn(&mut Environment<'_>) -> ComputeResult + Send + Sync>;

/// The functions of the test keys (Java: `tester.getOrCreate(key).setBuilder(...)`).
#[derive(Default)]
struct Builders(Mutex<HashMap<&'static str, RBuilder>>);

impl Builders {
    fn set(
        &self,
        name: &'static str,
        f: impl Fn(&mut Environment<'_>) -> ComputeResult + Send + Sync + 'static,
    ) {
        self.0.lock().insert(name, Arc::new(f));
    }

    /// `setConstantValue(new StringValue(value))`.
    fn set_constant(&self, name: &'static str, value: &'static str) {
        self.set(name, move |_| {
            Ok(Some(Arc::new(value.to_string()) as SkyValue))
        });
    }
}

/// Java: `InconsistencyData`, recorded by `recordInconsistencies()`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct InconsistencyData {
    key: SkyKey,
    other_keys: Option<Vec<SkyKey>>,
    inconsistency: Inconsistency,
}

impl InconsistencyData {
    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java InconsistencyData.resetRequested
    fn reset_requested(key: SkyKey) -> InconsistencyData {
        InconsistencyData {
            key,
            other_keys: None,
            inconsistency: Inconsistency::ResetRequested,
        }
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java InconsistencyData.rewind
    fn rewind(key: SkyKey, others: Vec<SkyKey>) -> InconsistencyData {
        InconsistencyData {
            key,
            other_keys: Some(others),
            inconsistency: Inconsistency::ParentForceRebuildOfChild,
        }
    }
}

#[derive(Default)]
struct RecordingReceiver(Mutex<Vec<InconsistencyData>>);

impl GraphInconsistencyReceiver for RecordingReceiver {
    fn note_inconsistency_and_maybe_throw(
        &self,
        key: &SkyKey,
        other_keys: Option<&[SkyKey]>,
        inconsistency: Inconsistency,
    ) {
        self.0.lock().push(InconsistencyData {
            key: key.clone(),
            other_keys: other_keys.map(|k| k.to_vec()),
            inconsistency,
        });
    }
}

fn evaluator(builders: Arc<Builders>, receiver: Arc<RecordingReceiver>) -> Evaluator {
    EvaluatorBuilder::new()
        .graph_inconsistency_receiver(receiver)
        .function(
            R::FUNCTION,
            move |key: &SkyKey, env: &mut Environment<'_>| {
                let name = key.downcast::<R>().expect("an R key").0;
                let f = builders.0.lock().get(name).cloned().expect("a builder");
                f(env)
            },
        )
        .build()
}

/// Java: `RewindableFunction`: `"stale"` on its first call, `"fresh"` afterwards.
#[derive(Default)]
struct RewindableFunction {
    calls: AtomicUsize,
}

const STALE_VALUE: &str = "stale";
const FRESH_VALUE: &str = "fresh";

impl RewindableFunction {
    fn compute(&self) -> ComputeResult {
        let calls = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(Some(
            Arc::new(if calls == 1 { STALE_VALUE } else { FRESH_VALUE }.to_string()) as SkyValue,
        ))
    }
}

fn string_of(value: &SkyValue) -> &str {
    value.downcast_ref::<String>().expect("a String value")
}

fn result_of(result: &EvaluationResult, key: &SkyKey) -> String {
    string_of(result.get_dyn(key).expect("a value")).to_owned()
}

fn sorted(mut keys: Vec<SkyKey>) -> Vec<String> {
    let mut names: Vec<String> = keys.drain(..).map(|k| format!("{k:?}")).collect();
    names.sort();
    names
}

fn rdeps(ev: &Evaluator, key: &SkyKey) -> Vec<String> {
    sorted(
        ev.get_reverse_deps(std::slice::from_ref(key))
            .into_iter()
            .flat_map(|(_, rdeps)| rdeps)
            .collect(),
    )
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2444 MemoizingEvaluatorTest.resetSelfOnly_singleDep_initialBuild
#[test]
fn reset_self_only_single_dep_initial_build() {
    let builders = Arc::new(Builders::default());
    let receiver = Arc::new(RecordingReceiver::default());
    let already_reset = Arc::new(AtomicBool::new(false));
    let top = r("top");
    let dep = r("dep");
    {
        let dep = dep.clone();
        let top = top.clone();
        builders.set("top", move |env| {
            if env.get_value_dyn(&dep)?.is_none() {
                return Ok(None);
            }
            if !already_reset.swap(true, Ordering::SeqCst) {
                return Ok(Some(Arc::new(Reset::self_only(top.clone())) as SkyValue));
            }
            Ok(Some(Arc::new("topVal".to_string()) as SkyValue))
        });
    }
    builders.set_constant("dep", "depVal");
    let ev = evaluator(builders, receiver.clone());

    let result = eval(&ev, false, 2, std::slice::from_ref(&top));

    assert_eq!(result_of(&result, &top), "topVal");
    assert_eq!(
        *receiver.0.lock(),
        vec![InconsistencyData::reset_requested(top.clone())]
    );
    assert_eq!(rdeps(&ev, &dep), vec!["top".to_string()]);
    assert_eq!(sorted(ev.get_direct_deps(&top)), vec!["dep".to_string()]);
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2664 MemoizingEvaluatorTest.resetSelfOnly_depNotRequestedAgainAfterReset
#[test]
fn reset_self_only_dep_not_requested_again_after_reset() {
    let builders = Arc::new(Builders::default());
    let receiver = Arc::new(RecordingReceiver::default());
    let already_reset = Arc::new(AtomicBool::new(false));
    let top = r("top");
    let flaky_dep = r("flakyDep");
    let stable_dep = r("stableDep");
    {
        let (top, flaky_dep, stable_dep) = (top.clone(), flaky_dep.clone(), stable_dep.clone());
        builders.set("top", move |env| {
            let reset = already_reset.load(Ordering::SeqCst);
            let keys = if reset {
                vec![stable_dep.clone()]
            } else {
                vec![stable_dep.clone(), flaky_dep.clone()]
            };
            env.get_values_and_exceptions(&keys)?;
            if env.values_missing() {
                return Ok(None);
            }
            if !reset {
                already_reset.store(true, Ordering::SeqCst);
                return Ok(Some(Arc::new(Reset::self_only(top.clone())) as SkyValue));
            }
            Ok(Some(Arc::new("topVal".to_string()) as SkyValue))
        });
    }
    builders.set_constant("stableDep", "stableDepVal");
    builders.set_constant("flakyDep", "flakyDepVal");
    let ev = evaluator(builders, receiver.clone());

    let result = eval(&ev, false, 2, std::slice::from_ref(&top));

    assert_eq!(result_of(&result, &top), "topVal");
    assert_eq!(
        *receiver.0.lock(),
        vec![InconsistencyData::reset_requested(top.clone())]
    );
    assert_eq!(
        sorted(ev.get_direct_deps(&top)),
        vec!["stableDep".to_string()]
    );
    assert_eq!(rdeps(&ev, &stable_dep), vec!["top".to_string()]);
    assert!(rdeps(&ev, &flaky_dep).is_empty());
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2768 MemoizingEvaluatorTest.rewindOneDep
#[test]
fn rewind_one_dep() {
    let builders = Arc::new(Builders::default());
    let receiver = Arc::new(RecordingReceiver::default());
    let rewindable_function = Arc::new(RewindableFunction::default());
    let top = r("top");
    let dep = r("dep");
    {
        let f = rewindable_function.clone();
        builders.set("dep", move |_| f.compute());
    }
    {
        let (top, dep) = (top.clone(), dep.clone());
        builders.set("top", move |env| {
            let Some(dep_value) = env.get_value_dyn(&dep)? else {
                return Ok(None);
            };
            if string_of(&dep_value) == STALE_VALUE {
                let mut rewind_graph = Reset::new_rewind_graph_for(top.clone());
                rewind_graph.put_edge(top.clone(), dep.clone());
                return Ok(Some(Arc::new(Reset::of(rewind_graph)) as SkyValue));
            }
            assert_eq!(string_of(&dep_value), FRESH_VALUE);
            Ok(Some(Arc::new("topVal".to_string()) as SkyValue))
        });
    }
    let ev = evaluator(builders, receiver.clone());

    let result = eval(&ev, false, 2, std::slice::from_ref(&top));

    assert_eq!(result_of(&result, &top), "topVal");
    let recorded = receiver.0.lock().clone();
    assert_eq!(recorded.len(), 2, "{recorded:?}");
    assert!(recorded.contains(&InconsistencyData::reset_requested(top.clone())));
    assert!(recorded.contains(&InconsistencyData::rewind(top.clone(), vec![dep.clone()])));
    assert_eq!(rewindable_function.calls.load(Ordering::SeqCst), 2);
    assert_eq!(rdeps(&ev, &dep), vec!["top".to_string()]);
    assert_eq!(sorted(ev.get_direct_deps(&top)), vec!["dep".to_string()]);
}

// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3122 MemoizingEvaluatorTest.rewindTransitiveDep
#[test]
fn rewind_transitive_dep() {
    let builders = Arc::new(Builders::default());
    let receiver = Arc::new(RecordingReceiver::default());
    let rewindable_function = Arc::new(RewindableFunction::default());
    let top = r("top");
    let mid = r("mid");
    let bottom = r("bottom");
    {
        let f = rewindable_function.clone();
        builders.set("bottom", move |_| f.compute());
    }
    {
        // addDependency(bottom).setComputedValue(COPY).
        let bottom = bottom.clone();
        builders.set("mid", move |env| {
            Ok(env
                .get_value_dyn(&bottom)?
                .map(|v| Arc::new(string_of(&v).to_owned()) as SkyValue))
        });
    }
    {
        let (top, mid, bottom) = (top.clone(), mid.clone(), bottom.clone());
        builders.set("top", move |env| {
            let Some(mid_value) = env.get_value_dyn(&mid)? else {
                return Ok(None);
            };
            if string_of(&mid_value) == STALE_VALUE {
                let mut rewind_graph = Reset::new_rewind_graph_for(top.clone());
                rewind_graph.put_edge(top.clone(), mid.clone());
                rewind_graph.put_edge(mid.clone(), bottom.clone());
                return Ok(Some(Arc::new(Reset::of(rewind_graph)) as SkyValue));
            }
            assert_eq!(string_of(&mid_value), FRESH_VALUE);
            Ok(Some(Arc::new("topVal".to_string()) as SkyValue))
        });
    }
    let ev = evaluator(builders, receiver.clone());

    let result = eval(&ev, false, 2, std::slice::from_ref(&top));

    assert_eq!(result_of(&result, &top), "topVal");
    let recorded = receiver.0.lock().clone();
    assert_eq!(recorded.len(), 2, "{recorded:?}");
    assert!(recorded.contains(&InconsistencyData::reset_requested(top.clone())));
    // Postorder: bottom before mid.
    assert!(recorded.contains(&InconsistencyData::rewind(
        top.clone(),
        vec![bottom.clone(), mid.clone()]
    )));
    assert_eq!(rewindable_function.calls.load(Ordering::SeqCst), 2);
    assert_eq!(sorted(ev.get_direct_deps(&top)), vec!["mid".to_string()]);
    assert_eq!(rdeps(&ev, &mid), vec!["top".to_string()]);
    assert_eq!(sorted(ev.get_direct_deps(&mid)), vec!["bottom".to_string()]);
    assert_eq!(rdeps(&ev, &bottom), vec!["mid".to_string()]);
}
