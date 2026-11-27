//! Port of `src/test/java/com/google/devtools/build/lib/runtime/LocalInstrumentationOutputTest.java`.
//!
//! PORT: Java's `InMemoryFileSystem` is not ported (see `blaze_vfs::in_memory_fs`); the tests
//! use the Unix file system in a temporary directory, with Java's absolute paths taken relative
//! to it.

use std::sync::Arc;

use blaze_buildtool::buildtool::build_result::{
    BuildToolLogCollection, LocalFile, LocalFileCompression, LocalFileType, LogFileEntry,
};
use blaze_runtime::profiler_init::LocalInstrumentationOutput;
use blaze_vfs::file_system::FileSystemRef;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};

/// `new InMemoryFileSystem(DigestHashFunction.SHA256).getPath(path)`, inside `dir`.
fn get_path(dir: &tempfile::TempDir, path: &str) -> Path {
    let fs: FileSystemRef = Arc::new(UnixFileSystem::new(DigestHashFunction::Sha256));
    let root = dir.path().to_str().expect("UTF-8 temporary directory");
    Path::create(PathFragment::create(&format!("{root}{path}")), &fs)
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/LocalInstrumentationOutputTest.java:44 LocalInstrumentationOutputTest.testLocalInstrumentationOutputBuilder_failToBuildWhenMissingName
#[test]
#[ignore = "LocalInstrumentationOutput.Builder is not ported: the callers pass every field to LocalInstrumentationOutput::new, whose name and path are not optional (the checkNotNull messages of Builder.build have no counterpart)"]
fn test_local_instrumentation_output_builder_fail_to_build_when_missing_name() {}

// Java: src/test/java/com/google/devtools/build/lib/runtime/LocalInstrumentationOutputTest.java:57 LocalInstrumentationOutputTest.testLocalInstrumentationOutputBuilder_failToBuildWhenMissingPath
#[test]
#[ignore = "LocalInstrumentationOutput.Builder is not ported: the callers pass every field to LocalInstrumentationOutput::new, whose name and path are not optional (the checkNotNull messages of Builder.build have no counterpart)"]
fn test_local_instrumentation_output_builder_fail_to_build_when_missing_path() {}

// Java: src/test/java/com/google/devtools/build/lib/runtime/LocalInstrumentationOutputTest.java:67 LocalInstrumentationOutputTest.testLocalInstrumentation_publishNameAndPath
#[test]
fn test_local_instrumentation_publish_name_and_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = get_path(&dir, "/file");
    // localInstrumentationOutputBuilder.setName("local").setPath(path).build()
    let local_instrumentation_output =
        LocalInstrumentationOutput::new("local", path.clone(), None, None, None, false);

    let mut build_tool_log_collection = BuildToolLogCollection::new();
    local_instrumentation_output.publish(&mut build_tool_log_collection);
    build_tool_log_collection.freeze();

    assert_eq!(
        build_tool_log_collection.get_local_files(),
        &[LogFileEntry {
            name: "local".to_string(),
            local_file: LocalFile {
                path,
                type_: LocalFileType::LOG,
                compression: LocalFileCompression::NONE,
            },
        }]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/LocalInstrumentationOutputTest.java:85 LocalInstrumentationOutputTest.testLocalInstrumentation_recursiveCreateParentDirectory
#[test]
fn test_local_instrumentation_recursive_create_parent_directory() {
    // @TestParameter boolean enableRecursiveCreateDirectory
    for enable_recursive_create_directory in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = get_path(&dir, "/subdir1/subdir2/file");
        assert!(!path.exists());

        if enable_recursive_create_directory {
            let local_instrumentation_output = LocalInstrumentationOutput::new(
                "recursive-dir-output",
                path.clone(),
                None,
                None,
                None,
                /* create_parent= */ true,
            );
            let _unused = local_instrumentation_output
                .create_output_stream()
                .expect("createOutputStream");
            assert!(path.exists());
        } else {
            let local_instrumentation_output = LocalInstrumentationOutput::new(
                "recursive-dir-output",
                path.clone(),
                None,
                None,
                None,
                false,
            );
            // assertThrows("No such file or directory", IOException.class, ...)
            assert!(local_instrumentation_output.create_output_stream().is_err());
        }
    }
}
