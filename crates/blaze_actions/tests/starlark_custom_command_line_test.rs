//! Port of `src/test/java/com/google/devtools/build/lib/analysis/starlark/
//! StarlarkCustomCommandLineTest.java`.
//!
//! Java runs every test twice (`@TestParameter boolean useNestedSet`); each test here loops over
//! both values. Expected values are the Java test's.

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use blaze_actions::actions::action::{CommandLineError, InputMetadataProvider};
use blaze_actions::actions::action_key_context::ActionKeyContext;
use blaze_actions::actions::action_lookup::{
    ActionLookupData, ActionLookupKey, ActionLookupKeyRef,
};
use blaze_actions::actions::artifact::{Artifact, SpecialArtifactType};
use blaze_actions::actions::artifact_root::{ArtifactRoot, RootType};
use blaze_actions::actions::command_line::{ArgChunk, CommandLineRef, SimpleArgChunk};
use blaze_actions::actions::path_mapper::{NOOP, OutputPathsMode, PathMapper};
use blaze_actions::analysis::starlark::starlark_custom_command_line::{
    ArgValue, Builder, VectorArgBuilder,
};
use blaze_cmdline::{Label, RepositoryMapping};
use blaze_collect::command_line_item::MapFnError;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_starlark::eval::{EvalOptions, compile, exec_module};
use blaze_starlark::loader::LoadMap;
use blaze_starlark::thread::{CollectingEvents, Phase, ThreadContext};
use blaze_starlark::{FileKind, Location, StarlarkSemantics};
use blaze_util::Fingerprint;
use blaze_vfs::{DigestHashFunction, FileSystemExt, PathFragment, UnixFileSystem};
use starlark::values::OwnedFrozenValue;

// ---------------------------------------------------------------------------------------------
// Test fixtures

/// Java `ActionsTestUtil.NULL_ARTIFACT_OWNER`.
#[derive(Debug)]
struct NullArtifactOwner;

impl fmt::Display for NullArtifactOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NULL_ARTIFACT_OWNER")
    }
}

impl ActionLookupKey for NullArtifactOwner {
    fn get_label(&self) -> Option<Label> {
        None
    }

    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }

    fn function_name(&self) -> &'static str {
        "NULL_ARTIFACT_OWNER"
    }

    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().is::<NullArtifactOwner>()
    }

    fn key_hash(&self) -> u64 {
        0
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn null_owner() -> ActionLookupKeyRef {
    ActionLookupKeyRef::new(NullArtifactOwner)
}

/// Java `FakeActionInputFileCache` (only tree artifact metadata is needed here).
#[derive(Default)]
struct FakeActionInputFileCache {
    trees: HashMap<Artifact, Vec<Artifact>>,
}

impl FakeActionInputFileCache {
    fn put_tree_artifact(&mut self, tree: &Artifact, children: Vec<Artifact>) {
        self.trees.insert(tree.clone(), children);
    }
}

impl InputMetadataProvider for FakeActionInputFileCache {
    fn get_tree_children(&self, tree: &Artifact) -> Option<Vec<Artifact>> {
        self.trees.get(tree).cloned()
    }
}

/// Stands in for `PathMappers.create(action, OutputPathsMode.STRIP, true, null)` on the Java
/// test's action (all inputs and outputs under `bazel-out`): `StrippingPathMapper.map` replaces
/// the configuration segment of output paths with `cfg`.
// Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:169 StrippingPathMapper.map
struct StrippingPathMapper;

impl PathMapper for StrippingPathMapper {
    fn map(&self, exec_path: &PathFragment) -> PathFragment {
        if exec_path.segment_count() < 2 || exec_path.get_segment(0) != "bazel-out" {
            return exec_path.clone();
        }
        // Java: StrippingPathMapper.strip
        exec_path
            .sub_fragment(0, 1)
            .get_relative_str("cfg")
            .get_relative(&exec_path.sub_fragment_from(2))
    }
}

struct Fixture {
    derived_root: ArtifactRoot,
    artifact1: Artifact,
    artifact2: Artifact,
    artifact3: Artifact,
}

// Java: StarlarkCustomCommandLineTest.createArtifacts
fn create_artifacts() -> Fixture {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_root = fs.get_path("/execroot");
    let derived_root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["bin"]);
    let derived_root2 = ArtifactRoot::as_derived_root(
        &exec_root,
        RootType::OUTPUT,
        &["bazel-out", "k8-fastbuild", "bin"],
    );
    let derived_root3 = ArtifactRoot::as_derived_root(
        &exec_root,
        RootType::OUTPUT,
        &["bazel-out", "k8-opt", "bin"],
    );
    Fixture {
        artifact1: create_artifact(&derived_root2, "pkg/artifact1"),
        artifact2: create_artifact(&derived_root3, "pkg/artifact2"),
        artifact3: create_artifact(&derived_root3, "artifact3"),
        derived_root,
    }
}

