//! Port of src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnStrategyTest.java.

use std::io::Write;

use blaze_exec::sandbox::sandbox_helpers::SandboxInputs;
use blaze_exec::worker::worker_spawn_runner::expand_argument;
use blaze_protos::blaze::worker::WorkRequest;
use blaze_vfs::{DigestHashFunction, FileSystemExt, PathFragment, UnixFileSystem};
use indexmap::IndexMap;

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnStrategyTest.java:44 WorkerSpawnStrategyTest.expandArgumentsPreservesEmptyLines
#[test]
fn expand_arguments_preserves_empty_lines() {
    let folder = tempfile::tempdir().unwrap();
    let flagfile = folder.path().join("flagfile.txt");

    let flags = ["--hello", "", "--world"];

    {
        // PrintWriter.println: the platform line separator.
        let mut pw = std::fs::File::create(&flagfile).unwrap();
        for flag in flags {
            writeln!(pw, "{flag}").unwrap();
        }
    }

    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let path = fs.get_path(flagfile.to_str().unwrap());
    let mut request_builder = WorkRequest::default();
    let inputs = SandboxInputs::new(
        [(PathFragment::create("flagfile.txt"), Some(path))]
            .into_iter()
            .collect(),
        IndexMap::new(),
        IndexMap::new(),
    );
    expand_argument(&inputs, "@flagfile.txt", &mut request_builder).unwrap();

    // containsExactlyElementsIn (any order).
    let mut actual = request_builder.arguments.clone();
    actual.sort();
    let mut expected = flags.to_vec();
    expected.sort();
    assert_eq!(actual, expected);
}
