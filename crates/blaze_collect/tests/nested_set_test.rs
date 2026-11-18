//! Port of `src/test/java/com/google/devtools/build/lib/collect/nestedset/NestedSetTest.java`.

use std::hash::{Hash, Hasher};
use std::panic::{AssertUnwindSafe, catch_unwind};

use blaze_collect::nestedset::MAX_ELEMENTS_TO_STRING;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};

fn s(x: &str) -> String {
    x.to_string()
}

fn nested_set_builder(direct_members: &[&str]) -> NestedSetBuilder<String> {
    let mut builder = NestedSetBuilder::stable_order();
    builder.add_all(direct_members.iter().map(|x| x.to_string()));
    builder
}

fn list(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|x| x.to_string()).collect()
}

#[test]
fn simple() {
    let set = nested_set_builder(&["a"]).build();

    assert_eq!(*set.to_list(), list(&["a"]));
    assert!(!set.is_empty());
}

#[test]
fn flat_to_string() {
    assert_eq!(nested_set_builder(&[]).build().to_string(), "[]");
    assert_eq!(nested_set_builder(&["a"]).build().to_string(), "[a]");
    assert_eq!(
        nested_set_builder(&["a", "b"]).build().to_string(),
        "[a, b]"
    );
}

#[test]
fn nested_to_string() {
    let b = nested_set_builder(&["b1", "b2"]).build();
    let c = nested_set_builder(&["c1", "c2"]).build();

    assert_eq!(
        nested_set_builder(&["a"])
            .add_transitive(&b)
            .build()
            .to_string(),
        "[b1, b2, a]"
    );
    assert_eq!(
        nested_set_builder(&["a"])
            .add_transitive(&b)
            .add_transitive(&c)
            .build()
            .to_string(),
        "[b1, b2, c1, c2, a]"
    );
    let link_order_set = NestedSetBuilder::<String>::link_order()
        .add(s("a"))
        .add_transitive(&b)
        .add_transitive(&c)
        .build();
    assert_eq!(link_order_set.to_string(), "[a, b2, b1, c2, c1]");

    assert_eq!(
        nested_set_builder(&[])
            .add_transitive(&b)
            .build()
            .to_string(),
        "[b1, b2]"
    );
}

#[test]
fn too_long_to_string() {
    let mut builder = NestedSetBuilder::<i32>::stable_order();
    for i in 0..(MAX_ELEMENTS_TO_STRING + 3) {
        builder.add(i as i32);
    }
    let string_rep = builder.build().to_string();
    assert!(string_rep.starts_with("[0, 1, 2, 3"));
    let suffix = format!(
        ", {}, {}] (truncated, full size {})",
        MAX_ELEMENTS_TO_STRING - 2,
        MAX_ELEMENTS_TO_STRING - 1,
        MAX_ELEMENTS_TO_STRING + 3
    );
    assert!(
        string_rep.ends_with(&suffix),
        "{}",
        &string_rep[string_rep.len() - 80..]
    );
}

#[test]
fn is_empty() {
    let trivially_empty = nested_set_builder(&[]).build();
    assert!(trivially_empty.is_empty());

    let empty_level1 = nested_set_builder(&[])
        .add_transitive(&trivially_empty)
        .build();
    assert!(empty_level1.is_empty());

    let empty_level2 = nested_set_builder(&[])
        .add_transitive(&empty_level1)
        .build();
    assert!(empty_level2.is_empty());

    let trivially_non_empty = nested_set_builder(&["mango"]).build();
    assert!(!trivially_non_empty.is_empty());

    let non_empty_level1 = nested_set_builder(&[])
        .add_transitive(&trivially_non_empty)
        .build();
    assert!(!non_empty_level1.is_empty());

    let non_empty_level2 = nested_set_builder(&[])
        .add_transitive(&non_empty_level1)
        .build();
    assert!(!non_empty_level2.is_empty());
}

#[test]
fn can_include_any_order_in_stable_order_and_vice_versa() {
    NestedSetBuilder::<String>::stable_order()
        .add_transitive(
            &NestedSetBuilder::compile_order()
                .add_transitive(&NestedSetBuilder::stable_order().build())
                .build(),
        )
        .add_transitive(
            &NestedSetBuilder::link_order()
                .add_transitive(&NestedSetBuilder::stable_order().build())
                .build(),
        )
        .add_transitive(
            &NestedSetBuilder::naive_link_order()
                .add_transitive(&NestedSetBuilder::stable_order().build())
                .build(),
        )
        .build();
    // Shouldn't be able to include a non-stable order inside a different non-stable order!
    let err = catch_unwind(|| {
        NestedSetBuilder::<String>::compile_order()
            .add_transitive(&NestedSetBuilder::link_order().build())
            .build()
    })
    .expect_err("expected IllegalArgumentException");
    assert_eq!(
        panic_message(&*err),
        "IllegalArgumentException: Order mismatch: topological != postorder"
    );
}