/// Java `ActionsTestUtil.createArtifact(root, path)` for a derived root.
fn create_artifact(root: &ArtifactRoot, path: &str) -> Artifact {
    Artifact::create_derived_artifact(
        root.clone(),
        root.get_exec_path().get_relative_str(path),
        null_owner(),
    )
}

impl Fixture {
    // Java: StarlarkCustomCommandLineTest.createTreeArtifact
    fn create_tree_artifact(&self, relative_path: &str) -> Artifact {
        let tree = self.create_special_artifact(relative_path, SpecialArtifactType::TREE);
        tree.set_generating_action_key(ActionLookupData::create(null_owner(), 0));
        tree
    }

    // Java: StarlarkCustomCommandLineTest.createSpecialArtifact
    fn create_special_artifact(&self, relative_path: &str, type_: SpecialArtifactType) -> Artifact {
        Artifact::create_special_artifact(
            self.derived_root.clone(),
            self.derived_root
                .get_exec_path()
                .get_relative_str(relative_path),
            null_owner(),
            type_,
        )
    }
}

fn new_builder() -> Builder {
    Builder::new(Arc::new(StarlarkSemantics::default()))
}

fn empty_mapping() -> Option<RepositoryMapping> {
    Some((*blaze_cmdline::repository_mapping::EMPTY).clone())
}

fn s(x: &str) -> ArgValue {
    ArgValue::from(x)
}

fn a(x: &Artifact) -> ArgValue {
    ArgValue::from(x)
}

fn r(x: &Artifact) -> ArgValue {
    ArgValue::from(x.get_root())
}

// Java: StarlarkCustomCommandLineTest.vectorArg
fn vector_arg(use_nested_set: bool, elems: Vec<ArgValue>) -> VectorArgBuilder {
    if use_nested_set {
        let common_type = if elems.iter().all(|e| matches!(e, ArgValue::Artifact(_))) {
            "File"
        } else if elems.iter().all(|e| matches!(e, ArgValue::FileRoot(_))) {
            "root"
        } else if elems.iter().all(|e| matches!(e, ArgValue::String(_))) {
            "string"
        } else {
            panic!("Unsupported element types");
        };
        VectorArgBuilder::from_nested_set(
            NestedSetBuilder::wrap(Order::STABLE_ORDER, elems),
            common_type,
        )
        .set_location(Location::builtin())
    } else {
        VectorArgBuilder::from_list(elems).set_location(Location::builtin())
    }
}

// Java: StarlarkCustomCommandLineTest.verifyCommandLine
fn verify_command_line_with(
    path_mapper: &dyn PathMapper,
    command_line: &CommandLineRef,
    expected: &[&str],
) {
    let cache = FakeActionInputFileCache::default();
    let chunk = command_line.expand(Some(&cache), path_mapper).unwrap();
    let arguments = chunk.arguments(path_mapper);
    assert_eq!(arguments, expected);
    // Check consistency of the total argument length calculation with SimpleArgChunk, which
    // materializes strings and adds up their lengths.
    assert_eq!(
        chunk.total_arg_length(path_mapper),
        SimpleArgChunk::new(chunk.arguments(path_mapper)).total_arg_length(path_mapper)
    );
}

fn verify_command_line(command_line: &CommandLineRef, expected: &[&str]) {
    verify_command_line_with(&NOOP, command_line, expected);
}

// Java: StarlarkCustomCommandLineTest.verifyStrippedCommandLine
fn verify_stripped_command_line(command_line: &CommandLineRef, expected: &[&str]) {
    verify_command_line_with(&StrippingPathMapper, command_line, expected);
}

/// Java `StarlarkCustomCommandLineTest.execStarlark`: evaluates `code` and returns its global
/// `map_each`.
fn exec_starlark(code: &str) -> OwnedFrozenValue {
    let semantics = Arc::new(StarlarkSemantics::default());
    let globals = blaze_starlark::env::universe_builder(&semantics).build();
    let compiled = compile(FileKind::Bzl, "test/label.bzl", code.to_owned()).unwrap();
    let ctx = ThreadContext::new(
        Phase::BzlLoad,
        semantics,
        Arc::new(CollectingEvents::default()),
    );
    let module = exec_module(
        compiled,
        &globals,
        &LoadMap::new(),
        &ctx,
        &EvalOptions::default(),
    )
    .unwrap();
    module.get("map_each").unwrap()
}

fn expansion_message(e: CommandLineError) -> String {
    match e {
        MapFnError::CommandLineExpansion(e) => e.message,
        MapFnError::Interrupted(_) => panic!("unexpected InterruptedException"),
    }
}

const USE_NESTED_SET: [bool; 2] = [false, true];

// ---------------------------------------------------------------------------------------------
// Tests

#[test]
fn add() {
    for _use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder.add("one").add("two").add("three");
        let command_line = builder.build(/* flag_per_line= */ false, empty_mapping());
        verify_command_line(&command_line, &["one", "two", "three"]);
    }
}

