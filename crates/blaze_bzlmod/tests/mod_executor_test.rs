//! Port of `src/test/java/com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutorTest.java`.

mod modcommand_common;

use std::borrow::Cow;

use blaze_bzlmod::inspector::{AugmentedModule, ResolutionReason};
use blaze_bzlmod::modcommand::{
    Explanation, IsExpanded, IsIndirect, MaybeCompleteSet, ModExecutor, ResultNode, get_formatter,
};
use blaze_bzlmod_model::{
    AttributeValue, AttributeValues, ModuleExtensionId, ModuleExtensionUsage,
    ModuleExtensionUsageProxy, ModuleKey, Table, Tag, Version,
};
use blaze_cmdline::label_constants::MODULE_DOT_BAZEL_FILE_NAME;
use blaze_cmdline::{Label, PackageIdentifier};
use blaze_events::Location;
use blaze_optdefs::misc::mod_options::{ExtensionShow, ModOptions, OutputFormat};
use indexmap::{IndexMap, IndexSet};
use modcommand_common::*;

fn keys(keys: &[ModuleKey]) -> IndexSet<ModuleKey> {
    keys.iter().cloned().collect()
}

fn entries(result: IndexMap<ModuleKey, ResultNode>) -> Vec<(ModuleKey, ResultNode)> {
    result.into_iter().collect()
}

// Tests for the ModExecutor::expandAndPrune core function.
//
// (* In the ASCII graph hints "__>" or "-->" mean a direct edge, while "..>" means an indirect
// edge. "aaa ..." means module "aaa" is unexpanded.)