#[test]
fn reuses_single_transitive_set_no_direct_members() {
    let set = NestedSetBuilder::create(Order::STABLE_ORDER, list(&["a", "b", "c"]));
    let built = NestedSetBuilder::<String>::stable_order()
        .add_transitive(&set)
        .build();
    assert!(built == set); // isSameInstanceAs
}

#[test]
fn reuses_single_transitive_set_singleton_equals_directs() {
    let set = NestedSetBuilder::create(Order::STABLE_ORDER, list(&["a"]));
    let built = NestedSetBuilder::<String>::stable_order()
        .add(s("a"))
        .add_transitive(&set)
        .build();
    assert!(built == set); // isSameInstanceAs
}

#[test]
fn no_reuse_of_single_transitive_set_order_would_differ() {
    let set = NestedSetBuilder::create(Order::NAIVE_LINK_ORDER, list(&["b", "a"]));
    let built = NestedSetBuilder::<String>::naive_link_order()
        .add(s("a"))
        .add(s("b"))
        .add_transitive(&set)
        .build();
    assert!(built != set); // isNotSameInstanceAs
    assert_eq!(*set.to_list(), list(&["b", "a"]));
    assert_eq!(*built.to_list(), list(&["a", "b"]));
}

/// A handy wrapper that allows us to test shallowEquals and shallowHashCode like EqualsTester.
#[derive(Clone)]
struct SetWrapper<E> {
    set: NestedSet<E>,
}

impl<E: Clone + Eq + Hash> PartialEq for SetWrapper<E> {
    fn eq(&self, other: &Self) -> bool {
        self.set.shallow_equals(Some(&other.set))
    }
}

impl<E: Clone + Eq + Hash> Hash for SetWrapper<E> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.set.shallow_hash_code().hash(state);
    }
}

fn flat<E: Clone + Eq + Hash>(direct_members: &[E]) -> SetWrapper<E> {
    let mut builder = NestedSetBuilder::stable_order();
    builder.add_all(direct_members.iter().cloned());
    SetWrapper {
        set: builder.build(),
    }
}

fn nest<E: Clone + Eq + Hash>(nested: &[&SetWrapper<E>]) -> SetWrapper<E> {
    let mut builder = NestedSetBuilder::stable_order();
    for wrap in nested {
        builder.add_transitive(&wrap.set);
    }
    SetWrapper {
        set: builder.build(),
    }
}

fn nest_elem(elem: i32, nested: &[&SetWrapper<i32>]) -> SetWrapper<i32> {
    let mut builder = NestedSetBuilder::stable_order();
    builder.add(elem);
    for wrap in nested {
        builder.add_transitive(&wrap.set);
    }
    SetWrapper {
        set: builder.build(),
    }
}

/// Guava `EqualsTester` for shallowEquals/shallowHashCode: members of a group are equal with
/// equal hash codes; members of different groups are unequal.
struct EqualsTester<E> {
    groups: Vec<Vec<SetWrapper<E>>>,
}

impl<E: Clone + Eq + Hash> EqualsTester<E> {
    fn new() -> Self {
        EqualsTester { groups: Vec::new() }
    }

    fn add_equality_group(mut self, group: Vec<SetWrapper<E>>) -> Self {
        self.groups.push(group);
        self
    }

    fn test_equals(self) {
        for (gi, group) in self.groups.iter().enumerate() {
            for (i, a) in group.iter().enumerate() {
                assert!(
                    !a.set.shallow_equals(None),
                    "group {gi} item {i} equals null"
                );
                assert!(
                    a.set.shallow_equals(Some(&a.set)),
                    "group {gi} item {i} not reflexive"
                );
                for (j, b) in group.iter().enumerate() {
                    assert!(a == b, "group {gi}: item {i} != item {j}");
                    assert_eq!(
                        a.set.shallow_hash_code(),
                        b.set.shallow_hash_code(),
                        "group {gi}: hash of item {i} != item {j}"
                    );
                }
                for (gj, other) in self.groups.iter().enumerate() {
                    if gi == gj {
                        continue;
                    }
                    for (j, b) in other.iter().enumerate() {
                        assert!(a != b, "group {gi} item {i} == group {gj} item {j}");
                    }
                }
            }
        }
    }
}

