//! Port of `src/test/java/com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorFunctionTest.java`.

mod modcommand_common;

use blaze_bzlmod::inspector::{BazelModuleInspectorFunction, ResolutionReason};
use blaze_bzlmod_model::{
    ArchiveRepoSpecBuilder, ModuleKey, ModuleOverride, MultipleVersionOverride,
    NonRegistryOverride, SingleVersionOverride, Version,
};
use indexmap::{IndexMap, IndexSet};
use modcommand_common::*;

fn used(keys: Vec<ModuleKey>) -> IndexSet<ModuleKey> {
    keys.into_iter().collect()
}

#[test]
fn test_diamond_simple() {
    let unpruned_dep_graph = map_of(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ddd_from_bbb", create_module_key("ddd", "2.0"))
            .add_original_dep("ddd_from_bbb", create_module_key("ddd", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ddd", "2.0", 1).build_entry(),
    ]);

    let used_modules = used(vec![
        ModuleKey::root(),
        create_module_key("bbb", "1.0"),
        create_module_key("ccc", "2.0"),
        create_module_key("ddd", "2.0"),
    ]);

    let dep_graph = BazelModuleInspectorFunction::compute_augmented_graph(
        &unpruned_dep_graph,
        &used_modules,
        /* overrides= */ &IndexMap::new(),
    );

    assert_eq!(
        dep_graph,
        map_of(vec![
            build_augmented_module_key(ModuleKey::root(), "aaa")
                .add_dep_repo("bbb_from_aaa", "bbb", "1.0")
                .add_dep_repo("ccc_from_aaa", "ccc", "2.0")
                .build_entry(),
            build_augmented_module("bbb", "1.0")
                .add_changed_dep_repo(
                    "ddd_from_bbb",
                    "ddd",
                    "2.0",
                    "1.0",
                    ResolutionReason::MINIMAL_VERSION_SELECTION
                )
                .add_still_dependant_key(ModuleKey::root())
                .build_entry(),
            build_augmented_module("ccc", "2.0")
                .add_dep_repo("ddd_from_ccc", "ddd", "2.0")
                .add_still_dependant_key(ModuleKey::root())
                .build_entry(),
            build_augmented_module("ddd", "2.0")
                .add_dependant("bbb", "1.0")
                .add_still_dependant("ccc", "2.0")
                .build_entry(),
            build_augmented_module("ddd", "1.0")
                .add_original_dependant("bbb", "1.0")
                .build_entry(),
        ])
    );
}

#[test]
fn test_diamond_with_further_removal() {
    let unpruned_dep_graph = map_of(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb", create_module_key("bbb", "1.0"))
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ddd", create_module_key("ddd", "2.0"))
            .add_original_dep("ddd", create_module_key("ddd", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("ddd", create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("ddd", "2.0").build_entry(),
        InterimModuleBuilder::create("ddd", "1.0")
            .add_dep("eee", create_module_key("eee", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("eee", "1.0").build_entry(),
    ]);

    let used_modules = used(vec![
        ModuleKey::root(),
        create_module_key("bbb", "1.0"),
        create_module_key("ccc", "2.0"),
        create_module_key("ddd", "2.0"),
    ]);

    let dep_graph = BazelModuleInspectorFunction::compute_augmented_graph(
        &unpruned_dep_graph,
        &used_modules,
        &IndexMap::new(),
    );

    assert_eq!(
        dep_graph,
        map_of(vec![
            build_augmented_module_key(ModuleKey::root(), "aaa")
                .add_dep("bbb", "1.0")
                .add_dep("ccc", "2.0")
                .build_entry(),
            build_augmented_module("bbb", "1.0")
                .add_changed_dep(
                    "ddd",
                    "2.0",
                    "1.0",
                    ResolutionReason::MINIMAL_VERSION_SELECTION
                )
                .add_still_dependant_key(ModuleKey::root())
                .build_entry(),
            build_augmented_module("ccc", "2.0")
                .add_dep("ddd", "2.0")
                .add_still_dependant_key(ModuleKey::root())
                .build_entry(),
            build_augmented_module("ddd", "2.0")
                .add_dependant("bbb", "1.0")
                .add_still_dependant("ccc", "2.0")
                .build_entry(),
            build_augmented_module("ddd", "1.0")
                .add_dep("eee", "1.0")
                .add_original_dependant("bbb", "1.0")
                .build_entry(),
            build_augmented_module("eee", "1.0")
                .add_original_dependant("ddd", "1.0")
                .build_entry(),
        ])
    );
}

#[test]
fn test_circular_dependency_due_to_selection() {
    let unpruned_dep_graph = map_of(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb", create_module_key("bbb", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("bbb", create_module_key("bbb", "1.0"))
            .add_original_dep("bbb", create_module_key("bbb", "1.0-pre"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0-pre")
            .add_dep("ddd", create_module_key("ddd", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ddd", "1.0").build_entry(),
    ]);

    let used_modules = used(vec![
        ModuleKey::root(),
        create_module_key("bbb", "1.0"),
        create_module_key("ccc", "2.0"),
    ]);

    let dep_graph = BazelModuleInspectorFunction::compute_augmented_graph(
        &unpruned_dep_graph,
        &used_modules,
        &IndexMap::new(),
    );

    assert_eq!(
        dep_graph,
        map_of(vec![
            build_augmented_module_key(ModuleKey::root(), "aaa")
                .add_dep("bbb", "1.0")
                .build_entry(),
            build_augmented_module("bbb", "1.0")
                .add_dep("ccc", "2.0")
                .add_still_dependant_key(ModuleKey::root())
                .add_dependant("ccc", "2.0")
                .build_entry(),
            build_augmented_module("ccc", "2.0")
                .add_changed_dep(
                    "bbb",
                    "1.0",
                    "1.0-pre",
                    ResolutionReason::MINIMAL_VERSION_SELECTION
                )
                .add_still_dependant("bbb", "1.0")
                .build_entry(),
            build_augmented_module("bbb", "1.0-pre")
                .add_dep("ddd", "1.0")
                .add_original_dependant("ccc", "2.0")
                .build_entry(),
            build_augmented_module("ddd", "1.0")
                .add_original_dependant("bbb", "1.0-pre")
                .build_entry(),
        ])
    );
}

#[test]
fn test_single_version_override_with_removal() {
    // Original (non-resolved) dep graph
    // single_version_override (ccc, 2.0)
    // aaa -> bbb 1.0 -> ccc 1.0 -> ddd 1.0
    //                   ccc 2.0 -> ddd 2.0
    let unpruned_dep_graph = map_of(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb", create_module_key("bbb", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .add_original_dep("ccc", create_module_key("ccc", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("ddd", create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("ddd", "2.0").build_entry(),
    ]);

    let overrides: IndexMap<String, ModuleOverride> = IndexMap::from([(
        "ccc".to_string(),
        ModuleOverride::SingleVersion(SingleVersionOverride {
            version: Version::parse("2.0").unwrap(),
            registry: String::new(),
            patches: vec![],
            patch_cmds: vec![],
            patch_strip: 0,
        }),
    )]);

    let used_modules = used(vec![
        ModuleKey::root(),
        create_module_key("bbb", "1.0"),
        create_module_key("ccc", "1.0"),
        create_module_key("ccc", "2.0"),
        create_module_key("ddd", "1.0"),
        create_module_key("ddd", "2.0"),
    ]);

    let dep_graph = BazelModuleInspectorFunction::compute_augmented_graph(
        &unpruned_dep_graph,
        &used_modules,
        &overrides,
    );

    assert_eq!(
        dep_graph,
        map_of(vec![
            build_augmented_module_key(ModuleKey::root(), "aaa")
                .add_dep("bbb", "1.0")
                .build_entry(),
            build_augmented_module("bbb", "1.0")
                .add_changed_dep(
                    "ccc",
                    "2.0",
                    "1.0",
                    ResolutionReason::SINGLE_VERSION_OVERRIDE
                )
                .add_still_dependant_key(ModuleKey::root())
                .build_entry(),
            build_augmented_module_loaded("ccc", "1.0", false)
                .add_original_dependant("bbb", "1.0")
                .build_entry(),
            build_augmented_module("ccc", "2.0")
                .add_dependant("bbb", "1.0")
                .add_dep("ddd", "2.0")
                .build_entry(),
            build_augmented_module("ddd", "2.0")
                .add_still_dependant("ccc", "2.0")
                .build_entry(),
        ])
    );
}

#[test]
fn test_non_registry_override_with_removal() {
    // Original (non-resolved) dep graph
    // archive_override "file://users/user/bbb.zip"
    // aaa    -> bbb 1.0        -> ccc 1.0 (not loaded)
    //   (local) bbb 1.0-hotfix -> ccc 1.1
    let unpruned_dep_graph = map_of(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb", create_module_key("bbb", ""))
            .add_original_dep("bbb", create_module_key("bbb", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .set_key(create_module_key("bbb", ""))
            .add_dep("ccc", create_module_key("ccc", "1.1"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "1.1").build_entry(),
    ]);

    let overrides: IndexMap<String, ModuleOverride> = IndexMap::from([(
        "bbb".to_string(),
        ModuleOverride::NonRegistry(NonRegistryOverride::new(
            ArchiveRepoSpecBuilder::new()
                .set_urls(vec!["file://users/user/bbb.zip".to_string()])
                .build(),
        )),
    )]);

    let used_modules = used(vec![
        ModuleKey::root(),
        create_module_key("bbb", ""),
        create_module_key("ccc", "1.1"),
    ]);

    let dep_graph = BazelModuleInspectorFunction::compute_augmented_graph(
        &unpruned_dep_graph,
        &used_modules,
        &overrides,
    );

    assert_eq!(
        dep_graph,
        map_of(vec![
            build_augmented_module_key(ModuleKey::root(), "aaa")
                .add_changed_dep("bbb", "", "1.0", ResolutionReason::NON_REGISTRY_OVERRIDE)
                .build_entry(),
            build_augmented_module_loaded("bbb", "1.0", false)
                .add_original_dependant_key(ModuleKey::root())
                .build_entry(),
            build_augmented_module_full(
                create_module_key("bbb", ""),
                "bbb",
                Version::parse("1.0").unwrap(),
                true
            )
            .add_dep("ccc", "1.1")
            .add_dependant_key(ModuleKey::root())
            .build_entry(),
            build_augmented_module("ccc", "1.1")
                .add_still_dependant("bbb", "")
                .build_entry(),
        ])
    );
}

#[test]
fn test_multiple_version_override_simple_snap_to_higher() {
    // Initial dep graph
    // aaa  -> (bbb1)bbb 1.0 -> ccc 1.0
    //     \-> (bbb2)bbb 2.0 -> ccc 1.5
    //     \-> ccc 2.0
    // multiple_version_override ccc: [1.5, 2.0]
    // multiple_version_override bbb: [1.0, 2.0]
    let unpruned_dep_graph = map_of(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb1", create_module_key("bbb", "1.0"))
            .add_dep("bbb2", create_module_key("bbb", "2.0"))
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ccc", create_module_key("ccc", "1.5"))
            .add_original_dep("ccc", create_module_key("ccc", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "2.0")
            .add_dep("ccc", create_module_key("ccc", "1.5"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "1.0").build_entry(),
        InterimModuleBuilder::create("ccc", "1.5").build_entry(),
        InterimModuleBuilder::create("ccc", "2.0").build_entry(),
    ]);

    let overrides: IndexMap<String, ModuleOverride> = IndexMap::from([
        (
            "bbb".to_string(),
            ModuleOverride::MultipleVersion(MultipleVersionOverride {
                versions: vec![
                    Version::parse("1.0").unwrap(),
                    Version::parse("2.0").unwrap(),
                ],
                registry: String::new(),
            }),
        ),
        (
            "ccc".to_string(),
            ModuleOverride::MultipleVersion(MultipleVersionOverride {
                versions: vec![
                    Version::parse("1.5").unwrap(),
                    Version::parse("2.0").unwrap(),
                ],
                registry: String::new(),
            }),
        ),
    ]);

    let used_modules = used(vec![
        ModuleKey::root(),
        create_module_key("bbb", "1.0"),
        create_module_key("bbb", "2.0"),
        create_module_key("ccc", "1.5"),
        create_module_key("ccc", "2.0"),
    ]);
    let dep_graph = BazelModuleInspectorFunction::compute_augmented_graph(
        &unpruned_dep_graph,
        &used_modules,
        &overrides,
    );

    assert_eq!(
        dep_graph,
        map_of(vec![
            build_augmented_module_key(ModuleKey::root(), "aaa")
                .add_dep_repo("bbb1", "bbb", "1.0")
                .add_dep_repo("bbb2", "bbb", "2.0")
                .add_dep("ccc", "2.0")
                .build_entry(),
            build_augmented_module("bbb", "1.0")
                .add_still_dependant_key(ModuleKey::root())
                .add_changed_dep(
                    "ccc",
                    "1.5",
                    "1.0",
                    ResolutionReason::MULTIPLE_VERSION_OVERRIDE
                )
                .build_entry(),
            build_augmented_module("bbb", "2.0")
                .add_still_dependant_key(ModuleKey::root())
                .add_dep("ccc", "1.5")
                .build_entry(),
            build_augmented_module("ccc", "1.0")
                .add_original_dependant("bbb", "1.0")
                .build_entry(),
            build_augmented_module("ccc", "1.5")
                .add_dependant("bbb", "1.0")
                .add_still_dependant("bbb", "2.0")
                .build_entry(),
            build_augmented_module("ccc", "2.0")
                .add_still_dependant_key(ModuleKey::root())
                .build_entry(),
        ])
    );
}

#[test]
fn test_multiple_version_override_bad_deps_unreferenced() {
    // Initial dep graph
    // aaa --> bbb1@1.0 --> ccc@1.0  [allowed]
    //     \            \-> bbb2@1.1
    //     \-> bbb2@1.0 --> ccc@1.5
    //     \-> bbb3@1.0 --> ccc@2.0  [allowed]
    //     \            \-> bbb4@1.1
    //     \-> bbb4@1.0 --> ccc@3.0
    //
    // Resolved dep graph
    // aaa --> bbb1@1.0 --> ccc@1.0  [allowed]
    //     \            \-> bbb2@1.1
    //     \-> bbb2@1.1
    //     \-> bbb3@1.0 --> ccc@2.0  [allowed]
    //     \            \-> bbb4@1.1
    //     \-> bbb4@1.1
    // ccc@1.5 and ccc@3.0, the versions violating the allowlist, are gone.
    let unpruned_dep_graph = map_of(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb1", create_module_key("bbb1", "1.0"))
            .add_dep("bbb2", create_module_key("bbb2", "1.1"))
            .add_original_dep("bbb2", create_module_key("bbb2", "1.0"))
            .add_dep("bbb3", create_module_key("bbb3", "1.0"))
            .add_dep("bbb4", create_module_key("bbb4", "1.1"))
            .add_original_dep("bbb4", create_module_key("bbb4", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb1", "1.0")
            .add_dep("ccc", create_module_key("ccc", "1.0"))
            .add_dep("bbb2", create_module_key("bbb2", "1.1"))
            .build_entry(),
        InterimModuleBuilder::create("bbb2", "1.0")
            .add_dep("ccc", create_module_key("ccc", "1.5"))
            .build_entry(),
        InterimModuleBuilder::create("bbb2", "1.1").build_entry(),
        InterimModuleBuilder::create("bbb3", "1.0")
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .add_dep("bbb4", create_module_key("bbb4", "1.1"))
            .build_entry(),
        InterimModuleBuilder::create("bbb4", "1.0")
            .add_dep("ccc", create_module_key("ccc", "3.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb4", "1.1").build_entry(),
        InterimModuleBuilder::create_cl("ccc", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ccc", "1.5", 1).build_entry(),
        InterimModuleBuilder::create_cl("ccc", "2.0", 2).build_entry(),
        InterimModuleBuilder::create_cl("ccc", "3.0", 3).build_entry(),
    ]);

    let overrides: IndexMap<String, ModuleOverride> = IndexMap::from([(
        "ccc".to_string(),
        ModuleOverride::MultipleVersion(MultipleVersionOverride {
            versions: vec![
                Version::parse("1.0").unwrap(),
                Version::parse("2.0").unwrap(),
            ],
            registry: String::new(),
        }),
    )]);

    let used_modules = used(vec![
        ModuleKey::root(),
        create_module_key("bbb1", "1.0"),
        create_module_key("bbb2", "1.1"),
        create_module_key("bbb3", "1.0"),
        create_module_key("bbb4", "1.1"),
        create_module_key("ccc", "1.0"),
        create_module_key("ccc", "2.0"),
    ]);
    let dep_graph = BazelModuleInspectorFunction::compute_augmented_graph(
        &unpruned_dep_graph,
        &used_modules,
        &overrides,
    );

    assert_eq!(
        dep_graph,
        map_of(vec![
            build_augmented_module_key(ModuleKey::root(), "aaa")
                .add_dep("bbb1", "1.0")
                .add_changed_dep(
                    "bbb2",
                    "1.1",
                    "1.0",
                    ResolutionReason::MINIMAL_VERSION_SELECTION
                )
                .add_dep("bbb3", "1.0")
                .add_changed_dep(
                    "bbb4",
                    "1.1",
                    "1.0",
                    ResolutionReason::MINIMAL_VERSION_SELECTION
                )
                .build_entry(),
            build_augmented_module("bbb1", "1.0")
                .add_dep("ccc", "1.0")
                .add_dep("bbb2", "1.1")
                .add_still_dependant_key(ModuleKey::root())
                .build_entry(),
            build_augmented_module("bbb2", "1.0")
                .add_dep("ccc", "1.5")
                .add_original_dependant_key(ModuleKey::root())
                .build_entry(),
            build_augmented_module("bbb2", "1.1")
                .add_dependant_key(ModuleKey::root())
                .add_still_dependant("bbb1", "1.0")
                .build_entry(),
            build_augmented_module("bbb3", "1.0")
                .add_dep("ccc", "2.0")
                .add_dep("bbb4", "1.1")
                .add_still_dependant_key(ModuleKey::root())
                .build_entry(),
            build_augmented_module("bbb4", "1.0")
                .add_dep("ccc", "3.0")
                .add_original_dependant_key(ModuleKey::root())
                .build_entry(),
            build_augmented_module("bbb4", "1.1")
                .add_dependant_key(ModuleKey::root())
                .add_still_dependant("bbb3", "1.0")
                .build_entry(),
            build_augmented_module("ccc", "1.0")
                .add_still_dependant("bbb1", "1.0")
                .build_entry(),
            build_augmented_module("ccc", "1.5")
                .add_original_dependant("bbb2", "1.0")
                .build_entry(),
            build_augmented_module("ccc", "2.0")
                .add_still_dependant("bbb3", "1.0")
                .build_entry(),
            build_augmented_module("ccc", "3.0")
                .add_original_dependant("bbb4", "1.0")
                .build_entry(),
        ])
    );
}
