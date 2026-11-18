//! Port of `LinkOrderExpanderTest.java`.

mod common;

expander_test_base! {
    order: blaze_collect::Order::LINK_ORDER,
    simple_result: ["c", "a", "b"],
    nested_result: ["b", "d", "c", "a", "e"],
    nested_duplicates_result: ["b", "d", "c", "a", "e"],
    chain_result: ["a", "b", "c"],
    diamond_result: ["a", "b", "c", "d"],
    extended_diamond_result: ["a", "b", "c", "e", "d"],
    extended_diamond_right_arm_result: ["a", "b", "c", "c2", "e", "d"],
    // Rightmost branch determines the order.
    order_conflict_result: ["b", "a"],
}
