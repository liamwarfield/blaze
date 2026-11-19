use super::*;
use blaze_events::{Event, EventKind, StoredEventHandler};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

fn ctx(keep_going: bool, parallelism: usize) -> EvaluationContext {
    EvaluationContext {
        keep_going,
        parallelism,
        thread_stack_size: 16 << 20,
        ..EvaluationContext::default()
    }
}

fn eval(
    ev: &Evaluator,
    keep_going: bool,
    parallelism: usize,
    roots: &[SkyKey],
) -> EvaluationResult {
    ev.evaluate(roots, &ctx(keep_going, parallelism))
        .expect("not interrupted")
}

// ---------------------------------------------------------------------------------------------
// Num / Cyc / Flag: the original engine tests.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Num(u64);
impl Key for Num {
    type Value = u64;
    const FUNCTION: SkyFunctionName = SkyFunctionName("NUM");
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Cyc(u32);
impl Key for Cyc {
    type Value = u32;
    const FUNCTION: SkyFunctionName = SkyFunctionName("CYC");
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Flag(&'static str);
impl Key for Flag {
    type Value = String;
    const FUNCTION: SkyFunctionName = SkyFunctionName("FLAG");
}

#[derive(Debug, Clone, PartialEq)]
struct BadNumber(u64);
impl fmt::Display for BadNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bad number {}", self.0)
    }
}
impl std::error::Error for BadNumber {}

/// num(n) = sum of num(d) for d in 0..n step 7 (+1); num(13) throws.
fn num_fn(k: &Num, env: &mut Environment<'_>) -> Result<Option<u64>, ComputeError> {
    if k.0 == 13 {
        return Err(SkyFunctionException::persistent(BadNumber(13)).into());
    }
    if k.0 == 0 {
        return Ok(Some(1));
    }
    let deps: Vec<Num> = (0..k.0).step_by(7).map(Num).collect();
    let vals = env.get_values_typed(&deps)?;
    if env.values_missing() {
        return Ok(None);
    }
    Ok(Some(vals.into_iter().map(|v| *v.unwrap()).sum::<u64>() + 1))
}

fn evaluator() -> Evaluator {
    EvaluatorBuilder::new()
        .typed_function::<Num, _>(num_fn)
        .typed_function::<Cyc, _>(|k: &Cyc, env| {
            // 1 -> 2 -> 3 -> 1 is a cycle; 5 -> 5 is a self-cycle; 10 -> {11, 12} with 12 -> 10.
            let next: Vec<Cyc> = match k.0 {
                1 => vec![Cyc(2)],
                2 => vec![Cyc(3)],
                3 => vec![Cyc(1)],
                5 => vec![Cyc(5)],
                10 => vec![Cyc(11), Cyc(12)],
                12 => vec![Cyc(10)],
                _ => vec![],
            };
            let vals = env.get_values_typed(&next)?;
            if env.values_missing() {
                return Ok(None);
            }
            Ok(Some(vals.len() as u32))
        })
        .typed_function::<Flag, _>(|_, _| panic!("injected keys are never computed"))
        .inject(Flag("compilation_mode"), "fastbuild".to_string())
        .build()
}

fn cyc(n: u32) -> SkyKey {
    SkyKey::new(Cyc(n))
}

#[test]
fn computes_and_memoizes() {
    let ev = evaluator();
    let r = eval(&ev, true, 8, &[SkyKey::new(Num(30))]);
    assert!(!r.has_error());
    let first = *r.get(&Num(30)).unwrap();
    let computed = ev.computed_count();
    let r2 = eval(&ev, true, 8, &[SkyKey::new(Num(30))]);
    assert_eq!(*r2.get(&Num(30)).unwrap(), first);
    assert_eq!(
        ev.computed_count(),
        computed,
        "second evaluation reuses the graph"
    );
}

#[test]
fn single_threaded_matches_parallel() {
    let a = eval(&evaluator(), true, 1, &[SkyKey::new(Num(50))]);
    let b = eval(&evaluator(), true, 16, &[SkyKey::new(Num(50))]);
    assert_eq!(a.get(&Num(50)), b.get(&Num(50)));
}

#[test]
fn error_of_root() {
    for keep_going in [false, true] {
        let ev = evaluator();
        let r = eval(&ev, keep_going, 4, &[SkyKey::new(Num(13))]);
        let err = r.get_first_error().expect("error");
        assert_eq!(err.exception_as::<BadNumber>(), Some(&BadNumber(13)));
        assert!(err.get_cycle_info().is_empty());
        assert!(!err.is_transitively_transient());
    }
}

#[test]
fn get_value_or_throw_catches() {
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct Catcher;
    impl Key for Catcher {
        type Value = String;
        const FUNCTION: SkyFunctionName = SkyFunctionName("CATCHER");
    }
    let ev = EvaluatorBuilder::new()
        .typed_function::<Num, _>(num_fn)
        .typed_function::<Catcher, _>(|_, env| {
            match env.get_value_or_throw::<Num, BadNumber>(Num(13))? {
                Ok(None) => Ok(None),
                Ok(Some(_)) => Ok(Some("no error".into())),
                Err(e) => Ok(Some(format!("caught: {e}"))),
            }
        })
        .build();
    // keep_going: the catcher's value stands (with the child's error attached).
    let r = eval(&ev, true, 2, &[SkyKey::new(Catcher)]);
    assert_eq!(*r.get(&Catcher).unwrap(), "caught: bad number 13");
    let stored = ev
        .get_existing_error_for_testing(&SkyKey::new(Catcher))
        .expect("error with value");
    assert_eq!(stored.exception_as::<BadNumber>(), Some(&BadNumber(13)));
    assert!(ev.get_existing_value(&SkyKey::new(Catcher)).is_some());

    // no keep_going: the child's exception does not reach the parent in normal evaluation (the
    // dep is null); in error bubbling it does, and the error bubbles through the parent whatever
    // it returns.
    let ev2 = EvaluatorBuilder::new()
        .typed_function::<Num, _>(num_fn)
        .typed_function::<Catcher, _>(|_, env| {
            match env.get_value_or_throw::<Num, BadNumber>(Num(13))? {
                Ok(None) => Ok(None),
                Ok(Some(_)) => Ok(Some("no error".into())),
                Err(e) => Ok(Some(format!("caught: {e}"))),
            }
        })
        .build();
    let r = eval(&ev2, false, 2, &[SkyKey::new(Catcher)]);
    assert!(r.get(&Catcher).is_none());
    let err = r.get_error(&SkyKey::new(Catcher)).expect("bubbled");
    assert_eq!(err.exception_as::<BadNumber>(), Some(&BadNumber(13)));
}

#[test]
fn injected_values() {
    let ev = evaluator();
    let r = eval(&ev, true, 2, &[SkyKey::new(Flag("compilation_mode"))]);
    assert_eq!(*r.get(&Flag("compilation_mode")).unwrap(), "fastbuild");
}

#[test]
fn inject_between_evaluations_invalidates_dependents() {
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct Mode;
    impl Key for Mode {
        type Value = String;
        const FUNCTION: SkyFunctionName = SkyFunctionName("MODE");
    }
    let ev = EvaluatorBuilder::new()
        .typed_function::<Mode, _>(|_, env| {
            let Some(f) = env.get_value(Flag("compilation_mode"))? else {
                return Ok(None);
            };
            Ok(Some(format!("mode={f}")))
        })
        .inject(Flag("compilation_mode"), "fastbuild".to_string())
        .build();
    let r = eval(&ev, false, 2, &[SkyKey::new(Mode)]);
    assert_eq!(*r.get(&Mode).unwrap(), "mode=fastbuild");
    ev.inject(Flag("compilation_mode"), "opt".to_string());
    let r = eval(&ev, false, 2, &[SkyKey::new(Mode)]);
    assert_eq!(*r.get(&Mode).unwrap(), "mode=opt");
}

fn cycle_of(ev: &Evaluator, keep_going: bool, parallelism: usize, root: u32) -> CycleInfo {
    let r = eval(ev, keep_going, parallelism, &[cyc(root)]);
    assert!(r.has_error(), "expected a cycle from {root}");
    let err = r.get_error(&cyc(root)).expect("root error");
    assert!(err.get_exception().is_none());
    assert_eq!(err.get_cycle_info().len(), 1, "{err:?}");
    err.get_cycle_info()[0].clone()
}

#[test]
fn self_cycle_is_detected() {
    for keep_going in [false, true] {
        for p in [1, 4] {
            let c = cycle_of(&evaluator(), keep_going, p, 5);
            assert_eq!(c, CycleInfo::create_cycle_info(vec![], vec![cyc(5)]));
        }
    }
}

#[test]
fn three_node_cycle_is_reported_from_the_top_level_key() {
    for keep_going in [false, true] {
        for p in [1, 4] {
            let c = cycle_of(&evaluator(), keep_going, p, 1);
            assert_eq!(
                c,
                CycleInfo::create_cycle_info(vec![], vec![cyc(1), cyc(2), cyc(3)])
            );
            let c = cycle_of(&evaluator(), keep_going, p, 2);
            assert_eq!(
                c,
                CycleInfo::create_cycle_info(vec![], vec![cyc(2), cyc(3), cyc(1)])
            );
        }
    }
}

#[test]
fn cycle_through_parallel_batch_is_detected() {
    // 10 requests {11, 12} as a batch; a helper computes 12, which asks for 10 (on the parent).
    for _ in 0..20 {
        for keep_going in [false, true] {
            let c = cycle_of(&evaluator(), keep_going, 8, 10);
            assert_eq!(
                c,
                CycleInfo::create_cycle_info(vec![], vec![cyc(10), cyc(12)])
            );
        }
    }
}

#[test]
fn many_roots_in_parallel() {
    let ev = evaluator();
    let roots: Vec<SkyKey> = (0..200u64)
        .filter(|n| n % 7 != 6 && *n != 13)
        .map(|n| SkyKey::new(Num(n)))
        .collect();
    let r = eval(&ev, true, 16, &roots);
    // Deps are multiples of 7, never 13.
    assert_eq!(r.errors.len(), 0);
    assert_eq!(r.values.len(), roots.len());
}

#[test]
#[should_panic(expected = "Unable to find SkyFunction 'NUM' for node with key Num(3)")]
fn unregistered_function_panics_with_java_text() {
    let ev = EvaluatorBuilder::new().build();
    let _ = ev.evaluate(&[SkyKey::new(Num(3))], &ctx(false, 2));
}

#[test]
#[should_panic(expected = "function crashed")]
fn function_panic_is_rethrown_without_hanging() {
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct Crash(u32);
    impl Key for Crash {
        type Value = u32;
        const FUNCTION: SkyFunctionName = SkyFunctionName("CRASH");
    }
    let ev = EvaluatorBuilder::new()
        .typed_function::<Crash, _>(|k: &Crash, env| {
            if k.0 == 0 {
                std::thread::sleep(Duration::from_millis(20));
                panic!("function crashed");
            }
            // Many parents wait on the crashing node from several threads.
            let _ = env.get_value(Crash(0))?;
            Ok(None)
        })
        .build();
    let roots: Vec<SkyKey> = (0..8).map(|i| SkyKey::new(Crash(i))).collect();
    let _ = ev.evaluate(&roots, &ctx(true, 8));
}

// ---------------------------------------------------------------------------------------------
// A port of the parts of GraphTester that ParallelEvaluatorTest uses.

#[derive(Clone, PartialEq, Eq, Hash)]
struct T(&'static str);
impl fmt::Debug for T {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Key for T {
    type Value = String;
    const FUNCTION: SkyFunctionName = SkyFunctionName("TEST");
    /// Java: `GraphTester.NODE_TYPE = SkyFunctionName.FOR_TESTING`, created with
    /// `SkyFunctionName.createHermetic("FOR_TESTING")` (SkyFunctionName.java:32). Java's
    /// non-hermetic test key is `GraphTester.NonHermeticKey` (`FOR_TESTING_NONHERMETIC`).
    const HERMETICITY: Option<FunctionHermeticity> = Some(FunctionHermeticity::Hermetic);
    /// Java: `GraphTester.StringValue.equals` compares the strings.
    // Java: src/test/java/com/google/devtools/build/skyframe/GraphTester.java:451 StringValue.equals
    fn values_equal(old: &String, new: &String) -> bool {
        old == new
    }
}

/// Java: `GraphTester.NonHermeticKey`, a key of the non-hermetic function
/// `FOR_TESTING_NONHERMETIC`, served by the same test function as [`T`].
// Java: src/test/java/com/google/devtools/build/skyframe/GraphTester.java:556 GraphTester.NonHermeticKey
#[derive(Clone, PartialEq, Eq, Hash)]
struct N(&'static str);
impl fmt::Debug for N {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Key for N {
    type Value = String;
    const FUNCTION: SkyFunctionName = SkyFunctionName("TEST_NONHERMETIC");
    // Java: src/test/java/com/google/devtools/build/skyframe/GraphTester.java:584 GraphTester.FOR_TESTING_NONHERMETIC
    const HERMETICITY: Option<FunctionHermeticity> = Some(FunctionHermeticity::Nonhermetic);
    // Java: src/test/java/com/google/devtools/build/skyframe/GraphTester.java:451 StringValue.equals
    fn values_equal(old: &String, new: &String) -> bool {
        old == new
    }
}

fn k(name: &'static str) -> SkyKey {
    SkyKey::new(T(name))
}

/// Java: `GraphTester.nonHermeticKey(name)`.
fn nk(name: &'static str) -> SkyKey {
    SkyKey::new(N(name))
}

/// The name of a test key ([`T`] or [`N`]).
fn name_of(key: &SkyKey) -> &'static str {
    if let Some(t) = key.downcast::<T>() {
        t.0
    } else if let Some(n) = key.downcast::<N>() {
        n.0
    } else if let Some(n) = key.downcast::<NotComparableKey>() {
        n.0
    } else {
        panic!("not a test key: {key:?}")
    }
}

/// Java: SomeErrorException.
#[derive(Debug, Clone, PartialEq)]
struct SomeErrorException(String);
impl fmt::Display for SomeErrorException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SomeErrorException {}

type Builder = Arc<dyn Fn(&T, &mut Environment<'_>) -> ComputeResultT + Send + Sync>;
type ComputeResultT = Result<Option<String>, ComputeError>;

/// Java: GraphTester.TestFunction.
#[derive(Default, Clone)]
struct Spec {
    /// (dep, value to use if the dep throws SomeErrorException).
    deps: Vec<(SkyKey, Option<&'static str>)>,
    has_error: bool,
    has_transient_error: bool,
    value: Option<String>,
    warning: Option<String>,
    progress: Option<String>,
    error_event: Option<String>,
    /// Java: TestFunction.postable.
    postable: Option<Arc<dyn blaze_events::Postable>>,
    /// Request all deps as one batch (getValuesAndExceptions) instead of one by one.
    batch: bool,
    /// The function the node runs (Java: `setBuilder`). It gets the key as a [`T`] of the same
    /// name, also for a [`N`] key.
    builder: Option<Builder>,
    /// Java: TestFunction.tag (extractTag).
    tag: Option<String>,
}

/// Java: `GraphTester`: the functions of the test keys, by key.
#[derive(Default)]
struct Tester {
    specs: Mutex<HashMap<SkyKey, Spec>>,
    /// Java: `GraphTester.modifiedValues`.
    modified_values: Mutex<indexmap::IndexSet<SkyKey>>,
}

#[allow(dead_code)] // the whole of Java's GraphTester API, for the ports still to come
impl Tester {
    fn with(&self, name: &'static str, f: impl FnOnce(&mut Spec)) -> &Self {
        self.with_key(k(name), f)
    }
    /// Java: `getOrCreate(key)` and a change to the returned `TestFunction`.
    fn with_key(&self, key: SkyKey, f: impl FnOnce(&mut Spec)) -> &Self {
        f(self.specs.lock().entry(key).or_default());
        self
    }
    /// Java: `getOrCreate(key, markAsModified)`: creates the function if there is none, else
    /// records the key as modified if asked.
    // Java: src/test/java/com/google/devtools/build/skyframe/GraphTester.java:79 GraphTester.getOrCreate
    fn get_or_create(&self, key: SkyKey, mark_as_modified: bool) -> &Self {
        let mut specs = self.specs.lock();
        if let std::collections::hash_map::Entry::Vacant(e) = specs.entry(key.clone()) {
            e.insert(Spec::default());
        } else if mark_as_modified {
            self.modified_values.lock().insert(key);
        }
        self
    }
    fn add_dependency(&self, name: &'static str, dep: &'static str) -> &Self {
        self.add_dependency_key(k(name), k(dep))
    }
    // Java: src/test/java/com/google/devtools/build/skyframe/GraphTester.java:273 TestFunction.addDependency
    fn add_dependency_key(&self, key: SkyKey, dep: SkyKey) -> &Self {
        self.with_key(key, |s| {
            if !s.deps.iter().any(|(d, alt)| *d == dep && alt.is_none()) {
                s.deps.push((dep, None))
            }
        })
    }
    // Java: src/test/java/com/google/devtools/build/skyframe/GraphTester.java:284 TestFunction.removeDependency
    fn remove_dependency_key(&self, key: SkyKey, dep: &SkyKey) -> &Self {
        self.with_key(key, |s| {
            s.deps.retain(|(d, alt)| !(d == dep && alt.is_none()))
        })
    }
    /// Java: `GraphTester.set(key, value)`: `getOrCreate(key, true).setConstantValue(value)`.
    fn set(&self, name: &'static str, value: &str) -> &Self {
        self.set_key(k(name), value)
    }
    // Java: src/test/java/com/google/devtools/build/skyframe/GraphTester.java:97 GraphTester.set
    fn set_key(&self, key: SkyKey, value: &str) -> &Self {
        self.get_or_create(key.clone(), true);
        self.with_key(key, |s| s.value = Some(value.to_string()))
    }
    // Java: src/test/java/com/google/devtools/build/skyframe/GraphTester.java:311 TestFunction.unsetConstantValue
    fn unset_constant_value(&self, key: SkyKey) -> &Self {
        self.with_key(key, |s| s.value = None)
    }
    fn set_has_error(&self, name: &'static str, has_error: bool) -> &Self {
        self.with(name, |s| s.has_error = has_error)
    }
    fn set_has_error_key(&self, key: SkyKey, has_error: bool) -> &Self {
        self.with_key(key, |s| s.has_error = has_error)
    }
    /// Java: TestFunction.addErrorDependency.
    fn add_error_dependency(
        &self,
        name: &'static str,
        dep: &'static str,
        alt: &'static str,
    ) -> &Self {
        self.with(name, |s| s.deps.push((k(dep), Some(alt))))
    }
    fn add_error_dependency_key(&self, key: SkyKey, dep: SkyKey, alt: &'static str) -> &Self {
        self.with_key(key, |s| s.deps.push((dep, Some(alt))))
    }
    fn set_has_transient_error(&self, name: &'static str, has_error: bool) -> &Self {
        self.with(name, |s| s.has_transient_error = has_error)
    }
    fn set_has_transient_error_key(&self, key: SkyKey, has_error: bool) -> &Self {
        self.with_key(key, |s| s.has_transient_error = has_error)
    }
    fn set_warning(&self, name: &'static str, warning: &str) -> &Self {
        self.with(name, |s| s.warning = Some(warning.to_string()))
    }
    fn set_warning_key(&self, key: SkyKey, warning: &str) -> &Self {
        self.with_key(key, |s| s.warning = Some(warning.to_string()))
    }
    fn set_progress(&self, name: &'static str, progress: &str) -> &Self {
        self.with(name, |s| s.progress = Some(progress.to_string()))
    }
    /// Java: TestFunction.setErrorEvent.
    fn set_error_event_key(&self, key: SkyKey, error: &str) -> &Self {
        self.with_key(key, |s| s.error_event = Some(error.to_string()))
    }
    /// Java: TestFunction.setBuilder(ChainedFunction).
    fn set_chained(&self, name: &'static str, f: Chained) -> &Self {
        self.set_builder(name, move |t, env| f.compute(t, env))
    }
    fn set_builder(
        &self,
        name: &'static str,
        f: impl Fn(&T, &mut Environment<'_>) -> ComputeResultT + Send + Sync + 'static,
    ) -> &Self {
        self.set_builder_key(k(name), f)
    }
    fn set_builder_key(
        &self,
        key: SkyKey,
        f: impl Fn(&T, &mut Environment<'_>) -> ComputeResultT + Send + Sync + 'static,
    ) -> &Self {
        self.with_key(key, |s| s.builder = Some(Arc::new(f)))
    }
    fn spec(&self, name: &'static str) -> Spec {
        self.spec_of(&k(name))
    }
    fn spec_of(&self, key: &SkyKey) -> Spec {
        self.specs
            .lock()
            .get(key)
            .cloned()
            .unwrap_or_else(|| panic!("No TestFunction for {key:?}"))
    }
    // Java: src/test/java/com/google/devtools/build/skyframe/GraphTester.java:101 GraphTester.getModifiedValues
    fn get_modified_values(&self) -> Vec<SkyKey> {
        self.modified_values.lock().iter().cloned().collect()
    }
    // Java: src/test/java/com/google/devtools/build/skyframe/GraphTester.java:105 GraphTester.clearModifiedValues
    fn clear_modified_values(&self) {
        self.modified_values.lock().clear();
    }
}

/// A dep's value as a string, or the alternative value if it threw `SomeErrorException`.
fn dep_string(v: Option<SkyValue>) -> Option<String> {
    v.map(|v| {
        v.downcast_ref::<String>()
            .expect("test values are strings")
            .clone()
    })
}

struct TestFunction;

impl SkyFunction for TestFunction {
    // Java: src/test/java/com/google/devtools/build/skyframe/GraphTester.java:110 GraphTester.compute
    fn compute(&self, key: &SkyKey, env: &mut Environment<'_>) -> ComputeResult {
        let t = T(name_of(key));
        let tester = env.client::<Tester>().expect("tester");
        let spec = tester.spec_of(key);
        if let Some(b) = &spec.builder {
            return Ok(b(&t, env)?.map(|v| Arc::new(v) as SkyValue));
        }
        if let Some(w) = &spec.warning {
            env.listener().handle(&Event::warn(w.clone()));
        }
        if let Some(p) = &spec.progress {
            env.listener().handle(&Event::progress(p.clone()));
        }
        if let Some(e) = &spec.error_event {
            env.listener().handle(&Event::error(e.clone()));
        }
        if let Some(p) = &spec.postable {
            env.listener().post(p.clone());
        }
        let mut deps = vec![];
        if spec.batch {
            let keys: Vec<SkyKey> = spec.deps.iter().map(|(d, _)| d.clone()).collect();
            let result = env.get_values_and_exceptions(&keys)?;
            for (d, alt) in &spec.deps {
                let v = match alt {
                    None => dep_string(result.get(d)),
                    Some(alt) => match result.get_or_throw::<SomeErrorException>(d) {
                        Ok(v) => dep_string(v),
                        Err(_) => Some(alt.to_string()),
                    },
                };
                if let Some(v) = v {
                    deps.push(v);
                }
            }
        } else {
            let mut one_missing = false;
            for (d, alt) in &spec.deps {
                let v = match alt {
                    None => dep_string(env.get_value_dyn(d)?),
                    Some(alt) => match env.get_value_or_throw_dyn::<SomeErrorException>(d)? {
                        Ok(v) => dep_string(v),
                        Err(_) => Some(alt.to_string()),
                    },
                };
                match v {
                    None => one_missing = true,
                    Some(v) => deps.push(v),
                }
                assert_eq!(one_missing, env.values_missing(), "{d:?} {one_missing}");
            }
        }
        if env.values_missing() {
            return Ok(None);
        }
        if spec.has_transient_error {
            return Err(
                SkyFunctionException::transient(SomeErrorException(format!("{t:?}"))).into(),
            );
        }
        if spec.has_error {
            return Err(
                SkyFunctionException::persistent(SomeErrorException(format!("{t:?}"))).into(),
            );
        }
        if let Some(v) = spec.value {
            return Ok(Some(Arc::new(v)));
        }
        // CONCATENATE
        Ok(Some(Arc::new(deps.concat())))
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/GraphTester.java:163 GraphTester.extractTag
    fn extract_tag(&self, _key: &SkyKey) -> Option<String> {
        None
    }
}

/// [`TestFunction`] with `extractTag` from the spec (the function has no access to the tester
/// outside `compute`, so the tester is captured).
struct TaggingTestFunction(Arc<Tester>);

impl SkyFunction for TaggingTestFunction {
    fn compute(&self, key: &SkyKey, env: &mut Environment<'_>) -> ComputeResult {
        TestFunction.compute(key, env)
    }
    fn extract_tag(&self, key: &SkyKey) -> Option<String> {
        self.0.spec_of(key).tag
    }
}

/// A `'static` name for a key made at run time.
fn leak(name: String) -> &'static str {
    Box::leak(name.into_boxed_str())
}

/// Java: `evalValueInError(key)`.
fn eval_value_in_error(ev: &Evaluator, key: &SkyKey) -> ErrorInfo {
    eval(ev, true, 4, std::slice::from_ref(key))
        .get_error(key)
        .unwrap_or_else(|| panic!("{key:?} is not in error"))
        .clone()
}

/// Java: `assertThatEvaluationResult(result).hasSingletonErrorThat(key)`.
fn singleton_error<'a>(r: &'a EvaluationResult, key: &SkyKey) -> &'a ErrorInfo {
    assert_eq!(r.errors.len(), 1, "{r:?}");
    assert_eq!(r.errors[0].0, *key, "{r:?}");
    &r.errors[0].1
}

fn exception_message(e: &ErrorInfo) -> String {
    e.get_exception().expect("exception").to_string()
}

/// Java: CountDownLatch, awaited through TrackingAwaiter (a timeout fails the test).
struct Latch {
    count: Mutex<usize>,
    cv: parking_lot::Condvar,
}

impl Latch {
    fn new(count: usize) -> Arc<Latch> {
        Arc::new(Latch {
            count: Mutex::new(count),
            cv: parking_lot::Condvar::new(),
        })
    }
    /// Java: `CountDownLatch.getCount()`.
    fn get_count(&self) -> usize {
        *self.count.lock()
    }
    fn count_down(&self) {
        let mut c = self.count.lock();
        *c = c.saturating_sub(1);
        if *c == 0 {
            self.cv.notify_all();
        }
    }
    fn wait(&self, what: &str) {
        let deadline = std::time::Instant::now() + WAIT_TIMEOUT;
        let mut c = self.count.lock();
        while *c > 0 {
            assert!(!self.cv.wait_until(&mut c, deadline).timed_out(), "{what}");
        }
    }
}

/// Java: TestUtils.WAIT_TIMEOUT_SECONDS.
const WAIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Java: `SkyFunctionEnvironment.getExceptionLatchForTesting().await()`: waits until the
/// evaluation has failed (new evaluations are prevented).
fn wait_for_exception(env: &Environment<'_>, what: &str) {
    let deadline = std::time::Instant::now() + WAIT_TIMEOUT;
    while !env.exception_latch_released_for_testing() {
        assert!(std::time::Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Java: ChainedFunction. A function that counts down a latch when it starts, waits for another
/// one, requests its deps, and throws `SomeErrorException("oops")` if it has no value.
#[derive(Default, Clone)]
struct Chained {
    notify_start: Option<Arc<Latch>>,
    wait_to_finish: Option<Arc<Latch>>,
    notify_finish: Option<Arc<Latch>>,
    wait_for_exception: bool,
    value: Option<&'static str>,
    deps: Vec<&'static str>,
}

impl Chained {
    // Java: src/test/java/com/google/devtools/build/skyframe/ChainedFunction.java:70 ChainedFunction.compute
    fn compute(&self, key: &T, env: &mut Environment<'_>) -> ComputeResultT {
        struct NotifyFinish<'a>(&'a Option<Arc<Latch>>);
        impl Drop for NotifyFinish<'_> {
            fn drop(&mut self) {
                if let Some(l) = self.0 {
                    l.count_down();
                }
            }
        }
        let _finally = NotifyFinish(&self.notify_finish);
        if let Some(l) = &self.notify_start {
            l.count_down();
        }
        if let Some(l) = &self.wait_to_finish {
            l.wait(&format!("{key:?} timed out waiting to finish"));
            if self.wait_for_exception {
                wait_for_exception(env, &format!("{key:?} timed out waiting for exception"));
            }
        }
        for dep in &self.deps {
            env.get_value(T(dep))?;
        }
        let Some(value) = self.value else {
            return Err(SkyFunctionException::persistent(SomeErrorException("oops".into())).into());
        };
        if env.values_missing() {
            return Ok(None);
        }
        Ok(Some(value.to_string()))
    }
}

fn tester_evaluator(tester: Arc<Tester>) -> Evaluator {
    EvaluatorBuilder::new()
        .client(tester)
        .function(T::FUNCTION, TestFunction)
        .function(N::FUNCTION, TestFunction)
        .build()
}

fn new_tester() -> Arc<Tester> {
    Arc::new(Tester::default())
}

/// Runs a ParallelEvaluatorTest scenario at several parallelisms, with deps requested one by one
/// (as GraphTester does) and as batches.
fn for_each_mode(build: impl Fn(&Tester), check: impl Fn(&Evaluator, usize)) {
    for batch in [false, true] {
        for p in [1, 3, 8] {
            let tester = new_tester();
            build(&tester);
            if batch {
                for spec in tester.specs.lock().values_mut() {
                    spec.batch = true;
                }
            }
            let ev = tester_evaluator(tester);
            check(&ev, p);
        }
    }
}

fn only_cycle(r: &EvaluationResult, key: &SkyKey) -> CycleInfo {
    let err = r.get_error(key).expect("error");
    assert_eq!(err.get_cycle_info().len(), 1, "{err:?}");
    err.get_cycle_info()[0].clone()
}

fn as_set(cycles: &[CycleInfo]) -> HashSet<CycleInfo> {
    cycles.iter().cloned().collect()
}

// Java: ParallelEvaluatorTest.simpleCycle
#[test]
fn simple_cycle() {
    for_each_mode(
        |t| {
            t.add_dependency("a", "b").add_dependency("b", "a");
        },
        |ev, p| {
            let r = eval(ev, false, p, &[k("a")]);
            let error_info = r.get_first_error().unwrap();
            assert!(error_info.get_exception().is_none());
            let cycle_info = only_cycle(&r, &k("a"));
            assert_eq!(cycle_info.get_cycle(), &[k("a"), k("b")]);
            assert!(cycle_info.get_path_to_cycle().is_empty());
        },
    );
}

// Java: ParallelEvaluatorTest.cycleWithHead
#[test]
fn cycle_with_head() {
    for_each_mode(
        |t| {
            t.add_dependency("top", "mid")
                .add_dependency("mid", "a")
                .add_dependency("a", "b")
                .add_dependency("b", "a");
        },
        |ev, p| {
            let r = eval(ev, false, p, &[k("top")]);
            assert!(r.get_first_error().unwrap().get_exception().is_none());
            let cycle_info = only_cycle(&r, &k("top"));
            assert_eq!(cycle_info.get_cycle(), &[k("a"), k("b")]);
            assert_eq!(cycle_info.get_path_to_cycle(), &[k("top"), k("mid")]);
        },
    );
}

// Java: ParallelEvaluatorTest.selfEdgeWithHead
#[test]
fn self_edge_with_head() {
    for_each_mode(
        |t| {
            t.add_dependency("top", "mid")
                .add_dependency("mid", "a")
                .add_dependency("a", "a");
        },
        |ev, p| {
            let r = eval(ev, false, p, &[k("top")]);
            let cycle_info = only_cycle(&r, &k("top"));
            assert_eq!(cycle_info.get_cycle(), &[k("a")]);
            assert_eq!(cycle_info.get_path_to_cycle(), &[k("top"), k("mid")]);
        },
    );
}

// Java: ParallelEvaluatorTest.cycleWithKeepGoing
#[test]
fn cycle_with_keep_going() {
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
        },
    );
}

// Java: ParallelEvaluatorTest.twoCycles
#[test]
fn two_cycles() {
    for_each_mode(
        |t| {
            t.add_dependency("top", "a")
                .add_dependency("top", "c")
                .add_dependency("a", "b")
                .add_dependency("b", "a")
                .add_dependency("c", "d")
                .add_dependency("d", "c");
        },
        |ev, p| {
            let r = eval(ev, false, p, &[k("top")]);
            assert!(r.get(&T("top")).is_none());
            let cycles = CycleInfo::prepare_cycles(
                &k("top"),
                &[
                    CycleInfo::create_cycle_info_without_path(vec![k("a"), k("b")]),
                    CycleInfo::create_cycle_info_without_path(vec![k("c"), k("d")]),
                ],
            );
            let got = only_cycle(&r, &k("top"));
            assert!(cycles.contains(&got), "{got:?}");
            // Deterministic: the detector visits the last requested dep first.
            assert_eq!(got, cycles[1]);
        },
    );
}

// Java: ParallelEvaluatorTest.twoCyclesKeepGoing
#[test]
fn two_cycles_keep_going() {
    for_each_mode(
        |t| {
            t.add_dependency("top", "a")
                .add_dependency("top", "c")
                .add_dependency("a", "b")
                .add_dependency("b", "a")
                .add_dependency("c", "d")
                .add_dependency("d", "c");
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("top")]);
            assert!(r.get(&T("top")).is_none());
            let error_info = r.get_error(&k("top")).unwrap();
            let a_cycle = CycleInfo::create_cycle_info(vec![k("top")], vec![k("a"), k("b")]);
            let c_cycle = CycleInfo::create_cycle_info(vec![k("top")], vec![k("c"), k("d")]);
            assert_eq!(
                as_set(error_info.get_cycle_info()),
                as_set(&[a_cycle, c_cycle])
            );
        },
    );
}

// Java: ParallelEvaluatorTest.triangleBelowHeadCycle
#[test]
fn triangle_below_head_cycle() {
    for_each_mode(
        |t| {
            t.add_dependency("top", "a")
                .add_dependency("a", "b")
                .add_dependency("a", "c")
                .add_dependency("b", "c")
                .add_dependency("c", "top");
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("top")]);
            assert!(r.get(&T("top")).is_none());
            let error_info = r.get_error(&k("top")).unwrap();
            let top_cycle =
                CycleInfo::create_cycle_info_without_path(vec![k("top"), k("a"), k("c")]);
            assert_eq!(error_info.get_cycle_info(), &[top_cycle]);
        },
    );
}

// Java: ParallelEvaluatorTest.longCycle
#[test]
fn long_cycle() {
    for_each_mode(
        |t| {
            t.add_dependency("top", "a")
                .add_dependency("a", "b")
                .add_dependency("b", "c")
                .add_dependency("c", "top");
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("top")]);
            assert!(r.get(&T("top")).is_none());
            let top_cycle =
                CycleInfo::create_cycle_info_without_path(vec![k("top"), k("a"), k("b"), k("c")]);
            assert_eq!(
                r.get_error(&k("top")).unwrap().get_cycle_info(),
                &[top_cycle]
            );
        },
    );
}

