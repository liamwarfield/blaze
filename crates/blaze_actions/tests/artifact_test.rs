//! Port of `src/test/java/com/google/devtools/build/lib/actions/ArtifactTest.java`.

use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::artifact::OwnerlessArtifactWrapper;
use blaze_actions::{
    ActionLookupData, ActionLookupKeyRef, Artifact, ArtifactOwner, ArtifactRoot, RootType,
    SpecialArtifactType,
};
use blaze_cmdline::Label;
use blaze_cmdline::label_constants::{
    EXPERIMENTAL_EXTERNAL_PATH_PREFIX, EXTERNAL_PATH_PREFIX, EXTERNAL_REPOSITORY_LOCATION,
};
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, Root, UnixFileSystem};

/// `ActionsTestUtil.NULL_ARTIFACT_OWNER`-like keys: identity is the `id`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TestKey {
    id: u32,
    function: &'static str,
}

impl fmt::Display for TestKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TestKey({})", self.id)
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
        self.function
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other
            .as_any()
            .downcast_ref::<TestKey>()
            .is_some_and(|o| o == self)
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

fn null_artifact_owner() -> ActionLookupKeyRef {
    ActionLookupKeyRef::new(TestKey {
        id: 0,
        function: "CONFIGURED_TARGET",
    })
}

fn null_action_lookup_data() -> ActionLookupData {
    ActionLookupData::create(null_artifact_owner(), 0)
}

struct Fixture {
    fs: FileSystemRef,
    exec_dir: Path,
    root_dir: ArtifactRoot,
}

fn fixture() -> Fixture {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_dir = Path::create(PathFragment::create("/base/exec"), &fs);
    let root_dir = ArtifactRoot::as_derived_root(&exec_dir, RootType::OUTPUT, &["root"]);
    Fixture {
        fs,
        exec_dir,
        root_dir,
    }
}

fn path(fs: &FileSystemRef, p: &str) -> Path {
    Path::create(PathFragment::create(p), fs)
}

// ActionsTestUtil.createArtifactWithExecPath
fn create_artifact_with_exec_path(root: &ArtifactRoot, exec_path: PathFragment) -> Artifact {
    if root.is_source_root() {
        Artifact::new_source_artifact(root.clone(), exec_path, ArtifactOwner::NullOwner)
    } else {
        Artifact::create_derived_artifact(root.clone(), exec_path, null_artifact_owner())
    }
}

// ActionsTestUtil.createArtifactWithRootRelativePath
fn create_artifact_with_root_relative_path(
    root: &ArtifactRoot,
    root_relative_path: &PathFragment,
) -> Artifact {
    create_artifact_with_exec_path(root, root.get_exec_path().get_relative(root_relative_path))
}

// ActionsTestUtil.createArtifact(root, Path)
fn create_artifact_at(root: &ArtifactRoot, p: &Path) -> Artifact {
    create_artifact_with_root_relative_path(root, &root.get_root().relativize(p))
}

#[test]
#[should_panic]
fn test_construction_bad_root_dir() {
    let f = fixture();
    let f1 = path(&f.fs, "/exec/dir/file.ext");
    // f1.relativeTo(execDir) itself fails for a path outside execDir; either way it throws.
    let rel = f1.as_fragment().relative_to(f.exec_dir.as_fragment());
    create_artifact_with_exec_path(
        &ArtifactRoot::as_derived_root(&f.exec_dir, RootType::OUTPUT, &["bogus"]),
        rel,
    )
    .get_root_relative_path();
}

#[test]
#[ignore = "measures JVM heap usage"]
fn test_memory_usage() {}

#[test]
fn test_equivalence_relation() {
    let f = fixture();
    let a_path = PathFragment::create("src/a");
    let b_path = PathFragment::create("src/b");
    assert_eq!(
        create_artifact_with_root_relative_path(&f.root_dir, &a_path),
        create_artifact_with_root_relative_path(&f.root_dir, &a_path)
    );
    assert_eq!(
        create_artifact_with_root_relative_path(&f.root_dir, &b_path),
        create_artifact_with_root_relative_path(&f.root_dir, &b_path)
    );
    assert_ne!(
        create_artifact_with_root_relative_path(&f.root_dir, &a_path),
        create_artifact_with_root_relative_path(&f.root_dir, &b_path)
    );
}

