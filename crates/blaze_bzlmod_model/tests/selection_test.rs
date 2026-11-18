//! Port of `SelectionTest` (src/test/java/com/google/devtools/build/lib/bazel/bzlmod).

mod common;

use blaze_bzlmod_model::{
    InterimModule, ModuleKey, ModuleOverride, MultipleVersionOverride, Selection, Version,
};
use common::*;
use indexmap::IndexMap;

/// `assertThat(graph.entrySet()).containsExactly(...).inOrder()`.
fn assert_in_order(
    actual: &IndexMap<ModuleKey, InterimModule>,
    expected: Vec<(ModuleKey, InterimModule)>,
) {
    let actual: Vec<(ModuleKey, InterimModule)> =
        actual.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    assert_eq!(actual, expected);
}

/// `assertThat(graph.entrySet()).containsExactly(...)` (any order).
fn assert_unordered(
    actual: &IndexMap<ModuleKey, InterimModule>,
    expected: Vec<(ModuleKey, InterimModule)>,
) {
    assert_eq!(actual.len(), expected.len(), "{actual:#?}");
    for (k, v) in &expected {
        assert_eq!(actual.get(k), Some(v), "entry {k}");
    }
}

#[test]
fn diamond_simple() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ddd_from_bbb", create_module_key("ddd", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ddd", "2.0", 1).build_entry(),
    ]);

    let selection_result = Selection::run(&dep_graph, &IndexMap::new()).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
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
            InterimModuleBuilder::create_cl("ddd", "2.0", 1).build_entry(),
        ],
    );

    assert_unordered(
        &selection_result.unpruned_dep_graph,
        vec![
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
        ],
    );
}

#[test]
fn diamond_nodeps() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ddd_from_bbb", create_module_key("ddd", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_nodep_dep(create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ddd", "2.0", 1).build_entry(),
    ]);

    let selection_result = Selection::run(&dep_graph, &IndexMap::new()).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
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
                .add_nodep_dep(create_module_key("ddd", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ddd", "2.0", 1).build_entry(),
        ],
    );

    assert_unordered(
        &selection_result.unpruned_dep_graph,
        vec![
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
                .add_nodep_dep(create_module_key("ddd", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
            InterimModuleBuilder::create_cl("ddd", "2.0", 1).build_entry(),
        ],
    );
}

#[test]
fn diamond_with_ignored_non_affecting_max_compatibility_level() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "1.0", 3))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ddd", "2.0", 1).build_entry(),
    ]);

    let selection_result = Selection::run(&dep_graph, &IndexMap::new()).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
                .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "2.0", 3))
                .add_original_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "1.0", 3))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ddd", "2.0", 1).build_entry(),
        ],
    );

    assert_unordered(
        &selection_result.unpruned_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
                .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "2.0", 3))
                .add_original_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "1.0", 3))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
            InterimModuleBuilder::create_cl("ddd", "2.0", 1).build_entry(),
        ],
    );
}

#[test]
fn diamond_with_selected_non_affecting_max_compatibility_level() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ddd_from_bbb", create_module_key("ddd", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep_spec("ddd_from_ccc", create_dep_spec("ddd", "2.0", 4))
            .build_entry(),
        InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ddd", "2.0", 1).build_entry(),
    ]);

    let selection_result = Selection::run(&dep_graph, &IndexMap::new()).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
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
                .add_dep_spec("ddd_from_ccc", create_dep_spec("ddd", "2.0", 4))
                .add_original_dep_spec("ddd_from_ccc", create_dep_spec("ddd", "2.0", 4))
                .build_entry(),
            InterimModuleBuilder::create_cl("ddd", "2.0", 1).build_entry(),
        ],
    );

    assert_unordered(
        &selection_result.unpruned_dep_graph,
        vec![
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
                .add_dep_spec("ddd_from_ccc", create_dep_spec("ddd", "2.0", 4))
                .add_original_dep_spec("ddd_from_ccc", create_dep_spec("ddd", "2.0", 4))
                .build_entry(),
            InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
            InterimModuleBuilder::create_cl("ddd", "2.0", 1).build_entry(),
        ],
    );
}

#[test]
fn diamond_with_further_removal() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb", create_module_key("bbb", "1.0"))
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ddd", create_module_key("ddd", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("ddd", create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("ddd", "1.0")
            .add_dep("eee", create_module_key("eee", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ddd", "2.0").build_entry(),
        // Only D@1.0 needs E. When D@1.0 is removed, E should be gone as well (even though
        // E@1.0 is selected for E).
        InterimModuleBuilder::create("eee", "1.0").build_entry(),
    ]);

    let selection_result = Selection::run(&dep_graph, &IndexMap::new()).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
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
        ],
    );

    assert_unordered(
        &selection_result.unpruned_dep_graph,
        vec![
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
        ],
    );
}