// Java: ParallelEvaluatorTest.cycleWithTail
#[test]
fn cycle_with_tail() {
    for_each_mode(
        |t| {
            t.add_dependency("top", "a")
                .add_dependency("top", "c")
                .add_dependency("a", "b")
                .add_dependency("b", "a")
                .add_dependency("b", "c")
                .set("c", "cValue");
        },
        |ev, p| {
            let r = eval(ev, false, p, &[k("top")]);
            assert!(r.get(&T("top")).is_none());
            let cycle_info = only_cycle(&r, &k("top"));
            assert_eq!(cycle_info.get_cycle(), &[k("a"), k("b")]);
            assert_eq!(cycle_info.get_path_to_cycle(), &[k("top")]);
        },
    );
}

// Java: ParallelEvaluatorTest.selfEdgeWithExtraChildrenUnderCycle
#[test]
fn self_edge_with_extra_children_under_cycle() {
    for_each_mode(
        |t| {
            t.add_dependency("a", "z")
                .add_dependency("z", "c")
                .add_dependency("z", "z")
                .add_dependency("c", "a");
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("a")]);
            assert!(r.get(&T("a")).is_none());
            let cycle_info = only_cycle(&r, &k("a"));
            assert_eq!(cycle_info.get_cycle(), &[k("z")]);
            assert_eq!(cycle_info.get_path_to_cycle(), &[k("a")]);
        },
    );
}

