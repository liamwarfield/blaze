//! Port of `NestedSetTopologyTest.java` (tests of toNode, getNonLeaves, getLeaves).

use std::collections::HashSet;
use std::panic::catch_unwind;

use blaze_collect::{NestedSet, NestedSetBuilder, NestedSetNode};
use indexmap::IndexSet;

fn s(x: &str) -> String {
    x.to_string()
}

#[test]
fn test_to_node() {
    let inner = NestedSetBuilder::<String>::stable_order()
        .add(s("a"))
        .add(s("b"))
        .build();
    let outer = NestedSetBuilder::<String>::stable_order()
        .add_transitive(&inner)
        .add(s("c"))
        .build();
    let flat = NestedSetBuilder::<String>::stable_order()
        .add(s("a"))
        .add(s("b"))
        .add(s("c"))
        .build();

    assert_eq!(inner.to_node(), inner.to_node());

    // Sets with different internal structure should have different nodes
    assert_ne!(flat.to_node(), outer.to_node());

    // Decomposing a set, the transitive sets should be correctly identified.
    let succs = outer.get_non_leaves();
    assert_eq!(succs.len(), 1);
    let succ0 = &succs[0];
    assert_eq!(succ0.to_node(), inner.to_node());
}

#[test]
fn test_get_leaves() {
    let inner = NestedSetBuilder::<String>::stable_order()
        .add(s("a"))
        .add(s("b"))
        .build();
    let outer = NestedSetBuilder::<String>::stable_order()
        .add(s("c"))
        .add_transitive(&inner)
        .add(s("d"))
        .add(s("e"))
        .build();

    // The direct members should correctly be identified.
    let mut leaves = outer.get_leaves();
    leaves.sort();
    assert_eq!(leaves, vec![s("c"), s("d"), s("e")]);
}

#[test]
fn test_get_non_leaves() {
    // The inner sets must have at least two elements, as NestedSet inlines singleton sets.
    let inner_a = NestedSetBuilder::<String>::stable_order()
        .add(s("a1"))
        .add(s("a2"))
        .build();
    let inner_b = NestedSetBuilder::<String>::stable_order()
        .add(s("b1"))
        .add(s("b2"))
        .build();
    let inner_c = NestedSetBuilder::<String>::stable_order()
        .add(s("c1"))
        .add(s("c2"))
        .build();
    let outer = NestedSetBuilder::<String>::stable_order()
        .add(s("x"))
        .add(s("y"))
        .add_transitive(&inner_a)
        .add_transitive(&inner_b)
        .add_transitive(&inner_c)
        .add(s("z"))
        .build();

    // Decomposing the nested set should give us the correct list of transitive members.
    // Compare using strings as NestedSet.equals uses identity.
    let to_strings =
        |sets: &[NestedSet<String>]| sets.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        to_strings(&outer.get_non_leaves()),
        to_strings(&[inner_a, inner_b, inner_c])
    );
}

/// Naively traverse a view and collect all elements reachable.
fn contents(set: &NestedSet<String>) -> IndexSet<String> {
    let mut builder = IndexSet::new();
    builder.extend(set.get_leaves());
    for nonleaf in set.get_non_leaves() {
        builder.extend(contents(&nonleaf));
    }
    builder
}

fn nodes(sets: &[NestedSet<String>]) -> HashSet<NestedSetNode<String>> {
    sets.iter().map(|set| set.to_node()).collect()
}

#[test]
fn test_contents() {
    // Verify that the elements reachable from view are the correct ones, regardless if singletons
    // are inlined or not. Also verify that sets with at least two elements are never inlined.
    let single_a = NestedSetBuilder::<String>::stable_order()
        .add(s("a"))
        .build();
    let single_b = NestedSetBuilder::<String>::stable_order()
        .add(s("b"))
        .build();
    let multi = NestedSetBuilder::<String>::stable_order()
        .add(s("c1"))
        .add(s("c2"))
        .build();
    let outer = NestedSetBuilder::<String>::stable_order()
        .add(s("x"))
        .add(s("y"))
        .add_transitive(&multi)
        .add_transitive(&single_a)
        .add_transitive(&single_b)
        .add(s("z"))
        .build();

    let mut c: Vec<String> = contents(&outer).into_iter().collect();
    c.sort();
    assert_eq!(
        c,
        vec![s("a"), s("b"), s("c1"), s("c2"), s("x"), s("y"), s("z")]
    );
    assert!(nodes(&outer.get_non_leaves()).contains(&multi.to_node()));
}

#[test]
fn test_split_fails() {
    let a = NestedSetBuilder::<String>::stable_order()
        .add(s("a"))
        .add(s("b"))
        .build();
    for max in [-100, 1] {
        let err = catch_unwind(|| a.split_if_exceeds_maximum_size(max))
            .expect_err("expected IllegalArgumentException");
        assert_eq!(
            panic_message(&*err),
            "IllegalArgumentException: maxDegree must be at least 2"
        );
    }
}

#[test]
fn test_split_no_split() {
    let a = NestedSetBuilder::<String>::stable_order()
        .add(s("a"))
        .add(s("b"))
        .build();
    assert!(a.split_if_exceeds_maximum_size(2) == a); // isSameInstanceAs
    assert!(a.split_if_exceeds_maximum_size(100) == a);
}

#[test]
fn test_split() {
    let a = NestedSetBuilder::<String>::stable_order()
        .add_all(vec![s("a"), s("b"), s("c")])
        .build();
    let v = a;
    let s_ = v.split_if_exceeds_maximum_size(2);
    assert!(s_ != v); // isNotSameInstanceAs
    let mut got = collect_check_size(&s_, 2);
    got.sort();
    assert_eq!(got, vec![s("a"), s("b"), s("c")]);
}

#[test]
fn test_recursive_split() {
    let a = NestedSetBuilder::<String>::stable_order()
        .add_all(vec![s("a"), s("b"), s("c"), s("d"), s("e")])
        .build();
    let v = a;
    let s_ = v.split_if_exceeds_maximum_size(2);
    assert!(s_ != v);
    let mut got = collect_check_size(&s_, 2);
    got.sort();
    assert_eq!(got, vec![s("a"), s("b"), s("c"), s("d"), s("e")]);

    // Splitting may increment the graph depth, possibly more than once.
    assert_eq!(v.get_approx_depth(), 2);
    assert_eq!(s_.get_approx_depth(), 4);
}

fn collect_check_size<T: Clone + Eq + std::hash::Hash>(
    set: &NestedSet<T>,
    max_size: usize,
) -> Vec<T> {
    let mut result = Vec::new();
    collect_check_size_into(&mut result, set, max_size);
    result
}

fn collect_check_size_into<T: Clone + Eq + std::hash::Hash>(
    result: &mut Vec<T>,
    set: &NestedSet<T>,
    max_size: usize,
) {
    assert!(set.get_leaves().len() <= max_size);
    assert!(set.get_non_leaves().len() <= max_size);
    for nonleaf in set.get_non_leaves() {
        collect_check_size_into(result, &nonleaf, max_size);
    }
    result.extend(set.get_leaves());
}

fn panic_message(err: &(dyn std::any::Any + Send)) -> String {
    err.downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}
