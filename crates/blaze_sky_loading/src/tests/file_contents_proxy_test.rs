//! Port of `FileContentsProxyTest`.
// Java: src/test/java/com/google/devtools/build/lib/actions/FileContentsProxyTest.java

use blaze_util::fingerprint::Fingerprint;
use blaze_vfs::FileStatus;

use super::equals_tester;
use crate::actions::file_state_value::FileContentsProxy;

/// `InjectedStat(mtime, ctime, size, nodeId)`: a regular file (`isFile()` true).
fn injected_stat(mtime: i64, ctime: i64, size: i64, node_id: i64) -> FileStatus {
    FileStatus::new(0o100644, mtime, ctime, size, node_id)
}

/// `InjectedStat(ctime, nodeId)`: mtime = ctime, size 0.
fn injected_stat2(ctime: i64, node_id: i64) -> FileStatus {
    injected_stat(ctime, ctime, 0, node_id)
}

#[test]
fn equals_and_hash_code() {
    equals_tester(&[
        vec![
            FileContentsProxy::create(&injected_stat2(1, 2)),
            FileContentsProxy::create(&injected_stat2(1, 2)),
        ],
        vec![FileContentsProxy::create(&injected_stat2(1, 4))],
        vec![FileContentsProxy::create(&injected_stat2(3, 4))],
        vec![FileContentsProxy::create(&injected_stat2(-1, -1))],
    ]);
}

#[test]
fn fingerprint() {
    let p1 = FileContentsProxy::create(&injected_stat(
        /* mtime= */ 1, /* ctime= */ 2, /* size= */ 3, /* node_id= */ 4,
    ));
    let mut fingerprint = Fingerprint::new();
    p1.add_to_fingerprint(&mut fingerprint);
    let mut expected = Fingerprint::new();
    expected.add_long(2).add_long(1).add_long(4);
    assert_eq!(fingerprint.digest_and_reset(), expected.digest_and_reset());
}