// Java: ParallelEvaluatorTest.cycleWithExtraChildrenUnderCycle
#[test]
fn cycle_with_extra_children_under_cycle() {
    for_each_mode(
        |t| {
            t.add_dependency("a", "b")
                .add_dependency("b", "c")
                .add_dependency("b", "d")
                .add_dependency("c", "a")
                .add_dependency("d", "b");
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("a")]);
            assert!(r.get(&T("a")).is_none());
            let cycle_info = only_cycle(&r, &k("a"));
            assert_eq!(cycle_info.get_cycle(), &[k("b"), k("d")]);
            assert_eq!(cycle_info.get_path_to_cycle(), &[k("a")]);
        },
    );
}

// Java: ParallelEvaluatorTest.cycleAboveIndependentCycle
#[test]
fn cycle_above_independent_cycle() {
    for_each_mode(
        |t| {
            t.add_dependency("a", "b")
                .add_dependency("b", "c")
                .add_dependency("c", "a")
                .add_dependency("c", "b");
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("a")]);
            assert!(r.get(&T("a")).is_none());
            assert_eq!(
                as_set(r.get_error(&k("a")).unwrap().get_cycle_info()),
                as_set(&[
                    CycleInfo::create_cycle_info_without_path(vec![k("a"), k("b"), k("c")]),
                    CycleInfo::create_cycle_info(vec![k("a")], vec![k("b"), k("c")]),
                ])
            );
        },
    );
}