#[test]
#[ignore = "needs serialization codecs (RoundTripping), which are not ported"]
fn serialization_with_formatted_arguments_works() {}

#[test]
fn add_path_mapped() {
    let f = create_artifacts();
    for _use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add(a(&f.artifact1))
            .add(a(&f.artifact2))
            .add(a(&f.artifact3))
            .add(r(&f.artifact1))
            .add(r(&f.artifact2))
            .add(r(&f.artifact3));
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(
            &command_line,
            &[
                "bazel-out/k8-fastbuild/bin/pkg/artifact1",
                "bazel-out/k8-opt/bin/pkg/artifact2",
                "bazel-out/k8-opt/bin/artifact3",
                "bazel-out/k8-fastbuild/bin",
                "bazel-out/k8-opt/bin",
                "bazel-out/k8-opt/bin",
            ],
        );
        verify_stripped_command_line(
            &command_line,
            &[
                "bazel-out/cfg/bin/pkg/artifact1",
                "bazel-out/cfg/bin/pkg/artifact2",
                "bazel-out/cfg/bin/artifact3",
                "bazel-out/k8-fastbuild/bin",
                "bazel-out/k8-opt/bin",
                "bazel-out/k8-opt/bin",
            ],
        );
    }
}

#[test]
fn add_formatted() {
    let f = create_artifacts();
    for _use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add_formatted("one", "--arg1=%s")
            .add_formatted("two", "--arg2=%s")
            .add_formatted("three", "--arg3=%s")
            .add_formatted(r(&f.artifact1), "--arg1_root=%s")
            .add_formatted(r(&f.artifact2), "--arg2_root=%s")
            .add_formatted(r(&f.artifact3), "--arg3_root=%s");
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(
            &command_line,
            &[
                "--arg1=one",
                "--arg2=two",
                "--arg3=three",
                "--arg1_root=bazel-out/k8-fastbuild/bin",
                "--arg2_root=bazel-out/k8-opt/bin",
                "--arg3_root=bazel-out/k8-opt/bin",
            ],
        );
    }
}

#[test]
fn add_formatted_path_mapped() {
    let f = create_artifacts();
    for _use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add_formatted(a(&f.artifact1), "--arg1=%s")
            .add_formatted(a(&f.artifact2), "--arg2=%s")
            .add_formatted(a(&f.artifact3), "--arg3=%s")
            .add_formatted(r(&f.artifact1), "--arg1_root=%s")
            .add_formatted(r(&f.artifact2), "--arg2_root=%s")
            .add_formatted(r(&f.artifact3), "--arg3_root=%s");
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(
            &command_line,
            &[
                "--arg1=bazel-out/k8-fastbuild/bin/pkg/artifact1",
                "--arg2=bazel-out/k8-opt/bin/pkg/artifact2",
                "--arg3=bazel-out/k8-opt/bin/artifact3",
                "--arg1_root=bazel-out/k8-fastbuild/bin",
                "--arg2_root=bazel-out/k8-opt/bin",
                "--arg3_root=bazel-out/k8-opt/bin",
            ],
        );
        verify_stripped_command_line(
            &command_line,
            &[
                "--arg1=bazel-out/cfg/bin/pkg/artifact1",
                "--arg2=bazel-out/cfg/bin/pkg/artifact2",
                "--arg3=bazel-out/cfg/bin/artifact3",
                "--arg1_root=bazel-out/k8-fastbuild/bin",
                "--arg2_root=bazel-out/k8-opt/bin",
                "--arg3_root=bazel-out/k8-opt/bin",
            ],
        );
    }
}

#[test]
fn arg_name() {
    for use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add_vector_arg(
                vector_arg(use_nested_set, vec![s("one"), s("two"), s("three")])
                    .set_arg_name("--arg"),
            )
            .add_vector_arg(
                vector_arg(use_nested_set, vec![s("four")]).set_arg_name("--other_arg"),
            );
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(
            &command_line,
            &["--arg", "one", "two", "three", "--other_arg", "four"],
        );
    }
}

#[test]
fn terminate_with() {
    for use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add_vector_arg(
                vector_arg(use_nested_set, vec![s("one"), s("two"), s("three")])
                    .set_terminate_with("end1"),
            )
            .add_vector_arg(vector_arg(use_nested_set, vec![s("four")]).set_terminate_with("end2"));
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(
            &command_line,
            &["one", "two", "three", "end1", "four", "end2"],
        );
    }
}

#[test]
fn format_each() {
    for use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add_vector_arg(
                vector_arg(use_nested_set, vec![s("one"), s("two"), s("three")])
                    .set_format_each("--arg=%s"),
            )
            .add_vector_arg(
                vector_arg(use_nested_set, vec![s("four")]).set_format_each("--other_arg=%s"),
            );
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(
            &command_line,
            &["--arg=one", "--arg=two", "--arg=three", "--other_arg=four"],
        );
    }
}

