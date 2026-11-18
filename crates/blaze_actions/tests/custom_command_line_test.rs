//! Port of `src/test/java/com/google/devtools/build/lib/actions/CustomCommandLineTest.java`.

#![allow(clippy::disallowed_names)] // Java test values (`foo`, `bar`) kept verbatim.

use std::collections::HashMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use blaze_actions::actions::action_lookup::{ActionLookupData, ActionLookupKey};
use blaze_actions::actions::command_line::CommandLine;
use blaze_actions::actions::path_mapper::OutputPathsMode;
use blaze_actions::analysis::actions::custom_command_line::{
    CommandLineArg, CustomCommandLine, OnDemandString, SimpleVectorArg, VectorArg,
    VectorArgBuilder, VectorArgElement, vector_arg,
};
use blaze_actions::{
    ActionKeyContext, ActionLookupKeyRef, Artifact, ArtifactRoot, RootType, SpecialArtifactType,
};
use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{Label, RepositoryMapping, RepositoryName};
use blaze_collect::command_line_item::{MapFn, MapFnError};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_util::Fingerprint;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};

/// A test `ActionLookupKey` standing in for `ActionsTestUtil.NULL_ARTIFACT_OWNER`'s key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TestKey;

impl fmt::Display for TestKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TestKey")
    }
}

impl ActionLookupKey for TestKey {
    fn get_label(&self) -> Option<Label> {
        None
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "CONFIGURED_TARGET"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().downcast_ref::<TestKey>().is_some()
    }
    fn key_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct Fixture {
    root_dir: ArtifactRoot,
    artifact1: Artifact,
    artifact2: Artifact,
}

// Java: CustomCommandLineTest.createArtifacts
fn fixture() -> Fixture {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_root = Path::create(PathFragment::create("/exec/root"), &fs);
    let root_dir = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["dir"]);
    let artifact1 = Artifact::create_derived_artifact(
        root_dir.clone(),
        PathFragment::create("dir/file1.txt"),
        ActionLookupKeyRef::new(TestKey),
    );
    let artifact2 = Artifact::create_derived_artifact(
        root_dir.clone(),
        PathFragment::create("dir/file2.txt"),
        ActionLookupKeyRef::new(TestKey),
    );
    Fixture {
        root_dir,
        artifact1,
        artifact2,
    }
}

impl Fixture {
    // Java: ActionsTestUtil.createTreeArtifactWithGeneratingAction
    fn create_tree_artifact(&self, root_relative_path: &str) -> Artifact {
        let owner = ActionLookupKeyRef::new(TestKey);
        let tree = Artifact::create_special_artifact(
            self.root_dir.clone(),
            self.root_dir
                .get_exec_path()
                .get_relative_str(root_relative_path),
            owner.clone(),
            SpecialArtifactType::TREE,
        );
        tree.set_generating_action_key(ActionLookupData::create(owner, 0));
        tree
    }
}

fn args(cl: &CustomCommandLine) -> Vec<String> {
    cl.arguments().unwrap()
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn nested_set<T: Clone + Eq + Hash>(objects: Vec<T>) -> NestedSet<T> {
    NestedSetBuilder::create(Order::STABLE_ORDER, objects)
}

fn label(s: &str) -> Label {
    Label::parse_canonical(s).unwrap()
}

fn path(s: &str) -> PathFragment {
    PathFragment::create(s)
}

#[derive(Debug)]
struct LazyFoo;

impl fmt::Display for LazyFoo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("foo")
    }
}

fn lazy_foo() -> Arc<dyn OnDemandString> {
    Arc::new(LazyFoo)
}