// Java: ParallelEvaluatorTest.valueAboveCycleAndExceptionReportsException
#[test]
fn value_above_cycle_and_exception_reports_exception() {
    for_each_mode(
        |t| {
            t.add_dependency("a", "b")
                .add_dependency("a", "error")
                .add_dependency("b", "b")
                .set_has_error("error", true);
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("a")]);
            assert!(r.get(&T("a")).is_none());
            assert!(r.get_error(&k("a")).unwrap().get_exception().is_some());
            let cycle_info = only_cycle(&r, &k("a"));
            assert_eq!(cycle_info.get_cycle(), &[k("b")]);
            assert_eq!(cycle_info.get_path_to_cycle(), &[k("a")]);
        },
    );
}

// Java: ParallelEvaluatorTest.errorValueStored
#[test]
fn error_value_stored() {
    let tester = new_tester();
    tester.set_has_error("my_error_value", true);
    let ev = tester_evaluator(tester.clone());
    let r = eval(&ev, false, 2, &[k("my_error_value")]);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].0, k("my_error_value"));
    // Update value. But builder won't rebuild it.
    tester
        .set_has_error("my_error_value", false)
        .set("my_error_value", "no error?");
    let r = eval(&ev, false, 2, &[k("my_error_value")]);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].0, k("my_error_value"));
}

