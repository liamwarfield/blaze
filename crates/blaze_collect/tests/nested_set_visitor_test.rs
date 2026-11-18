//! Port of `NestedSetVisitorTest.java`.

use std::collections::HashSet;

use blaze_collect::{NestedSet, NestedSetBuilder, NestedSetNode, NestedSetVisitor};

fn visit_all(set: &NestedSet<i32>) -> Vec<i32> {
    let mut visited: Vec<i32> = Vec::new();
    let mut seen: HashSet<NestedSetNode<i32>> = HashSet::new();
    NestedSetVisitor::new(
        |e: &i32| visited.push(*e),
        |n: &NestedSetNode<i32>| seen.insert(n.clone()),
    )
    .visit(set);
    visited
}

#[test]
fn stable_order() {
    let set = NestedSetBuilder::<i32>::stable_order()
        .add_transitive(
            &NestedSetBuilder::stable_order()
                .add(1)
                .add(2)
                .add(3)
                .build(),
        )
        .add(4)
        .add(5)
        .add(6)
        .add_transitive(
            &NestedSetBuilder::stable_order()
                .add(7)
                .add(8)
                .add(9)
                .build(),
        )
        .build();

    assert_eq!(visit_all(&set), *set.to_list());
}

#[test]
fn compile_order() {
    let set = NestedSetBuilder::<i32>::compile_order()
        .add_transitive(
            &NestedSetBuilder::compile_order()
                .add(1)
                .add(2)
                .add(3)
                .build(),
        )
        .add(4)
        .add(5)
        .add(6)
        .add_transitive(
            &NestedSetBuilder::compile_order()
                .add(7)
                .add(8)
                .add(9)
                .build(),
        )
        .build();

    assert_eq!(visit_all(&set), *set.to_list());
}

#[test]
fn link_order() {
    let set = NestedSetBuilder::<i32>::link_order()
        .add_transitive(&NestedSetBuilder::link_order().add(1).add(2).add(3).build())
        .add(4)
        .add(5)
        .add(6)
        .add_transitive(&NestedSetBuilder::link_order().add(7).add(8).add(9).build())
        .build();

    // #toList() for LINK_ORDER reverses the result list.
    let mut reversed = (*set.to_list()).clone();
    reversed.reverse();
    assert_eq!(visit_all(&set), reversed);
}

#[test]
fn naive_link_order() {
    let set = NestedSetBuilder::<i32>::naive_link_order()
        .add_transitive(
            &NestedSetBuilder::naive_link_order()
                .add(1)
                .add(2)
                .add(3)
                .build(),
        )
        .add(4)
        .add(5)
        .add(6)
        .add_transitive(
            &NestedSetBuilder::naive_link_order()
                .add(7)
                .add(8)
                .add(9)
                .build(),
        )
        .build();

    assert_eq!(visit_all(&set), *set.to_list());
}

#[test]
fn mixed_orders() {
    let set = NestedSetBuilder::<i32>::stable_order()
        .add_transitive(
            &NestedSetBuilder::link_order()
                .add(1)
                .add_transitive(&NestedSetBuilder::link_order().add(2).add(3).build())
                .add_transitive(&NestedSetBuilder::link_order().add(4).add(5).build())
                .build(),
        )
        .add(6)
        .add(7)
        .add(8)
        .add_transitive(
            &NestedSetBuilder::naive_link_order()
                .add_transitive(&NestedSetBuilder::naive_link_order().add(7).add(8).build())
                .add_transitive(&NestedSetBuilder::naive_link_order().add(9).add(10).build())
                .add(11)
                .build(),
        )
        .build();

    assert_eq!(visit_all(&set), *set.to_list());
}

#[test]
fn duplicates_skipped() {
    let subset = NestedSetBuilder::<i32>::compile_order()
        .add(1)
        .add(2)
        .add(3)
        .build();
    let set = NestedSetBuilder::<i32>::compile_order()
        .add_transitive(&subset)
        .add_transitive(
            &NestedSetBuilder::compile_order()
                .add(4)
                .add_transitive(&subset)
                .build(),
        )
        .add(5)
        .add(6)
        .add(7)
        .add_transitive(
            &NestedSetBuilder::compile_order()
                .add(8)
                .add(9)
                .add_transitive(&subset)
                .build(),
        )
        .build();

    let visited = visit_all(&set);
    assert_eq!(visited, *set.to_list());
    assert_eq!(visited.len(), 9);
}