#[test]
fn test_comparison() {
    use std::cmp::Ordering;
    let f = fixture();
    let a = create_artifact_with_root_relative_path(&f.root_dir, &PathFragment::create("src/a"));
    let b = create_artifact_with_root_relative_path(&f.root_dir, &PathFragment::create("src/b"));
    assert_eq!(Artifact::exec_path_comparator(&a, &b), Ordering::Less);
    assert_eq!(Artifact::exec_path_comparator(&a, &a), Ordering::Equal);
    assert_eq!(Artifact::exec_path_comparator(&b, &b), Ordering::Equal);
    assert_eq!(Artifact::exec_path_comparator(&b, &a), Ordering::Greater);
}

#[test]
#[ignore = "needs JavaSemantics.JAVA_SOURCE / CppFileTypes; CppFileTypes is in blaze_rules, which \
            depends on this crate"]
fn test_get_filename() {}

#[test]
fn test_get_extension() {
    let f = fixture();
    let root = ArtifactRoot::as_source_root(Root::from_path(path(&f.fs, "/foo")));
    let java_file = create_artifact_at(&root, &path(&f.fs, "/foo/Bar.java"));
    assert_eq!(java_file.get_extension(), "java");
}

#[test]
#[ignore = "needs util/FileType, which no crate below blaze_actions has (blaze_packages and \
            blaze_rules each carry one; neither is a dependency of this crate)"]
fn test_is_file_type() {}

#[test]
#[ignore = "needs util/FileTypeSet, which no crate below blaze_actions has (blaze_packages and \
            blaze_rules each carry a FileType; neither is a dependency of this crate)"]
fn test_is_file_type_set() {}

#[test]
fn test_mangled_path() {
    let path = "dir/sub_dir/name:end";
    assert_eq!(
        blaze_actions::rules::cpp::solib_symlink_action::escaped_path(path),
        "dir_Ssub_Udir_Sname_Cend"
    );
}

#[test]
fn test_root_relative_path_is_same_as_exec_path() {
    let f = fixture();
    let root = ArtifactRoot::as_source_root(Root::from_path(path(&f.fs, "/foo")));
    let a = create_artifact_at(&root, &path(&f.fs, "/foo/bar1.h"));
    // PORT: Java checks instance identity; the values are equal here.
    assert_eq!(a.get_root_relative_path(), *a.get_exec_path());
}

#[test]
fn test_to_detail_string() {
    let f = fixture();
    let exec_root = path(&f.fs, "/execroot/workspace");
    let a = create_artifact_with_root_relative_path(
        &ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["b"]),
        &PathFragment::create("c"),
    );
    assert_eq!(a.to_detail_string(), "[[<execution_root>]b]c");
}

#[test]
#[should_panic]
fn test_weird_artifact() {
    let f = fixture();
    let exec_root = path(&f.fs, "/");
    create_artifact_with_exec_path(
        &ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["a"]),
        PathFragment::create("c"),
    )
    .get_root_relative_path();
}

#[test]
#[ignore = "needs serialization codecs (not ported: single-process Blaze)"]
fn derived_artifact_codecs() {}

#[test]
#[ignore = "needs serialization codecs (not ported: single-process Blaze)"]
fn source_artifact_codec_recycles_source_artifact_instances() {}

fn create_dir_name_artifact(f: &Fixture) -> Artifact {
    create_artifact_at(
        &ArtifactRoot::as_source_root(Root::from_path(path(&f.fs, "/"))),
        &path(&f.fs, "/aaa/bbb/ccc/ddd"),
    )
}

#[test]
fn test_long_dirname() {
    let f = fixture();
    assert_eq!(create_dir_name_artifact(&f).get_dirname(), "aaa/bbb/ccc");
}

#[test]
fn test_dirname_in_execution_dir() {
    let f = fixture();
    let artifact = create_artifact_at(
        &ArtifactRoot::as_source_root(Root::from_path(path(&f.fs, "/foo"))),
        &path(&f.fs, "/foo/bar.txt"),
    );
    assert_eq!(artifact.get_dirname(), ".");
}

#[test]
fn test_can_construct_path_from_dir_and_filename() {
    let f = fixture();
    let artifact = create_dir_name_artifact(&f);
    let constructed = format!("{}/{}", artifact.get_dirname(), artifact.get_filename());
    assert_eq!(constructed, "aaa/bbb/ccc/ddd");
}