#[test]
fn format_each_path_mapped() {
    let f = create_artifacts();
    for use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add_vector_arg(
                vector_arg(
                    use_nested_set,
                    vec![a(&f.artifact1), a(&f.artifact2), a(&f.artifact3)],
                )
                .set_format_each("--arg=%s"),
            )
            .add_vector_arg(
                vector_arg(use_nested_set, vec![r(&f.artifact1), r(&f.artifact2)])
                    .set_format_each("--arg=%s"),
            );
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(
            &command_line,
            &[
                "--arg=bazel-out/k8-fastbuild/bin/pkg/artifact1",
                "--arg=bazel-out/k8-opt/bin/pkg/artifact2",
                "--arg=bazel-out/k8-opt/bin/artifact3",
                "--arg=bazel-out/k8-fastbuild/bin",
                "--arg=bazel-out/k8-opt/bin",
            ],
        );
        verify_stripped_command_line(
            &command_line,
            &[
                "--arg=bazel-out/cfg/bin/pkg/artifact1",
                "--arg=bazel-out/cfg/bin/pkg/artifact2",
                "--arg=bazel-out/cfg/bin/artifact3",
                "--arg=bazel-out/k8-fastbuild/bin",
                "--arg=bazel-out/k8-opt/bin",
            ],
        );
    }
}

#[test]
fn before_each() {
    for use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add_vector_arg(
                vector_arg(use_nested_set, vec![s("one"), s("two"), s("three")])
                    .set_before_each("b4"),
            )
            .add_vector_arg(vector_arg(use_nested_set, vec![s("four")]).set_before_each("and"));
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(
            &command_line,
            &["b4", "one", "b4", "two", "b4", "three", "and", "four"],
        );
    }
}

#[test]
fn before_each_path_mapped() {
    let f = create_artifacts();
    for use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add_vector_arg(
                vector_arg(
                    use_nested_set,
                    vec![a(&f.artifact1), a(&f.artifact2), a(&f.artifact3)],
                )
                .set_before_each("b4"),
            )
            .add_vector_arg(
                vector_arg(use_nested_set, vec![r(&f.artifact1), r(&f.artifact2)])
                    .set_before_each("b4"),
            )
            .add_vector_arg(
                vector_arg(use_nested_set, vec![a(&f.artifact3)]).set_before_each("and"),
            )
            .add_vector_arg(
                vector_arg(use_nested_set, vec![r(&f.artifact3)]).set_before_each("and"),
            );
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(
            &command_line,
            &[
                "b4",
                "bazel-out/k8-fastbuild/bin/pkg/artifact1",
                "b4",
                "bazel-out/k8-opt/bin/pkg/artifact2",
                "b4",
                "bazel-out/k8-opt/bin/artifact3",
                "b4",
                "bazel-out/k8-fastbuild/bin",
                "b4",
                "bazel-out/k8-opt/bin",
                "and",
                "bazel-out/k8-opt/bin/artifact3",
                "and",
                "bazel-out/k8-opt/bin",
            ],
        );
        verify_stripped_command_line(
            &command_line,
            &[
                "b4",
                "bazel-out/cfg/bin/pkg/artifact1",
                "b4",
                "bazel-out/cfg/bin/pkg/artifact2",
                "b4",
                "bazel-out/cfg/bin/artifact3",
                "b4",
                "bazel-out/k8-fastbuild/bin",
                "b4",
                "bazel-out/k8-opt/bin",
                "and",
                "bazel-out/cfg/bin/artifact3",
                "and",
                "bazel-out/k8-opt/bin",
            ],
        );
    }
}

#[test]
fn join_with() {
    for use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add_vector_arg(
                vector_arg(use_nested_set, vec![s("one"), s("two"), s("three")])
                    .set_join_with("..."),
            )
            .add_vector_arg(vector_arg(use_nested_set, vec![s("four")]).set_join_with("n/a"));
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(&command_line, &["one...two...three", "four"]);
    }
}

#[test]
fn join_with_path_mapped() {
    let f = create_artifacts();
    for use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add_vector_arg(
                vector_arg(
                    use_nested_set,
                    vec![a(&f.artifact1), a(&f.artifact2), a(&f.artifact3)],
                )
                .set_join_with("..."),
            )
            .add_vector_arg(
                vector_arg(use_nested_set, vec![r(&f.artifact1), r(&f.artifact2)])
                    .set_join_with("..."),
            )
            .add_vector_arg(vector_arg(use_nested_set, vec![a(&f.artifact3)]).set_join_with("..."))
            .add_vector_arg(vector_arg(use_nested_set, vec![r(&f.artifact3)]).set_join_with("..."));
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(
            &command_line,
            &[
                "bazel-out/k8-fastbuild/bin/pkg/artifact1...bazel-out/k8-opt/bin/pkg/artifact2...bazel-out/k8-opt/bin/artifact3",
                "bazel-out/k8-fastbuild/bin...bazel-out/k8-opt/bin",
                "bazel-out/k8-opt/bin/artifact3",
                "bazel-out/k8-opt/bin",
            ],
        );
        verify_stripped_command_line(
            &command_line,
            &[
                "bazel-out/cfg/bin/pkg/artifact1...bazel-out/cfg/bin/pkg/artifact2...bazel-out/cfg/bin/artifact3",
                "bazel-out/k8-fastbuild/bin...bazel-out/k8-opt/bin",
                "bazel-out/cfg/bin/artifact3",
                "bazel-out/k8-opt/bin",
            ],
        );
    }
}

