//! Port of `src/test/java/com/google/devtools/build/lib/packages/EnvironmentGroupTest.java`.
//!
//! Every test loads pkg/BUILD (environments foo (fulfills bar, baz), bar (fulfills baz), baz,
//! not_in_group, and environment_group "group" with environments [foo, bar, baz] and defaults
//! [foo]); they are ignored until BUILD loading works.

#[test]
#[ignore = "needs BUILD loading"]
fn test_group_membership() {
    // group.getEnvironments() == {//pkg:foo, //pkg:bar, //pkg:baz}.
    unimplemented!("needs BUILD loading")
}

#[test]
#[ignore = "needs BUILD loading"]
fn defaults_membership() {
    // group.getDefaults() == {//pkg:foo}.
    unimplemented!("needs BUILD loading")
}

#[test]
#[ignore = "needs BUILD loading"]
fn is_default() {
    // isDefault: foo true; bar, baz, not_in_group false.
    unimplemented!("needs BUILD loading")
}

#[test]
#[ignore = "needs BUILD loading"]
fn fulfillers() {
    // getFulfillers(baz) == {foo, bar}; getFulfillers(bar) == {foo}; getFulfillers(foo) empty.
    unimplemented!("needs BUILD loading")
}

#[test]
#[ignore = "needs BUILD loading"]
fn empty_groups_not_allowed() {
    // a/BUILD: environment_group(name = 'empty_group', environments = [], defaults = []);
    // package contains errors; event
    // "environment group empty_group must contain at least one environment".
    unimplemented!("needs BUILD loading")
}

#[test]
#[ignore = "needs BUILD loading (and serialization is not ported)"]
fn reduce_for_serialization_has_consistent_values() {
    unimplemented!("needs BUILD loading")
}
