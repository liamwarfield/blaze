//! Port of `GroupedDepsTest` and `ErrorTransienceValueTest`.

use super::*;

fn key(arg: &'static str) -> SkyKey {
    k(arg)
}

// Java: GroupedDepsTest.checkGroups
// PORT: checkCompression is skipped (`GroupedDeps.compress` is a memory optimisation of the
// incremental graph, not ported).
fn check_groups(deps: &GroupedDeps, expected_groups: &[Vec<SkyKey>]) {
    assert_eq!(deps.is_empty(), expected_groups.is_empty());
    assert_eq!(deps.num_groups(), expected_groups.len());
    assert_eq!(deps.groups(), expected_groups);

    let expected_flattened: Vec<SkyKey> = expected_groups.iter().flatten().cloned().collect();
    assert_eq!(deps.num_elements(), expected_flattened.len());
    assert_eq!(
        deps.get_all_elements_as_iterable()
            .cloned()
            .collect::<Vec<_>>(),
        expected_flattened
    );
    assert_eq!(
        deps.to_set(),
        expected_flattened.iter().cloned().collect::<HashSet<_>>()
    );
}

// Java: GroupedDepsTest.singleGroup
#[test]
fn single_group() {
    for size in [0usize, 1, 2, 10] {
        let mut deps = GroupedDeps::new();
        let elements: Vec<SkyKey> = (0..size).map(|i| SkyKey::new(Num(i as u64))).collect();
        deps.append_group(elements.clone());
        let expected: Vec<Vec<SkyKey>> = if size == 0 { vec![] } else { vec![elements] };
        check_groups(&deps, &expected);
    }
}

// Java: GroupedDepsTest.appendEmptyGroup_noOp
#[test]
fn append_empty_group_no_op() {
    let mut deps = GroupedDeps::new();
    deps.append_group(vec![]);
    assert!(deps.is_empty());
    deps.append_singleton(key("a"));
    deps.append_group(vec![]);
    deps.append_singleton(key("b"));
    check_groups(&deps, &[vec![key("a")], vec![key("b")]]);
}

// Java: GroupedDepsTest.identical_equal
#[test]
fn identical_equal() {
    let mut abc1 = GroupedDeps::new();
    let mut abc2 = GroupedDeps::new();
    abc1.append_group(vec![key("a"), key("b"), key("c")]);
    abc2.append_group(vec![key("a"), key("b"), key("c")]);
    assert_eq!(abc1, abc2);
}

// Java: GroupedDepsTest.appendSingletonAndAppendGroupSizeOne_equal
#[test]
fn append_singleton_and_append_group_size_one_equal() {
    let mut a_singleton = GroupedDeps::new();
    let mut a_group = GroupedDeps::new();
    a_singleton.append_singleton(key("a"));
    a_group.append_group(vec![key("a")]);
    assert_eq!(a_singleton, a_group);
}

// Java: GroupedDepsTest.differentOrderWithinGroup_equal
#[test]
fn different_order_within_group_equal() {
    let mut ab = GroupedDeps::new();
    let mut ba = GroupedDeps::new();
    ab.append_group(vec![key("a"), key("b")]);
    ba.append_group(vec![key("b"), key("a")]);
    assert_eq!(ab, ba);
}

// Java: GroupedDepsTest.differentElements_notEqual
#[test]
fn different_elements_not_equal() {
    let mut abc = GroupedDeps::new();
    let mut xyz = GroupedDeps::new();
    abc.append_group(vec![key("a"), key("b"), key("c")]);
    xyz.append_group(vec![key("x"), key("y"), key("z")]);
    assert_ne!(abc, xyz);
}

// Java: GroupedDepsTest.differentOrderOfGroups_notEqual
#[test]
fn different_order_of_groups_not_equal() {
    let mut ab = GroupedDeps::new();
    let mut ba = GroupedDeps::new();
    ab.append_singleton(key("a"));
    ab.append_singleton(key("b"));
    ba.append_singleton(key("b"));
    ba.append_singleton(key("a"));
    assert_ne!(ab, ba);
}

// Java: GroupedDepsTest.differentGroupings_notEqual
#[test]
fn different_groupings_not_equal() {
    let mut ab_group = GroupedDeps::new();
    let mut ab_singletons = GroupedDeps::new();
    ab_group.append_group(vec![key("a"), key("b")]);
    ab_singletons.append_singleton(key("a"));
    ab_singletons.append_singleton(key("b"));
    assert_ne!(ab_group, ab_singletons);
}