#[test]
fn format_joined() {
    for use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add_vector_arg(
                vector_arg(use_nested_set, vec![s("one"), s("two"), s("three")])
                    .set_join_with("...")
                    .set_format_joined("--arg=%s"),
            )
            .add_vector_arg(
                vector_arg(use_nested_set, vec![s("four")])
                    .set_join_with("n/a")
                    .set_format_joined("--other_arg=%s"),
            );
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(
            &command_line,
            &["--arg=one...two...three", "--other_arg=four"],
        );
    }
}

#[test]
fn format_joined_path_mapped() {
    let f = create_artifacts();
    for use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add_vector_arg(
                vector_arg(
                    use_nested_set,
                    vec![a(&f.artifact1), a(&f.artifact2), a(&f.artifact3)],
                )
                .set_join_with("...")
                .set_format_joined("--arg=%s"),
            )
            .add_vector_arg(
                vector_arg(use_nested_set, vec![r(&f.artifact1), r(&f.artifact2)])
                    .set_join_with("...")
                    .set_format_joined("--arg=%s"),
            )
            .add_vector_arg(
                vector_arg(use_nested_set, vec![a(&f.artifact3)])
                    .set_join_with("...")
                    .set_format_joined("--other_arg=%s"),
            )
            .add_vector_arg(
                vector_arg(use_nested_set, vec![r(&f.artifact3)])
                    .set_join_with("...")
                    .set_format_joined("--other_arg=%s"),
            );
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(
            &command_line,
            &[
                "--arg=bazel-out/k8-fastbuild/bin/pkg/artifact1...bazel-out/k8-opt/bin/pkg/artifact2...bazel-out/k8-opt/bin/artifact3",
                "--arg=bazel-out/k8-fastbuild/bin...bazel-out/k8-opt/bin",
                "--other_arg=bazel-out/k8-opt/bin/artifact3",
                "--other_arg=bazel-out/k8-opt/bin",
            ],
        );
        verify_stripped_command_line(
            &command_line,
            &[
                "--arg=bazel-out/cfg/bin/pkg/artifact1...bazel-out/cfg/bin/pkg/artifact2...bazel-out/cfg/bin/artifact3",
                "--arg=bazel-out/k8-fastbuild/bin...bazel-out/k8-opt/bin",
                "--other_arg=bazel-out/cfg/bin/artifact3",
                "--other_arg=bazel-out/k8-opt/bin",
            ],
        );
    }
}

#[test]
fn empty_vector_arg_omit() {
    for use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add("before")
            .add_vector_arg(
                vector_arg(use_nested_set, vec![])
                    .omit_if_empty(true)
                    .set_join_with(",")
                    .set_format_joined("--empty=%s"),
            )
            .add("after");
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(&command_line, &["before", "after"]);
    }
}

#[test]
fn empty_vector_arg_no_omit() {
    for use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .add("before")
            .add_vector_arg(
                vector_arg(use_nested_set, vec![])
                    .omit_if_empty(false)
                    .set_join_with(",")
                    .set_format_joined("--empty=%s"),
            )
            .add("after");
        let command_line = builder.build(false, empty_mapping());
        verify_command_line(&command_line, &["before", "--empty=", "after"]);
    }
}

#[test]
fn uniquify_path_mapped() {
    let f = create_artifacts();
    // NestedSet doesn't support mixed types.
    let use_nested_set = false;
    let mut builder = new_builder();
    builder.add_vector_arg(
        vector_arg(
            use_nested_set,
            vec![
                a(&f.artifact1),
                a(&f.artifact1),
                a(&f.artifact2),
                a(&f.artifact3),
                s(f.artifact1.get_exec_path_string()),
                s(f.artifact1.get_exec_path_string()),
                s(f.artifact2.get_exec_path_string()),
                s(f.artifact3.get_exec_path_string()),
            ],
        )
        .uniquify(true),
    );
    let command_line = builder.build(false, empty_mapping());
    verify_command_line(
        &command_line,
        &[
            "bazel-out/k8-fastbuild/bin/pkg/artifact1",
            "bazel-out/k8-opt/bin/pkg/artifact2",
            "bazel-out/k8-opt/bin/artifact3",
        ],
    );
    verify_stripped_command_line(
        &command_line,
        &[
            "bazel-out/cfg/bin/pkg/artifact1",
            "bazel-out/cfg/bin/pkg/artifact2",
            "bazel-out/cfg/bin/artifact3",
            "bazel-out/k8-fastbuild/bin/pkg/artifact1",
            "bazel-out/k8-opt/bin/pkg/artifact2",
            "bazel-out/k8-opt/bin/artifact3",
        ],
    );
}

