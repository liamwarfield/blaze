//! `EvaluatorBuilder::build_reusing`: a graph handed from one evaluator to the next (Blaze-only,
//! the server's kept graph; no Java test).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{
    EvaluationContext, EvaluatorBuilder, FunctionHermeticity, Key, SkyFunctionName, SkyKey,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Leaf(&'static str);
impl Key for Leaf {
    type Value = String;
    const FUNCTION: SkyFunctionName = SkyFunctionName("REUSE_LEAF");
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
    fn values_equal(old: &String, new: &String) -> bool {
        old == new
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Top;
impl Key for Top {
    type Value = String;
    const FUNCTION: SkyFunctionName = SkyFunctionName("REUSE_TOP");
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
}

fn builder(runs: Arc<AtomicUsize>, leaf: &str) -> EvaluatorBuilder {
    EvaluatorBuilder::new()
        .typed_function::<Top, _>(move |_, env| {
            runs.fetch_add(1, Ordering::SeqCst);
            let Some(v) = env.get_value(Leaf("a"))? else {
                return Ok(None);
            };
            Ok(Some(format!("top({v})")))
        })
        .inject(Leaf("a"), leaf.to_owned())
}

fn top(ev: &crate::Evaluator) -> String {
    let r = ev
        .evaluate(&[SkyKey::new(Top)], &EvaluationContext::default())
        .unwrap();
    (*r.get(&Top).unwrap()).clone()
}

#[test]
fn equal_injection_keeps_the_graph_clean() {
    let first_runs = Arc::new(AtomicUsize::new(0));
    let ev = builder(first_runs.clone(), "x").build();
    assert_eq!(top(&ev), "top(x)");
    assert_eq!(first_runs.load(Ordering::SeqCst), 1);

    // The next command's functions over the same graph; the precomputed value is re-injected
    // with an equal value: nothing runs.
    let second_runs = Arc::new(AtomicUsize::new(0));
    let ev = builder(second_runs.clone(), "x").build_reusing(ev);
    assert_eq!(top(&ev), "top(x)");
    assert_eq!(second_runs.load(Ordering::SeqCst), 0);
}

#[test]
fn changed_injection_reruns_with_the_new_functions() {
    let first_runs = Arc::new(AtomicUsize::new(0));
    let ev = builder(first_runs.clone(), "x").build();
    assert_eq!(top(&ev), "top(x)");

    let second_runs = Arc::new(AtomicUsize::new(0));
    let ev = builder(second_runs.clone(), "y").build_reusing(ev);
    assert_eq!(top(&ev), "top(y)");
    assert_eq!(first_runs.load(Ordering::SeqCst), 1);
    assert_eq!(second_runs.load(Ordering::SeqCst), 1);
}

#[test]
fn pending_invalidation_moves_with_the_graph() {
    let runs = Arc::new(AtomicUsize::new(0));
    let ev = builder(runs.clone(), "x").build();
    assert_eq!(top(&ev), "top(x)");
    // An invalidation recorded on the old evaluator applies at the new one's first evaluation.
    ev.inject(Leaf("a"), "z".to_owned());
    let ev = EvaluatorBuilder::new().build_reusing(ev);
    let runs2 = Arc::new(AtomicUsize::new(0));
    let ev = EvaluatorBuilder::new()
        .typed_function::<Top, _>({
            let runs2 = runs2.clone();
            move |_, env| {
                runs2.fetch_add(1, Ordering::SeqCst);
                let Some(v) = env.get_value(Leaf("a"))? else {
                    return Ok(None);
                };
                Ok(Some(format!("top({v})")))
            }
        })
        .build_reusing(ev);
    assert_eq!(top(&ev), "top(z)");
    assert_eq!(runs2.load(Ordering::SeqCst), 1);
}