// Java: ParallelEvaluatorTest.manyCycles
#[test]
fn many_cycles() {
    const NAMES: [&str; 100] = {
        let mut n = [""; 100];
        let digits = [
            "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15",
            "16", "17", "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29",
            "30", "31", "32", "33", "34", "35", "36", "37", "38", "39", "40", "41", "42", "43",
            "44", "45", "46", "47", "48", "49", "50", "51", "52", "53", "54", "55", "56", "57",
            "58", "59", "60", "61", "62", "63", "64", "65", "66", "67", "68", "69", "70", "71",
            "72", "73", "74", "75", "76", "77", "78", "79", "80", "81", "82", "83", "84", "85",
            "86", "87", "88", "89", "90", "91", "92", "93", "94", "95", "96", "97", "98", "99",
        ];
        let mut i = 0;
        while i < 100 {
            n[i] = digits[i];
            i += 1;
        }
        n
    };
    for_each_mode(
        |t| {
            for d in NAMES {
                t.add_dependency("top", d).add_dependency(d, d);
            }
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("top")]);
            assert!(r.get(&T("top")).is_none());
            let error_info = r.get_error(&k("top")).unwrap();
            // assertManyCycles(errorInfo, topKey, /* selfEdge= */ false)
            assert!(error_info.get_cycle_info().len() > 1);
            assert!(error_info.get_cycle_info().len() < 50);
            for cycle in error_info.get_cycle_info() {
                assert_eq!(cycle.get_cycle().len(), 1); // Self-edge.
                assert_eq!(cycle.get_path_to_cycle(), &[k("top")]);
            }
        },
    );
}

// Java: ParallelEvaluatorTest.manyPathsToCycle
#[test]
fn many_paths_to_cycle() {
    const NAMES: [&str; 20] = [
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
        "17", "18", "19",
    ];
    for_each_mode(
        |t| {
            t.add_dependency("top", "mid")
                .add_dependency("cycle", "cycle");
            for d in NAMES {
                t.add_dependency("mid", d).add_dependency(d, "cycle");
            }
        },
        |ev, p| {
            let r = eval(ev, true, p, &[k("top")]);
            assert!(r.get(&T("top")).is_none());
            let cycle_info = only_cycle(&r, &k("top"));
            assert_eq!(cycle_info.get_cycle().len(), 1);
            assert_eq!(cycle_info.get_path_to_cycle().len(), 3);
            assert_eq!(&cycle_info.get_path_to_cycle()[..2], &[k("top"), k("mid")]);
        },
    );
}

// Java: ParallelEvaluatorTest.noKeepGoingAfterKeepGoingFails
#[test]
fn no_keep_going_after_keep_going_fails() {
    let tester = new_tester();
    tester
        .set_has_error("my_error_value", true)
        .add_dependency("parent", "my_error_value");
    let ev = tester_evaluator(tester);
    let r = eval(&ev, true, 2, &[k("parent")]);
    assert!(r.get_error(&k("parent")).is_some());
    let r = eval(&ev, false, 2, &[k("parent")]);
    assert_eq!(r.errors.len(), 1);
    let err = r.get_error(&k("parent")).unwrap();
    assert_eq!(err.get_exception().unwrap().to_string(), "my_error_value");
}

// Java: ParallelEvaluatorTest.twoErrors
#[test]
fn two_errors() {
    for _ in 0..20 {
        let tester = new_tester();
        tester
            .set_has_error("error1", true)
            .set_has_error("error2", true);
        let ev = tester_evaluator(tester);
        let r = eval(&ev, false, 8, &[k("error1"), k("error2")]);
        assert!(r.has_error());
        // With keepGoing=false, the eval call will terminate with exactly one error (the first
        // one thrown).
        assert_eq!(r.errors.len(), 1);
        assert!([k("error1"), k("error2")].contains(&r.errors[0].0));
    }
}

// Java: ParallelEvaluatorTest.topCatastrophe
#[test]
fn top_catastrophe() {
    let catastrophe: SkyError = Arc::new(SomeErrorException("bad".into()));
    let tester = new_tester();
    let c = catastrophe.clone();
    tester.set_builder("catastrophe", move |_, _| {
        Err(SkyFunctionException {
            cause: c.clone(),
            transience: Transience::Persistent,
            catastrophic: true,
        }
        .into())
    });
    let ev = tester_evaluator(tester);
    let r = eval(&ev, true, 2, &[k("catastrophe")]);
    assert!(Arc::ptr_eq(r.get_catastrophe().unwrap(), &catastrophe));
}

// Java: ParallelEvaluatorTest.catastropheBubblesIntoNonCatastrophe
#[test]
fn catastrophe_bubbles_into_non_catastrophe() {
    let tester = new_tester();
    tester
        .set_builder("catastrophe", |_, _| {
            Err(SkyFunctionException::catastrophic(
                SomeErrorException("bad".into()),
                Transience::Persistent,
            )
            .into())
        })
        .set_builder("top", |_, env| {
            if let Err(e) = env.get_value_or_throw::<T, SomeErrorException>(T("catastrophe"))? {
                return Err(SkyFunctionException::persistent(SomeErrorException(format!(
                    "We got: {e}"
                )))
                .into());
            }
            Ok(None)
        });
    let ev = tester_evaluator(tester);
    let r = eval(&ev, true, 2, &[k("top")]);
    let err = r.get_error(&k("top")).unwrap();
    assert_eq!(
        err.exception_as::<SomeErrorException>(),
        Some(&SomeErrorException("We got: bad".into()))
    );
    assert!(r.get_catastrophe().is_some());
}

