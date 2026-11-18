//! Port of `CompileOrderExpanderTest.java`.

mod common;

expander_test_base! {
    order: blaze_collect::Order::COMPILE_ORDER,
    simple_result: ["c", "a", "b"],
    nested_result: ["c", "a", "e", "b", "d"],
    nested_duplicates_result: ["c", "a", "e", "b", "d"],
    chain_result: ["c", "b", "a"],
    diamond_result: ["d", "b", "c", "a"],
    extended_diamond_result: ["d", "e", "b", "c", "a"],
    extended_diamond_right_arm_result: ["d", "e", "b", "c2", "c", "a"],
    order_conflict_result: ["a", "b"],
}