#[test]
fn diamond_with_further_removal_and_no_deps() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb", create_module_key("bbb", "1.0"))
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ddd", create_module_key("ddd", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("ddd", create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("ddd", "1.0")
            .add_dep("eee", create_module_key("eee", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ddd", "2.0")
            .add_nodep_dep(create_module_key("eee", "2.0"))
            .build_entry(),
        // eee@2.0 ends up being selected over eee@1.0. But eee@2.0 is not actually reachable
        // from the root, since ddd@1.0 isn't selected, and ddd@2.0 only has a nodep dep on
        // eee@2.0. So neither version of eee@2.0 ends up in the final dep graph.
        InterimModuleBuilder::create("eee", "1.0").build_entry(),
        InterimModuleBuilder::create("eee", "2.0").build_entry(),
    ]);

    let selection_result = Selection::run(&dep_graph, &IndexMap::new()).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
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
            InterimModuleBuilder::create("ddd", "2.0")
                .add_nodep_dep(create_module_key("eee", "2.0"))
                .build_entry(),
        ],
    );

    assert_unordered(
        &selection_result.unpruned_dep_graph,
        vec![
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
            InterimModuleBuilder::create("ddd", "1.0")
                .add_dep("eee", create_module_key("eee", "2.0"))
                .add_original_dep("eee", create_module_key("eee", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("ddd", "2.0")
                .add_nodep_dep(create_module_key("eee", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("eee", "1.0").build_entry(),
            InterimModuleBuilder::create("eee", "2.0").build_entry(),
        ],
    );
}

#[test]
fn circular_dependency_due_to_selection() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb", create_module_key("bbb", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("bbb", create_module_key("bbb", "1.0-pre"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0-pre")
            .add_dep("ddd", create_module_key("ddd", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ddd", "1.0").build_entry(),
    ]);

    let selection_result = Selection::run(&dep_graph, &IndexMap::new()).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
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
        ],
    );
    // D is completely gone.

    assert_unordered(
        &selection_result.unpruned_dep_graph,
        vec![
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
        ],
    );
}

#[test]
fn different_compatibility_level_is_rejected() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ddd_from_bbb", create_module_key("ddd", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ddd", "2.0", 2).build_entry(),
    ]);

    let e = Selection::run(&dep_graph, &IndexMap::new()).unwrap_err();
    let error = e.get_message().to_string();
    assert!(
        error.contains("bbb@1.0 depends on ddd@1.0 with compatibility level 1"),
        "{error}"
    );
    assert!(
        error.contains("ccc@2.0 depends on ddd@2.0 with compatibility level 2"),
        "{error}"
    );
    assert!(error.contains("which is different"), "{error}");
}

#[test]
fn different_compatibility_level_is_rejected_even_for_nodep() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ddd_from_bbb", create_module_key("ddd", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_nodep_dep(create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ddd", "2.0", 2).build_entry(),
    ]);

    let e = Selection::run(&dep_graph, &IndexMap::new()).unwrap_err();
    let error = e.get_message().to_string();
    assert!(
        error.contains("bbb@1.0 depends on ddd@1.0 with compatibility level 1"),
        "{error}"
    );
    assert!(
        error.contains("ccc@2.0 depends on ddd@2.0 with compatibility level 2"),
        "{error}"
    );
    assert!(error.contains("which is different"), "{error}");
}

#[test]
fn different_compatibility_level_with_max_compatibility_level_is_rejected() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ddd_from_bbb", create_module_key("ddd", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep_spec("ddd_from_ccc", create_dep_spec("ddd", "2.0", 3))
            .build_entry(),
        InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ddd", "2.0", 2).build_entry(),
    ]);

    let e = Selection::run(&dep_graph, &IndexMap::new()).unwrap_err();
    let error = e.get_message().to_string();
    assert!(
        error.contains("bbb@1.0 depends on ddd@1.0 with compatibility level 1"),
        "{error}"
    );
    assert!(
        error.contains("ccc@2.0 depends on ddd@2.0 with compatibility level 2"),
        "{error}"
    );
    assert!(error.contains("which is different"), "{error}");
}

#[test]
fn max_compatibility_based_selection() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "1.0", 2))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ddd", "2.0", 2).build_entry(),
    ]);

    let selection_result = Selection::run(&dep_graph, &IndexMap::new()).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
                .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "2.0", 2))
                .add_original_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "1.0", 2))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ddd", "2.0", 2).build_entry(),
        ],
    );

    assert_unordered(
        &selection_result.unpruned_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
                .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "2.0", 2))
                .add_original_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "1.0", 2))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
            InterimModuleBuilder::create_cl("ddd", "2.0", 2).build_entry(),
        ],
    );
}

