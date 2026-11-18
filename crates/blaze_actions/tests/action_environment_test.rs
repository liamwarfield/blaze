//! Port of `src/test/java/com/google/devtools/build/lib/actions/ActionEnvironmentTest.java`.

use std::sync::Arc;

use blaze_actions::ActionEnvironment;
use indexmap::{IndexMap, IndexSet};

fn map(kv: &[(&str, &str)]) -> IndexMap<String, String> {
    kv.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn set(v: &[&str]) -> IndexSet<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn compound_env_ordering() {
    let env1 = Arc::new(ActionEnvironment::create(
        map(&[("FOO", "foo1"), ("BAR", "bar")]),
        set(&["baz"]),
    ));
    // entries added by env2 override the existing entries
    let env2 = env1.with_additional_fixed_variables(map(&[("FOO", "foo2")]));

    assert_eq!(
        env1.get_fixed_env(),
        map(&[("FOO", "foo1"), ("BAR", "bar")])
    );
    assert_eq!(env1.get_inherited_env(), set(&["baz"]));

    assert_eq!(
        env2.get_fixed_env(),
        map(&[("FOO", "foo2"), ("BAR", "bar")])
    );
    assert_eq!(env2.get_inherited_env(), set(&["baz"]));
}

#[test]
fn fixed_inherited_interaction() {
    let env = Arc::new(ActionEnvironment::create(
        map(&[("FIXED_ONLY", "fixed")]),
        set(&["INHERITED_ONLY", "FIXED_AND_INHERITED"]),
    ))
    .with_additional_fixed_variables(map(&[("FIXED_AND_INHERITED", "fixed")]));
    let client_env = map(&[
        ("INHERITED_ONLY", "inherited"),
        ("FIXED_AND_INHERITED", "inherited"),
    ]);
    let mut result = IndexMap::new();
    env.resolve(&mut result, &client_env);

    // containsExactly ignores order.
    let mut got: Vec<(String, String)> = result.into_iter().collect();
    got.sort();
    let mut want: Vec<(String, String)> = map(&[
        ("FIXED_ONLY", "fixed"),
        ("FIXED_AND_INHERITED", "inherited"),
        ("INHERITED_ONLY", "inherited"),
    ])
    .into_iter()
    .collect();
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn empty_environment_interning() {
    let empty_environment = ActionEnvironment::create(IndexMap::new(), IndexSet::new());
    assert_eq!(empty_environment, ActionEnvironment::Empty);

    let base = Arc::new(ActionEnvironment::create(
        map(&[("FOO", "foo1")]),
        set(&["baz"]),
    ));
    assert!(Arc::ptr_eq(
        &base.with_additional_fixed_variables(IndexMap::new()),
        &base
    ));
}