#[test]
fn flag_per_line() {
    for use_nested_set in USE_NESTED_SET {
        let mut builder = new_builder();
        builder
            .record_arg_start()
            .add_vector_arg(
                vector_arg(use_nested_set, vec![s("is"), s("line"), s("one")])
                    .set_arg_name("--this"),
            )
            .record_arg_start()
            .add_vector_arg(
                vector_arg(
                    use_nested_set,
                    vec![s("this"), s("is"), s("line"), s("two")],
                )
                .set_arg_name("--and"),
            )
            .record_arg_start()
            .add("--line_three")
            .add("single_arg")
            .record_arg_start()
            .add_vector_arg(
                vector_arg(
                    use_nested_set,
                    vec![s(""), s("line"), s("four"), s("has"), s("no")],
                )
                .set_terminate_with("flag"),
            );
        let command_line = builder.build(/* flag_per_line= */ true, empty_mapping());
        verify_command_line(
            &command_line,
            &[
                "--this=is line one",
                "--and=this is line two",
                "--line_three=single_arg",
                "line four has no flag",
            ],
        );
    }
}

#[test]
fn flag_per_line_path_mapped() {
    let f = create_artifacts();
    // NestedSet doesn't support mixed types.
    let use_nested_set = false;
    let mut builder = new_builder();
    builder
        .record_arg_start()
        .add_vector_arg(
            vector_arg(
                use_nested_set,
                vec![a(&f.artifact1), a(&f.artifact2), a(&f.artifact3)],
            )
            .set_arg_name("--this"),
        )
        .record_arg_start()
        .add_vector_arg(
            vector_arg(
                use_nested_set,
                vec![a(&f.artifact3), a(&f.artifact2), a(&f.artifact1)],
            )
            .set_arg_name("--and"),
        )
        .record_arg_start()
        .add("--line_three")
        .add("single_arg")
        .record_arg_start()
        .add_vector_arg(
            vector_arg(
                use_nested_set,
                vec![s(""), a(&f.artifact1), a(&f.artifact2), a(&f.artifact3)],
            )
            .set_terminate_with("flag"),
        );
    let command_line = builder.build(true, empty_mapping());
    verify_command_line(
        &command_line,
        &[
            "--this=bazel-out/k8-fastbuild/bin/pkg/artifact1 bazel-out/k8-opt/bin/pkg/artifact2 bazel-out/k8-opt/bin/artifact3",
            "--and=bazel-out/k8-opt/bin/artifact3 bazel-out/k8-opt/bin/pkg/artifact2 bazel-out/k8-fastbuild/bin/pkg/artifact1",
            "--line_three=single_arg",
            "bazel-out/k8-fastbuild/bin/pkg/artifact1 bazel-out/k8-opt/bin/pkg/artifact2 bazel-out/k8-opt/bin/artifact3 flag",
        ],
    );
    verify_stripped_command_line(
        &command_line,
        &[
            "--this=bazel-out/cfg/bin/pkg/artifact1 bazel-out/cfg/bin/pkg/artifact2 bazel-out/cfg/bin/artifact3",
            "--and=bazel-out/cfg/bin/artifact3 bazel-out/cfg/bin/pkg/artifact2 bazel-out/cfg/bin/pkg/artifact1",
            "--line_three=single_arg",
            "bazel-out/cfg/bin/pkg/artifact1 bazel-out/cfg/bin/pkg/artifact2 bazel-out/cfg/bin/artifact3 flag",
        ],
    );
}

#[test]
fn vector_arg_tree_artifact_missing_expansion_fails() {
    let f = create_artifacts();
    for use_nested_set in USE_NESTED_SET {
        let tree = f.create_tree_artifact("tree");
        let mut builder = new_builder();
        builder.add_vector_arg(
            vector_arg(use_nested_set, vec![a(&tree)]).set_expand_directories(true),
        );
        let command_line = builder.build(false, empty_mapping());

        let e = command_line
            .arguments_with(Some(&FakeActionInputFileCache::default()), &NOOP)
            .unwrap_err();
        let message = expansion_message(e);
        assert!(
            message.contains("Failed to expand directory <generated file tree>"),
            "{message}"
        );
    }
}

#[test]
#[ignore = "needs fileset expansion (InputMetadataProvider.getFileset, FilesetOutputTree), not ported"]
fn vector_arg_add_to_fingerprint_expand_fileset_includes_in_digest() {}