// Java: ParallelEvaluatorTest.catastrophicBuild
#[test]
fn catastrophic_build() {
    for keep_going in [false, true] {
        let catastrophe: SkyError = Arc::new(SomeErrorException("bad".into()));
        let tester = new_tester();
        let c = catastrophe.clone();
        tester
            .set_builder("catastrophe", move |_, _| {
                Err(SkyFunctionException {
                    cause: c.clone(),
                    transience: Transience::Persistent,
                    catastrophic: true,
                }
                .into())
            })
            .set_builder("someKey", |_, env| {
                // Java: new CountDownLatch(1).await(), released by the evaluator's interrupt.
                loop {
                    env.check_interrupted()?;
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
            .add_dependency("top", "catastrophe");
        let ev = tester_evaluator(tester);
        let r = eval(&ev, keep_going, 4, &[k("top"), k("someKey")]);
        assert!(r.get_error(&k("top")).is_some());
        assert!(r.get(&T("someKey")).is_none());
        assert!(r.get_error(&k("someKey")).is_none());
        if keep_going {
            assert!(Arc::ptr_eq(r.get_catastrophe().unwrap(), &catastrophe));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// No-keep-going result shape and per-call context.

#[test]
fn no_keep_going_reports_one_bubbled_root_and_drops_aborted_roots() {
    let tester = new_tester();
    let started = Arc::new(AtomicBool::new(false));
    let s = started.clone();
    tester
        .set_has_error("err", true)
        .add_dependency("p1", "err")
        .add_dependency("p2", "err")
        .set_builder("slow", move |_, env| {
            s.store(true, Ordering::SeqCst);
            loop {
                env.check_interrupted()?;
                std::thread::sleep(Duration::from_millis(1));
            }
        })
        .add_dependency("waits_for_slow", "slow");
    let ev = tester_evaluator(tester);
    let r = eval(&ev, false, 4, &[k("waits_for_slow"), k("p1"), k("p2")]);
    // Exactly one bubbled error: Java bubbles up one chain of parents
    // (`Iterables.getFirst(reverseDeps)`); which parent registered first is a race.
    assert_eq!(r.errors.len(), 1, "{r:?}");
    assert!([k("p1"), k("p2")].contains(&r.errors[0].0), "{r:?}");
    assert_eq!(
        r.errors[0].1.get_exception().unwrap().to_string(),
        "err",
        "the real error is reported"
    );
    assert!(r.values.is_empty());
    // Only the committed leaf error is done in the graph; bubbled and aborted nodes are not
    // committed. They stay in flight until the next evaluation deletes them
    // (AbstractInMemoryMemoizingEvaluator.java:134).
    assert!(ev.get_existing_error_for_testing(&k("err")).is_some());
    for dropped in ["p1", "p2", "slow", "waits_for_slow"] {
        assert!(
            ev.committed_entry(&k(dropped)).is_none(),
            "{dropped} was memoized"
        );
    }
    eval(&ev, false, 4, &[k("err")]);
    for dropped in ["p1", "p2", "slow", "waits_for_slow"] {
        assert!(
            !ev.get_all_keys_for_testing().contains(&k(dropped)),
            "{dropped} was not deleted by the next evaluation"
        );
    }
    let _ = started;
}

#[test]
fn abort_is_reset_per_call_and_aborted_nodes_reevaluate() {
    let tester = new_tester();
    let fail = Arc::new(AtomicBool::new(true));
    let f = fail.clone();
    tester
        .set_builder("flaky_slow", move |_, env| {
            // Waits until the evaluation aborts, the first time only.
            while f.load(Ordering::SeqCst) {
                env.check_interrupted()?;
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(Some("slow done".into()))
        })
        .set_has_error("err", true)
        .add_dependency("good", "flaky_slow");
    let ev = tester_evaluator(tester);
    let r = eval(&ev, false, 4, &[k("good"), k("err")]);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].0, k("err"));
    assert!(r.get(&T("good")).is_none());
    assert!(r.get_error(&k("good")).is_none());

    // Next call: a fresh abort flag, and the aborted nodes are evaluated again.
    fail.store(false, Ordering::SeqCst);
    let r = eval(&ev, false, 4, &[k("good")]);
    assert!(!r.has_error(), "{r:?}");
    assert_eq!(*r.get(&T("good")).unwrap(), "slow done");
    // And keep_going can differ per call.
    let r = eval(&ev, true, 4, &[k("good"), k("err")]);
    assert_eq!(*r.get(&T("good")).unwrap(), "slow done");
    assert!(r.get_error(&k("err")).is_some());
}

#[test]
fn no_keep_going_with_cached_error_root_returns_it_without_evaluating() {
    let tester = new_tester();
    tester.set_has_error("err", true).set("other", "other");
    let ev = tester_evaluator(tester);
    let _ = eval(&ev, true, 2, &[k("err")]);
    let computed = ev.computed_count();
    let r = eval(&ev, false, 2, &[k("other"), k("err")]);
    // Java: ParallelEvaluator.eval — errors, even cached ones, halt no-keep-going evaluations.
    assert_eq!(r.errors.len(), 1);
    assert!(r.values.is_empty());
    assert_eq!(ev.computed_count(), computed);
}

#[test]
fn keep_going_throw_with_missing_dep_is_replaced_by_child_error() {
    let tester = new_tester();
    tester
        .set_has_error("child", true)
        .set_builder("parent", |_, env| {
            let v = env.get_value(T("child"))?;
            assert!(v.is_none() && env.values_missing());
            Err(SkyFunctionException::persistent(SomeErrorException("parent's own".into())).into())
        });
    let ev = tester_evaluator(tester);
    let r = eval(&ev, true, 2, &[k("parent")]);
    // Java: in keep_going a thrown error with missing deps is ignored; the child's error is used.
    assert_eq!(
        r.get_error(&k("parent"))
            .unwrap()
            .get_exception()
            .unwrap()
            .to_string(),
        "child"
    );
}

#[test]
fn in_error_bubbling_is_true_only_in_the_rerun_after_a_failure() {
    let tester = new_tester();
    let runs = Arc::new(Mutex::new(vec![]));
    let seen = runs.clone();
    tester
        .set_has_error("child", true)
        .set_builder("parent", move |_, env| {
            let _ = env.get_value(T("child"))?;
            seen.lock().push(env.in_error_bubbling());
            Ok(None)
        });
    let ev = tester_evaluator(tester);
    // keep_going: one run, never in error bubbling.
    let r = eval(&ev, true, 2, &[k("parent")]);
    assert!(r.has_error());
    assert_eq!(*runs.lock(), vec![false]);
    // no keep_going (fresh graph): the normal run, then the error-bubbling run.
    runs.lock().clear();
    let tester = new_tester();
    let seen = runs.clone();
    tester
        .set_has_error("child", true)
        .set_builder("parent", move |_, env| {
            let _ = env.get_value(T("child"))?;
            seen.lock().push(env.in_error_bubbling());
            Ok(None)
        });
    let ev = tester_evaluator(tester);
    let r = eval(&ev, false, 2, &[k("parent")]);
    assert!(r.has_error());
    assert_eq!(*runs.lock(), vec![false, true]);
}

// ---------------------------------------------------------------------------------------------
// Transience.

#[test]
fn transient_errors_are_memoized_until_invalidated() {
    let tester = new_tester();
    tester
        .with("flaky", |s| s.has_transient_error = true)
        .add_dependency("parent", "flaky");
    let ev = tester_evaluator(tester.clone());
    let r = eval(&ev, true, 2, &[k("parent")]);
    let err = r.get_error(&k("parent")).unwrap();
    assert!(err.is_transitively_transient());
    assert!(!err.is_directly_transient());
    let flaky = ev.get_existing_error_for_testing(&k("flaky")).unwrap();
    assert!(flaky.is_directly_transient());
    // Directly transient errors depend on ErrorTransienceValue.KEY, last.
    assert_eq!(
        ev.get_direct_deps(&k("flaky")),
        vec![ERROR_TRANSIENCE_KEY.clone()]
    );
    assert_eq!(ev.get_direct_deps(&k("parent")), vec![k("flaky")]);

    // Java keeps transient errors for the rest of the command.
    tester.with("flaky", |s| {
        s.has_transient_error = false;
        s.value = Some("ok".into());
    });
    let computed = ev.computed_count();
    let r = eval(&ev, true, 2, &[k("parent")]);
    assert!(r.has_error());
    assert_eq!(ev.computed_count(), computed);

    // invalidateTransientErrors re-evaluates them and their dependents.
    ev.invalidate_transient_errors();
    let r = eval(&ev, true, 2, &[k("parent")]);
    assert_eq!(*r.get(&T("parent")).unwrap(), "ok");
    assert!(
        ev.get_reverse_deps(std::slice::from_ref(&*ERROR_TRANSIENCE_KEY))[0]
            .1
            .is_empty()
    );
}

// ---------------------------------------------------------------------------------------------
// Walkable graph.

#[test]
fn walkable_graph_has_deps_in_request_order_and_reverse_deps() {
    let tester = new_tester();
    tester
        .add_dependency("top", "z")
        .add_dependency("top", "a")
        .add_dependency("top", "m")
        .set("z", "z")
        .set("a", "a")
        .add_dependency("m", "a");
    let ev = tester_evaluator(tester);
    let r = eval(&ev, false, 4, &[k("top")]);
    assert_eq!(*r.get(&T("top")).unwrap(), "zaa");
    assert_eq!(ev.get_direct_deps(&k("top")), vec![k("z"), k("a"), k("m")]);
    let rdeps = ev.get_reverse_deps(&[k("a")]);
    assert_eq!(rdeps.len(), 1);
    let mut a_rdeps = rdeps[0].1.clone();
    a_rdeps.sort_by_key(|k| format!("{k:?}"));
    assert_eq!(a_rdeps, vec![k("m"), k("top")]);
    assert!(ev.get_value(&k("top")).is_some());
    assert!(!ev.is_cycle(&k("top")));
    assert_eq!(
        ev.get_missing_and_exceptions(&[k("top"), k("nope")])
            .into_iter()
            .map(|(k, e)| (k, e.is_none()))
            .collect::<Vec<_>>(),
        vec![(k("nope"), true)]
    );
}

#[test]
fn batch_requests_record_one_group() {
    let tester = new_tester();
    tester
        .set("a", "a")
        .set("b", "b")
        .set("c", "c")
        .set_builder("top", |_, env| {
            let _ = env.get_value(T("c"))?;
            let r = env.get_values_and_exceptions(&[k("b"), k("a"), k("b"), k("c")])?;
            assert!(r.get(&k("b")).is_some());
            assert_eq!(
                env.get_temporary_direct_deps().groups(),
                &[vec![k("c")], vec![k("b"), k("a")]]
            );
            Ok(Some("top".into()))
        });
    let ev = tester_evaluator(tester);
    let r = eval(&ev, false, 4, &[k("top")]);
    assert!(!r.has_error());
    assert_eq!(ev.get_direct_deps(&k("top")), vec![k("c"), k("b"), k("a")]);
}

// ---------------------------------------------------------------------------------------------
// Interrupts.

#[test]
fn interrupt_surfaces_and_interrupted_nodes_are_not_memoized() {
    let tester = new_tester();
    let entered = Arc::new(AtomicBool::new(false));
    let e = entered.clone();
    tester
        .set_builder("spin", move |_, env| {
            e.store(true, Ordering::SeqCst);
            loop {
                env.check_interrupted()?;
                std::thread::sleep(Duration::from_millis(1));
            }
        })
        .add_dependency("top", "spin")
        .set("leaf", "leaf")
        .set_builder("asks_after_interrupt", |_, env| {
            while env.check_interrupted().is_ok() {
                std::thread::sleep(Duration::from_millis(1));
            }
            // Dependency requests fail once interrupted.
            assert_eq!(env.get_value(T("leaf")), Err(Interrupted));
            Err(Interrupted.into())
        });
    let ev = Arc::new(tester_evaluator(tester.clone()));
    let ev2 = ev.clone();
    let interrupter = std::thread::spawn(move || {
        while !entered.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        ev2.interrupt();
    });
    let r = ev.evaluate(&[k("top"), k("asks_after_interrupt")], &ctx(true, 4));
    interrupter.join().unwrap();
    assert_eq!(r.err(), Some(Interrupted));
    // Still interrupted: the next evaluation fails immediately.
    assert_eq!(
        ev.evaluate(&[k("leaf")], &ctx(true, 4)).err(),
        Some(Interrupted)
    );
    ev.clear_interrupt();
    assert!(!ev.get_all_keys_for_testing().contains(&k("top")));
    assert!(!ev.get_all_keys_for_testing().contains(&k("spin")));
    tester.with("spin", |s| {
        s.builder = None;
        s.value = Some("spun".into());
    });
    let r = eval(&ev, true, 4, &[k("top")]);
    assert_eq!(*r.get(&T("top")).unwrap(), "spun");
}

// ---------------------------------------------------------------------------------------------
// Events.

fn messages(h: &StoredEventHandler) -> Vec<String> {
    h.get_events()
        .iter()
        .map(|e| e.get_message().to_string())
        .collect()
}

fn ctx_with(handler: Arc<StoredEventHandler>, keep_going: bool) -> EvaluationContext {
    EvaluationContext {
        event_handler: handler,
        ..ctx(keep_going, 4)
    }
}

// Java: ParallelEvaluatorTest.simpleWarning
#[test]
fn simple_warning() {
    let tester = new_tester();
    tester
        .set("a", "a")
        .with("a", |s| s.warning = Some("warning on 'a'".into()));
    let ev = tester_evaluator(tester);
    let events = Arc::new(StoredEventHandler::new());
    let r = ev
        .evaluate(&[k("a")], &ctx_with(events.clone(), false))
        .unwrap();
    assert_eq!(*r.get(&T("a")).unwrap(), "a");
    assert_eq!(messages(&events), vec!["warning on 'a'"]);
}

// Java: MemoizingEvaluatorTest.warningViaMultiplePaths
#[test]
fn warning_via_multiple_paths() {
    let tester = new_tester();
    tester
        .set("d1", "d1")
        .with("d1", |s| s.warning = Some("warn-d1".into()))
        .set("d2", "d2")
        .with("d2", |s| s.warning = Some("warn-d2".into()))
        .add_dependency("top", "d1")
        .add_dependency("top", "d2");
    let ev = tester_evaluator(tester);
    let events = Arc::new(StoredEventHandler::new());
    ev.evaluate(&[k("top")], &ctx_with(events.clone(), false))
        .unwrap();
    let mut got = messages(&events);
    got.sort();
    assert_eq!(got, vec!["warn-d1", "warn-d2"]);
}

#[test]
fn identical_warnings_are_reported_once_per_evaluator() {
    let tester = new_tester();
    for n in ["w1", "w2", "w3"] {
        tester
            .set(n, n)
            .with(n, |s| s.warning = Some("same warning".into()))
            .add_dependency("top", n);
    }
    let ev = tester_evaluator(tester);
    let events = Arc::new(StoredEventHandler::new());
    ev.evaluate(&[k("top")], &ctx_with(events.clone(), true))
        .unwrap();
    assert_eq!(messages(&events), vec!["same warning"]);
}

// Java: ParallelEvaluatorTest.errorOfTopLevelTargetReported
#[test]
fn error_of_top_level_target_reported() {
    let tester = new_tester();
    tester.set_has_error("b", true).set_builder("a", |_, env| {
        // Continue silently on an exception.
        if let Ok(None) = env.get_value_or_throw::<T, SomeErrorException>(T("b"))? {
            return Ok(None);
        }
        env.listener().handle(&Event::error("foobar"));
        Err(SkyFunctionException::persistent(SomeErrorException("bazbar".into())).into())
    });
    let ev = tester_evaluator(tester);
    let events = Arc::new(StoredEventHandler::new());
    ev.evaluate(&[k("a")], &ctx_with(events.clone(), false))
        .unwrap();
    assert_eq!(events.get_events(), vec![Event::error("foobar")]);
}

#[test]
fn stored_events_are_tagged_and_dropped_when_the_node_does_not_finish() {
    struct Tagged;
    impl SkyFunction for Tagged {
        fn compute(&self, key: &SkyKey, env: &mut Environment<'_>) -> ComputeResult {
            let t = key.downcast::<T>().unwrap();
            env.listener().handle(&Event::warn(format!("warn {t:?}")));
            env.listener().handle(&Event::error(format!("error {t:?}")));
            env.listener()
                .handle(&Event::progress(format!("progress {t:?}")));
            if t.0 == "in_cycle" {
                // A cycle: this node never finishes, so its stored (error) event is dropped.
                let _ = env.get_value(T("in_cycle"))?;
                return Ok(None);
            }
            Ok(Some(Arc::new(String::new())))
        }
        fn extract_tag(&self, key: &SkyKey) -> Option<String> {
            Some(format!("//pkg:{:?}", key.downcast::<T>().unwrap()))
        }
    }
    let ev = EvaluatorBuilder::new()
        .function(T::FUNCTION, Tagged)
        .build();
    let events = Arc::new(StoredEventHandler::new());
    let r = ev
        .evaluate(&[k("ok"), k("in_cycle")], &ctx_with(events.clone(), true))
        .unwrap();
    assert!(r.get_error(&k("in_cycle")).is_some());
    let got: Vec<(EventKind, String, Option<String>)> = events
        .get_events()
        .iter()
        .map(|e| {
            (
                e.get_kind(),
                e.get_message().to_string(),
                e.get_tag().map(str::to_string),
            )
        })
        .collect();
    let has = |kind: EventKind, msg: &str, tag: Option<&str>| {
        got.contains(&(kind, msg.to_string(), tag.map(str::to_string)))
    };
    assert!(has(EventKind::Warning, "warn ok", Some("//pkg:ok")));
    assert!(has(EventKind::Error, "error ok", Some("//pkg:ok")));
    assert!(has(EventKind::Progress, "progress ok", None));
    assert!(has(
        EventKind::Warning,
        "warn in_cycle",
        Some("//pkg:in_cycle")
    ));
    assert!(
        !got.iter().any(|(_, m, _)| m == "error in_cycle"),
        "{got:?}"
    );
    assert_eq!(got.len(), 5, "{got:?}");
}

// ---------------------------------------------------------------------------------------------
// Threads.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Fan(u32, u32);
impl Key for Fan {
    type Value = u64;
    const FUNCTION: SkyFunctionName = SkyFunctionName("FAN");
}

/// Fan(depth, i): a batch of 6 children per level, 4 levels deep (1555 nodes), shared children.
fn fan_evaluator() -> Evaluator {
    EvaluatorBuilder::new()
        .typed_function::<Fan, _>(|k: &Fan, env| {
            if k.0 == 0 {
                return Ok(Some(1));
            }
            let kids: Vec<Fan> = (0..6).map(|j| Fan(k.0 - 1, (k.1 * 6 + j) % 50)).collect();
            let vals = env.get_values_typed(&kids)?;
            if env.values_missing() {
                return Ok(None);
            }
            Ok(Some(vals.into_iter().map(|v| *v.unwrap()).sum()))
        })
        .build()
}

#[test]
fn thread_cap_falls_back_to_inline_evaluation() {
    let ev = fan_evaluator();
    let c = EvaluationContext {
        max_threads: 3,
        ..ctx(true, 8)
    };
    let r = ev.evaluate(&[SkyKey::new(Fan(4, 0))], &c).unwrap();
    assert_eq!(*r.get(&Fan(4, 0)).unwrap(), 6u64.pow(4));
    assert!(ev.peak_threads() <= 3, "peak {}", ev.peak_threads());
    assert_eq!(ev.live_threads(), 0);

    // Without the cap, helpers are used.
    let ev = fan_evaluator();
    let r = eval(&ev, true, 8, &[SkyKey::new(Fan(4, 0))]);
    assert_eq!(*r.get(&Fan(4, 0)).unwrap(), 6u64.pow(4));
    assert!(ev.peak_threads() > 1);
    assert_eq!(ev.live_threads(), 0);
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Chain(u32, u32);
impl Key for Chain {
    type Value = u32;
    const FUNCTION: SkyFunctionName = SkyFunctionName("CHAIN");
}

/// Chain(len, i) depends on Chain(len, i + 1); Chain(len, len) depends on Chain(len, 0) when
/// `len` is odd (a long cycle), else is a leaf.
fn chain_evaluator() -> Evaluator {
    EvaluatorBuilder::new()
        .typed_function::<Chain, _>(|k: &Chain, env| {
            if k.1 == k.0 {
                if k.0 % 2 == 1 {
                    let _ = env.get_value(Chain(k.0, 0))?;
                    return Ok(None);
                }
                return Ok(Some(0));
            }
            let Some(v) = env.get_value(Chain(k.0, k.1 + 1))? else {
                return Ok(None);
            };
            Ok(Some(*v + 1))
        })
        .build()
}

#[test]
fn depth_guard_hands_deep_keys_to_fresh_threads() {
    let ev = chain_evaluator();
    let c = EvaluationContext {
        max_inline_depth: 4,
        thread_stack_size: 1 << 20,
        ..ctx(false, 2)
    };
    let r = ev.evaluate(&[SkyKey::new(Chain(2000, 0))], &c).unwrap();
    assert_eq!(*r.get(&Chain(2000, 0)).unwrap(), 2000);
    assert!(ev.peak_threads() >= 2000 / 4, "peak {}", ev.peak_threads());
    assert_eq!(ev.live_threads(), 0);
}

#[test]
fn cycle_across_fresh_threads_is_detected_deterministically() {
    for keep_going in [false, true] {
        let ev = chain_evaluator();
        let c = EvaluationContext {
            max_inline_depth: 3,
            ..ctx(keep_going, 4)
        };
        let root = SkyKey::new(Chain(41, 0));
        let r = ev.evaluate(std::slice::from_ref(&root), &c).unwrap();
        let cycle = only_cycle(&r, &root);
        let expected: Vec<SkyKey> = (0..=41).map(|i| SkyKey::new(Chain(41, i))).collect();
        assert!(cycle.get_path_to_cycle().is_empty());
        assert_eq!(cycle.get_cycle(), &expected[..]);
    }
}

// ---------------------------------------------------------------------------------------------
// Stress: rerun the parallel and cycle scenarios many times in-process to catch races (a false
// cycle between a parent and its batch helpers was once fixed with frame-aware waits).

#[test]
fn stress_parallel_and_cycles() {
    let iterations = 150;
    let false_cycles = AtomicUsize::new(0);
    for i in 0..iterations {
        let p = [2, 8, 16][i % 3];
        // No false cycles in a heavily shared DAG.
        let ev = evaluator();
        let roots: Vec<SkyKey> = (0..120u64)
            .filter(|n| n % 7 != 6 && *n != 13)
            .map(|n| SkyKey::new(Num(n)))
            .collect();
        let r = eval(&ev, i % 2 == 0, p, &roots);
        if r.has_error() {
            false_cycles.fetch_add(1, Ordering::SeqCst);
        }
        assert_eq!(r.values.len(), roots.len(), "iteration {i}: {r:?}");

        // Cycle through a parallel batch: always the same report.
        for keep_going in [false, true] {
            let c = cycle_of(&evaluator(), keep_going, p, 10);
            assert_eq!(
                c,
                CycleInfo::create_cycle_info(vec![], vec![cyc(10), cyc(12)])
            );
        }

        // Ported cycle scenarios, batched (parallel).
        let tester = new_tester();
        tester
            .add_dependency("top", "a")
            .add_dependency("a", "b")
            .add_dependency("a", "c")
            .add_dependency("b", "c")
            .add_dependency("c", "top");
        for spec in tester.specs.lock().values_mut() {
            spec.batch = true;
        }
        let ev = tester_evaluator(tester);
        let r = eval(&ev, true, p, &[k("top")]);
        assert_eq!(
            r.get_error(&k("top")).unwrap().get_cycle_info(),
            &[CycleInfo::create_cycle_info_without_path(vec![
                k("top"),
                k("a"),
                k("c")
            ])]
        );

        let tester = new_tester();
        tester
            .add_dependency("top", "a")
            .add_dependency("top", "c")
            .add_dependency("a", "b")
            .add_dependency("b", "a")
            .add_dependency("c", "d")
            .add_dependency("d", "c")
            .add_dependency("top2", "c");
        for spec in tester.specs.lock().values_mut() {
            spec.batch = true;
        }
        let ev = tester_evaluator(tester);
        let r = eval(&ev, false, p, &[k("top"), k("top2")]);
        assert_eq!(r.errors.len(), 1);
        assert_eq!(
            only_cycle(&r, &k("top")),
            CycleInfo::create_cycle_info(vec![k("top")], vec![k("c"), k("d")])
        );
    }
    assert_eq!(false_cycles.load(Ordering::SeqCst), 0);
}

/// `getValueOrThrow(key, A.class, B.class)`: one request that catches several exception types
/// must not mark values missing when the second type matches.
#[test]
fn get_value_or_throw_with_catches_several_types() {
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct Multi;
    impl Key for Multi {
        type Value = String;
        const FUNCTION: SkyFunctionName = SkyFunctionName("MULTI");
    }
    #[derive(Debug, Clone)]
    struct OtherError;
    impl std::fmt::Display for OtherError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("other")
        }
    }
    impl std::error::Error for OtherError {}
    let ev = EvaluatorBuilder::new()
        .typed_function::<Num, _>(num_fn)
        .typed_function::<Multi, _>(|_, env| {
            let r = env.get_value_or_throw_with(Num(13), |e| {
                if let Some(o) = e.downcast_ref::<OtherError>() {
                    Some(format!("other: {o}"))
                } else {
                    e.downcast_ref::<BadNumber>().map(|b| format!("bad: {b}"))
                }
            })?;
            assert!(
                !env.values_missing(),
                "a caught exception is not a missing value"
            );
            match r {
                Ok(_) => Ok(Some("no error".into())),
                Err(s) => Ok(Some(s)),
            }
        })
        .build();
    let r = eval(&ev, true, 2, &[SkyKey::new(Multi)]);
    assert_eq!(*r.get(&Multi).unwrap(), "bad: bad number 13");
}

// ---------------------------------------------------------------------------------------------
// Java: MemoizingEvaluatorTest.MemoizingEvaluatorTester.

/// Java: `GraphTester.NonHermeticKey` whose values are `NotComparableStringValue`s.
///
/// PORT: Java's `NotComparableStringValue` is a value class (`NotComparableSkyValue`); value
/// equality is the key's here (`Key::values_equal`, docs/decisions.md D10), so a not comparable
/// value has its own key type, served by the same test function.
#[derive(Clone, PartialEq, Eq, Hash)]
struct NotComparableKey(&'static str);
impl fmt::Debug for NotComparableKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Key for NotComparableKey {
    type Value = String;
    const FUNCTION: SkyFunctionName = SkyFunctionName("TEST_NONHERMETIC_NOT_COMPARABLE");
    const HERMETICITY: Option<FunctionHermeticity> = Some(FunctionHermeticity::Nonhermetic);
}

/// Java: `MemoizingEvaluatorTest.MemoizingEvaluatorTester`: a [`Tester`] (its `GraphTester`
/// part, reached through `Deref`), the evaluator built on it with a `TrackingProgressReceiver`
/// and `EventFilter.FULL_STORAGE`, and the event collector of the test (`eventCollector`).
// Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5880 MemoizingEvaluatorTester
struct MemoizingTester {
    tester: Arc<Tester>,
    progress_receiver: Arc<tracking_progress_receiver::TrackingProgressReceiver>,
    evaluator: Evaluator,
    /// Java: `eventCollector`, replaced by `initializeReporter()`.
    event_collector: Mutex<Arc<StoredEventHandler>>,
}

impl std::ops::Deref for MemoizingTester {
    type Target = Tester;
    fn deref(&self) -> &Tester {
        &self.tester
    }
}

impl MemoizingTester {
    /// Java: `initializeTester()` + `initializeReporter()`.
    fn new() -> Self {
        Self::with_progress_receiver(Arc::new(
            tracking_progress_receiver::TrackingProgressReceiver::new(
                /* check_evaluation_results= */ true,
            ),
        ))
    }

    /// Java: `initializeTester(customProgressReceiver)` + `initializeReporter()`.
    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5890 MemoizingEvaluatorTester.initialize
    fn with_progress_receiver(
        progress_receiver: Arc<tracking_progress_receiver::TrackingProgressReceiver>,
    ) -> Self {
        Self::with_options(progress_receiver, Arc::new(<dyn EventFilter>::FULL_STORAGE))
    }

    fn with_options(
        progress_receiver: Arc<tracking_progress_receiver::TrackingProgressReceiver>,
        event_filter: Arc<dyn EventFilter>,
    ) -> Self {
        let tester = new_tester();
        let evaluator = EvaluatorBuilder::new()
            .client(tester.clone())
            .function(T::FUNCTION, TestFunction)
            .function(N::FUNCTION, TestFunction)
            .function(NotComparableKey::FUNCTION, TestFunction)
            .progress_receiver(progress_receiver.clone())
            .event_filter(event_filter)
            .build();
        let t = MemoizingTester {
            tester,
            progress_receiver,
            evaluator,
            event_collector: Mutex::new(Arc::new(StoredEventHandler::new())),
        };
        t.initialize_reporter();
        t
    }

    /// Java: `initializeReporter()`: a new event collector, and the played events forgotten.
    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:166 MemoizingEvaluatorTest.initializeReporter
    fn initialize_reporter(&self) {
        *self.event_collector.lock() = Arc::new(StoredEventHandler::new());
        self.reset_played_events();
    }

    /// The messages of the events collected so far, in order.
    fn events(&self) -> Vec<String> {
        messages(&self.event_collector.lock())
    }

    /// Java: `eventCollector.clear()`.
    fn clear_events(&self) {
        *self.event_collector.lock() = Arc::new(StoredEventHandler::new());
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5930 MemoizingEvaluatorTester.invalidate
    fn invalidate(&self) {
        self.evaluator.invalidate(self.get_modified_values());
        self.clear_modified_values();
        self.progress_receiver.clear();
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5937 MemoizingEvaluatorTester.invalidateTransientErrors
    fn invalidate_transient_errors(&self) {
        self.evaluator.invalidate_transient_errors();
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5941 MemoizingEvaluatorTester.delete
    fn delete(&self, key: &'static str) {
        let key = k(key);
        self.evaluator.delete(move |k, _| *k == key);
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5945 MemoizingEvaluatorTester.resetPlayedEvents
    fn reset_played_events(&self) {
        self.evaluator.emitted_event_state().clear();
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5949 MemoizingEvaluatorTester.getDirtyKeys
    fn get_dirty_keys(&self) -> HashSet<SkyKey> {
        self.progress_receiver.dirty.lock().clone()
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5953 MemoizingEvaluatorTester.getDeletedKeys
    fn get_deleted_keys(&self) -> HashSet<SkyKey> {
        self.progress_receiver.deleted.lock().clone()
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5957 MemoizingEvaluatorTester.getEnqueuedValues
    fn get_enqueued_values(&self) -> HashSet<SkyKey> {
        self.progress_receiver.enqueued.lock().clone()
    }

    /// Java: `tester.eval(keepGoing, keys)` (100 threads; here 8), or the interrupt.
    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5961 MemoizingEvaluatorTester.eval
    fn try_eval(&self, keep_going: bool, keys: &[SkyKey]) -> Result<EvaluationResult, Interrupted> {
        assert!(self.get_modified_values().is_empty());
        let ctx = EvaluationContext {
            event_handler: self.event_collector.lock().clone(),
            ..ctx(keep_going, 8)
        };
        self.evaluator.evaluate(keys, &ctx)
    }

    fn eval(&self, keep_going: bool, keys: &[SkyKey]) -> EvaluationResult {
        self.try_eval(keep_going, keys).expect("not interrupted")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:6010 MemoizingEvaluatorTester.evalAndGet
    fn eval_and_get(&self, keep_going: bool, key: &SkyKey) -> String {
        let r = self.eval(keep_going, std::slice::from_ref(key));
        r.get_dyn(key)
            .unwrap_or_else(|| panic!("{r:?}"))
            .downcast_ref::<String>()
            .expect("a string")
            .clone()
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:6017 MemoizingEvaluatorTester.evalAndGetError
    fn eval_and_get_error(&self, keep_going: bool, key: &SkyKey) -> ErrorInfo {
        let r = self.eval(keep_going, std::slice::from_ref(key));
        r.get_error(key)
            .unwrap_or_else(|| panic!("no error entry for {key:?}: {r:?}"))
            .clone()
    }

    /// Java: `tester.getExistingValue(key)`.
    fn get_existing_value(&self, key: &SkyKey) -> Option<String> {
        self.evaluator
            .get_existing_value(key)
            .map(|v| v.downcast_ref::<String>().expect("a string").clone())
    }
}

mod eager_invalidator_test;
mod engine_test;
mod graph_test;
mod grouped_deps_test;
mod in_memory_graph_test;
mod in_memory_node_entry_test;
mod incremental_in_memory_node_entry_test;
mod invalidation_progress_receiver;
mod memoizing_evaluator_test;
mod model;
mod non_incremental_in_memory_node_entry_test;
mod not_ported;
mod parallel_evaluator_test;
mod progress_receiver_test;
mod reverse_deps_utility_test;
mod tracking_progress_receiver;
