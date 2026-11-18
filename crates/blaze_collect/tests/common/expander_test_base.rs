//! Port of `src/test/java/com/google/devtools/build/lib/collect/nestedset/ExpanderTestBase.java`.
//!
//! Base for tests of NestedSet iteration behavior. `expander_test_base!` instantiates the shared
//! test cases for one order; the expected results are supplied by the invoking test file.

use std::panic::{AssertUnwindSafe, catch_unwind};

use blaze_collect::{NestedSet, NestedSetBuilder, Order};

pub fn prepare_builder(order: Order, direct_members: &[&str]) -> NestedSetBuilder<String> {
    let mut builder = NestedSetBuilder::new_builder(order);
    builder.add_all(direct_members.iter().map(|x| x.to_string()));
    builder
}

pub fn assert_set_contents(expected: &[&str], set: &NestedSet<String>) {
    let expected: Vec<String> = expected.iter().map(|x| x.to_string()).collect();
    assert_eq!(*set.to_list(), expected);
    assert_eq!(set.to_set().into_iter().collect::<Vec<_>>(), expected);
}

pub fn assert_collections_equal(expected: &[String], actual: &[String]) {
    assert_eq!(actual, expected);
}

/// Java: ExpanderTestBase.nestingValidation — `addTransitive` must throw
/// IllegalArgumentException exactly for incompatible orders.
pub fn nesting_validation(expander_order: Order) {
    for ordering in Order::VALUES {
        let a = prepare_builder(expander_order, &["a", "b"]).build();
        let mut b = NestedSetBuilder::<String>::new_builder(ordering);
        let result = catch_unwind(AssertUnwindSafe(|| {
            b.add_transitive(&a);
        }));
        match result {
            Ok(()) => {
                if ordering != expander_order && ordering != Order::STABLE_ORDER {
                    panic!("An exception was expected.");
                }
            }
            Err(e) => {
                let msg = panic_message(&*e);
                assert!(msg.starts_with("IllegalArgumentException: "), "{msg}");
                if ordering == expander_order || ordering == Order::STABLE_ORDER {
                    panic!("No exception was expected.");
                }
            }
        }
    }
}