#[test]
fn shallow_equality() {
    // Used below to check that inner nested sets can be compared by reference equality.
    let my_ref = nest(&[&nest(&[&flat(&[7, 8])]), &flat(&[9])]);
    // PORT: the Java group of two `NestedSet.withFuture` sets sharing one future is dropped:
    // serialization (deserialization futures) is skipped.

    // Each "equality group" contains elements that are equal to one another
    // (according to equals() and hashCode()), yet distinct from all elements
    // of all other equality groups.
    EqualsTester::new()
        .add_equality_group(vec![flat::<i32>(&[]), flat(&[]), nest(&[&flat(&[])])]) // Empty set elision.
        .add_equality_group(vec![SetWrapper {
            set: NestedSetBuilder::<i32>::link_order().build(),
        }])
        .add_equality_group(vec![flat(&[3]), flat(&[3]), flat(&[3, 3])]) // Element de-duplication.
        .add_equality_group(vec![flat(&[4]), nest(&[&flat(&[4])])]) // Automatic elision of one-element nested sets.
        .add_equality_group(vec![SetWrapper {
            set: NestedSetBuilder::<i32>::link_order().add(4).build(),
        }])
        .add_equality_group(vec![flat(&[3, 4]), flat(&[3, 4])])
        // Make a couple sets deep enough that shallowEquals() fails.
        // If this test case fails because you improve the representation, just delete it.
        .add_equality_group(vec![nest(&[
            &nest(&[&flat(&[3, 4]), &flat(&[5])]),
            &nest(&[&flat(&[6, 7]), &flat(&[8])]),
        ])])
        .add_equality_group(vec![nest(&[
            &nest(&[&flat(&[3, 4]), &flat(&[5])]),
            &nest(&[&flat(&[6, 7]), &flat(&[8])]),
        ])])
        .add_equality_group(vec![
            nest(&[&my_ref]),
            nest(&[&my_ref]),
            nest(&[&my_ref, &my_ref]),
        ]) // Set de-duplication.
        .add_equality_group(vec![nest_elem(3, &[&my_ref])])
        .add_equality_group(vec![nest_elem(4, &[&my_ref])])
        .test_equals();

    // PORT: Java's group `nestedSetBuilder("4").build()` (a String set, unequal to everything
    // above) cannot share a tester with Integer sets in Rust; check it on its own.
    let str4 = nested_set_builder(&["4"]).build();
    assert!(str4.shallow_equals(Some(&nested_set_builder(&["4"]).build())));
    assert!(!str4.shallow_equals(Some(&nested_set_builder(&["3", "4"]).build())));

    // Some things that are not tested by the above:
    //  - ordering among direct members
    //  - ordering among transitive sets
}

#[test]
fn shallow_inequality() {
    assert!(!nested_set_builder(&["a"]).build().shallow_equals(None));
    // PORT: the remaining Java assertions use `NestedSet.withFuture`; serialization is skipped.
}

#[test]
#[ignore = "needs deserialization futures (NestedSet.withFuture); serialization skipped"]
fn shallow_inequality_with_futures() {}

fn create_nested_set(
    order: Order,
    num_directs: i32,
    num_transitives: i32,
    transitive_order: Order,
) -> NestedSet<i32> {
    let mut builder = NestedSetBuilder::new_builder(order);

    for direct in 0..num_directs {
        builder.add(direct);
    }
    for transitive in 0..num_transitives {
        builder.add_transitive(&NestedSet::builder(transitive_order).add(transitive).build());
    }
    builder.build()
}

/// Checks that the builder always return a nested set with the correct order.
#[test]
fn correct_order() {
    for order in Order::VALUES {
        for num_directs in 0..3 {
            for num_transitives in 0..3 {
                assert_eq!(
                    create_nested_set(order, num_directs, num_transitives, order).get_order(),
                    order
                );
                // We allow mixing orders if one of them is stable. This tests that the top level
                // order is the correct one.
                assert_eq!(
                    create_nested_set(order, num_directs, num_transitives, Order::STABLE_ORDER)
                        .get_order(),
                    order
                );
            }
        }
    }
}

fn check_size<E: Clone + Eq + Hash>(set: &NestedSet<E>, size: i32) {
    assert_eq!(set.memoized_flatten_and_get_size(), size); // first call: flattens
    assert_eq!(set.memoized_flatten_and_get_size(), size); // second call: memoized
}