#[test]
fn vector_arg_add_to_fingerprint_expand_tree_artifact_includes_in_digest() {
    let f = create_artifacts();
    for use_nested_set in USE_NESTED_SET {
        let tree = f.create_tree_artifact("tree");
        let child = Artifact::create_tree_output_str(&tree, "child");

        let mut builder = new_builder();
        builder.add_vector_arg(
            vector_arg(use_nested_set, vec![a(&tree)]).set_expand_directories(true),
        );
        let command_line = builder.build(false, empty_mapping());

        let action_key_context = ActionKeyContext::new();
        let mut cache = FakeActionInputFileCache::default();
        cache.put_tree_artifact(&tree, vec![child]);

        let mut fingerprint = Fingerprint::new();
        command_line
            .add_to_fingerprint(
                &action_key_context,
                Some(&cache),
                OutputPathsMode::OFF,
                &mut fingerprint,
            )
            .unwrap();
        assert!(!fingerprint.digest_and_reset().is_empty());
    }
}

#[test]
#[ignore = "needs fileset expansion (InputMetadataProvider.getFileset, FilesetOutputTree), not ported"]
fn vector_arg_expand_fileset_missing_expansion_fails() {}

#[test]
fn vector_arg_arguments_expands_tree_artifact() {
    let f = create_artifacts();
    for use_nested_set in USE_NESTED_SET {
        let tree = f.create_tree_artifact("tree");
        let child1 = Artifact::create_tree_output_str(&tree, "child1");
        let child2 = Artifact::create_tree_output_str(&tree, "child2");

        let mut cache = FakeActionInputFileCache::default();
        cache.put_tree_artifact(&tree, vec![child1, child2]);

        let mut builder = new_builder();
        builder.add_vector_arg(
            vector_arg(use_nested_set, vec![a(&tree)]).set_expand_directories(true),
        );
        let command_line = builder.build(false, empty_mapping());
        let arguments = command_line.arguments_with(Some(&cache), &NOOP).unwrap();
        assert_eq!(arguments, vec!["bin/tree/child1", "bin/tree/child2"]);
    }
}

#[test]
#[ignore = "needs fileset expansion (InputMetadataProvider.getFileset, FilesetOutputTree), not ported"]
fn vector_arg_arguments_expands_fileset() {}

#[test]
fn vector_arg_arguments_tree_artifact_missing_expansion_fails() {
    let f = create_artifacts();
    for use_nested_set in USE_NESTED_SET {
        let tree = f.create_tree_artifact("tree");
        let mut builder = new_builder();
        builder.add_vector_arg(
            vector_arg(use_nested_set, vec![a(&tree)]).set_expand_directories(true),
        );
        let command_line = builder.build(false, empty_mapping());

        let result = command_line.arguments_with(Some(&FakeActionInputFileCache::default()), &NOOP);
        assert!(matches!(result, Err(MapFnError::CommandLineExpansion(_))));
    }
}

#[test]
fn vector_arg_arguments_manually_expanded_tree_artifact_missing_expansion_fails() {
    let f = create_artifacts();
    let map_each = exec_starlark("def map_each(x, expander):\n  expander.expand(x)\n");
    for use_nested_set in USE_NESTED_SET {
        let tree = f.create_tree_artifact("tree");
        let mut builder = new_builder();
        builder.add_vector_arg(
            vector_arg(use_nested_set, vec![a(&tree)])
                .set_expand_directories(false)
                .set_location(Location::builtin())
                .set_map_each(map_each.clone()),
        );
        let command_line = builder.build(false, empty_mapping());

        let e = command_line
            .arguments_with(Some(&FakeActionInputFileCache::default()), &NOOP)
            .unwrap_err();
        let message = expansion_message(e);
        assert!(
            message.contains("Failed to expand directory <generated file tree>"),
            "{message}"
        );
    }
}

#[test]
#[ignore = "needs fileset expansion (InputMetadataProvider.getFileset, FilesetOutputTree), not ported"]
fn vector_arg_arguments_fileset_missing_expansion_fails() {}

fn fingerprint_of(command_line: &CommandLineRef, cache: &FakeActionInputFileCache) -> String {
    let mut fingerprint = Fingerprint::new();
    command_line
        .add_to_fingerprint(
            &ActionKeyContext::new(),
            Some(cache),
            OutputPathsMode::OFF,
            &mut fingerprint,
        )
        .unwrap();
    fingerprint.hex_digest_and_reset()
}

