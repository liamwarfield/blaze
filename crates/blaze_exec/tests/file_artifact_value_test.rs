//! Port of `skyframe/FileArtifactValueTest.java` (the equality part and the unresolved symlink
//! tests, on a temporary directory of the Unix file system instead of Java's in-memory one; the
//! other tests need the file-system fixtures of `FileArtifactValue.createForTesting` and the
//! manual clock, and are not ported here).

use blaze_exec::actions::FileArtifactValue;
use blaze_exec::actions::file_artifact_value::FileStateType;
use blaze_vfs::{DigestHashFunction, FileSystemExt, PathFragment, UnixFileSystem};

fn to_bytes(hex: &str) -> Vec<u8> {
    hex::decode(hex).unwrap()
}

/// `EqualsTester`: every member equals the members of its group and no member of another group.
/// PORT: `hashCode` has no counterpart (`FileArtifactValue` does not implement `Hash`).
fn test_equals(groups: &[Vec<FileArtifactValue>]) {
    for (i, group) in groups.iter().enumerate() {
        for (j, other_group) in groups.iter().enumerate() {
            for a in group {
                for b in other_group {
                    if i == j {
                        assert_eq!(a, b, "group {i}");
                    } else {
                        assert_ne!(a, b, "groups {i} and {j}");
                    }
                }
            }
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/FileArtifactValueTest.java:80 FileArtifactValueTest.testEqualsAndHashCode
#[test]
fn test_equals_and_hash_code() {
    test_equals(&[
        vec![
            FileArtifactValue::create_for_normal_file(
                Some(to_bytes("00112233445566778899AABBCCDDEEFF")),
                /* proxy= */ None,
                1,
            ),
            FileArtifactValue::create_for_normal_file(
                Some(to_bytes("00112233445566778899AABBCCDDEEFF")),
                /* proxy= */ None,
                1,
            ),
        ],
        vec![FileArtifactValue::create_for_normal_file(
            Some(to_bytes("00112233445566778899AABBCCDDEEFF")),
            /* proxy= */ None,
            2,
        )],
        vec![FileArtifactValue::create_for_directory_with_mtime(1)],
        vec![FileArtifactValue::create_for_normal_file(
            Some(to_bytes("FFFFFF00000000000000000000000000")),
            /* proxy= */ None,
            1,
        )],
        vec![
            FileArtifactValue::create_for_directory_with_mtime(2),
            FileArtifactValue::create_for_directory_with_mtime(2),
        ],
        // expireAtEpochMilli doesn't contribute to the equality
        vec![
            FileArtifactValue::create_for_remote_file_with_materialization_data(
                to_bytes("00112233445566778899AABBCCDDEEFF"),
                /* size= */ 1,
                /* location_index= */ 1,
                /* expiration_time= */ Some(1),
                /* in_memory_output= */ false,
            ),
            FileArtifactValue::create_for_remote_file_with_materialization_data(
                to_bytes("00112233445566778899AABBCCDDEEFF"),
                /* size= */ 1,
                /* location_index= */ 1,
                /* expiration_time= */ Some(2),
                /* in_memory_output= */ false,
            ),
        ],
        // A ResolvedSymlinkArtifactValue is not equal to the FileArtifactValue it wraps.
        vec![FileArtifactValue::create_from_existing_with_resolved_path(
            FileArtifactValue::create_for_normal_file(
                Some(to_bytes("00112233445566778899AABBCCDDEEFF")),
                /* proxy= */ None,
                1,
            ),
            PathFragment::create("/some/path"),
        )],
        vec![FileArtifactValue::MissingFileMarker],
        vec![FileArtifactValue::RunfilesTreeMarker],
        // PORT: the `"a string"` group has no counterpart (Rust equality is typed).
    ]);
}

/// A fresh directory of the Unix file system standing for Java's in-memory file system root.
fn scratch_root() -> (tempfile::TempDir, blaze_vfs::Path) {
    let dir = tempfile::tempdir().unwrap();
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let root = fs.get_path(dir.path().to_str().unwrap());
    (dir, root)
}

// Java: com/google/devtools/build/lib/skyframe/FileArtifactValueTest.java:190 FileArtifactValueTest.testUnresolvedSymlink
#[test]
fn test_unresolved_symlink() {
    let (_dir, root) = scratch_root();
    // scratchSymlink("/sym", "/some/path")
    let path = root.get_relative_str("sym");
    path.create_symbolic_link_to_fragment(&PathFragment::create("/some/path"))
        .unwrap();
    let value = FileArtifactValue::create_for_unresolved_symlink(&path).unwrap();
    let value2 = FileArtifactValue::create_for_unresolved_symlink(&path).unwrap();
    assert_eq!(value.get_type(), FileStateType::Symlink);
    assert_eq!(value.get_unresolved_symlink_target(), Some("/some/path"));
    test_equals(&[vec![value, value2]]);
}

// Java: com/google/devtools/build/lib/skyframe/FileArtifactValueTest.java:441 FileArtifactValueTest.testUptodateUnresolvedSymlink
#[test]
fn test_uptodate_unresolved_symlink() {
    let (_dir, root) = scratch_root();
    let path = root.get_relative_str("dir/symlink");
    path.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    path.create_symbolic_link_to_fragment(&PathFragment::create("target_path"))
        .unwrap();
    let value = FileArtifactValue::create_for_unresolved_symlink(&path).unwrap();

    // PORT: clock.advanceMillis(1) has no counterpart (the digest does not depend on time).
    assert!(!value.was_modified_since_digest(&path).unwrap());

    path.delete().unwrap();
    path.create_symbolic_link_to_fragment(&PathFragment::create("modified_target_path"))
        .unwrap();

    assert!(value.was_modified_since_digest(&path).unwrap());
}

/// The digest is the digest of the target string with its first byte inverted, so a symlink and
/// a file holding the target text differ (`UnresolvedSymlinkArtifactValue.<init>`).
// Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:1149 UnresolvedSymlinkArtifactValue.<init>
#[test]
fn unresolved_symlink_digest_is_tweaked_target_digest() {
    let (_dir, root) = scratch_root();
    let path = root.get_relative_str("sym");
    path.create_symbolic_link_to_fragment(&PathFragment::create("data.txt"))
        .unwrap();
    let value = FileArtifactValue::create_for_unresolved_symlink(&path).unwrap();
    let mut expected = DigestHashFunction::Sha256
        .hash_reader(&mut &b"data.txt"[..])
        .unwrap();
    expected[0] ^= 0xff;
    assert_eq!(value.get_digest(), Some(&expected[..]));
    assert_eq!(value.get_size(), 0);

    // A path that is not a symlink: NotASymlinkException.
    let file = root.get_relative_str("file");
    std::fs::write(file.get_path_string(), "data.txt").unwrap();
    let e = FileArtifactValue::create_for_unresolved_symlink(&file).unwrap_err();
    assert!(blaze_vfs::NotASymlinkException::is(&e), "{e}");
    assert_eq!(
        e.to_string(),
        format!("{} is not a symlink", file.get_path_string())
    );
}
