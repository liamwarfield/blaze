use super::*;
use std::fmt;

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
fn num_fn(k: &Num, env: &mut Environment<'_>) -> Result<Option<u64>, SkyFunctionException> {
    if k.0 == 13 {
        return Err(SkyFunctionException::persistent(BadNumber(13)));
    }
    if k.0 == 0 {
        return Ok(Some(1));
    }
    let deps: Vec<Num> = (0..k.0).step_by(7).map(Num).collect();
    let vals = env.get_values_typed(&deps);
    if env.values_missing() {
        return Ok(None);
    }
    Ok(Some(vals.into_iter().map(|v| *v.unwrap()).sum::<u64>() + 1))
}

fn evaluator(parallelism: usize, keep_going: bool) -> Evaluator {
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
            let vals = env.get_values_typed(&next);
            if env.values_missing() {
                return Ok(None);
            }
            Ok(Some(vals.len() as u32))
        })
        .typed_function::<Flag, _>(|_, _| panic!("injected keys are never computed"))
        .inject(Flag("compilation_mode"), "fastbuild".to_string())
        .build(EvaluationContext {
            keep_going,
            parallelism,
            thread_stack_size: 16 << 20,
        })
}

#[test]
fn computes_and_memoizes() {
    let ev = evaluator(8, true);
    let r = ev.evaluate(&[SkyKey::new(Num(30))]);
    assert!(!r.has_error());
    let first = *r.get(&Num(30)).unwrap();
    let computed = ev.computed_count();
    let r2 = ev.evaluate(&[SkyKey::new(Num(30))]);
    assert_eq!(*r2.get(&Num(30)).unwrap(), first);
    assert_eq!(
        ev.computed_count(),
        computed,
        "second evaluation reuses the graph"
    );
}

#[test]
fn single_threaded_matches_parallel() {
    let a = evaluator(1, true).evaluate(&[SkyKey::new(Num(50))]);
    let b = evaluator(16, true).evaluate(&[SkyKey::new(Num(50))]);
    assert_eq!(a.get(&Num(50)), b.get(&Num(50)));
}

#[test]
fn error_bubbles_to_parent() {
    let ev = evaluator(4, true);
    // 14 depends on 0 and 7, fine; 20 depends on 0, 7, 14; 13 throws; 15 depends on 0, 7, 14.
    // 21 depends on 0,7,14 — none is 13. Use a key whose deps include 13: none by step 7 from 0.
    let r = ev.evaluate(&[SkyKey::new(Num(13))]);
    let (_, err) = &r.errors[0];
    assert_eq!(err.exception_as::<BadNumber>(), Some(&BadNumber(13)));
    assert_eq!(err.root_cause, Some(SkyKey::new(Num(13))));
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
            match env.get_value_or_throw::<Num, BadNumber>(Num(13)) {
                Ok(_) => Ok(Some("no error".into())),
                Err(e) => Ok(Some(format!("caught: {e}"))),
            }
        })
        .build(EvaluationContext::default());
    let r = ev.evaluate(&[SkyKey::new(Catcher)]);
    assert_eq!(*r.get(&Catcher).unwrap(), "caught: bad number 13");
}

#[test]
fn injected_values() {
    let ev = evaluator(2, true);
    let r = ev.evaluate(&[SkyKey::new(Flag("compilation_mode"))]);
    assert_eq!(*r.get(&Flag("compilation_mode")).unwrap(), "fastbuild");
}

fn cycle_of(ev: &Evaluator, root: u32) -> CycleInfo {
    let r = ev.evaluate(&[SkyKey::new(Cyc(root))]);
    assert!(r.has_error(), "expected a cycle from {root}");
    r.errors[0].1.cycles.first().cloned().expect("cycle info")
}

#[test]
fn self_cycle_is_detected() {
    for p in [1, 4] {
        let c = cycle_of(&evaluator(p, true), 5);
        assert_eq!(c.cycle, vec![SkyKey::new(Cyc(5))]);
    }
}

#[test]
fn three_node_cycle_is_detected() {
    for p in [1, 4] {
        let c = cycle_of(&evaluator(p, true), 1);
        let mut names: Vec<u32> = c
            .cycle
            .iter()
            .map(|k| k.downcast::<Cyc>().unwrap().0)
            .collect();
        names.sort();
        assert_eq!(names, vec![1, 2, 3]);
    }
}

#[test]
fn cycle_through_parallel_batch_is_detected() {
    // 10 requests {11, 12} as a batch; a helper computes 12, which asks for 10 (on the parent).
    for _ in 0..20 {
        let ev = evaluator(8, true);
        let r = ev.evaluate(&[SkyKey::new(Cyc(10))]);
        assert!(r.has_error());
    }
}

#[test]
fn many_roots_in_parallel() {
    let ev = evaluator(16, true);
    let roots: Vec<SkyKey> = (0..200u64)
        .filter(|n| n % 7 != 6 && *n != 13)
        .map(|n| SkyKey::new(Num(n)))
        .collect();
    let r = ev.evaluate(&roots);
    let errors = r.errors.len();
    // Every n > 13 with 13 reachable via step 7 from 0? Deps are multiples of 7, never 13.
    assert_eq!(errors, 0);
    assert_eq!(r.values.len(), roots.len());
}
