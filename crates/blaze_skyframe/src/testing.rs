//! Test support for crates that declare keys: change pruning of one key's values, observed
//! through the engine.
//!
//! Not a Java class. Java tests change-prune with `GraphTester` keys; this checks a crate's own
//! [`Key::values_equal`] the way the evaluator uses it (`DirtyBuildingState.unchangedFromLastBuild`).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use parking_lot::Mutex;

use crate::{
    EvaluationContext, EvaluatorBuilder, FunctionHermeticity, Key, SkyFunctionName, SkyKey,
};

/// The input the child reads: a non-hermetic variable, re-injected to dirty the child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Input;

impl Key for Input {
    type Value = u64;
    const FUNCTION: SkyFunctionName = SkyFunctionName("CHANGE_PRUNING_TEST_INPUT");
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
}

/// The parent: requests the child; counts its runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Parent;

impl Key for Parent {
    type Value = ();
    const FUNCTION: SkyFunctionName = SkyFunctionName("CHANGE_PRUNING_TEST_PARENT");
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
}

/// Runs of the child and of its parent over two evaluations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PruningRuns {
    pub child: usize,
    pub parent: usize,
}

/// Evaluates a parent that requests `key`, whose function returns `old`; then changes the
/// child's input so that the child is recomputed and returns `new`, and evaluates the parent
/// again. The parent runs again only if `K::values_equal(old, new)` is false (Java: the child
/// is change-pruned when its new value `equals` the last build's).
pub fn runs_after_recompute<K: Key + Clone>(key: K, old: K::Value, new: K::Value) -> PruningRuns {
    let next: Arc<Mutex<Vec<K::Value>>> = Arc::new(Mutex::new(vec![new, old]));
    let child_runs = Arc::new(AtomicUsize::new(0));
    let parent_runs = Arc::new(AtomicUsize::new(0));
    let ev = {
        let (next, child_runs, parent_runs) =
            (next.clone(), child_runs.clone(), parent_runs.clone());
        let key = key.clone();
        EvaluatorBuilder::new()
            .inject(Input, 0)
            .typed_function::<K, _>(move |_, env| {
                if env.get_value(Input)?.is_none() {
                    return Ok(None);
                }
                child_runs.fetch_add(1, Ordering::SeqCst);
                Ok(Some(next.lock().pop().expect("a value per computation")))
            })
            .typed_function::<Parent, _>(move |_, env| {
                if env.get_value(key.clone())?.is_none() {
                    return Ok(None);
                }
                parent_runs.fetch_add(1, Ordering::SeqCst);
                Ok(Some(()))
            })
            .build()
    };
    let ctx = EvaluationContext {
        keep_going: false,
        parallelism: 1,
        ..EvaluationContext::default()
    };
    let roots = [SkyKey::new(Parent)];
    let evaluate = || {
        let r = ev.evaluate(&roots, &ctx).expect("not interrupted");
        assert!(!r.has_error(), "{r:?}");
    };
    evaluate();
    ev.inject(Input, 1);
    evaluate();
    PruningRuns {
        child: child_runs.load(Ordering::SeqCst),
        parent: parent_runs.load(Ordering::SeqCst),
    }
}

/// Whether the parent of a child recomputed from `old` to `new` was rebuilt (asserts that the
/// child itself was recomputed).
pub fn parent_rebuilt<K: Key + Clone>(key: K, old: K::Value, new: K::Value) -> bool {
    let runs = runs_after_recompute(key, old, new);
    assert_eq!(runs.child, 2, "the child is recomputed");
    match runs.parent {
        1 => false,
        2 => true,
        n => panic!("parent ran {n} times"),
    }
}

/// Guava `EqualsTester` over [`Key::values_equal`]: the values of a group are equal to each
/// other (as old and as new value), the values of different groups are not. A value is not
/// compared with itself: `values_equal` compares a recomputed value with the last build's, and
/// there is no identity shortcut (docs/decisions.md D10), so a value of an identity-equality
/// class is a group of one.
pub fn values_equal_tester<K: Key>(groups: &[Vec<K::Value>])
where
    K::Value: std::fmt::Debug,
{
    for (i, group) in groups.iter().enumerate() {
        for (ia, a) in group.iter().enumerate() {
            for (ib, b) in group.iter().enumerate() {
                if ia != ib {
                    assert!(K::values_equal(a, b), "group {i}: {a:?} vs {b:?}");
                }
            }
            for (j, other) in groups.iter().enumerate() {
                if i != j {
                    for b in other {
                        assert!(!K::values_equal(a, b), "groups {i} and {j}: {a:?} vs {b:?}");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct Comparable;
    impl Key for Comparable {
        type Value = String;
        const FUNCTION: SkyFunctionName = SkyFunctionName("COMPARABLE");
        const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
        fn values_equal(old: &String, new: &String) -> bool {
            old == new
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct NotComparable;
    impl Key for NotComparable {
        type Value = String;
        const FUNCTION: SkyFunctionName = SkyFunctionName("NOT_COMPARABLE");
        const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    }

    #[test]
    fn equal_value_prunes_the_parent() {
        assert!(!parent_rebuilt(Comparable, "a".into(), "a".into()));
        assert!(parent_rebuilt(Comparable, "a".into(), "b".into()));
    }

    #[test]
    fn not_comparable_value_never_prunes() {
        assert!(parent_rebuilt(NotComparable, "a".into(), "a".into()));
    }
}