pub fn panic_message(err: &(dyn std::any::Any + Send)) -> String {
    err.downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

#[macro_export]
macro_rules! expander_test_base {
    (
        order: $order:expr,
        simple_result: $simple:expr,
        nested_result: $nested:expr,
        nested_duplicates_result: $nested_dup:expr,
        chain_result: $chain:expr,
        diamond_result: $diamond:expr,
        extended_diamond_result: $ext_diamond:expr,
        extended_diamond_right_arm_result: $ext_right:expr,
        order_conflict_result: $conflict:expr $(,)?
    ) => {
        use common::expander_test_base::{
            assert_collections_equal, assert_set_contents, prepare_builder,
        };

        fn expander_order() -> blaze_collect::Order {
            $order
        }

        fn pb(direct: &[&str]) -> blaze_collect::NestedSetBuilder<String> {
            prepare_builder(expander_order(), direct)
        }

        fn s(x: &str) -> String {
            x.to_string()
        }

        #[test]
        fn simple() {
            let s_ = pb(&["c", "a", "b"]).build();
            let expected: Vec<String> = $simple.iter().map(|x: &&str| x.to_string()).collect();
            assert_eq!(*s_.to_list(), expected);
            assert_set_contents(&$simple, &s_);
        }

        #[test]
        fn simple_no_duplicates() {
            let s_ = pb(&["c", "a", "a", "a", "b"]).build();
            let expected: Vec<String> = $simple.iter().map(|x: &&str| x.to_string()).collect();
            assert_eq!(*s_.to_list(), expected);
            assert_set_contents(&$simple, &s_);
        }

        #[test]
        fn nesting() {
            let subset = pb(&["c", "a", "e"]).build();
            let s_ = pb(&["b", "d"]).add_transitive(&subset).build();
            assert_set_contents(&$nested, &s_);
        }

        #[test]
        fn builder_reuse() {
            let mut builder = pb(&[]);
            assert_set_contents(&[], &builder.build());

            builder.add(s("b"));
            assert_set_contents(&["b"], &builder.build());

            builder.add_all(vec![s("d")]);
            let expected = pb(&["b", "d"]).build().to_list();
            let expected: Vec<&str> = expected.iter().map(|x| x.as_str()).collect();
            assert_set_contents(&expected, &builder.build());

            let child = pb(&["c", "a", "e"]).build();
            builder.add_transitive(&child);
            assert_set_contents(&$nested, &builder.build());
        }

        #[test]
        fn builder_chaining() {
            let s_ = pb(&[])
                .add(s("b"))
                .add_all(vec![s("d")])
                .add_transitive(&pb(&["c", "a", "e"]).build())
                .build();
            assert_set_contents(&$nested, &s_);
        }

        #[test]
        fn add_all_ordering() {
            let s1 = pb(&[]).add(s("a")).add(s("c")).add(s("b")).build();
            let s2 = pb(&[]).add_all(vec![s("a"), s("c"), s("b")]).build();
            assert_collections_equal(&s1.to_list(), &s2.to_list());
        }

        #[test]
        fn mixed_add_all_ordering() {
            let s1 = pb(&[])
                .add(s("a"))
                .add(s("b"))
                .add(s("c"))
                .add(s("d"))
                .build();
            let s2 = pb(&[])
                .add(s("a"))
                .add_all(vec![s("b"), s("c")])
                .add(s("d"))
                .build();
            assert_collections_equal(&s1.to_list(), &s2.to_list());
        }

        #[test]
        fn transitive_deps_handled_separately() {
            let subset = pb(&["c", "a", "e"]).build();
            let mut b = pb(&[]);
            // The fact that we add the transitive subset between the add("b") and add("d") calls
            // should not change the result.
            b.add(s("b"));
            b.add_transitive(&subset);
            b.add(s("d"));
            let s_ = b.build();
            assert_set_contents(&$nested, &s_);
        }

        #[test]
        fn nesting_no_duplicates() {
            let subset = pb(&["c", "a", "e"]).build();
            let s_ = pb(&["b", "d", "e"]).add_transitive(&subset).build();
            assert_set_contents(&$nested_dup, &s_);
        }

        #[test]
        fn chain() {
            let c = pb(&["c"]).build();
            let b = pb(&["b"]).add_transitive(&c).build();
            let a = pb(&["a"]).add_transitive(&b).build();
            assert_set_contents(&$chain, &a);
        }

        #[test]
        fn diamond() {
            let d = pb(&["d"]).build();
            let c = pb(&["c"]).add_transitive(&d).build();
            let b = pb(&["b"]).add_transitive(&d).build();
            let a = pb(&["a"]).add_transitive(&b).add_transitive(&c).build();
            assert_set_contents(&$diamond, &a);
        }

        #[test]
        fn extended_diamond() {
            let d = pb(&["d"]).build();
            let e = pb(&["e"]).build();
            let b = pb(&["b"]).add_transitive(&d).add_transitive(&e).build();
            let c = pb(&["c"]).add_transitive(&e).add_transitive(&d).build();
            let a = pb(&["a"]).add_transitive(&b).add_transitive(&c).build();
            assert_set_contents(&$ext_diamond, &a);
        }

        #[test]
        fn extended_diamond_right_arm() {
            let d = pb(&["d"]).build();
            let e = pb(&["e"]).build();
            let b = pb(&["b"]).add_transitive(&d).add_transitive(&e).build();
            let c2 = pb(&["c2"]).add_transitive(&e).add_transitive(&d).build();
            let c = pb(&["c"]).add_transitive(&c2).build();
            let a = pb(&["a"]).add_transitive(&b).add_transitive(&c).build();
            assert_set_contents(&$ext_right, &a);
        }

        #[test]
        fn order_conflict() {
            let child1 = pb(&["a", "b"]).build();
            let child2 = pb(&["b", "a"]).build();
            let parent = pb(&[])
                .add_transitive(&child1)
                .add_transitive(&child2)
                .build();
            assert_set_contents(&$conflict, &parent);
        }

        #[test]
        fn order_conflict_nested() {
            let a = pb(&["a"]).build();
            let b = pb(&["b"]).build();
            let child1 = pb(&[]).add_transitive(&a).add_transitive(&b).build();
            let child2 = pb(&[]).add_transitive(&b).add_transitive(&a).build();
            let parent = pb(&[])
                .add_transitive(&child1)
                .add_transitive(&child2)
                .build();
            assert_set_contents(&$conflict, &parent);
        }

        #[test]
        fn get_ordering_empty() {
            let s_ = pb(&[]).build();
            assert!(s_.is_empty());
            assert_eq!(s_.get_order(), expander_order());
        }

        #[test]
        fn get_ordering() {
            let s_ = pb(&["a", "b"]).build();
            assert!(!s_.is_empty());
            assert_eq!(s_.get_order(), expander_order());
        }

        #[test]
        fn nesting_validation() {
            common::expander_test_base::nesting_validation(expander_order());
        }
    };
}