#[test]
fn test_is_source_artifact() {
    let f = fixture();
    assert!(
        Artifact::new_source_artifact(
            ArtifactRoot::as_source_root(Root::from_path(path(&f.fs, "/"))),
            PathFragment::create("src/foo.cc"),
            ArtifactOwner::NullOwner,
        )
        .is_source_artifact()
    );
    assert!(
        !create_artifact_at(
            &ArtifactRoot::as_derived_root(&path(&f.fs, "/genfiles"), RootType::OUTPUT, &["aaa"]),
            &path(&f.fs, "/genfiles/aaa/bar.out"),
        )
        .is_source_artifact()
    );
}

#[test]
fn test_get_root() {
    let f = fixture();
    let exec_root = path(&f.fs, "/");
    let root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["newRoot"]);
    assert_eq!(
        *create_artifact_at(&root, &path(&f.fs, "/newRoot/foo")).get_root(),
        root
    );
}

#[test]
fn hash_code_and_equals() {
    let f = fixture();
    let exec_root = path(&f.fs, "/");
    let root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["newRoot"]);
    let first_owner = ActionLookupKeyRef::new(TestKey {
        id: 1,
        function: "NONE",
    });
    let second_owner = ActionLookupKeyRef::new(TestKey {
        id: 2,
        function: "NONE",
    });
    let derived1 = Artifact::create_derived_artifact(
        root.clone(),
        PathFragment::create("newRoot/shared"),
        first_owner.clone(),
    );
    derived1.set_generating_action_key(ActionLookupData::create(first_owner.clone(), 0));
    let derived2 = Artifact::create_derived_artifact(
        root.clone(),
        PathFragment::create("newRoot/shared"),
        second_owner.clone(),
    );
    derived2.set_generating_action_key(ActionLookupData::create(second_owner.clone(), 0));
    let source_root =
        ArtifactRoot::as_source_root(Root::from_path(root.get_root().as_path().unwrap().clone()));
    let source1 = Artifact::new_source_artifact(
        source_root.clone(),
        PathFragment::create("shared"),
        ArtifactOwner::ActionLookupKey(first_owner),
    );
    let source2 = Artifact::new_source_artifact(
        source_root,
        PathFragment::create("shared"),
        ArtifactOwner::ActionLookupKey(second_owner),
    );
    // EqualsTester: [derived1], [derived2], [source1, source2]
    assert_ne!(derived1, derived2);
    assert_ne!(derived1, source1);
    assert_ne!(derived2, source1);
    assert_eq!(source1, source2);
    let h = |a: &Artifact| {
        let mut s = DefaultHasher::new();
        a.hash(&mut s);
        s.finish()
    };
    assert_ne!(h(&derived1), h(&derived2));
    assert_ne!(h(&derived1), h(&source1));
    assert_eq!(h(&source1), h(&source2));
    let wrapper1 = OwnerlessArtifactWrapper(derived1.clone());
    let wrapper2 = OwnerlessArtifactWrapper(derived2.clone());
    let wrapper3 = OwnerlessArtifactWrapper(source1.clone());
    let wrapper4 = OwnerlessArtifactWrapper(source2.clone());
    assert_eq!(wrapper1, wrapper2);
    assert_eq!(wrapper3, wrapper4);
    assert_ne!(wrapper1, wrapper3);
    let paths = [
        derived1.get_path(),
        derived2.get_path(),
        source1.get_path(),
        source2.get_path(),
    ];
    for p in &paths {
        assert_eq!(*p, paths[0]);
    }
}

#[test]
fn test_get_repository_relative_path_external_source_artifacts() {
    let f = fixture();
    let external_root = ArtifactRoot::as_external_source_root(Root::from_path(
        path(&f.fs, "/output_base")
            .get_relative(&EXTERNAL_REPOSITORY_LOCATION)
            .get_relative_str("foo"),
    ));

    // --experimental_sibling_repository_layout not set
    assert_eq!(
        Artifact::new_source_artifact(
            external_root.clone(),
            EXTERNAL_PATH_PREFIX.get_relative_str("foo/bar/baz.cc"),
            ArtifactOwner::NullOwner,
        )
        .get_repository_relative_path(),
        PathFragment::create("bar/baz.cc")
    );

    // --experimental_sibling_repository_layout set
    assert_eq!(
        Artifact::new_source_artifact(
            external_root,
            EXPERIMENTAL_EXTERNAL_PATH_PREFIX.get_relative_str("foo/bar/baz.cc"),
            ArtifactOwner::NullOwner,
        )
        .get_repository_relative_path(),
        PathFragment::create("bar/baz.cc")
    );
}