#[test]
fn vector_arg_arguments_expand_directories_disabled_manual_expansion_reflected_in_action_key() {
    let f = create_artifacts();
    let map_each = exec_starlark(
        "def map_each(x, expander):\n  return [f.path for f in expander.expand(x)]\n",
    );
    for use_nested_set in USE_NESTED_SET {
        let tree = f.create_tree_artifact("tree");
        let child1 = Artifact::create_tree_output_str(&tree, "child1");
        let child2 = Artifact::create_tree_output_str(&tree, "child2");
        let mut builder = new_builder();
        builder.add_vector_arg(
            vector_arg(use_nested_set, vec![a(&tree)])
                .set_expand_directories(false)
                .set_location(Location::builtin())
                .set_map_each(map_each.clone()),
        );
        let command_line = builder.build(false, empty_mapping());

        let mut before = FakeActionInputFileCache::default();
        before.put_tree_artifact(&tree, vec![child1.clone(), child2]);
        let arguments_before = command_line.arguments_with(Some(&before), &NOOP).unwrap();
        let fingerprint_before = fingerprint_of(&command_line, &before);
        assert_eq!(arguments_before, vec!["bin/tree/child1", "bin/tree/child2"]);

        let mut after = FakeActionInputFileCache::default();
        after.put_tree_artifact(&tree, vec![child1]);
        let arguments_after = command_line.arguments_with(Some(&after), &NOOP).unwrap();
        let fingerprint_after = fingerprint_of(&command_line, &after);
        assert_eq!(arguments_after, vec!["bin/tree/child1"]);

        assert_ne!(fingerprint_before, fingerprint_after);
    }
}

#[test]
fn vector_arg_arguments_expand_directories_disabled_no_map_each_expansion_does_not_affect_action_key()
 {
    let f = create_artifacts();
    for use_nested_set in USE_NESTED_SET {
        let tree = f.create_tree_artifact("tree");
        let child1 = Artifact::create_tree_output_str(&tree, "child1");
        let child2 = Artifact::create_tree_output_str(&tree, "child2");
        let mut builder = new_builder();
        builder.add_vector_arg(
            vector_arg(use_nested_set, vec![a(&tree)])
                .set_expand_directories(false)
                .set_location(Location::builtin()),
        );
        let command_line = builder.build(false, empty_mapping());

        let mut before = FakeActionInputFileCache::default();
        before.put_tree_artifact(&tree, vec![child1.clone(), child2]);
        let arguments_before = command_line.arguments_with(Some(&before), &NOOP).unwrap();
        let fingerprint_before = fingerprint_of(&command_line, &before);
        assert_eq!(arguments_before, vec!["bin/tree"]);

        let mut after = FakeActionInputFileCache::default();
        after.put_tree_artifact(&tree, vec![child1]);
        let arguments_after = command_line.arguments_with(Some(&after), &NOOP).unwrap();
        let fingerprint_after = fingerprint_of(&command_line, &after);
        assert_eq!(arguments_after, vec!["bin/tree"]);

        assert_eq!(fingerprint_before, fingerprint_after);
    }
}

#[test]
fn vector_arg_arguments_expand_directories_disabled_no_manual_expansion_expansion_does_not_affect_action_key()
 {
    let f = create_artifacts();
    let map_each = exec_starlark("def map_each(x):\n  return x.path\n");
    for use_nested_set in USE_NESTED_SET {
        let tree = f.create_tree_artifact("tree");
        let child1 = Artifact::create_tree_output_str(&tree, "child1");
        let child2 = Artifact::create_tree_output_str(&tree, "child2");
        let mut builder = new_builder();
        builder.add_vector_arg(
            vector_arg(use_nested_set, vec![a(&tree)])
                .set_expand_directories(false)
                .set_location(Location::builtin())
                .set_map_each(map_each.clone()),
        );
        let command_line = builder.build(false, empty_mapping());

        let mut before = FakeActionInputFileCache::default();
        before.put_tree_artifact(&tree, vec![child1.clone(), child2]);
        let arguments_before = command_line.arguments_with(Some(&before), &NOOP).unwrap();
        let fingerprint_before = fingerprint_of(&command_line, &before);
        assert_eq!(arguments_before, vec!["bin/tree"]);

        let mut after = FakeActionInputFileCache::default();
        after.put_tree_artifact(&tree, vec![child1]);
        let arguments_after = command_line.arguments_with(Some(&after), &NOOP).unwrap();
        let fingerprint_after = fingerprint_of(&command_line, &after);
        assert_eq!(arguments_after, vec!["bin/tree"]);

        assert_eq!(fingerprint_before, fingerprint_after);
    }
}

// Not a Java test: the fingerprint of a plain list differs from that of a nested set with the
// same elements (VectorArg.addToFingerprint adds the nested set order).
#[test]
fn list_and_nested_set_fingerprints_differ() {
    let mut list_builder = new_builder();
    list_builder.add_vector_arg(vector_arg(false, vec![s("a"), s("b")]));
    let list_command_line = list_builder.build(false, None);
    let mut set_builder = new_builder();
    set_builder.add_vector_arg(vector_arg(true, vec![s("a"), s("b")]));
    let set_command_line = set_builder.build(false, None);
    assert_eq!(list_command_line.arguments().unwrap(), vec!["a", "b"]);
    assert_eq!(set_command_line.arguments().unwrap(), vec!["a", "b"]);
    let cache = FakeActionInputFileCache::default();
    assert_ne!(
        fingerprint_of(&list_command_line, &cache),
        fingerprint_of(&set_command_line, &cache)
    );
}