#[test]
fn max_compatibility_based_selection_nodep_causes_upgrade() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "1.0", 2))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_nodep_dep(create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ddd", "2.0", 2).build_entry(),
    ]);

    let selection_result = Selection::run(&dep_graph, &IndexMap::new()).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
                .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "2.0", 2))
                .add_original_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "1.0", 2))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .add_nodep_dep(create_module_key("ddd", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ddd", "2.0", 2).build_entry(),
        ],
    );

    assert_unordered(
        &selection_result.unpruned_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
                .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "2.0", 2))
                .add_original_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "1.0", 2))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .add_nodep_dep(create_module_key("ddd", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
            InterimModuleBuilder::create_cl("ddd", "2.0", 2).build_entry(),
        ],
    );
}

#[test]
fn max_compatibility_based_selection_same_version() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "2.0", 3))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ddd", "2.0", 2).build_entry(),
    ]);

    let selection_result = Selection::run(&dep_graph, &IndexMap::new()).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
                .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "2.0", 3))
                .add_original_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "2.0", 3))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ddd", "2.0", 2).build_entry(),
        ],
    );

    assert_unordered(
        &selection_result.unpruned_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
                .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "2.0", 3))
                .add_original_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "2.0", 3))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ddd", "2.0", 2).build_entry(),
        ],
    );
}

#[test]
fn max_compatibility_based_selection_limited_to_max() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep_spec("ddd_from_bbb", create_dep_spec("ddd", "1.0", 2))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("ddd_from_ccc", create_module_key("ddd", "3.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ddd", "3.0", 3).build_entry(),
    ]);

    let e = Selection::run(&dep_graph, &IndexMap::new()).unwrap_err();
    let error = e.get_message().to_string();
    assert!(
        error.contains("bbb@1.0 depends on ddd@1.0 with compatibility level 1"),
        "{error}"
    );
    assert!(
        error.contains("ccc@2.0 depends on ddd@3.0 with compatibility level 3"),
        "{error}"
    );
    assert!(error.contains("which is different"), "{error}");
}

