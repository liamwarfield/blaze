//! Port of `src/test/java/com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArgTest.java`.

mod modcommand_common;

use blaze_bzlmod::inspector::AugmentedModule;
use blaze_bzlmod::modcommand::{ExtensionArg, ModuleArg};
use blaze_bzlmod_model::{ModuleExtensionId, ModuleKey};
use blaze_cmdline::{Label, RepositoryName};
use indexmap::{IndexMap, IndexSet};
use modcommand_common::*;

#[test]
fn converter() {
    assert_eq!(
        ExtensionArg::convert("<root>//abc:haha.bzl%ext").unwrap(),
        ExtensionArg::create(
            ModuleArg::SpecificVersionOfModule(ModuleKey::root()),
            "//abc:haha.bzl",
            "ext"
        )
    );
    assert_eq!(
        ExtensionArg::convert("@@abc//:def.bzl%ghi").unwrap(),
        ExtensionArg::create(
            ModuleArg::CanonicalRepoName(RepositoryName::create_unvalidated("abc")),
            "//:def.bzl",
            "ghi"
        )
    );
    assert_eq!(
        ExtensionArg::convert("@abc//:def.bzl%ghi").unwrap(),
        ExtensionArg::create(
            ModuleArg::ApparentRepoName("abc".to_string()),
            "//:def.bzl",
            "ghi"
        )
    );
    assert_eq!(
        ExtensionArg::convert("abc//:def.bzl%ghi").unwrap(),
        ExtensionArg::create(
            ModuleArg::AllVersionsOfModule("abc".to_string()),
            "//:def.bzl",
            "ghi"
        )
    );
    assert_eq!(
        ExtensionArg::convert("a@b//:def.bzl%ghi").unwrap(),
        ExtensionArg::create(
            ModuleArg::SpecificVersionOfModule(create_module_key("a", "b")),
            "//:def.bzl",
            "ghi"
        )
    );

    assert!(ExtensionArg::convert("abc@//:def.bzl%ghi").is_err());
    assert!(ExtensionArg::convert("@_abc//:def.bzl%ghi").is_err());
    assert!(ExtensionArg::convert("abc").is_err());
    assert!(ExtensionArg::convert("abc%def").is_err());
    assert!(ExtensionArg::convert("@_abc%ghi//def").is_err());
}

fn canonical_names(
    dep_graph: &IndexMap<ModuleKey, AugmentedModule>,
) -> IndexMap<ModuleKey, RepositoryName> {
    dep_graph
        .keys()
        .map(|k| (k.clone(), k.get_canonical_repo_name_with_version()))
        .collect()
}

#[test]
fn resolve_good() {
    let key = create_module_key("foo", "1.0");
    let modules_index = IndexMap::from([
        ("".to_string(), IndexSet::from([ModuleKey::root()])),
        ("foo".to_string(), IndexSet::from([key.clone()])),
    ]);
    let dep_graph = map_of(vec![
        build_augmented_module("", "")
            .add_dep_repo("fred", "foo", "1.0")
            .build_entry(),
        build_augmented_module("foo", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .build_entry(),
    ]);
    let module_key_to_canonical_names = canonical_names(&dep_graph);
    let base_module_deps = IndexMap::from([("fred".to_string(), key.clone())]);
    let base_module_unused_deps: IndexMap<String, ModuleKey> = IndexMap::new();

    assert_eq!(
        ExtensionArg::create(
            ModuleArg::SpecificVersionOfModule(key.clone()),
            "//:abc.bzl",
            "def"
        )
        .resolve_to_extension_id(
            &modules_index,
            &dep_graph,
            &module_key_to_canonical_names,
            &base_module_deps,
            &base_module_unused_deps,
        )
        .unwrap(),
        ModuleExtensionId::create(
            Label::parse_canonical("@@foo+1.0//:abc.bzl").unwrap(),
            "def",
            None
        )
    );
}

#[test]
fn resolve_bad_label() {
    let key = create_module_key("foo", "1.0");
    let modules_index = IndexMap::from([
        ("".to_string(), IndexSet::from([ModuleKey::root()])),
        ("foo".to_string(), IndexSet::from([key.clone()])),
    ]);
    let dep_graph = map_of(vec![
        build_augmented_module("", "")
            .add_dep_repo("fred", "foo", "1.0")
            .build_entry(),
        build_augmented_module("foo", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .build_entry(),
    ]);
    let module_key_to_canonical_names = canonical_names(&dep_graph);
    let base_module_deps = IndexMap::from([("fred".to_string(), key.clone())]);
    let base_module_unused_deps: IndexMap<String, ModuleKey> = IndexMap::new();

    assert!(
        ExtensionArg::create(
            ModuleArg::SpecificVersionOfModule(key.clone()),
            "/:def.bzl",
            "ext"
        )
        .resolve_to_extension_id(
            &modules_index,
            &dep_graph,
            &module_key_to_canonical_names,
            &base_module_deps,
            &base_module_unused_deps,
        )
        .is_err()
    );
    assert!(
        ExtensionArg::create(
            ModuleArg::SpecificVersionOfModule(key.clone()),
            "///////",
            "ext"
        )
        .resolve_to_extension_id(
            &modules_index,
            &dep_graph,
            &module_key_to_canonical_names,
            &base_module_deps,
            &base_module_unused_deps,
        )
        .is_err()
    );
}

#[test]
fn resolve_none_ortoo_many_modules() {
    let foo1 = create_module_key("foo", "1.0");
    let foo2 = create_module_key("foo", "2.0");
    let modules_index = IndexMap::from([
        ("".to_string(), IndexSet::from([ModuleKey::root()])),
        (
            "foo".to_string(),
            IndexSet::from([foo1.clone(), foo2.clone()]),
        ),
    ]);
    let dep_graph = map_of(vec![
        build_augmented_module("", "")
            .add_dep_repo("foo1", "foo", "1.0")
            .add_dep_repo("foo2", "foo", "2.0")
            .build_entry(),
        build_augmented_module("foo", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .build_entry(),
        build_augmented_module("foo", "2.0")
            .add_still_dependant_key(ModuleKey::root())
            .build_entry(),
    ]);
    let module_key_to_canonical_names = canonical_names(&dep_graph);
    let base_module_deps = IndexMap::from([
        ("foo1".to_string(), foo1.clone()),
        ("foo2".to_string(), foo2.clone()),
    ]);
    let base_module_unused_deps: IndexMap<String, ModuleKey> = IndexMap::new();

    // Found too many, bad!
    assert!(
        ExtensionArg::create(
            ModuleArg::AllVersionsOfModule("foo".to_string()),
            "//:def.bzl",
            "ext"
        )
        .resolve_to_extension_id(
            &modules_index,
            &dep_graph,
            &module_key_to_canonical_names,
            &base_module_deps,
            &base_module_unused_deps,
        )
        .is_err()
    );
    // Found none, bad!
    assert!(
        ExtensionArg::create(
            ModuleArg::AllVersionsOfModule("bar".to_string()),
            "//:def.bzl",
            "ext"
        )
        .resolve_to_extension_id(
            &modules_index,
            &dep_graph,
            &module_key_to_canonical_names,
            &base_module_deps,
            &base_module_unused_deps,
        )
        .is_err()
    );
}