#[test]
fn add_scalar_adds_single_argument() {
    let f = fixture();
    assert_eq!(
        args(&CustomCommandLine::builder().add("--arg").build()),
        strs(&["--arg"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_dynamic_string("--arg")
                .build()
        ),
        strs(&["--arg"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_label(label("//a:b"))
                .build()
        ),
        strs(&["//a:b"])
    );
    assert_eq!(
        args(&CustomCommandLine::builder().add_path(path("path")).build()),
        strs(&["path"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_exec_path(f.artifact1.clone())
                .build()
        ),
        strs(&["dir/file1.txt"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_lazy_string(lazy_foo())
                .build()
        ),
        strs(&["foo"])
    );
}

#[test]
fn add_scalar_with_constant_arg_adds_string_prepended_by_arg() {
    let f = fixture();
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_with_arg("--arg", Some("val".to_string()))
                .build()
        ),
        strs(&["--arg", "val"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_label_with_arg("--arg", Some(label("//a:b")))
                .build()
        ),
        strs(&["--arg", "//a:b"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_path_with_arg("--arg", Some(path("path")))
                .build()
        ),
        strs(&["--arg", "path"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_exec_path_with_arg("--arg", Some(f.artifact1.clone()))
                .build()
        ),
        strs(&["--arg", "dir/file1.txt"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_lazy_string_with_arg("--arg", Some(lazy_foo()))
                .build()
        ),
        strs(&["--arg", "foo"])
    );
}

#[test]
fn add_formatted_adds_correctly_formatted_argument() {
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_formatted("%s%s", vec!["hello".into(), "world".into()])
                .build()
        ),
        strs(&["helloworld"])
    );
}

#[test]
fn add_prefixed_adds_prefix_for_arguments() {
    let f = fixture();
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_prefixed("prefix-", "foo")
                .build()
        ),
        strs(&["prefix-foo"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_prefixed_label(
                    "prefix-",
                    label("//a:b"),
                    /* main_repo_mapping= */ None
                )
                .build()
        ),
        strs(&["prefix-//a:b"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_prefixed_path("prefix-", path("path"))
                .build()
        ),
        strs(&["prefix-path"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_prefixed_exec_path("prefix-", f.artifact1.clone())
                .build()
        ),
        strs(&["prefix-dir/file1.txt"])
    );
}

#[test]
fn add_prefixed_label_emits_external_label_in_display_form() {
    let mapping = RepositoryMapping::create(
        [(
            "apparent_name",
            RepositoryName::create_unvalidated("canonical_name"),
        )],
        MAIN.clone(),
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_prefixed_label("prefix-", label("@@canonical_name//a:b"), Some(mapping))
                .build()
        ),
        strs(&["prefix-@apparent_name//a:b"])
    );
}

#[test]
fn add_all_adds_all_arguments() {
    let f = fixture();
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_all(["val1", "val2"])
                .build()
        ),
        strs(&["val1", "val2"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_all_nested_set(nested_set(strs(&["val1", "val2"])))
                .build()
        ),
        strs(&["val1", "val2"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_paths(vec![path("path1"), path("path2")])
                .build()
        ),
        strs(&["path1", "path2"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_paths_nested_set(nested_set(vec![path("path1"), path("path2")]))
                .build()
        ),
        strs(&["path1", "path2"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_exec_paths(vec![f.artifact1.clone(), f.artifact2.clone()])
                .build()
        ),
        strs(&["dir/file1.txt", "dir/file2.txt"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_exec_paths_nested_set(nested_set(vec![
                    f.artifact1.clone(),
                    f.artifact2.clone()
                ]))
                .build()
        ),
        strs(&["dir/file1.txt", "dir/file2.txt"])
    );
}

#[test]
fn vector_adds_with_compile_time_arg_adds_all_values_prepended_by_arg() {
    let f = fixture();
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_all_with_arg("--arg", ["val1", "val2"])
                .build()
        ),
        strs(&["--arg", "val1", "val2"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_all_nested_set_with_arg("--arg", nested_set(strs(&["val1", "val2"])))
                .build()
        ),
        strs(&["--arg", "val1", "val2"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_paths_with_arg("--arg", vec![path("path1"), path("path2")])
                .build()
        ),
        strs(&["--arg", "path1", "path2"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_paths_nested_set_with_arg(
                    "--arg",
                    nested_set(vec![path("path1"), path("path2")])
                )
                .build()
        ),
        strs(&["--arg", "path1", "path2"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_exec_paths_with_arg("--arg", vec![f.artifact1.clone(), f.artifact2.clone()])
                .build()
        ),
        strs(&["--arg", "dir/file1.txt", "dir/file2.txt"])
    );
    assert_eq!(
        args(
            &CustomCommandLine::builder()
                .add_exec_paths_nested_set_with_arg(
                    "--arg",
                    nested_set(vec![f.artifact1.clone(), f.artifact2.clone()])
                )
                .build()
        ),
        strs(&["--arg", "dir/file1.txt", "dir/file2.txt"])
    );
}

/// Java `CustomCommandLineMode`.
#[derive(Clone, Copy, Debug)]
enum CustomCommandLineMode {
    Regular,
    WithConstantArg,
}

const CUSTOM_COMMAND_LINE_MODES: [CustomCommandLineMode; 2] = [
    CustomCommandLineMode::Regular,
    CustomCommandLineMode::WithConstantArg,
];

impl CustomCommandLineMode {
    fn add_all(self, vector_arg: impl Into<VectorArg<String>>) -> CustomCommandLine {
        let mut b = CustomCommandLine::builder();
        match self {
            CustomCommandLineMode::Regular => b.add_all_vector_arg(vector_arg),
            CustomCommandLineMode::WithConstantArg => {
                b.add_all_vector_arg_with_arg("--arg", vector_arg)
            }
        };
        b.build()
    }

    fn add_paths(self, vector_arg: impl Into<VectorArg<PathFragment>>) -> CustomCommandLine {
        let mut b = CustomCommandLine::builder();
        match self {
            CustomCommandLineMode::Regular => b.add_paths_vector_arg(vector_arg),
            CustomCommandLineMode::WithConstantArg => {
                b.add_paths_vector_arg_with_arg("--arg", vector_arg)
            }
        };
        b.build()
    }

    fn add_exec_paths(self, vector_arg: impl Into<VectorArg<Artifact>>) -> CustomCommandLine {
        let mut b = CustomCommandLine::builder();
        match self {
            CustomCommandLineMode::Regular => b.add_exec_paths_vector_arg(vector_arg),
            CustomCommandLineMode::WithConstantArg => {
                b.add_exec_paths_vector_arg_with_arg("--arg", vector_arg)
            }
        };
        b.build()
    }

    fn expected(self, values: &[&str]) -> Vec<String> {
        match self {
            CustomCommandLineMode::Regular => strs(values),
            CustomCommandLineMode::WithConstantArg => {
                let mut v = vec!["--arg".to_string()];
                v.extend(strs(values));
                v
            }
        }
    }
}

/// Java `VectorArgMode`.
#[derive(Clone, Copy, Debug)]
enum VectorArgMode {
    List,
    NestedSet,
}

const VECTOR_ARG_MODES: [VectorArgMode; 2] = [VectorArgMode::List, VectorArgMode::NestedSet];

impl VectorArgMode {
    fn of<T: VectorArgElement>(self, objects: Vec<T>) -> SimpleVectorArg<T> {
        match self {
            VectorArgMode::List => vector_arg::of(objects),
            VectorArgMode::NestedSet => vector_arg::of_nested_set(nested_set(objects)),
        }
    }

    fn each<T: VectorArgElement>(
        self,
        builder: VectorArgBuilder,
        objects: Vec<T>,
    ) -> SimpleVectorArg<T> {
        match self {
            VectorArgMode::List => builder.each(objects),
            VectorArgMode::NestedSet => builder.each_nested_set(nested_set(objects)),
        }
    }
}

/// Java `Foo`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Foo {
    str: String,
}

fn foo(s: &str) -> Foo {
    Foo { str: s.to_string() }
}

impl VectorArgElement for Foo {
    // Java: Object.toString (never reached; Foo is always mapped).
    fn expand_item_to_command_line(&self) -> String {
        format!("Foo({})", self.str)
    }
}

/// Java `Foo::expandToStr`.
struct ExpandToStr;

impl MapFn<Foo> for ExpandToStr {
    fn expand_to_command_line(
        &self,
        foo: &Foo,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        args(&foo.str);
        Ok(())
    }
}

static EXPAND_TO_STR: ExpandToStr = ExpandToStr;

#[test]
fn add_all_vector_adds_all_arguments() {
    for custom_command_line in CUSTOM_COMMAND_LINE_MODES {
        for mode in VECTOR_ARG_MODES {
            assert_eq!(
                args(&custom_command_line.add_all(mode.of(strs(&["1", "2"])))),
                custom_command_line.expected(&["1", "2"])
            );
            assert_eq!(
                args(
                    &custom_command_line
                        .add_all(mode.of(vec![foo("1"), foo("2")]).mapped(&EXPAND_TO_STR))
                ),
                custom_command_line.expected(&["1", "2"])
            );
        }
    }
}

#[test]
fn add_joined_vector_adds_joined_arguments() {
    let f = fixture();
    for custom_command_line in CUSTOM_COMMAND_LINE_MODES {
        for mode in VECTOR_ARG_MODES {
            assert_eq!(
                args(
                    &custom_command_line
                        .add_all(mode.each(vector_arg::join(":"), strs(&["val1", "val2"])))
                ),
                custom_command_line.expected(&["val1:val2"])
            );
            assert_eq!(
                args(&custom_command_line.add_paths(
                    mode.each(vector_arg::join(":"), vec![path("path1"), path("path2")])
                )),
                custom_command_line.expected(&["path1:path2"])
            );
            assert_eq!(
                args(&custom_command_line.add_exec_paths(mode.each(
                    vector_arg::join(":"),
                    vec![f.artifact1.clone(), f.artifact2.clone()]
                ))),
                custom_command_line.expected(&["dir/file1.txt:dir/file2.txt"])
            );
            assert_eq!(
                args(
                    &custom_command_line.add_all(
                        mode.each(vector_arg::join(":"), vec![foo("1"), foo("2")])
                            .mapped(&EXPAND_TO_STR)
                    )
                ),
                custom_command_line.expected(&["1:2"])
            );
        }
    }
}

#[test]
fn add_format_each_vector_adds_formatted_strings() {
    let f = fixture();
    for custom_command_line in CUSTOM_COMMAND_LINE_MODES {
        for mode in VECTOR_ARG_MODES {
            assert_eq!(
                args(
                    &custom_command_line
                        .add_all(mode.each(vector_arg::format("-D%s"), strs(&["val1", "val2"])))
                ),
                custom_command_line.expected(&["-Dval1", "-Dval2"])
            );
            assert_eq!(
                args(&custom_command_line.add_paths(mode.each(
                    vector_arg::format("-D%s"),
                    vec![path("path1"), path("path2")]
                ))),
                custom_command_line.expected(&["-Dpath1", "-Dpath2"])
            );
            assert_eq!(
                args(&custom_command_line.add_exec_paths(mode.each(
                    vector_arg::format("-D%s"),
                    vec![f.artifact1.clone(), f.artifact2.clone()]
                ))),
                custom_command_line.expected(&["-Ddir/file1.txt", "-Ddir/file2.txt"])
            );
            assert_eq!(
                args(
                    &custom_command_line.add_all(
                        mode.each(vector_arg::format("-D%s"), vec![foo("1"), foo("2")])
                            .mapped(&EXPAND_TO_STR)
                    )
                ),
                custom_command_line.expected(&["-D1", "-D2"])
            );
        }
    }
}

#[test]
fn add_format_each_joined_vector_adds_joined_formatted_strings() {
    let f = fixture();
    for custom_command_line in CUSTOM_COMMAND_LINE_MODES {
        for mode in VECTOR_ARG_MODES {
            assert_eq!(
                args(&custom_command_line.add_all(mode.each(
                    vector_arg::format("-D%s").join(":"),
                    strs(&["val1", "val2"])
                ))),
                custom_command_line.expected(&["-Dval1:-Dval2"])
            );
            assert_eq!(
                args(&custom_command_line.add_paths(mode.each(
                    vector_arg::format("-D%s").join(":"),
                    vec![path("path1"), path("path2")]
                ))),
                custom_command_line.expected(&["-Dpath1:-Dpath2"])
            );
            assert_eq!(
                args(&custom_command_line.add_exec_paths(mode.each(
                    vector_arg::format("-D%s").join(":"),
                    vec![f.artifact1.clone(), f.artifact2.clone()]
                ))),
                custom_command_line.expected(&["-Ddir/file1.txt:-Ddir/file2.txt"])
            );
            assert_eq!(
                args(
                    &custom_command_line.add_all(
                        mode.each(
                            vector_arg::format("-D%s").join(":"),
                            vec![foo("1"), foo("2")]
                        )
                        .mapped(&EXPAND_TO_STR)
                    )
                ),
                custom_command_line.expected(&["-D1:-D2"])
            );
        }
    }
}

#[test]
fn add_before_each_vector_adds_arguments_each_prepended_with_arg() {
    let f = fixture();
    for custom_command_line in CUSTOM_COMMAND_LINE_MODES {
        for mode in VECTOR_ARG_MODES {
            assert_eq!(
                args(
                    &custom_command_line
                        .add_all(mode.each(vector_arg::add_before("-D"), strs(&["val1", "val2"])))
                ),
                custom_command_line.expected(&["-D", "val1", "-D", "val2"])
            );
            assert_eq!(
                args(&custom_command_line.add_paths(mode.each(
                    vector_arg::add_before("-D"),
                    vec![path("path1"), path("path2")]
                ))),
                custom_command_line.expected(&["-D", "path1", "-D", "path2"])
            );
            assert_eq!(
                args(&custom_command_line.add_exec_paths(mode.each(
                    vector_arg::add_before("-D"),
                    vec![f.artifact1.clone(), f.artifact2.clone()]
                ))),
                custom_command_line.expected(&["-D", "dir/file1.txt", "-D", "dir/file2.txt"])
            );
            assert_eq!(
                args(
                    &custom_command_line.add_all(
                        mode.each(vector_arg::add_before("-D"), vec![foo("1"), foo("2")])
                            .mapped(&EXPAND_TO_STR)
                    )
                ),
                custom_command_line.expected(&["-D", "1", "-D", "2"])
            );
        }
    }
}

#[test]
fn add_before_each_formatted_vector_adds_formatted_strings_prepended_with_arg() {
    let f = fixture();
    for mode in VECTOR_ARG_MODES {
        assert_eq!(
            args(
                &CustomCommandLine::builder()
                    .add_all_vector_arg(mode.each(
                        vector_arg::add_before("-D").format("D%s"),
                        strs(&["val1", "val2"])
                    ))
                    .build()
            ),
            strs(&["-D", "Dval1", "-D", "Dval2"])
        );
        assert_eq!(
            args(
                &CustomCommandLine::builder()
                    .add_paths_vector_arg(mode.each(
                        vector_arg::add_before("-D").format("D%s"),
                        vec![path("path1"), path("path2")]
                    ))
                    .build()
            ),
            strs(&["-D", "Dpath1", "-D", "Dpath2"])
        );
        assert_eq!(
            args(
                &CustomCommandLine::builder()
                    .add_exec_paths_vector_arg(mode.each(
                        vector_arg::add_before("-D").format("D%s"),
                        vec![f.artifact1.clone(), f.artifact2.clone()]
                    ))
                    .build()
            ),
            strs(&["-D", "Ddir/file1.txt", "-D", "Ddir/file2.txt"])
        );
        assert_eq!(
            args(
                &CustomCommandLine::builder()
                    .add_all_vector_arg(
                        mode.each(
                            vector_arg::add_before("-D").format("D%s"),
                            vec![foo("1"), foo("2")]
                        )
                        .mapped(&EXPAND_TO_STR)
                    )
                    .build()
            ),
            strs(&["-D", "D1", "-D", "D2"])
        );
    }
}

#[test]
fn add_combined_args_adds_all_arguments() {
    let f = fixture();
    let cl = CustomCommandLine::builder()
        .add("--arg")
        .add_all_with_arg("--args", ["abc"])
        .add_exec_paths_with_arg("--path1", vec![f.artifact1.clone()])
        .add_exec_path_with_arg("--path2", Some(f.artifact2.clone()))
        .build();
    assert_eq!(
        args(&cl),
        strs(&[
            "--arg",
            "--args",
            "abc",
            "--path1",
            "dir/file1.txt",
            "--path2",
            "dir/file2.txt"
        ])
    );
}

#[test]
fn add_nulls_adds_nothing() {
    let f = fixture();
    let tree_artifact = f.create_tree_artifact("myTreeArtifact");
    assert!(tree_artifact.is_tree_artifact());

    // PORT: the Java calls passing a bare null (`addDynamicString(null)`, `addLabel(null)`,
    // `addPath(null)`, `addExecPath(null)`, `addLazyString(null)`, the `addPrefixed*` calls with
    // null, and null collections / nested sets / vector arg values) have no Rust counterpart:
    // those parameters are not optional. The calls that remain are the ones Rust can express.
    let empty_strings: Vec<String> = Vec::new();
    let empty_paths: Vec<PathFragment> = Vec::new();
    let empty_artifacts: Vec<Artifact> = Vec::new();
    let cl = CustomCommandLine::builder()
        .add_with_arg("foo", None)
        .add_label_with_arg("foo", None)
        .add_path_with_arg("foo", None)
        .add_exec_path_with_arg("foo", None)
        .add_lazy_string_with_arg("foo", None)
        .add_all(empty_strings.clone())
        .add_paths(empty_paths.clone())
        .add_exec_paths(empty_artifacts.clone())
        .add_all_nested_set(NestedSetBuilder::empty_set(Order::STABLE_ORDER))
        .add_paths_nested_set(NestedSetBuilder::empty_set(Order::STABLE_ORDER))
        .add_exec_paths_nested_set(NestedSetBuilder::empty_set(Order::STABLE_ORDER))
        .add_all_vector_arg(vector_arg::of_nested_set(
            NestedSetBuilder::<String>::empty_set(Order::STABLE_ORDER),
        ))
        .add_all_with_arg("foo", empty_strings.clone())
        .add_paths_with_arg("foo", empty_paths.clone())
        .add_exec_paths_with_arg("foo", empty_artifacts.clone())
        .add_all_nested_set_with_arg("foo", NestedSetBuilder::empty_set(Order::STABLE_ORDER))
        .add_paths_nested_set_with_arg("foo", NestedSetBuilder::empty_set(Order::STABLE_ORDER))
        .add_exec_paths_nested_set_with_arg("foo", NestedSetBuilder::empty_set(Order::STABLE_ORDER))
        .add_all_vector_arg_with_arg(
            "foo",
            vector_arg::of_nested_set(NestedSetBuilder::<String>::empty_set(Order::STABLE_ORDER)),
        )
        .add_placeholder_tree_artifact_exec_path_with_arg("foo", None)
        .build();

    assert!(args(&cl).is_empty());
}

#[test]
fn evaluate_tree_file_artifacts_replaces_tree_artifacts_with_children_exec_paths() {
    let f = fixture();
    let tree_artifact_one = f.create_tree_artifact("myArtifact/treeArtifact1");
    let tree_artifact_two = f.create_tree_artifact("myArtifact/treeArtifact2");

    let command_line_template = CustomCommandLine::builder()
        .add_placeholder_tree_artifact_exec_path_with_arg(
            "--argOne",
            Some(tree_artifact_one.clone()),
        )
        .add_placeholder_tree_artifact_exec_path_with_arg(
            "--argTwo",
            Some(tree_artifact_two.clone()),
        )
        .build();

    let tree_file_artifact_one =
        Artifact::create_tree_output_str(&tree_artifact_one, "children/child1");
    let tree_file_artifact_two =
        Artifact::create_tree_output_str(&tree_artifact_two, "children/child2");

    let command_line = command_line_template
        .evaluate_tree_file_artifacts(vec![tree_file_artifact_one, tree_file_artifact_two]);

    assert_eq!(
        args(&command_line),
        strs(&[
            "--argOne",
            "dir/myArtifact/treeArtifact1/children/child1",
            "--argTwo",
            "dir/myArtifact/treeArtifact2/children/child2"
        ])
    );
}

/// Java `expandParentRelativePath`.
struct ExpandParentRelativePath;

impl MapFn<Artifact> for ExpandParentRelativePath {
    fn expand_to_command_line(
        &self,
        src: &Artifact,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        match src.get_tree_relative_path_string() {
            Ok(s) => args(&s),
            Err(e) => panic!("IllegalStateException: Unexpected EvalException thown. {e}"),
        }
        Ok(())
    }
}

static EXPAND_PARENT_RELATIVE_PATH: ExpandParentRelativePath = ExpandParentRelativePath;

#[test]
fn add_all_mapped_tree_file_artifacts_map_to_relative_path_adds_tree_file_relative_paths() {
    let f = fixture();
    let tree_artifact = f.create_tree_artifact("myArtifact/treeArtifact");

    let tree_file_artifact_one =
        Artifact::create_tree_output_str(&tree_artifact, "children/child1");
    let tree_file_artifact_two =
        Artifact::create_tree_output_str(&tree_artifact, "children/child2");

    let command_line_template = CustomCommandLine::builder()
        .add_all_vector_arg(
            SimpleVectorArg::of(vec![tree_file_artifact_one, tree_file_artifact_two])
                .mapped(&EXPAND_PARENT_RELATIVE_PATH),
        )
        .build();

    assert_eq!(
        args(&command_line_template),
        strs(&["children/child1", "children/child2"])
    );
}

#[test]
fn arguments_unsubstituted_tree_artifact_placeholder_fails() {
    let f = fixture();
    let tree_artifact_one = f.create_tree_artifact("myArtifact/treeArtifact1");
    let tree_artifact_two = f.create_tree_artifact("myArtifact/treeArtifact2");

    let command_line_template = CustomCommandLine::builder()
        .add_placeholder_tree_artifact_exec_path_with_arg("--argOne", Some(tree_artifact_one))
        .add_placeholder_tree_artifact_exec_path_with_arg("--argTwo", Some(tree_artifact_two))
        .build();

    let result = catch_unwind(AssertUnwindSafe(|| command_line_template.arguments()));
    assert!(result.is_err());
}

/// Java `(s, args) -> args.accept(s + "_mapped")`.
struct AppendMapped;

impl MapFn<String> for AppendMapped {
    fn expand_to_command_line(
        &self,
        s: &String,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        args(&format!("{s}_mapped"));
        Ok(())
    }
}

static APPEND_MAPPED: AppendMapped = AppendMapped;

#[test]
fn add_to_finger_print_computes_unique_key_for_different_command_lines() {
    let values: NestedSet<String> = {
        let mut b = NestedSetBuilder::<String>::stable_order();
        b.add("a".to_string()).add("b".to_string());
        b.build()
    };
    let command_lines: Vec<CustomCommandLine> = vec![
        CustomCommandLine::builder().add("arg").build(),
        CustomCommandLine::builder()
            .add_formatted("--foo=%s", vec![CommandLineArg::from("arg")])
            .build(),
        CustomCommandLine::builder()
            .add_prefixed("--foo=%s", "arg")
            .build(),
        CustomCommandLine::builder()
            .add_all_nested_set(values.clone())
            .build(),
        CustomCommandLine::builder()
            .add_all_vector_arg(vector_arg::add_before("--foo=%s").each_nested_set(values.clone()))
            .build(),
        CustomCommandLine::builder()
            .add_all_vector_arg(vector_arg::join("--foo=%s").each_nested_set(values.clone()))
            .build(),
        CustomCommandLine::builder()
            .add_all_vector_arg(vector_arg::format("--foo=%s").each_nested_set(values.clone()))
            .build(),
        CustomCommandLine::builder()
            .add_all_vector_arg(vector_arg::of_nested_set(values.clone()).mapped(&APPEND_MAPPED))
            .build(),
    ];

    // Ensure all these command lines have distinct keys
    let action_key_context = ActionKeyContext::new();
    let mut digests: HashMap<String, CustomCommandLine> = HashMap::new();
    for command_line in command_lines {
        let mut fingerprint = Fingerprint::new();
        command_line
            .add_to_fingerprint(
                &action_key_context,
                /* input_metadata_provider= */ None,
                OutputPathsMode::OFF,
                &mut fingerprint,
            )
            .unwrap();
        let digest = fingerprint.hex_digest_and_reset();
        if let Some(previous) = digests.get(&digest) {
            panic!(
                "Found two command lines with identical digest {}: '{}' and '{}'",
                digest,
                previous.arguments().unwrap().join(" "),
                command_line.arguments().unwrap().join(" ")
            );
        }
        digests.insert(digest, command_line);
    }
}
