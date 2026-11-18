//! Port of `src/test/java/com/google/devtools/build/lib/actions/ArtifactRootTest.java`.

use blaze_actions::{ArtifactRoot, RootType};
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, Root, UnixFileSystem};

fn fs() -> FileSystemRef {
    UnixFileSystem::create(DigestHashFunction::Sha256)
}

fn dir(fs: &FileSystemRef, p: &str) -> Path {
    Path::create(PathFragment::create(p), fs)
}

#[test]
fn as_source_root_creates_valid_source_root() {
    let fs = fs();
    let source_dir = dir(&fs, "/source");
    let root = ArtifactRoot::as_source_root(Root::from_path(source_dir.clone()));
    assert!(root.is_source_root());
    assert_eq!(*root.get_exec_path(), PathFragment::empty_fragment());
    assert_eq!(*root.get_root(), Root::from_path(source_dir));
    assert_eq!(root.to_string(), "/source[source]");
}

#[test]
#[ignore = "Rust has no null; ArtifactRoot.asSourceRoot(null) cannot be expressed"]
fn as_source_root_null_root_fails() {}

#[test]
fn as_derived_root_creates_valid_derived_root() {
    let fs = fs();
    let exec_root = dir(&fs, "/exec");
    let root_dir = dir(&fs, "/exec/root");
    let root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["root"]);
    assert!(!root.is_source_root());
    assert_eq!(*root.get_exec_path(), PathFragment::create("root"));
    assert_eq!(*root.get_root(), Root::from_path(root_dir));
    assert_eq!(root.to_string(), "/exec/root[derived]");
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn as_derived_root_derived_root_is_exec_root_fails_not_ok() {
    let fs = fs();
    let exec_root = dir(&fs, "/exec");
    ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &[""]);
}

#[test]
fn as_derived_root_empty_prefix_creates_artifact_root() {
    let fs = fs();
    let exec_root = dir(&fs, "/exec");
    assert_eq!(
        ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["", "suffix", ""]),
        ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["suffix"])
    );
}

#[test]
#[should_panic]
fn as_derived_root_prefix_with_slash_fails() {
    let fs = fs();
    let exec_root = dir(&fs, "/exec");
    ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["suffix/"]);
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn as_derived_root_no_prefixes_fails() {
    let fs = fs();
    let exec_root = dir(&fs, "/exec");
    ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &[]);
}

#[test]
#[ignore = "Rust has no null; ArtifactRoot.asDerivedRoot(null, ...) cannot be expressed"]
fn as_derived_root_null_exec_path_fails() {}

#[test]
fn as_derived_root_path_fragment_simple_exec_path_creates_artifact_root() {
    let fs = fs();
    let exec_root = dir(&fs, "/exec");
    let root_dir = dir(&fs, "/exec/root");
    let root = ArtifactRoot::as_derived_root_fragment(
        &exec_root,
        RootType::OUTPUT,
        PathFragment::create("root"),
    );
    assert!(!root.is_source_root());
    assert_eq!(*root.get_exec_path(), PathFragment::create("root"));
    assert_eq!(*root.get_root(), Root::from_path(root_dir));
    assert_eq!(root.to_string(), "/exec/root[derived]");
}

#[test]
fn as_derived_root_path_fragment_nested_exec_path_creates_artifact_root() {
    let fs = fs();
    let exec_root = dir(&fs, "/exec");
    let root_dir = dir(&fs, "/exec/dir1/dir2/dir3");
    let root = ArtifactRoot::as_derived_root_fragment(
        &exec_root,
        RootType::OUTPUT,
        PathFragment::create("dir1/dir2/dir3"),
    );
    assert!(!root.is_source_root());
    assert_eq!(
        *root.get_exec_path(),
        PathFragment::create("dir1/dir2/dir3")
    );
    assert_eq!(*root.get_root(), Root::from_path(root_dir));
    assert_eq!(root.to_string(), "/exec/dir1/dir2/dir3[derived]");
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn as_derived_root_path_fragment_empty_exec_path_fails() {
    let fs = fs();
    let exec_root = dir(&fs, "/exec");
    ArtifactRoot::as_derived_root_fragment(
        &exec_root,
        RootType::OUTPUT,
        PathFragment::empty_fragment(),
    );
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn as_derived_root_path_fragment_exec_path_is_current_directory_fails() {
    let fs = fs();
    let exec_root = dir(&fs, "/exec");
    ArtifactRoot::as_derived_root_fragment(&exec_root, RootType::OUTPUT, PathFragment::create("."));
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn as_derived_root_path_fragment_exec_path_is_directory_up_fails() {
    let fs = fs();
    let exec_root = dir(&fs, "/exec");
    ArtifactRoot::as_derived_root_fragment(
        &exec_root,
        RootType::OUTPUT,
        PathFragment::create(".."),
    );
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn as_derived_root_path_fragment_exec_path_contains_directory_up_fails() {
    let fs = fs();
    let exec_root = dir(&fs, "/exec");
    ArtifactRoot::as_derived_root_fragment(
        &exec_root,
        RootType::OUTPUT,
        PathFragment::create("../outsideExecRoot"),
    );
}

#[test]
#[ignore = "needs serialization codecs (not ported: single-process Blaze)"]
fn derived_root_serialization_root_matches_designated_likely_root_skips_root_in_serialization() {}

#[test]
fn equals_returns_true_for_identical_root_and_detects_differences_on_each_field() {
    let fs = fs();
    let exec_root = dir(&fs, "/exec");
    let root_segment = "root";
    let root_dir = exec_root.get_child(root_segment);
    let other_root_dir = dir(&fs, "/");
    let source_dir = dir(&fs, "/source");

    let groups: Vec<Vec<ArtifactRoot>> = vec![
        vec![
            ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &[root_segment]),
            ArtifactRoot::as_derived_root_fragment(
                &exec_root,
                RootType::OUTPUT,
                PathFragment::create(root_segment),
            ),
        ],
        vec![ArtifactRoot::as_derived_root(
            &other_root_dir,
            RootType::OUTPUT,
            &["exec", root_segment],
        )],
        vec![ArtifactRoot::as_derived_root(
            &exec_root,
            RootType::OUTPUT,
            &["otherSegment"],
        )],
        vec![ArtifactRoot::as_source_root(Root::from_path(source_dir))],
        vec![ArtifactRoot::as_source_root(Root::from_path(root_dir))],
    ];
    // EqualsTester: equal within a group, unequal across groups.
    for (i, gi) in groups.iter().enumerate() {
        for (j, gj) in groups.iter().enumerate() {
            for a in gi {
                for b in gj {
                    assert_eq!(a == b, i == j, "groups {i} and {j}: {a} vs {b}");
                }
            }
        }
    }
}