#[test]
fn max_compatibility_based_selection_unreferenced_not_selected() {
    // aaa 1.0 -> bbb 1.0 -> ccc 2.0
    //       \-> ccc 1.0 (max_compatibility_level=2)
    //        \-> ddd 1.0 -> bbb 1.1
    //         \-> eee 1.0 -> ccc 1.1
    let dep_graph = graph(vec![
        InterimModuleBuilder::create("aaa", "1.0")
            .set_key(ModuleKey::root())
            .add_dep("bbb", create_module_key("bbb", "1.0"))
            .add_dep_spec("ccc", create_dep_spec("ccc", "1.0", 2))
            .add_dep("ddd", create_module_key("ddd", "1.0"))
            .add_dep("eee", create_module_key("eee", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ccc", "2.0", 2).build_entry(),
        InterimModuleBuilder::create_cl("ccc", "1.0", 1).build_entry(),
        InterimModuleBuilder::create("ddd", "1.0")
            .add_dep("bbb", create_module_key("bbb", "1.1"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.1").build_entry(),
        InterimModuleBuilder::create("eee", "1.0")
            .add_dep("ccc", create_module_key("ccc", "1.1"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ccc", "1.1", 1).build_entry(),
    ]);

    // After selection, ccc 2.0 is gone, so ccc 1.0 (max_compatibility_level=2) doesn't upgrade to
    // ccc 2.0, and only upgrades to ccc 1.1
    // aaa 1.0 -> bbb 1.1
    //       \-> ccc 1.1
    //        \-> ddd 1.0 -> bbb 1.1
    //         \-> eee 1.0 -> ccc 1.1
    let selection_result = Selection::run(&dep_graph, &IndexMap::new()).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
            InterimModuleBuilder::create("aaa", "1.0")
                .set_key(ModuleKey::root())
                .add_dep("bbb", create_module_key("bbb", "1.1"))
                .add_original_dep("bbb", create_module_key("bbb", "1.0"))
                .add_dep_spec("ccc", create_dep_spec("ccc", "1.1", 2))
                .add_original_dep_spec("ccc", create_dep_spec("ccc", "1.0", 2))
                .add_dep("ddd", create_module_key("ddd", "1.0"))
                .add_dep("eee", create_module_key("eee", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.1").build_entry(),
            InterimModuleBuilder::create_cl("ccc", "1.1", 1).build_entry(),
            InterimModuleBuilder::create("ddd", "1.0")
                .add_dep("bbb", create_module_key("bbb", "1.1"))
                .build_entry(),
            InterimModuleBuilder::create("eee", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.1"))
                .build_entry(),
        ],
    );

    assert_unordered(
        &selection_result.unpruned_dep_graph,
        vec![
            InterimModuleBuilder::create("aaa", "1.0")
                .set_key(ModuleKey::root())
                .add_dep("bbb", create_module_key("bbb", "1.1"))
                .add_original_dep("bbb", create_module_key("bbb", "1.0"))
                .add_dep_spec("ccc", create_dep_spec("ccc", "1.1", 2))
                .add_original_dep_spec("ccc", create_dep_spec("ccc", "1.0", 2))
                .add_dep("ddd", create_module_key("ddd", "1.0"))
                .add_dep("eee", create_module_key("eee", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("ccc", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.1").build_entry(),
            InterimModuleBuilder::create_cl("ccc", "1.0", 1).build_entry(),
            InterimModuleBuilder::create_cl("ccc", "1.1", 1).build_entry(),
            InterimModuleBuilder::create_cl("ccc", "2.0", 2).build_entry(),
            InterimModuleBuilder::create("ddd", "1.0")
                .add_dep("bbb", create_module_key("bbb", "1.1"))
                .build_entry(),
            InterimModuleBuilder::create("eee", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.1"))
                .build_entry(),
        ],
    );
}

#[test]
fn max_compatibility_based_selection_with_multiple_version_override() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep_spec("ccc_from_bbb", create_dep_spec("ccc", "1.5", 2))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "1.0").build_entry(),
        InterimModuleBuilder::create("ccc", "1.5").build_entry(),
        InterimModuleBuilder::create_cl("ccc", "2.0", 2).build_entry(),
    ]);
    let overrides: IndexMap<String, ModuleOverride> = IndexMap::from([(
        "ccc".to_string(),
        ModuleOverride::MultipleVersion(MultipleVersionOverride {
            versions: vec![
                Version::parse("1.0").unwrap(),
                Version::parse("2.0").unwrap(),
            ],
            registry: "".to_string(),
        }),
    )]);

    let selection_result = Selection::run(&dep_graph, &overrides).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
                .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep_spec("ccc_from_bbb", create_dep_spec("ccc", "2.0", 2))
                .add_original_dep_spec("ccc_from_bbb", create_dep_spec("ccc", "1.5", 2))
                .build_entry(),
            InterimModuleBuilder::create_cl("ccc", "2.0", 2).build_entry(),
        ],
    );

    assert_in_order(
        &selection_result.unpruned_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
                .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep_spec("ccc_from_bbb", create_dep_spec("ccc", "2.0", 2))
                .add_original_dep_spec("ccc_from_bbb", create_dep_spec("ccc", "1.5", 2))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "1.0").build_entry(),
            InterimModuleBuilder::create("ccc", "1.5").build_entry(),
            InterimModuleBuilder::create_cl("ccc", "2.0", 2).build_entry(),
        ],
    );
}

#[test]
fn max_compatibility_based_selection_non_greedy_selection() {
    // A dep graph in which always picking the highest (or lowest) reachable compatibility level for
    // each module does *not* result in a valid selection: c@1.0 and b@2.0 mutually depend on each
    // other and so do c@2.0 and b@1.0.
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep_spec("bbb_from_aaa", create_dep_spec("bbb", "1.0", 2))
            .add_dep_spec("ccc_from_aaa", create_dep_spec("ccc", "1.0", 2))
            .build_entry(),
        InterimModuleBuilder::create_cl("bbb", "1.0", 1)
            .add_dep("ccc_from_bbb", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("bbb", "2.0", 2)
            .add_dep("ccc_from_bbb", create_module_key("ccc", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ccc", "1.0", 1)
            .add_dep("bbb_from_ccc", create_module_key("bbb", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ccc", "2.0", 2)
            .add_dep("bbb_from_ccc", create_module_key("bbb", "1.0"))
            .build_entry(),
    ]);

    let selection_result = Selection::run(&dep_graph, &IndexMap::new()).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep_spec("bbb_from_aaa", create_dep_spec("bbb", "1.0", 2))
                .add_dep_spec("ccc_from_aaa", create_dep_spec("ccc", "2.0", 2))
                .add_original_dep_spec("ccc_from_aaa", create_dep_spec("ccc", "1.0", 2))
                .build_entry(),
            InterimModuleBuilder::create_cl("bbb", "1.0", 1)
                .add_dep("ccc_from_bbb", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ccc", "2.0", 2)
                .add_dep("bbb_from_ccc", create_module_key("bbb", "1.0"))
                .build_entry(),
        ],
    );

    assert_in_order(
        &selection_result.unpruned_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep_spec("bbb_from_aaa", create_dep_spec("bbb", "1.0", 2))
                .add_dep_spec("ccc_from_aaa", create_dep_spec("ccc", "2.0", 2))
                .add_original_dep_spec("ccc_from_aaa", create_dep_spec("ccc", "1.0", 2))
                .build_entry(),
            InterimModuleBuilder::create_cl("bbb", "1.0", 1)
                .add_dep("ccc_from_bbb", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("bbb", "2.0", 2)
                .add_dep("ccc_from_bbb", create_module_key("ccc", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ccc", "1.0", 1)
                .add_dep("bbb_from_ccc", create_module_key("bbb", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ccc", "2.0", 2)
                .add_dep("bbb_from_ccc", create_module_key("bbb", "1.0"))
                .build_entry(),
        ],
    );
}

#[test]
fn different_compatibility_level_is_ok_if_unreferenced() {
    // aaa 1.0 -> bbb 1.0 -> ccc 2.0
    //       \-> ccc 1.0
    //        \-> ddd 1.0 -> bbb 1.1
    //         \-> eee 1.0 -> ccc 1.1
    let dep_graph = graph(vec![
        InterimModuleBuilder::create("aaa", "1.0")
            .set_key(ModuleKey::root())
            .add_dep("bbb", create_module_key("bbb", "1.0"))
            .add_dep("ccc", create_module_key("ccc", "1.0"))
            .add_dep("ddd", create_module_key("ddd", "1.0"))
            .add_dep("eee", create_module_key("eee", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ccc", "2.0", 2).build_entry(),
        InterimModuleBuilder::create_cl("ccc", "1.0", 1).build_entry(),
        InterimModuleBuilder::create("ddd", "1.0")
            .add_dep("bbb", create_module_key("bbb", "1.1"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.1").build_entry(),
        InterimModuleBuilder::create("eee", "1.0")
            .add_dep("ccc", create_module_key("ccc", "1.1"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ccc", "1.1", 1).build_entry(),
    ]);

    // After selection, ccc 2.0 is gone, so we're okay.
    // aaa 1.0 -> bbb 1.1
    //       \-> ccc 1.1
    //        \-> ddd 1.0 -> bbb 1.1
    //         \-> eee 1.0 -> ccc 1.1
    let selection_result = Selection::run(&dep_graph, &IndexMap::new()).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
            InterimModuleBuilder::create("aaa", "1.0")
                .set_key(ModuleKey::root())
                .add_dep("bbb", create_module_key("bbb", "1.1"))
                .add_original_dep("bbb", create_module_key("bbb", "1.0"))
                .add_dep("ccc", create_module_key("ccc", "1.1"))
                .add_original_dep("ccc", create_module_key("ccc", "1.0"))
                .add_dep("ddd", create_module_key("ddd", "1.0"))
                .add_dep("eee", create_module_key("eee", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.1").build_entry(),
            InterimModuleBuilder::create_cl("ccc", "1.1", 1).build_entry(),
            InterimModuleBuilder::create("ddd", "1.0")
                .add_dep("bbb", create_module_key("bbb", "1.1"))
                .build_entry(),
            InterimModuleBuilder::create("eee", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.1"))
                .build_entry(),
        ],
    );

    assert_unordered(
        &selection_result.unpruned_dep_graph,
        vec![
            InterimModuleBuilder::create("aaa", "1.0")
                .set_key(ModuleKey::root())
                .add_dep("bbb", create_module_key("bbb", "1.1"))
                .add_original_dep("bbb", create_module_key("bbb", "1.0"))
                .add_dep("ccc", create_module_key("ccc", "1.1"))
                .add_original_dep("ccc", create_module_key("ccc", "1.0"))
                .add_dep("ddd", create_module_key("ddd", "1.0"))
                .add_dep("eee", create_module_key("eee", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("ccc", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.1").build_entry(),
            InterimModuleBuilder::create_cl("ccc", "1.0", 1).build_entry(),
            InterimModuleBuilder::create_cl("ccc", "1.1", 1).build_entry(),
            InterimModuleBuilder::create_cl("ccc", "2.0", 2).build_entry(),
            InterimModuleBuilder::create("ddd", "1.0")
                .add_dep("bbb", create_module_key("bbb", "1.1"))
                .build_entry(),
            InterimModuleBuilder::create("eee", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.1"))
                .build_entry(),
        ],
    );
}

#[test]
fn multiple_version_override_fork_allowed_version_missing_in_dep_graph() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb1", create_module_key("bbb", "1.0"))
            .add_dep("bbb2", create_module_key("bbb", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0").build_entry(),
        InterimModuleBuilder::create("bbb", "2.0").build_entry(),
    ]);
    let overrides: IndexMap<String, ModuleOverride> = IndexMap::from([(
        "bbb".to_string(),
        ModuleOverride::MultipleVersion(MultipleVersionOverride {
            versions: vec![
                Version::parse("1.0").unwrap(),
                Version::parse("2.0").unwrap(),
                Version::parse("3.0").unwrap(),
            ],
            registry: "".to_string(),
        }),
    )]);

    let e = Selection::run(&dep_graph, &overrides).unwrap_err();
    assert!(e.get_message().contains("multiple_version_override for module bbb contains version 3.0, but it doesn't exist in the dependency graph"), "{}", e.get_message());
}

#[test]
fn multiple_version_override_fork_good_case() {
    // For more complex good cases, see the "diamond" test cases below.
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb1", create_module_key("bbb", "1.0"))
            .add_dep("bbb2", create_module_key("bbb", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0").build_entry(),
        InterimModuleBuilder::create("bbb", "2.0").build_entry(),
    ]);
    let overrides: IndexMap<String, ModuleOverride> = IndexMap::from([(
        "bbb".to_string(),
        ModuleOverride::MultipleVersion(MultipleVersionOverride {
            versions: vec![
                Version::parse("1.0").unwrap(),
                Version::parse("2.0").unwrap(),
            ],
            registry: "".to_string(),
        }),
    )]);

    let selection_result = Selection::run(&dep_graph, &overrides).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb1", create_module_key("bbb", "1.0"))
                .add_dep("bbb2", create_module_key("bbb", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0").build_entry(),
            InterimModuleBuilder::create("bbb", "2.0").build_entry(),
        ],
    );

    assert_eq!(
        selection_result.unpruned_dep_graph,
        selection_result.resolved_dep_graph
    );
}

#[test]
fn multiple_version_override_fork_same_version_used_twice() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb1", create_module_key("bbb", "1.0"))
            .add_dep("bbb2", create_module_key("bbb", "1.3"))
            .add_dep("bbb3", create_module_key("bbb", "1.5"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0").build_entry(),
        InterimModuleBuilder::create("bbb", "1.3").build_entry(),
        InterimModuleBuilder::create("bbb", "1.5").build_entry(),
    ]);
    let overrides: IndexMap<String, ModuleOverride> = IndexMap::from([(
        "bbb".to_string(),
        ModuleOverride::MultipleVersion(MultipleVersionOverride {
            versions: vec![
                Version::parse("1.0").unwrap(),
                Version::parse("1.5").unwrap(),
            ],
            registry: "".to_string(),
        }),
    )]);

    let e = Selection::run(&dep_graph, &overrides).unwrap_err();
    assert!(regex::Regex::new("aaa@_ depends on bbb@1.5 at least twice \\(with repo names (bbb2 and bbb3)|(bbb3 and bbb2)\\)").unwrap().is_match(e.get_message()), "{}", e.get_message());
    assert!(
        e.get_message()
            .contains("if you want to depend on multiple versions of bbb simultaneously"),
        "{}",
        e.get_message()
    );
}

#[test]
fn multiple_version_override_diamond_different_compatibility_levels() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ddd_from_bbb", create_module_key("ddd", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ddd", "2.0", 2).build_entry(),
    ]);
    let overrides: IndexMap<String, ModuleOverride> = IndexMap::from([(
        "ddd".to_string(),
        ModuleOverride::MultipleVersion(MultipleVersionOverride {
            versions: vec![
                Version::parse("1.0").unwrap(),
                Version::parse("2.0").unwrap(),
            ],
            registry: "".to_string(),
        }),
    )]);

    let selection_result = Selection::run(&dep_graph, &overrides).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
                .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("ddd_from_bbb", create_module_key("ddd", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ddd", "1.0", 1).build_entry(),
            InterimModuleBuilder::create_cl("ddd", "2.0", 2).build_entry(),
        ],
    );

    assert_eq!(
        selection_result.unpruned_dep_graph,
        selection_result.resolved_dep_graph
    );
}

#[test]
fn multiple_version_override_diamond_same_compatibility_level() {
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
            .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ddd_from_bbb", create_module_key("ddd", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("ccc", "2.0")
            .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("ddd", "1.0").build_entry(),
        InterimModuleBuilder::create("ddd", "2.0").build_entry(),
    ]);
    let overrides: IndexMap<String, ModuleOverride> = IndexMap::from([(
        "ddd".to_string(),
        ModuleOverride::MultipleVersion(MultipleVersionOverride {
            versions: vec![
                Version::parse("1.0").unwrap(),
                Version::parse("2.0").unwrap(),
            ],
            registry: "".to_string(),
        }),
    )]);

    let selection_result = Selection::run(&dep_graph, &overrides).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb_from_aaa", create_module_key("bbb", "1.0"))
                .add_dep("ccc_from_aaa", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("ddd_from_bbb", create_module_key("ddd", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .add_dep("ddd_from_ccc", create_module_key("ddd", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("ddd", "1.0").build_entry(),
            InterimModuleBuilder::create("ddd", "2.0").build_entry(),
        ],
    );

    assert_eq!(
        selection_result.unpruned_dep_graph,
        selection_result.resolved_dep_graph
    );
}

#[test]
fn multiple_version_override_diamond_snapping_to_next_highest_version() {
    // aaa --> bbb1@1.0 -> ccc@1.0
    //     \-> bbb2@1.0 -> ccc@1.3  [allowed]
    //     \-> bbb3@1.0 -> ccc@1.5
    //     \-> bbb4@1.0 -> ccc@1.7  [allowed]
    //     \-> bbb5@1.0 -> ccc@2.0  [allowed]
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb1", create_module_key("bbb1", "1.0"))
            .add_dep("bbb2", create_module_key("bbb2", "1.0"))
            .add_dep("bbb3", create_module_key("bbb3", "1.0"))
            .add_dep("bbb4", create_module_key("bbb4", "1.0"))
            .add_dep("bbb5", create_module_key("bbb5", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb1", "1.0")
            .add_dep("ccc", create_module_key("ccc", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb2", "1.0")
            .add_dep("ccc", create_module_key("ccc", "1.3"))
            .build_entry(),
        InterimModuleBuilder::create("bbb3", "1.0")
            .add_dep("ccc", create_module_key("ccc", "1.5"))
            .build_entry(),
        InterimModuleBuilder::create("bbb4", "1.0")
            .add_dep("ccc", create_module_key("ccc", "1.7"))
            .build_entry(),
        InterimModuleBuilder::create("bbb5", "1.0")
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ccc", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ccc", "1.3", 1).build_entry(),
        InterimModuleBuilder::create_cl("ccc", "1.5", 1).build_entry(),
        InterimModuleBuilder::create_cl("ccc", "1.7", 1).build_entry(),
        InterimModuleBuilder::create_cl("ccc", "2.0", 2).build_entry(),
    ]);
    let overrides: IndexMap<String, ModuleOverride> = IndexMap::from([(
        "ccc".to_string(),
        ModuleOverride::MultipleVersion(MultipleVersionOverride {
            versions: vec![
                Version::parse("1.3").unwrap(),
                Version::parse("1.7").unwrap(),
                Version::parse("2.0").unwrap(),
            ],
            registry: "".to_string(),
        }),
    )]);

    // aaa --> bbb1@1.0 -> ccc@1.3  [originally ccc@1.0]
    //     \-> bbb2@1.0 -> ccc@1.3  [allowed]
    //     \-> bbb3@1.0 -> ccc@1.7  [originally ccc@1.5]
    //     \-> bbb4@1.0 -> ccc@1.7  [allowed]
    //     \-> bbb5@1.0 -> ccc@2.0  [allowed]
    let selection_result = Selection::run(&dep_graph, &overrides).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb1", create_module_key("bbb1", "1.0"))
                .add_dep("bbb2", create_module_key("bbb2", "1.0"))
                .add_dep("bbb3", create_module_key("bbb3", "1.0"))
                .add_dep("bbb4", create_module_key("bbb4", "1.0"))
                .add_dep("bbb5", create_module_key("bbb5", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb1", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.3"))
                .add_original_dep("ccc", create_module_key("ccc", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb2", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.3"))
                .build_entry(),
            InterimModuleBuilder::create("bbb3", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.7"))
                .add_original_dep("ccc", create_module_key("ccc", "1.5"))
                .build_entry(),
            InterimModuleBuilder::create("bbb4", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.7"))
                .build_entry(),
            InterimModuleBuilder::create("bbb5", "1.0")
                .add_dep("ccc", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ccc", "1.3", 1).build_entry(),
            InterimModuleBuilder::create_cl("ccc", "1.7", 1).build_entry(),
            InterimModuleBuilder::create_cl("ccc", "2.0", 2).build_entry(),
        ],
    );

    assert_unordered(
        &selection_result.unpruned_dep_graph,
        vec![
            InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
                .set_key(ModuleKey::root())
                .add_dep("bbb1", create_module_key("bbb1", "1.0"))
                .add_dep("bbb2", create_module_key("bbb2", "1.0"))
                .add_dep("bbb3", create_module_key("bbb3", "1.0"))
                .add_dep("bbb4", create_module_key("bbb4", "1.0"))
                .add_dep("bbb5", create_module_key("bbb5", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb1", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.3"))
                .add_original_dep("ccc", create_module_key("ccc", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb2", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.3"))
                .build_entry(),
            InterimModuleBuilder::create("bbb3", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.7"))
                .add_original_dep("ccc", create_module_key("ccc", "1.5"))
                .build_entry(),
            InterimModuleBuilder::create("bbb4", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.7"))
                .build_entry(),
            InterimModuleBuilder::create("bbb5", "1.0")
                .add_dep("ccc", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create_cl("ccc", "1.0", 1).build_entry(),
            InterimModuleBuilder::create_cl("ccc", "1.3", 1).build_entry(),
            InterimModuleBuilder::create_cl("ccc", "1.5", 1).build_entry(),
            InterimModuleBuilder::create_cl("ccc", "1.7", 1).build_entry(),
            InterimModuleBuilder::create_cl("ccc", "2.0", 2).build_entry(),
        ],
    );
}

#[test]
fn multiple_version_override_diamond_dont_snap_to_different_compatibility() {
    // aaa --> bbb1@1.0 -> ccc@1.0  [allowed]
    //     \-> bbb2@1.0 -> ccc@1.7
    //     \-> bbb3@1.0 -> ccc@2.0  [allowed]
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb1", create_module_key("bbb1", "1.0"))
            .add_dep("bbb2", create_module_key("bbb2", "1.0"))
            .add_dep("bbb3", create_module_key("bbb3", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb1", "1.0")
            .add_dep("ccc", create_module_key("ccc", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb2", "1.0")
            .add_dep("ccc", create_module_key("ccc", "1.7"))
            .build_entry(),
        InterimModuleBuilder::create("bbb3", "1.0")
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ccc", "1.0", 1).build_entry(),
        InterimModuleBuilder::create_cl("ccc", "1.7", 1).build_entry(),
        InterimModuleBuilder::create_cl("ccc", "2.0", 2).build_entry(),
    ]);
    let overrides: IndexMap<String, ModuleOverride> = IndexMap::from([(
        "ccc".to_string(),
        ModuleOverride::MultipleVersion(MultipleVersionOverride {
            versions: vec![
                Version::parse("1.0").unwrap(),
                Version::parse("2.0").unwrap(),
            ],
            registry: "".to_string(),
        }),
    )]);

    let e = Selection::run(&dep_graph, &overrides).unwrap_err();
    assert!(e.get_message().contains("bbb2@1.0 depends on ccc@1.7 which is not allowed by the multiple_version_override on ccc, which allows only [1.0, 2.0]"), "{}", e.get_message());
}

#[test]
fn multiple_version_override_diamond_unknown_compatibility() {
    // aaa --> bbb1@1.0 -> ccc@1.0  [allowed]
    //     \-> bbb2@1.0 -> ccc@2.0  [allowed]
    //     \-> bbb3@1.0 -> ccc@3.0
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb1", create_module_key("bbb1", "1.0"))
            .add_dep("bbb2", create_module_key("bbb2", "1.0"))
            .add_dep("bbb3", create_module_key("bbb3", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb1", "1.0")
            .add_dep("ccc", create_module_key("ccc", "1.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb2", "1.0")
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .build_entry(),
        InterimModuleBuilder::create("bbb3", "1.0")
            .add_dep("ccc", create_module_key("ccc", "3.0"))
            .build_entry(),
        InterimModuleBuilder::create_cl("ccc", "1.0", 1).build_entry(),
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
            registry: "".to_string(),
        }),
    )]);

    let e = Selection::run(&dep_graph, &overrides).unwrap_err();
    assert!(e.get_message().contains("bbb3@1.0 depends on ccc@3.0 which is not allowed by the multiple_version_override on ccc, which allows only [1.0, 2.0]"), "{}", e.get_message());
}

#[test]
fn multiple_version_override_diamond_bad_versions_are_okay_if_unreferenced() {
    // aaa --> bbb1@1.0 --> ccc@1.0  [allowed]
    //     \            \-> bbb2@1.1
    //     \-> bbb2@1.0 --> ccc@1.5
    //     \-> bbb3@1.0 --> ccc@2.0  [allowed]
    //     \            \-> bbb4@1.1
    //     \-> bbb4@1.0 --> ccc@3.0
    let dep_graph = graph(vec![
        InterimModuleBuilder::create_v("aaa", Version::empty(), 0)
            .set_key(ModuleKey::root())
            .add_dep("bbb1", create_module_key("bbb1", "1.0"))
            .add_dep("bbb2", create_module_key("bbb2", "1.0"))
            .add_dep("bbb3", create_module_key("bbb3", "1.0"))
            .add_dep("bbb4", create_module_key("bbb4", "1.0"))
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
            registry: "".to_string(),
        }),
    )]);

    // aaa --> bbb1@1.0 --> ccc@1.0  [allowed]
    //     \            \-> bbb2@1.1
    //     \-> bbb2@1.1
    //     \-> bbb3@1.0 --> ccc@2.0  [allowed]
    //     \            \-> bbb4@1.1
    //     \-> bbb4@1.1
    // ccc@1.5 and ccc@3.0, the versions violating the allowlist, are gone.
    let selection_result = Selection::run(&dep_graph, &overrides).unwrap();
    assert_in_order(
        &selection_result.resolved_dep_graph,
        vec![
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
            InterimModuleBuilder::create("bbb2", "1.1").build_entry(),
            InterimModuleBuilder::create("bbb3", "1.0")
                .add_dep("ccc", create_module_key("ccc", "2.0"))
                .add_dep("bbb4", create_module_key("bbb4", "1.1"))
                .build_entry(),
            InterimModuleBuilder::create("bbb4", "1.1").build_entry(),
            InterimModuleBuilder::create_cl("ccc", "1.0", 1).build_entry(),
            InterimModuleBuilder::create_cl("ccc", "2.0", 2).build_entry(),
        ],
    );

    assert_unordered(
        &selection_result.unpruned_dep_graph,
        vec![
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
        ],
    );
}