#[test]
fn memoized_flatten_and_get_size() {
    let empty = NestedSetBuilder::<String>::stable_order().build();
    check_size(&empty, 0); // {}

    let singleton = NestedSetBuilder::<String>::stable_order()
        .add(s("a"))
        .build();
    check_size(&singleton, 1); // {a}

    let deuce = NestedSetBuilder::<String>::stable_order()
        .add(s("a"))
        .add(s("b"))
        .build();
    check_size(&deuce, 2); // {a, b}

    check_size(
        &NestedSetBuilder::<String>::stable_order()
            .add(s("a"))
            .add_transitive(&deuce)
            .add_transitive(&singleton)
            .add_transitive(&empty)
            .build(),
        2,
    ); // {a, b}
    check_size(
        &NestedSetBuilder::<String>::stable_order()
            .add(s("c"))
            .add_transitive(&deuce)
            .add_transitive(&singleton)
            .add_transitive(&empty)
            .build(),
        3,
    ); // {a, b, c}

    // 25000 has a 3-digit base128 encoding.
    let mut large_shallow = NestedSetBuilder::<i32>::stable_order();
    for i in 0..25000 {
        large_shallow.add(i);
    }
    check_size(&large_shallow.build(), 25000); // {0, 1, ..., 24999}

    // a deep and narrow graph
    let mut deep = deuce;
    for _ in 0..200 {
        deep = NestedSetBuilder::<String>::stable_order()
            .add_transitive(&deep)
            .add(s("c"))
            .build();
    }
    check_size(&deep, 3); // {a, b, c}
}

#[test]
fn concurrent_memoized_flatten_and_get_size() {
    let mut deep = NestedSetBuilder::<String>::stable_order()
        .add(s("a"))
        .add(s("b"))
        .build();
    for _ in 0..200 {
        deep = NestedSetBuilder::<String>::stable_order()
            .add_transitive(&deep)
            .add(s("c"))
            .build();
    }
    let under_test = deep;
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..20)
            .map(|_| scope.spawn(|| under_test.memoized_flatten_and_get_size()))
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap(), 3);
        }
    });
}

#[test]
fn hoisting_keeps_set_small() {
    let first = NestedSetBuilder::<String>::stable_order()
        .add(s("a"))
        .build();
    let second = NestedSetBuilder::<String>::stable_order()
        .add(s("a"))
        .build();
    let singleton = NestedSetBuilder::<String>::stable_order()
        .add_transitive(&first)
        .add_transitive(&second)
        .build();
    assert_eq!(*singleton.to_list(), list(&["a"]));
    assert!(singleton.is_singleton());
}

#[test]
#[ignore = "needs deserialization futures (NestedSet.withFuture); serialization skipped"]
fn build_interruptibly_propagates_interrupt() {}

#[test]
#[ignore = "needs deserialization futures (NestedSet.withFuture); serialization skipped"]
fn get_children_interruptibly_propagates_interrupt() {}

#[test]
#[ignore = "needs deserialization futures (NestedSet.withFuture); serialization skipped"]
fn to_list_interruptibly_propagates_interrupt() {}

#[test]
#[ignore = "needs deserialization futures (NestedSet.withFuture); serialization skipped"]
fn to_list_interruptibly_propagates_missing_fingerprint_value_exception() {}

#[test]
#[ignore = "needs deserialization futures (NestedSet.withFuture); serialization skipped"]
fn to_list_with_timeout_propagates_interrupt() {}

#[test]
#[ignore = "needs deserialization futures (NestedSet.withFuture); serialization skipped"]
fn to_list_with_timeout_propagates_missing_fingerprint_value_exception() {}

#[test]
#[ignore = "needs deserialization futures (NestedSet.withFuture); serialization skipped"]
fn to_list_with_timeout_times_out() {}

#[test]
#[ignore = "needs deserialization futures (NestedSet.withFuture); serialization skipped"]
fn to_list_with_timeout_waits() {}

#[test]
#[ignore = "needs deserialization futures (NestedSet.withFuture); serialization skipped"]
fn is_from_storage_true() {}

#[test]
fn is_from_storage_false() {
    let in_memory_nested_set = NestedSetBuilder::create(Order::STABLE_ORDER, list(&["a", "b"]));
    assert!(!in_memory_nested_set.is_from_storage());
}