fn create_tree_artifact_with(
    root: &ArtifactRoot,
    relative_path: &str,
    action_lookup_data: ActionLookupData,
) -> Artifact {
    let tree = Artifact::create_special_artifact(
        root.clone(),
        root.get_exec_path().get_relative_str(relative_path),
        action_lookup_data.get_action_lookup_key().clone(),
        SpecialArtifactType::TREE,
    );
    tree.set_generating_action_key(action_lookup_data);
    tree
}

fn create_tree_artifact(root: &ArtifactRoot, relative_path: &str) -> Artifact {
    create_tree_artifact_with(root, relative_path, null_action_lookup_data())
}

#[test]
fn archived_tree_artifact_create_returns_artifact_in_archived_root() {
    let f = fixture();
    let root =
        ArtifactRoot::as_derived_root(&f.exec_dir, RootType::OUTPUT, &["blaze-out", "fastbuild"]);
    let tree = create_tree_artifact(&root, "tree");

    let archived = Artifact::create_archived_tree_artifact_for_tree(&tree);

    assert!(Artifact::ptr_eq(archived.get_parent().unwrap(), &tree));
    assert_eq!(*archived.get_action_lookup_key(), null_artifact_owner());
    assert_eq!(
        archived.get_exec_path_string(),
        "blaze-out/:archived_tree_artifacts/fastbuild/tree.zip"
    );
    assert_eq!(
        archived.get_root().get_exec_path_string(),
        "blaze-out/:archived_tree_artifacts/fastbuild"
    );
}

#[test]
fn archived_tree_artifact_create_returns_artifact_with_generating_action_from_parent() {
    let f = fixture();
    let action_lookup_key = ActionLookupKeyRef::new(TestKey {
        id: 7,
        function: "CONFIGURED_TARGET",
    });
    let action_lookup_data = ActionLookupData::create(action_lookup_key.clone(), 0);
    let tree = create_tree_artifact_with(&f.root_dir, "tree", action_lookup_data.clone());

    let archived = Artifact::create_archived_tree_artifact_for_tree(&tree);

    assert_eq!(
        archived.get_exec_path_string(),
        "root/:archived_tree_artifacts/tree.zip"
    );
    assert_eq!(*archived.get_action_lookup_key(), action_lookup_key);
    assert_eq!(*archived.get_generating_action_key(), action_lookup_data);
}

#[test]
fn archived_tree_artifact_create_with_custom_derived_tree_root_returns_artifact_with_custom_root() {
    let f = fixture();
    let root =
        ArtifactRoot::as_derived_root(&f.exec_dir, RootType::OUTPUT, &["blaze-out", "fastbuild"]);
    let tree = create_tree_artifact(&root, "dir/tree");

    let archived = Artifact::create_archived_with_custom_derived_tree_root(
        &tree,
        &PathFragment::create("custom/custom2"),
        &PathFragment::create("treePath/file.xyz"),
    );

    assert!(Artifact::ptr_eq(archived.get_parent().unwrap(), &tree));
    assert_eq!(
        archived.get_exec_path_string(),
        "blaze-out/custom/custom2/fastbuild/treePath/file.xyz"
    );
    assert_eq!(
        archived.get_root().get_exec_path_string(),
        "blaze-out/custom/custom2/fastbuild"
    );
}

#[test]
#[ignore = "needs serialization codecs (not ported: single-process Blaze)"]
fn archived_tree_artifact_codec_round_trips_archived_artifact() {}

#[test]
fn archived_tree_artifact_get_exec_path_within_archived_artifacts_tree_returns_correct_path() {
    assert_eq!(
        Artifact::get_exec_path_within_archived_artifacts_tree(&PathFragment::create(
            "bazel-out/k8-fastbuild/bin/dir/subdir"
        )),
        PathFragment::create("bazel-out/:archived_tree_artifacts/k8-fastbuild/bin/dir/subdir")
    );
}

#[test]
#[ignore = "needs PathMapper.storeIn / loadFrom (a PathMapper stored in StarlarkSemantics), not \
            ported: starlark_custom_command_line.rs calls unported! for it"]
fn mapped_artifact() {}
