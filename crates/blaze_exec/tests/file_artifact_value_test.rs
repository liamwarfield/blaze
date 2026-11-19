//! Port of `skyframe/FileArtifactValueTest.java` (the equality part; the other tests need the
//! file-system fixtures of `FileArtifactValue.createForTesting` and are not ported here).

use blaze_exec::actions::FileArtifactValue;
use blaze_vfs::PathFragment;

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