#[test]
fn test_expand_from_targets_first() {
    // aaa -> bbb -> ccc -> ddd
    let dep_graph = map_of(vec![
        build_augmented_module_full(
            ModuleKey::root(),
            "aaa",
            Version::parse("1.0").unwrap(),
            true,
        )
        .add_dep("bbb", "1.0")
        .build_entry(),
        build_augmented_module("bbb", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .add_dep("ccc", "1.0")
            .build_entry(),
        build_augmented_module("ccc", "1.0")
            .add_still_dependant("bbb", "1.0")
            .add_dep("ddd", "1.0")
            .build_entry(),
        build_augmented_module("ddd", "1.0")
            .add_still_dependant("ccc", "1.0")
            .build_entry(),
    ]);

    let options = ModOptions::get_default_options();
    let mut out: Vec<u8> = Vec::new();
    let mut executor = ModExecutor::new(Cow::Borrowed(&dep_graph), options, &mut out);

    // RESULT:
    // <root> ...> ccc -> ddd
    //       \___> bbb -> ccc ...
    assert_eq!(
        entries(
            executor.expand_and_prune(&keys(&[ModuleKey::root(), create_module_key("ccc", "1.0")]))
        ),
        vec![
            (
                ModuleKey::root(),
                ResultNode::builder()
                    .add_child(
                        create_module_key("bbb", "1.0"),
                        IsExpanded::TRUE,
                        IsIndirect::FALSE
                    )
                    .add_child(
                        create_module_key("ccc", "1.0"),
                        IsExpanded::TRUE,
                        IsIndirect::TRUE
                    )
                    .build()
            ),
            (
                create_module_key("bbb", "1.0"),
                ResultNode::builder()
                    .add_child(
                        create_module_key("ccc", "1.0"),
                        IsExpanded::FALSE,
                        IsIndirect::FALSE
                    )
                    .build()
            ),
            (
                create_module_key("ccc", "1.0"),
                ResultNode::builder()
                    .add_child(
                        create_module_key("ddd", "1.0"),
                        IsExpanded::TRUE,
                        IsIndirect::FALSE
                    )
                    .build()
            ),
            (
                create_module_key("ddd", "1.0"),
                ResultNode::builder().build()
            ),
        ]
    );
}

#[test]
fn test_paths_depth1_contains_all_targets_with_nested_indirect() {
    // <root> -> bbb -> ccc -> ddd -> eee -> fff -> ggg -> hhh
    //                          ^     /
    //                           \___/
    let dep_graph = map_of(vec![
        build_augmented_module_full(
            ModuleKey::root(),
            "aaa",
            Version::parse("1.0").unwrap(),
            true,
        )
        .add_dep("bbb", "1.0")
        .build_entry(),
        build_augmented_module("bbb", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .add_dep("ccc", "1.0")
            .build_entry(),
        build_augmented_module("ccc", "1.0")
            .add_still_dependant("bbb", "1.0")
            .add_dep("ddd", "1.0")
            .build_entry(),
        build_augmented_module("ddd", "1.0")
            .add_still_dependant("ccc", "1.0")
            .add_still_dependant("eee", "1.0")
            .add_dep("eee", "1.0")
            .build_entry(),
        build_augmented_module("eee", "1.0")
            .add_still_dependant("ddd", "1.0")
            .add_dep("fff", "1.0")
            .add_dep("ddd", "1.0")
            .build_entry(),
        build_augmented_module("fff", "1.0")
            .add_still_dependant("eee", "1.0")
            .add_dep("ggg", "1.0")
            .build_entry(),
        build_augmented_module("ggg", "1.0")
            .add_still_dependant("fff", "1.0")
            .add_dep("hhh", "1.0")
            .build_entry(),
        build_augmented_module("hhh", "1.0")
            .add_still_dependant("ggg", "1.0")
            .build_entry(),
    ]);

    let mut options = ModOptions::get_default_options();
    options.cycles = true;
    options.depth = 1;
    let mut out: Vec<u8> = Vec::new();
    let mut executor = ModExecutor::new(Cow::Borrowed(&dep_graph), options, &mut out);
    let targets = keys(&[
        create_module_key("eee", "1.0"),
        create_module_key("hhh", "1.0"),
    ]);

    // RESULT:
    // <root> --> bbb ..> ddd --> eee --> ddd (cycle)
    //                               \..> ggg --> hhh
    assert_eq!(
        entries(executor.expand_paths_to_targets(&keys(&[ModuleKey::root()]), &targets, false)),
        vec![
            (
                ModuleKey::root(),
                ResultNode::builder()
                    .add_child(
                        create_module_key("bbb", "1.0"),
                        IsExpanded::TRUE,
                        IsIndirect::FALSE
                    )
                    .build()
            ),
            (
                create_module_key("bbb", "1.0"),
                ResultNode::builder()
                    .add_child(
                        create_module_key("ddd", "1.0"),
                        IsExpanded::TRUE,
                        IsIndirect::TRUE
                    )
                    .build()
            ),
            (
                create_module_key("ddd", "1.0"),
                ResultNode::builder()
                    .add_child(
                        create_module_key("eee", "1.0"),
                        IsExpanded::TRUE,
                        IsIndirect::FALSE
                    )
                    .build()
            ),
            (
                create_module_key("eee", "1.0"),
                ResultNode::builder()
                    .set_target(true)
                    .add_child(
                        create_module_key("ggg", "1.0"),
                        IsExpanded::TRUE,
                        IsIndirect::TRUE
                    )
                    .build()
            ),
            (
                create_module_key("ggg", "1.0"),
                ResultNode::builder()
                    .add_child(
                        create_module_key("hhh", "1.0"),
                        IsExpanded::TRUE,
                        IsIndirect::FALSE
                    )
                    .build()
            ),
            (
                create_module_key("hhh", "1.0"),
                ResultNode::builder().set_target(true).build()
            ),
        ]
    );
}

#[test]
fn test_paths_depth1_target_parent_is_direct_and_indirect_child() {
    // <root> --> bbb --> ccc
    //             \       |________
    //              \      V       |
    //               \__> ddd --> eee
    let dep_graph = map_of(vec![
        build_augmented_module_full(
            ModuleKey::root(),
            "aaa",
            Version::parse("1.0").unwrap(),
            true,
        )
        .add_dep("bbb", "1.0")
        .build_entry(),
        build_augmented_module("bbb", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .add_dep("ccc", "1.0")
            .add_dep("ddd", "1.0")
            .build_entry(),
        build_augmented_module("ccc", "1.0")
            .add_still_dependant("bbb", "1.0")
            .add_dep("ddd", "1.0")
            .build_entry(),
        build_augmented_module("ddd", "1.0")
            .add_still_dependant("bbb", "1.0")
            .add_still_dependant("ccc", "1.0")
            .add_still_dependant("eee", "1.0")
            .add_dep("eee", "1.0")
            .build_entry(),
        build_augmented_module("eee", "1.0")
            .add_still_dependant("ddd", "1.0")
            .add_still_dependant("eee", "1.0")
            .add_dep("ddd", "1.0")
            .build_entry(),
    ]);

    let mut options = ModOptions::get_default_options();
    options.cycles = true;
    options.depth = 1;
    let mut out: Vec<u8> = Vec::new();
    let mut executor = ModExecutor::new(Cow::Borrowed(&dep_graph), options, &mut out);
    let targets = keys(&[create_module_key("eee", "1.0")]);

    // RESULT:
    // <root> --> bbb --- ddd --> eee --> ddd (c)
    //             \
    //              \..> ddd ...
    assert_eq!(
        entries(executor.expand_paths_to_targets(&keys(&[ModuleKey::root()]), &targets, false)),
        vec![
            (
                ModuleKey::root(),
                ResultNode::builder()
                    .add_child(
                        create_module_key("bbb", "1.0"),
                        IsExpanded::TRUE,
                        IsIndirect::FALSE
                    )
                    .build()
            ),
            (
                create_module_key("bbb", "1.0"),
                ResultNode::builder()
                    .add_child(
                        create_module_key("ddd", "1.0"),
                        IsExpanded::TRUE,
                        IsIndirect::FALSE
                    )
                    .build()
            ),
            (
                create_module_key("ddd", "1.0"),
                ResultNode::builder()
                    .add_child(
                        create_module_key("eee", "1.0"),
                        IsExpanded::TRUE,
                        IsIndirect::FALSE
                    )
                    .build()
            ),
            (
                create_module_key("eee", "1.0"),
                ResultNode::builder().set_target(true).build()
            ),
        ]
    );
}

// Tests for the ModExecutor OutputFormatters

#[test]
fn test_resolution_explanation_most_cases() {
    let dep_graph = map_of(vec![
        build_augmented_module_full(ModuleKey::root(), "A", Version::parse("1.0").unwrap(), true)
            .add_dep("B", "1.0")
            .add_dep("C", "1.0")
            .build_entry(),
        build_augmented_module("B", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .add_changed_dep(
                "C",
                "1.0",
                "0.1",
                ResolutionReason::MINIMAL_VERSION_SELECTION,
            )
            .add_changed_dep("E", "", "1.0", ResolutionReason::NON_REGISTRY_OVERRIDE)
            .build_entry(),
        build_augmented_module("C", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .add_dependant("B", "1.0")
            .add_changed_dep("D", "1.5", "1.0", ResolutionReason::SINGLE_VERSION_OVERRIDE)
            .build_entry(),
        build_augmented_module("C", "0.1")
            .add_original_dependant("B", "1.0")
            .build_entry(),
        build_augmented_module("D", "1.0")
            .add_original_dependant("C", "1.0")
            .build_entry(),
        build_augmented_module("D", "1.5")
            .add_dependant("C", "1.0")
            .build_entry(),
        build_augmented_module("E", "1.0")
            .add_original_dependant("B", "1.0")
            .build_entry(),
        build_augmented_module("E", "")
            .add_dependant("B", "1.0")
            .build_entry(),
    ]);

    let mut options = ModOptions::get_default_options();
    options.verbose = true;
    options.include_unused = true;

    let formatter = get_formatter(OutputFormat::TEXT);
    assert_eq!(
        formatter.get_extra_resolution_explanation(&ModuleKey::root(), None, &dep_graph, &options),
        None
    );
    assert_eq!(
        formatter.get_extra_resolution_explanation(
            &create_module_key("B", "1.0"),
            Some(&ModuleKey::root()),
            &dep_graph,
            &options
        ),
        None
    );

    assert_eq!(
        formatter.get_extra_resolution_explanation(
            &create_module_key("C", "1.0"),
            Some(&create_module_key("B", "1.0")),
            &dep_graph,
            &options
        ),
        Some(Explanation::create(
            Version::parse("0.1").unwrap(),
            ResolutionReason::MINIMAL_VERSION_SELECTION,
            Some(keys(&[ModuleKey::root()]))
        ))
    );

    assert_eq!(
        formatter.get_extra_resolution_explanation(
            &create_module_key("C", "0.1"),
            Some(&create_module_key("B", "1.0")),
            &dep_graph,
            &options
        ),
        Some(Explanation::create(
            Version::parse("1.0").unwrap(),
            ResolutionReason::MINIMAL_VERSION_SELECTION,
            Some(keys(&[ModuleKey::root()]))
        ))
    );

    assert_eq!(
        formatter.get_extra_resolution_explanation(
            &create_module_key("D", "1.0"),
            Some(&create_module_key("C", "1.0")),
            &dep_graph,
            &options
        ),
        Some(Explanation::create(
            Version::parse("1.5").unwrap(),
            ResolutionReason::SINGLE_VERSION_OVERRIDE,
            None
        ))
    );

    assert_eq!(
        formatter.get_extra_resolution_explanation(
            &create_module_key("D", "1.5"),
            Some(&create_module_key("C", "1.0")),
            &dep_graph,
            &options
        ),
        Some(Explanation::create(
            Version::parse("1.0").unwrap(),
            ResolutionReason::SINGLE_VERSION_OVERRIDE,
            None
        ))
    );

    assert_eq!(
        formatter.get_extra_resolution_explanation(
            &create_module_key("E", "1.0"),
            Some(&create_module_key("B", "1.0")),
            &dep_graph,
            &options
        ),
        Some(Explanation::create(
            Version::empty(),
            ResolutionReason::NON_REGISTRY_OVERRIDE,
            None
        ))
    );

    assert_eq!(
        formatter.get_extra_resolution_explanation(
            &create_module_key("E", ""),
            Some(&create_module_key("B", "1.0")),
            &dep_graph,
            &options
        ),
        Some(Explanation::create(
            Version::parse("1.0").unwrap(),
            ResolutionReason::NON_REGISTRY_OVERRIDE,
            None
        ))
    );
}

/// The dep graph shared by the two output tests below.
fn indirect_and_nested_dep_graph() -> IndexMap<ModuleKey, AugmentedModule> {
    map_of(vec![
        build_augmented_module_full(ModuleKey::root(), "A", Version::parse("1.0").unwrap(), true)
            .add_dep("B", "1.0")
            .build_entry(),
        build_augmented_module("B", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .add_changed_dep("C", "1.0", "0.1", ResolutionReason::SINGLE_VERSION_OVERRIDE)
            .add_changed_dep(
                "Y",
                "2.0",
                "1.0",
                ResolutionReason::MINIMAL_VERSION_SELECTION,
            )
            .build_entry(),
        build_augmented_module("C", "0.1")
            .add_original_dependant("B", "1.0")
            .build_entry(),
        build_augmented_module("C", "1.0")
            .add_dependant("B", "1.0")
            .add_dep("D", "1.0")
            .build_entry(),
        build_augmented_module("D", "1.0")
            .add_still_dependant("C", "1.0")
            .add_still_dependant("E", "1.0")
            .add_dep("E", "1.0")
            .build_entry(),
        build_augmented_module("E", "1.0")
            .add_still_dependant("D", "1.0")
            .add_dep("F", "1.0")
            .add_dep("D", "1.0")
            .build_entry(),
        build_augmented_module("F", "1.0")
            .add_still_dependant("E", "1.0")
            .add_dep("G", "1.0")
            .build_entry(),
        build_augmented_module("G", "1.0")
            .add_still_dependant("F", "1.0")
            .add_dep("H", "1.0")
            .add_dep("Y", "2.0")
            .build_entry(),
        build_augmented_module("H", "1.0")
            .add_still_dependant("G", "1.0")
            .build_entry(),
        build_augmented_module("Y", "1.0")
            .add_original_dependant("B", "1.0")
            .build_entry(),
        build_augmented_module("Y", "2.0")
            .add_dependant("B", "1.0")
            .add_still_dependant("G", "1.0")
            .build_entry(),
    ])
}

#[test]
fn test_text_and_graph_output_indirect_and_nested_target_paths_with_unused() {
    let dep_graph = indirect_and_nested_dep_graph();

    let result: IndexMap<ModuleKey, ResultNode> = IndexMap::from([
        (
            ModuleKey::root(),
            ResultNode::builder()
                .add_child(
                    create_module_key("B", "1.0"),
                    IsExpanded::TRUE,
                    IsIndirect::FALSE,
                )
                .build(),
        ),
        (
            create_module_key("B", "1.0"),
            ResultNode::builder()
                .add_child(
                    create_module_key("Y", "1.0"),
                    IsExpanded::TRUE,
                    IsIndirect::FALSE,
                )
                .add_child(
                    create_module_key("Y", "2.0"),
                    IsExpanded::TRUE,
                    IsIndirect::FALSE,
                )
                .add_child(
                    create_module_key("C", "1.0"),
                    IsExpanded::TRUE,
                    IsIndirect::FALSE,
                )
                .add_child(
                    create_module_key("C", "0.1"),
                    IsExpanded::TRUE,
                    IsIndirect::FALSE,
                )
                .build(),
        ),
        (
            create_module_key("C", "0.1"),
            ResultNode::builder().set_target(true).build(),
        ),
        (
            create_module_key("C", "1.0"),
            ResultNode::builder()
                .set_target(true)
                .add_child(
                    create_module_key("D", "1.0"),
                    IsExpanded::TRUE,
                    IsIndirect::FALSE,
                )
                .build(),
        ),
        (
            create_module_key("D", "1.0"),
            ResultNode::builder()
                .add_child(
                    create_module_key("E", "1.0"),
                    IsExpanded::TRUE,
                    IsIndirect::FALSE,
                )
                .build(),
        ),
        (
            create_module_key("E", "1.0"),
            ResultNode::builder()
                .set_target(true)
                .add_child(
                    create_module_key("G", "1.0"),
                    IsExpanded::TRUE,
                    IsIndirect::TRUE,
                )
                .build(),
        ),
        (
            create_module_key("G", "1.0"),
            ResultNode::builder()
                .add_child(
                    create_module_key("H", "1.0"),
                    IsExpanded::TRUE,
                    IsIndirect::FALSE,
                )
                .add_child(
                    create_module_key("Y", "2.0"),
                    IsExpanded::TRUE,
                    IsIndirect::FALSE,
                )
                .build(),
        ),
        (
            create_module_key("H", "1.0"),
            ResultNode::builder().set_target(true).build(),
        ),
        (
            create_module_key("Y", "1.0"),
            ResultNode::builder().set_target(true).build(),
        ),
        (
            create_module_key("Y", "2.0"),
            ResultNode::builder().set_target(true).build(),
        ),
    ]);

    let mut options = ModOptions::get_default_options();
    options.cycles = true;
    options.include_unused = true;
    options.verbose = true;
    options.depth = 1;
    options.output_format = OutputFormat::TEXT;

    let targets = keys(&[
        create_module_key("C", "0.1"),
        create_module_key("C", "1.0"),
        create_module_key("Y", "1.0"),
        create_module_key("Y", "2.0"),
        create_module_key("E", "1.0"),
        create_module_key("H", "1.0"),
    ]);

    let mut out: Vec<u8> = Vec::new();
    {
        let mut executor = ModExecutor::new(Cow::Borrowed(&dep_graph), options.clone(), &mut out);

        // Double check for human error
        assert_eq!(
            executor.expand_paths_to_targets(&keys(&[ModuleKey::root()]), &targets, false),
            result
        );

        executor.all_paths(&keys(&[ModuleKey::root()]), &targets);
    }

    assert_eq!(
        read_all_lines(&out),
        vec![
            "<root> (A@1.0)",
            "└───B@1.0 ",
            "    ├───C@0.1 # (to 1.0, cause single_version_override)",
            "    ├───C@1.0 # (was 0.1, cause single_version_override)",
            "    │   └───D@1.0 ",
            "    │       └───E@1.0 # ",
            "    │           └╌╌╌G@1.0 ",
            "    │               ├───H@1.0 # ",
            "    │               └───Y@2.0 # ",
            "    ├───Y@1.0 # (to 2.0, cause G@1.0)",
            "    └───Y@2.0 # (was 1.0, cause G@1.0)",
            "",
        ]
    );

    options.output_format = OutputFormat::GRAPH;
    let mut out_graph: Vec<u8> = Vec::new();
    {
        let mut executor = ModExecutor::new(Cow::Borrowed(&dep_graph), options, &mut out_graph);
        executor.all_paths(&keys(&[ModuleKey::root()]), &targets);
    }

    assert_eq!(
        read_all_lines(&out_graph),
        vec![
            "digraph mygraph {",
            "  node [ shape=box ]",
            "  edge [ fontsize=8 ]",
            "  \"<root>\" [ label=\"<root> (A@1.0)\" ]",
            "  \"<root>\" -> \"B@1.0\" [  ]",
            "  \"B@1.0\" -> \"C@0.1\" [ label=SVO ]",
            "  \"B@1.0\" -> \"C@1.0\" [ label=SVO ]",
            "  \"B@1.0\" -> \"Y@1.0\" [ label=MVS ]",
            "  \"B@1.0\" -> \"Y@2.0\" [ label=MVS ]",
            "  \"C@0.1\" [ shape=diamond style=dotted ]",
            "  \"C@1.0\" [ shape=diamond style=solid ]",
            "  \"C@1.0\" -> \"D@1.0\" [  ]",
            "  \"Y@1.0\" [ shape=diamond style=dotted ]",
            "  \"Y@2.0\" [ shape=diamond style=solid ]",
            "  \"D@1.0\" -> \"E@1.0\" [  ]",
            "  \"E@1.0\" [ shape=diamond style=solid ]",
            "  \"E@1.0\" -> \"G@1.0\" [ style=dashed ]",
            "  \"G@1.0\" -> \"H@1.0\" [  ]",
            "  \"G@1.0\" -> \"Y@2.0\" [  ]",
            "  \"H@1.0\" [ shape=diamond style=solid ]",
            "}",
        ]
    );
}

// Java: ModExecutorTest.createExtensionId(String targetName, String extensionName)
fn create_extension_id(target_name: &str, extension_name: &str) -> ModuleExtensionId {
    ModuleExtensionId::create(
        Label::create_in_package(
            &PackageIdentifier::create_in_main_repo_str(target_name),
            target_name,
        )
        .unwrap(),
        extension_name,
        None,
    )
}

// Java: ModExecutorTest.createExtensionId(String moduleName, String version, String path, String extensionName)
fn create_extension_id_in_module(
    module_name: &str,
    version: &str,
    path: &str,
    extension_name: &str,
) -> ModuleExtensionId {
    ModuleExtensionId::create(
        Label::parse_canonical(&format!("@@{module_name}+{version}//{path}:{path}")).unwrap(),
        extension_name,
        None,
    )
}

/// `ModuleExtensionUsage.builder()` with one proxy.
fn usage(
    extension_name: &str,
    location: Location,
    imports: &[(&str, &str)],
    tags: Vec<Tag>,
) -> ModuleExtensionUsage {
    ModuleExtensionUsage {
        extension_bzl_file: "//extensions:extensions.bzl".to_string(),
        extension_name: extension_name.to_string(),
        isolation_key: None,
        proxies: vec![ModuleExtensionUsageProxy {
            location,
            proxy_name: String::new(),
            containing_module_file_path: MODULE_DOT_BAZEL_FILE_NAME.clone(),
            dev_dependency: false,
            imports: imports
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }],
        tags,
        repo_overrides: IndexMap::new(),
    }
}

/// `BzlmodTestUtil.buildTag(name).addAttr(...).build()`.
fn tag(tag_name: &str, attrs: Vec<(&str, AttributeValue)>) -> Tag {
    Tag {
        tag_name: tag_name.to_string(),
        attribute_values: AttributeValues::create(attrs),
        dev_dependency: false,
        location: Location::builtin().clone(),
    }
}

fn repos(names: &[&str]) -> IndexSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn test_extensions_info_text_and_graph() {
    let dep_graph = indirect_and_nested_dep_graph();

    let maven_id = create_extension_id("extensions", "maven");
    let gradle_id = create_extension_id("extensions", "gradle");
    let mut extension_usages: Table<ModuleExtensionId, ModuleKey, ModuleExtensionUsage> =
        Table::new();
    extension_usages.put(
        maven_id.clone(),
        create_module_key("C", "1.0"),
        usage(
            "maven",
            Location::from_file_line_column("C@1.0/MODULE.bazel", 2, 23),
            &[("repo1", "repo1"), ("repo3", "repo3")],
            vec![],
        ),
    );
    extension_usages.put(
        maven_id.clone(),
        create_module_key("D", "1.0"),
        usage(
            "maven",
            Location::from_file_line_column("D@1.0/MODULE.bazel", 1, 10),
            &[("repo1", "repo1"), ("repo2", "repo2")],
            vec![],
        ),
    );
    extension_usages.put(
        gradle_id.clone(),
        create_module_key("Y", "2.0"),
        usage(
            "gradle",
            Location::from_file_line_column("Y@2.0/MODULE.bazel", 2, 13),
            &[("repo2", "repo2")],
            vec![],
        ),
    );
    extension_usages.put(
        maven_id.clone(),
        create_module_key("Y", "2.0"),
        usage(
            "maven",
            Location::from_file_line_column("Y@2.0/MODULE.bazel", 13, 10),
            &[("myrepo", "repo5")],
            vec![
                tag("dep", vec![("coord", AttributeValue::string("junit"))]),
                tag("dep", vec![("coord", AttributeValue::string("guava"))]),
                tag(
                    "pom",
                    vec![(
                        "pom_xmls",
                        AttributeValue::string_list(["//:pom.xml", "@bar//:pom.xml"]),
                    )],
                ),
            ],
        ),
    );

    // Contains the already-filtered map of target extensions along with their full list of repos
    let extension_repos: IndexMap<ModuleExtensionId, IndexSet<String>> = IndexMap::from([
        (
            maven_id.clone(),
            repos(&["repo6", "repo1", "repo2", "repo3", "repo4", "repo5"]),
        ),
        (gradle_id.clone(), repos(&["repo1", "repo2"])),
    ]);

    let mut options = ModOptions::get_default_options();
    options.output_format = OutputFormat::TEXT;
    options.extension_info = ExtensionShow::ALL;

    let mut out: Vec<u8> = Vec::new();
    {
        let mut executor = ModExecutor::with_extensions(
            Cow::Borrowed(&dep_graph),
            Cow::Borrowed(&extension_usages),
            Cow::Borrowed(&extension_repos),
            None,
            options.clone(),
            &mut out,
        );
        executor.graph(&keys(&[ModuleKey::root()]));
    }

    assert_eq!(
        read_all_lines(&out),
        vec![
            "<root> (A@1.0)",
            "└───B@1.0 ",
            "    ├───C@1.0 ",
            "    │   ├───$@@//extensions:extensions%maven ",
            "    │   │   ├───repo1",
            "    │   │   ├───repo3",
            "    │   │   ├╌╌╌repo4",
            "    │   │   └╌╌╌repo6",
            "    │   └───D@1.0 ",
            "    │       ├───$@@//extensions:extensions%maven ... ",
            "    │       │   ├───repo1",
            "    │       │   └───repo2",
            "    │       └───E@1.0 ",
            "    │           └───F@1.0 ",
            "    │               └───G@1.0 ",
            "    │                   ├───Y@2.0 (*) ",
            "    │                   └───H@1.0 ",
            "    └───Y@2.0 ",
            "        ├───$@@//extensions:extensions%gradle ",
            "        │   ├───repo2",
            "        │   └╌╌╌repo1",
            "        └───$@@//extensions:extensions%maven ... ",
            "            └───repo5",
            "",
        ]
    );

    options.output_format = OutputFormat::GRAPH;
    let mut out_graph: Vec<u8> = Vec::new();
    {
        let mut executor = ModExecutor::with_extensions(
            Cow::Borrowed(&dep_graph),
            Cow::Borrowed(&extension_usages),
            Cow::Borrowed(&extension_repos),
            None,
            options.clone(),
            &mut out_graph,
        );
        executor.graph(&keys(&[ModuleKey::root()]));
    }

    assert_eq!(
        read_all_lines(&out_graph),
        vec![
            "digraph mygraph {",
            "  node [ shape=box ]",
            "  edge [ fontsize=8 ]",
            "  \"<root>\" [ label=\"<root> (A@1.0)\" ]",
            "  \"<root>\" -> \"B@1.0\" [  ]",
            "  \"B@1.0\" -> \"C@1.0\" [  ]",
            "  \"B@1.0\" -> \"Y@2.0\" [  ]",
            "  subgraph \"cluster_@@//extensions:extensions%maven\" {",
            "    label=\"@@//extensions:extensions%maven\"",
            "    \"@@//extensions:extensions%maven%repo1\" [ label=\"repo1\" ]",
            "    \"@@//extensions:extensions%maven%repo2\" [ label=\"repo2\" ]",
            "    \"@@//extensions:extensions%maven%repo3\" [ label=\"repo3\" ]",
            "    \"@@//extensions:extensions%maven%repo5\" [ label=\"repo5\" ]",
            "    \"@@//extensions:extensions%maven%repo4\" [ label=\"repo4\" style=dotted ]",
            "    \"@@//extensions:extensions%maven%repo6\" [ label=\"repo6\" style=dotted ]",
            "  }",
            "  \"C@1.0\" -> \"@@//extensions:extensions%maven%repo1\"",
            "  \"C@1.0\" -> \"@@//extensions:extensions%maven%repo3\"",
            "  \"C@1.0\" -> \"D@1.0\" [  ]",
            "  subgraph \"cluster_@@//extensions:extensions%gradle\" {",
            "    label=\"@@//extensions:extensions%gradle\"",
            "    \"@@//extensions:extensions%gradle%repo2\" [ label=\"repo2\" ]",
            "    \"@@//extensions:extensions%gradle%repo1\" [ label=\"repo1\" style=dotted ]",
            "  }",
            "  \"Y@2.0\" -> \"@@//extensions:extensions%gradle%repo2\"",
            "  \"Y@2.0\" -> \"@@//extensions:extensions%maven%repo5\"",
            "  \"D@1.0\" -> \"@@//extensions:extensions%maven%repo1\"",
            "  \"D@1.0\" -> \"@@//extensions:extensions%maven%repo2\"",
            "  \"D@1.0\" -> \"E@1.0\" [  ]",
            "  \"E@1.0\" -> \"F@1.0\" [  ]",
            "  \"F@1.0\" -> \"G@1.0\" [  ]",
            "  \"G@1.0\" -> \"H@1.0\" [  ]",
            "  \"G@1.0\" -> \"Y@2.0\" [  ]",
            "}",
        ]
    );

    options.output_format = OutputFormat::TEXT;
    options.depth = 1;
    let mut out_text2: Vec<u8> = Vec::new();
    {
        let mut executor = ModExecutor::with_extensions(
            Cow::Borrowed(&dep_graph),
            Cow::Borrowed(&extension_usages),
            Cow::Borrowed(&extension_repos),
            Some(MaybeCompleteSet::copy_of([&maven_id])),
            options,
            &mut out_text2,
        );
        executor.all_paths(
            &keys(&[ModuleKey::root()]),
            &keys(&[create_module_key("Y", "2.0")]),
        );
    }

    assert_eq!(
        read_all_lines(&out_text2),
        vec![
            "<root> (A@1.0)",
            "└───B@1.0 ",
            "    ├───C@1.0 # ",
            "    │   ├───$@@//extensions:extensions%maven ",
            "    │   │   ├───repo1",
            "    │   │   ├───repo3",
            "    │   │   ├╌╌╌repo4",
            "    │   │   └╌╌╌repo6",
            "    │   └───D@1.0 # ",
            "    │       ├───$@@//extensions:extensions%maven ... ",
            "    │       │   ├───repo1",
            "    │       │   └───repo2",
            "    │       └╌╌╌G@1.0 ",
            "    │           └───Y@2.0 # ",
            "    │               └───$@@//extensions:extensions%maven ... ",
            "    │                   └───repo5",
            "    └───Y@2.0 # ",
            "        └───$@@//extensions:extensions%maven ... ",
            "            └───repo5",
            "",
        ]
    );
}

/// The dep graph of `testModCommandPath_complexGraphFiltersCorrectly`.
///
/// ```text
/// <root> -> A -> B -> C -> D
///   |       |         ^
///   |       `-> E -> F/
///   |
///   `-> G -> H
///   |
///   `-> I -> D
/// ```
fn complex_dep_graph() -> IndexMap<ModuleKey, AugmentedModule> {
    map_of(vec![
        build_augmented_module_full(
            ModuleKey::root(),
            "main",
            Version::parse("1.0").unwrap(),
            true,
        )
        .add_dep("A", "1.0")
        .add_dep("G", "1.0")
        .add_dep("I", "1.0")
        .build_entry(),
        build_augmented_module("A", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .add_dep("B", "1.0")
            .add_dep("E", "1.0")
            .build_entry(),
        build_augmented_module("B", "1.0")
            .add_still_dependant("A", "1.0")
            .add_dep("C", "1.0")
            .build_entry(),
        build_augmented_module("C", "1.0")
            .add_still_dependant("B", "1.0")
            .add_still_dependant("F", "1.0")
            .add_dep("D", "1.0")
            .build_entry(),
        build_augmented_module("D", "1.0")
            .add_still_dependant("C", "1.0")
            .add_still_dependant("I", "1.0")
            .build_entry(),
        build_augmented_module("E", "1.0")
            .add_still_dependant("A", "1.0")
            .add_dep("F", "1.0")
            .build_entry(),
        build_augmented_module("F", "1.0")
            .add_still_dependant("E", "1.0")
            .add_dep("C", "1.0")
            .build_entry(),
        build_augmented_module("G", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .add_dep("H", "1.0")
            .build_entry(),
        build_augmented_module("H", "1.0")
            .add_still_dependant("G", "1.0")
            .build_entry(),
        build_augmented_module("I", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .add_dep("D", "1.0")
            .build_entry(),
    ])
}

#[test]
fn test_mod_command_path_complex_graph_filters_correctly() {
    let dep_graph = complex_dep_graph();

    let mut options = ModOptions::get_default_options();
    options.output_format = OutputFormat::TEXT;

    let run = |from: &[ModuleKey], to: &[ModuleKey], all: bool| -> Vec<String> {
        let mut out: Vec<u8> = Vec::new();
        {
            let mut executor =
                ModExecutor::new(Cow::Borrowed(&dep_graph), options.clone(), &mut out);
            if all {
                executor.all_paths(&keys(from), &keys(to));
            } else {
                executor.path(&keys(from), &keys(to));
            }
        }
        read_all_lines(&out)
    };

    // Test `executor.allPaths`, it should output all "interesting" paths to the target modules.
    assert_eq!(
        run(&[ModuleKey::root()], &[create_module_key("D", "1.0")], true),
        vec![
            "<root> (main@1.0)",
            "├───A@1.0 ",
            "│   └───B@1.0 ",
            "│       └───C@1.0 ",
            "│           └───D@1.0 # ",
            "└───I@1.0 ",
            "    └───D@1.0 # ",
            "",
        ]
    );

    // Also test `executor.path`, it should output a single path to the target module and it
    // should be the shortest one
    assert_eq!(
        run(
            &[ModuleKey::root()],
            &[create_module_key("D", "1.0")],
            false
        ),
        vec!["<root> (main@1.0)", "└───I@1.0 ", "    └───D@1.0 # ", ""]
    );

    // Test multiple targets D and G for allPaths
    assert_eq!(
        run(
            &[ModuleKey::root()],
            &[create_module_key("D", "1.0"), create_module_key("G", "1.0")],
            true
        ),
        vec![
            "<root> (main@1.0)",
            "├───A@1.0 ",
            "│   └───B@1.0 ",
            "│       └───C@1.0 ",
            "│           └───D@1.0 # ",
            "├───G@1.0 # ",
            "└───I@1.0 ",
            "    └───D@1.0 # ",
            "",
        ]
    );

    // Test multiple targets D and G for path (shortest path)
    assert_eq!(
        run(
            &[ModuleKey::root()],
            &[create_module_key("D", "1.0"), create_module_key("G", "1.0")],
            false
        ),
        vec![
            "<root> (main@1.0)",
            "├───G@1.0 # ",
            "└───I@1.0 ",
            "    └───D@1.0 # ",
            "",
        ]
    );

    // Test starting from E to D for allPaths
    assert_eq!(
        run(
            &[create_module_key("E", "1.0")],
            &[create_module_key("D", "1.0")],
            true
        ),
        vec![
            "<root> (main@1.0)",
            "└╌╌╌E@1.0 ",
            "    └───F@1.0 ",
            "        └───C@1.0 ",
            "            └───D@1.0 # ",
            "",
        ]
    );
}

#[test]
fn test_mod_command_graph_with_cycle() {
    // <root> -> A -> B -> A (cycle)
    let dep_graph = map_of(vec![
        build_augmented_module_full(
            ModuleKey::root(),
            "main",
            Version::parse("1.0").unwrap(),
            true,
        )
        .add_dep("A", "1.0")
        .build_entry(),
        build_augmented_module("A", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .add_still_dependant("B", "1.0")
            .add_dep("B", "1.0")
            .build_entry(),
        build_augmented_module("B", "1.0")
            .add_still_dependant("A", "1.0")
            .add_dep("A", "1.0")
            .build_entry(),
    ]);

    let mut options = ModOptions::get_default_options();
    options.output_format = OutputFormat::TEXT;
    options.cycles = true;

    let mut out: Vec<u8> = Vec::new();
    {
        let mut executor = ModExecutor::new(Cow::Borrowed(&dep_graph), options, &mut out);
        executor.graph(&keys(&[ModuleKey::root()]));
    }

    assert_eq!(
        read_all_lines(&out),
        vec![
            "<root> (main@1.0)",
            "└───A@1.0 ",
            "    └───B@1.0 ",
            "        └───A@1.0 (cycle) ",
            "",
        ]
    );
}

#[test]
fn test_graph_with_extension_filter_on_root() {
    let dep_graph = map_of(vec![
        build_augmented_module_full(
            ModuleKey::root(),
            "main",
            Version::parse("1.0").unwrap(),
            true,
        )
        .build_entry(),
    ]);

    let maven_id = create_extension_id("extensions", "maven");
    let mut extension_usages: Table<ModuleExtensionId, ModuleKey, ModuleExtensionUsage> =
        Table::new();
    extension_usages.put(
        maven_id.clone(),
        ModuleKey::root(),
        usage(
            "maven",
            Location::from_file_line_column("MODULE.bazel", 1, 1),
            &[("repo1", "repo1")],
            vec![],
        ),
    );

    let extension_repos: IndexMap<ModuleExtensionId, IndexSet<String>> =
        IndexMap::from([(maven_id.clone(), repos(&["repo1"]))]);

    let mut options = ModOptions::get_default_options();
    options.output_format = OutputFormat::TEXT;
    options.extension_info = ExtensionShow::ALL;

    let mut out: Vec<u8> = Vec::new();
    {
        let mut executor = ModExecutor::with_extensions(
            Cow::Borrowed(&dep_graph),
            Cow::Borrowed(&extension_usages),
            Cow::Borrowed(&extension_repos),
            Some(MaybeCompleteSet::copy_of([&maven_id])),
            options,
            &mut out,
        );
        // This should not throw NPE
        executor.graph(&keys(&[ModuleKey::root()]));
    }

    assert_eq!(
        read_all_lines(&out),
        vec![
            "<root> (main@1.0)",
            "└───$@@//extensions:extensions%maven ",
            "    └───repo1",
            "",
        ]
    );
}

#[test]
fn test_graph_with_extension_filter_and_cycle() {
    // <root> -> A -> B -> A (cycle)
    // A and B both use extension defined in A.
    let dep_graph = map_of(vec![
        build_augmented_module_full(
            ModuleKey::root(),
            "main",
            Version::parse("1.0").unwrap(),
            true,
        )
        .add_dep("A", "1.0")
        .add_dep("B", "1.0")
        .build_entry(),
        build_augmented_module("A", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .add_still_dependant("B", "1.0")
            .add_dep("B", "1.0")
            .build_entry(),
        build_augmented_module("B", "1.0")
            .add_still_dependant_key(ModuleKey::root())
            .add_still_dependant("A", "1.0")
            .add_dep("A", "1.0")
            .build_entry(),
    ]);

    let extension_id = create_extension_id_in_module("A", "1.0", "extensions", "ext");
    let mut extension_usages: Table<ModuleExtensionId, ModuleKey, ModuleExtensionUsage> =
        Table::new();
    extension_usages.put(
        extension_id.clone(),
        create_module_key("A", "1.0"),
        usage(
            "ext",
            Location::from_file_line_column("MODULE.bazel", 1, 1),
            &[("repo1", "repo1")],
            vec![],
        ),
    );
    extension_usages.put(
        extension_id.clone(),
        create_module_key("B", "1.0"),
        usage(
            "ext",
            Location::from_file_line_column("MODULE.bazel", 1, 1),
            &[("repo2", "repo2")],
            vec![],
        ),
    );

    let extension_repos: IndexMap<ModuleExtensionId, IndexSet<String>> =
        IndexMap::from([(extension_id.clone(), repos(&["repo1", "repo2"]))]);

    let mut options = ModOptions::get_default_options();
    options.output_format = OutputFormat::TEXT;
    options.extension_info = ExtensionShow::ALL;
    options.cycles = true;

    let mut out: Vec<u8> = Vec::new();
    {
        let mut executor = ModExecutor::with_extensions(
            Cow::Borrowed(&dep_graph),
            Cow::Borrowed(&extension_usages),
            Cow::Borrowed(&extension_repos),
            Some(MaybeCompleteSet::copy_of([&extension_id])),
            options,
            &mut out,
        );
        executor.graph(&keys(&[ModuleKey::root()]));
    }

    assert_eq!(
        read_all_lines(&out),
        vec![
            "<root> (main@1.0)",
            "├───A@1.0 # ",
            "│   ├───$@@A+1.0//extensions:extensions%ext ",
            "│   │   └───repo1",
            "│   ├───B@1.0 (cycle) ",
            "│   └───B@1.0 # ",
            "│       ├───$@@A+1.0//extensions:extensions%ext ... ",
            "│       │   └───repo2",
            "│       ├───A@1.0 (cycle) ",
            "│       └───A@1.0 (cycle) ",
            "└───B@1.0 # ",
            "    ├───$@@A+1.0//extensions:extensions%ext ... ",
            "    │   └───repo2",
            "    ├───A@1.0 (cycle) ",
            "    └───A@1.0 # ",
            "        ├───$@@A+1.0//extensions:extensions%ext ... ",
            "        │   └───repo1",
            "        ├───B@1.0 (cycle) ",
            "        └───B@1.0 (cycle) ",
            "",
        ]
    );
}

/// Not in the Java test class. `mod show_extension` prints tag values (internal strings) through
/// the UTF-8 `PrintWriter`, so `é` comes out double encoded. Expected bytes: the oracle's
/// `bazel mod show_extension bytes_b1//:ext.bzl%ext` in the workspace of
/// `tests/data/bytes-B1` (its MODULE.bazel), saved as `tests/data/bytes-B1/show_extension.stdout`
/// with the workspace path replaced by `/ws`.
#[test]
fn test_show_extension_non_ascii_tags() {
    let internal = |t: &str| blaze_util::string_encoding::latin1_decode(t.as_bytes()).into_owned();
    let e = internal("é");
    let pig = internal("🐷");
    let dep_graph = map_of(vec![
        build_augmented_module_full(
            ModuleKey::root(),
            "bytes_b1",
            Version::parse("1.0").unwrap(),
            true,
        )
        .build_entry(),
    ]);
    let ext_id =
        ModuleExtensionId::create(Label::parse_canonical("@@//:ext.bzl").unwrap(), "ext", None);
    let mut extension_usages: Table<ModuleExtensionId, ModuleKey, ModuleExtensionUsage> =
        Table::new();
    extension_usages.put(
        ext_id.clone(),
        ModuleKey::root(),
        usage(
            "ext",
            Location::from_file_line_column("/ws/MODULE.bazel", 3, 20),
            &[("ext_repo", "ext_repo")],
            vec![tag(
                "tag",
                vec![
                    ("s", AttributeValue::string(format!("{e}{pig}"))),
                    (
                        "l",
                        AttributeValue::string_list([
                            e.clone(),
                            pig.clone(),
                            format!("@@not_a_label_{e}"),
                        ]),
                    ),
                    ("d", AttributeValue::string_dict([(e.clone(), pig.clone())])),
                    ("x", AttributeValue::string("\u{ff}")),
                ],
            )],
        ),
    );
    let extension_repos: IndexMap<ModuleExtensionId, IndexSet<String>> =
        IndexMap::from([(ext_id.clone(), repos(&["ext_repo"]))]);
    let mut out: Vec<u8> = Vec::new();
    {
        let mut executor = ModExecutor::with_extensions(
            Cow::Borrowed(&dep_graph),
            Cow::Borrowed(&extension_usages),
            Cow::Borrowed(&extension_repos),
            None,
            ModOptions::get_default_options(),
            &mut out,
        );
        executor
            .show_extension(std::slice::from_ref(&ext_id), &IndexSet::new())
            .unwrap();
    }
    let expected = std::fs::read(format!(
        "{}/tests/data/bytes-B1/show_extension.stdout",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(&expected)
    );
    assert_eq!(out, expected);
}