// Java: GroupedDepsTest.groups
#[test]
fn groups() {
    let mut deps = GroupedDeps::new();
    let groups = vec![
        vec![key("1")],
        vec![key("2a"), key("2b")],
        vec![key("3")],
        vec![key("4")],
        vec![key("5a"), key("5b"), key("5c")],
        vec![key("6a"), key("6b"), key("6c")],
    ];
    for g in &groups {
        deps.append_group(g.clone());
    }
    check_groups(&deps, &groups);
}

fn three_groups() -> GroupedDeps {
    let mut deps = GroupedDeps::new();
    deps.append_group(vec![key("1a"), key("1b")]);
    deps.append_group(vec![key("2a"), key("2b"), key("2c")]);
    deps.append_group(vec![key("3a"), key("3b")]);
    deps
}

fn set_of(keys: &[&'static str]) -> HashSet<SkyKey> {
    keys.iter().map(|n| key(n)).collect()
}

// Java: GroupedDepsTest.remove_groupsIntact
#[test]
fn remove_groups_intact() {
    let mut deps = three_groups();
    deps.remove(&set_of(&["2c"]));
    check_groups(
        &deps,
        &[
            vec![key("1a"), key("1b")],
            vec![key("2a"), key("2b")],
            vec![key("3a"), key("3b")],
        ],
    );
}

// Java: GroupedDepsTest.remove_groupBecomesSingleton
#[test]
fn remove_group_becomes_singleton() {
    let mut deps = three_groups();
    deps.remove(&set_of(&["2b", "2c"]));
    check_groups(
        &deps,
        &[
            vec![key("1a"), key("1b")],
            vec![key("2a")],
            vec![key("3a"), key("3b")],
        ],
    );
}

// Java: GroupedDepsTest.remove_groupBecomesEmpty
#[test]
fn remove_group_becomes_empty() {
    let mut deps = three_groups();
    deps.remove(&set_of(&["2a", "2b", "2c"]));
    check_groups(
        &deps,
        &[vec![key("1a"), key("1b")], vec![key("3a"), key("3b")]],
    );
}

// Java: GroupedDepsTest.remove_singleton
#[test]
fn remove_singleton() {
    let mut deps = GroupedDeps::new();
    deps.append_group(vec![key("1a"), key("1b")]);
    deps.append_singleton(key("2"));
    deps.append_group(vec![key("3a"), key("3b")]);
    deps.remove(&set_of(&["2"]));
    check_groups(
        &deps,
        &[vec![key("1a"), key("1b")], vec![key("3a"), key("3b")]],
    );
}

// Java: GroupedDepsTest.remove_wholeGroupedDepsBecomesEmpty
#[test]
fn remove_whole_grouped_deps_becomes_empty() {
    let mut deps = three_groups();
    deps.remove(&set_of(&["1a", "1b", "2a", "2b", "2c", "3a", "3b"]));
    check_groups(&deps, &[]);
}

// Java: GroupedDepsTest.remove_elementNotPresent_throws
#[test]
#[should_panic(expected = "Requested removal of absent element(s)")]
fn remove_element_not_present_throws() {
    let mut deps = three_groups();
    deps.remove(&set_of(&["2c", "2d"]));
}

// Java: ErrorTransienceValueTest.testHashCodeNotSupported
#[test]
#[ignore = "ErrorTransienceValue.hashCode throws in Java; Rust values have no hashCode to call"]
fn test_hash_code_not_supported() {
    unimplemented!("not ported: ErrorTransienceValueTest.testHashCodeNotSupported")
}

// Java: ErrorTransienceValueTest.testToStringWorks
#[test]
fn test_to_string_works() {
    // Java only checks that toString does not throw.
    let _ = format!("{:?}", ErrorTransienceValue);
    assert_eq!(
        format!("{:?}", *ERROR_TRANSIENCE_KEY),
        "ErrorTransienceValue.KEY"
    );
}

// Java: ErrorTransienceValueTest.testIsntEqualToSelf
#[test]
#[ignore = "ErrorTransienceValue.equals is never called here: values have no equality (no \
            change pruning)"]
fn test_isnt_equal_to_self() {
    unimplemented!("not ported: ErrorTransienceValueTest.testIsntEqualToSelf")
}