#[test]
fn is_ready_in_memory() {
    let in_memory_nested_set = NestedSetBuilder::create(Order::STABLE_ORDER, list(&["a", "b"]));
    assert!(in_memory_nested_set.is_ready());
}

#[test]
#[ignore = "needs deserialization futures (NestedSet.withFuture); serialization skipped"]
fn is_ready_from_storage() {}

#[test]
#[ignore = "needs deserialization futures (NestedSet.withFuture); serialization skipped"]
fn is_ready_from_storage_cancelled() {}

#[test]
#[ignore = "needs deserialization futures (NestedSet.withFuture); serialization skipped"]
fn is_ready_from_storage_failed() {}

#[test]
fn get_approx_depth() {
    let empty = nested_set_builder(&[]).build();
    let just_a = nested_set_builder(&["a"]).build();
    let just_b = nested_set_builder(&["b"]).build();
    let ab = nested_set_builder(&[])
        .add_transitive(&just_a)
        .add_transitive(&just_b)
        .build();

    assert_eq!(empty.get_approx_depth(), 0);
    assert_eq!(
        nested_set_builder(&[])
            .add_transitive(&empty)
            .add_transitive(&empty)
            .build()
            .get_approx_depth(),
        0
    );
    assert_eq!(just_a.get_approx_depth(), 1);
    assert_eq!(just_b.get_approx_depth(), 1);
    assert_eq!(
        nested_set_builder(&[])
            .add_transitive(&empty)
            .add_transitive(&empty)
            .build()
            .get_approx_depth(),
        0
    );
    assert_eq!(
        nested_set_builder(&[])
            .add_transitive(&empty)
            .add_transitive(&just_a)
            .build()
            .get_approx_depth(),
        1
    );
    assert_eq!(
        nested_set_builder(&[])
            .add_transitive(&just_a)
            .add_transitive(&empty)
            .build()
            .get_approx_depth(),
        1
    );
    assert_eq!(
        nested_set_builder(&[])
            .add_transitive(&just_a)
            .add_transitive(&just_a)
            .build()
            .get_approx_depth(),
        1
    );
    assert_eq!(
        nested_set_builder(&[])
            .add_transitive(&just_a)
            .add_transitive(&just_b)
            .build()
            .get_approx_depth(),
        2
    );
    assert_eq!(
        nested_set_builder(&["a", "b", "c"])
            .add_transitive(&just_a)
            .add_transitive(&just_b)
            .add_transitive(&ab)
            .build()
            .get_approx_depth(),
        3
    );
}

#[test]
fn link_order_to_list_with_transitive_input_aliases_are_consistent() {
    let input_a = NestedSetBuilder::create(Order::LINK_ORDER, list(&["A"]));
    let input_b = NestedSetBuilder::create(Order::LINK_ORDER, list(&["B"]));
    let input_c = NestedSetBuilder::create(Order::LINK_ORDER, list(&["C"]));
    let input_b2 = NestedSetBuilder::create(Order::LINK_ORDER, list(&["B"]));

    let with_duplicates = NestedSet::<String>::builder(Order::LINK_ORDER)
        .add_transitive(&input_a)
        .add_transitive(&input_b)
        .add_transitive(&input_c)
        .add_transitive(&input_b)
        .build();

    let with_alias = NestedSet::<String>::builder(Order::LINK_ORDER)
        .add_transitive(&input_a)
        .add_transitive(&input_b)
        .add_transitive(&input_c)
        .add_transitive(&input_b2)
        .build();

    assert_eq!(*with_alias.to_list(), *with_duplicates.to_list());
}

#[test]
fn link_order_to_list_with_duplicate_direct_inputs_keeps_first() {
    let duplicate_inputs = NestedSetBuilder::create(Order::LINK_ORDER, list(&["A", "B", "C", "A"]));
    assert_eq!(*duplicate_inputs.to_list(), list(&["A", "B", "C"]));
}

#[test]
fn nesting_incompatible_order_panics_with_java_message() {
    // Not a Java test: pins the IllegalArgumentException text of addTransitive.
    let err = catch_unwind(AssertUnwindSafe(|| {
        NestedSetBuilder::<String>::link_order().add_transitive(&NestedSetBuilder::create(
            Order::COMPILE_ORDER,
            list(&["a"]),
        ));
    }))
    .expect_err("expected panic");
    assert_eq!(
        panic_message(&*err),
        "IllegalArgumentException: Order mismatch: postorder != topological"
    );
}

fn panic_message(err: &(dyn std::any::Any + Send)) -> String {
    err.downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}
