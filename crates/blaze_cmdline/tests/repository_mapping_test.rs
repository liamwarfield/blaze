//! Port of `RepositoryMappingTest`.

use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{RepositoryMapping, RepositoryName};

fn rn(name: &str) -> RepositoryName {
    RepositoryName::create(name).unwrap()
}

#[test]
fn never_fallback() {
    let mapping = RepositoryMapping::create([("A", rn("com_foo_bar_a"))], rn("fake_owner_repo"));
    assert_eq!(mapping.get("A"), rn("com_foo_bar_a"));
    assert_eq!(
        mapping.get("B"),
        rn("B").to_non_visible(&rn("fake_owner_repo"))
    );
}

#[test]
fn additional_mappings_basic() {
    let mapping = RepositoryMapping::create([("A", rn("com_foo_bar_a"))], rn("fake_owner_repo"))
        .with_additional_mappings([("B", rn("com_foo_bar_b"))]);
    assert_eq!(mapping.get("A"), rn("com_foo_bar_a"));
    assert_eq!(mapping.get("B"), rn("com_foo_bar_b"));
    assert_eq!(
        mapping.get("C"),
        rn("C").to_non_visible(&rn("fake_owner_repo"))
    );
}

#[test]
fn additional_mappings_precedence() {
    let mapping = RepositoryMapping::create([("A", rn("A1"))], MAIN.clone())
        .with_additional_mappings([("A", rn("A2"))]);
    assert_eq!(mapping.get("A"), rn("A1"));
}

#[test]
fn unknown_repo_did_you_mean() {
    let mapping = RepositoryMapping::create([("foo", rn("foo_internal"))], MAIN.clone());
    assert_eq!(
        mapping.get("boo").get_name_with_at(),
        "@@[unknown repo 'boo' requested from @@ (did you mean 'foo'?)]"
    );
}
