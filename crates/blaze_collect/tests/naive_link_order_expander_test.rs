//! Port of `NaiveLinkOrderExpanderTest.java`.

mod common;

expander_test_base! {
    order: blaze_collect::Order::NAIVE_LINK_ORDER,
    simple_result: ["c", "a", "b"],
    nested_result: ["b", "d", "c", "a", "e"],
    nested_duplicates_result: ["b", "d", "e", "c", "a"],
    chain_result: ["a", "b", "c"],
    // This case illustrates why this implementation is called "naive".
    diamond_result: ["a", "b", "d", "c"],
    extended_diamond_result: ["a", "b", "d", "e", "c"],
    extended_diamond_right_arm_result: ["a", "b", "d", "e", "c", "c2"],
    // Leftmost branch determines the order.
    order_conflict_result: ["a", "b"],
}
