//! Ports of the small `lib/buildeventstream` and `lib/runtime` JUnit tests:
//! `LocalFilesArtifactUploaderTest`, `PathConverterTest`, `LastBuildEventTest` and
//! `BuildEventStreamerUtilsTest`.

/// Port of `buildeventstream/LocalFilesArtifactUploaderTest.java`.
mod local_files_artifact_uploader_test {
    use std::collections::HashMap;
    use std::sync::Arc;

    use blaze_bep::buildeventstream::{
        BuildEventArtifactUploader, LocalFile, LocalFileType, LocalFilesArtifactUploader,
    };
    use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};

    // PORT: Java uses an `InMemoryFileSystem`, which is not ported. No test touches the file
    // system (the local uploader only looks at the `LocalFileType`), so a Unix file system path
    // that is never created stands in for it.
    fn file_system() -> FileSystemRef {
        Arc::new(UnixFileSystem::new(DigestHashFunction::Sha256))
    }

    fn upload_one(path: &Path, type_: LocalFileType) -> Option<String> {
        let artifact_uploader = LocalFilesArtifactUploader;
        let mut files = HashMap::new();
        files.insert(
            path.clone(),
            LocalFile::new(path.clone(), type_, /* artifact_metadata= */ None),
        );
        let path_converter = artifact_uploader.upload(files).get().unwrap();
        path_converter.apply(path)
    }

    #[test]
    fn test_file() {
        let file = Path::create_from_str("/test", &file_system());
        // No need to create the file since LocalFileType.OUTPUT_FILE skips the filesystem check.
        assert_eq!(
            upload_one(&file, LocalFileType::OutputFile),
            Some("file:///test".to_string())
        );
    }

    #[test]
    fn test_directory_not_uploaded() {
        let file = Path::create_from_str("/test", &file_system());
        // No need to create the file since LocalFileType.OUTPUT_DIRECTORY skips the filesystem
        // check.
        assert_eq!(upload_one(&file, LocalFileType::OutputDirectory), None);
    }

    #[test]
    fn test_symlink_not_uploaded() {
        let file = Path::create_from_str("/test", &file_system());
        // No need to create the file since LocalFileType.OUTPUT_FILE skips the filesystem check.
        assert_eq!(upload_one(&file, LocalFileType::OutputSymlink), None);
    }

    #[test]
    fn test_unknown_uploaded_if_file() {
        let file = Path::create_from_str("/test", &file_system());
        assert_eq!(
            upload_one(&file, LocalFileType::OutputFile),
            Some("file:///test".to_string())
        );
    }

    #[test]
    fn test_unknown_not_uploaded_if_directory() {
        let dir = Path::create_from_str("/test", &file_system());
        assert_eq!(upload_one(&dir, LocalFileType::OutputDirectory), None);
    }

    #[test]
    fn test_unknown_not_uploaded_if_symlink() {
        let dir = Path::create_from_str("/test", &file_system());
        assert_eq!(upload_one(&dir, LocalFileType::OutputSymlink), None);
    }
}

/// Port of `buildeventstream/PathConverterTest.java`.
mod path_converter_test {
    use blaze_bep::buildeventstream::FileUriPathConverter;

    #[test]
    fn test_path_to_uri_string() {
        assert_eq!(
            FileUriPathConverter::path_to_uri_string("/foo/bar"),
            "file:///foo/bar"
        );
    }

    #[test]
    fn test_w_indows_path_with_whitespace() {
        // See https://blogs.msdn.microsoft.com/ie/2006/12/06/file-uris-in-windows/
        assert_eq!(
            FileUriPathConverter::path_to_uri_string("C:/Temp/Foo Bar.txt"),
            "file:///C:/Temp/Foo%20Bar.txt"
        );
    }

    /// The path is an internal string: its Latin-1 chars are kept, except the ISO controls
    /// (U+0080 to U+009F) and space chars (U+00A0), which are percent-encoded as their UTF-8
    /// bytes. Expected value from `new URI("file", "", p, null, null)` on the JDK.
    #[test]
    fn path_to_uri_string_of_internal_string() {
        let mut path = blaze_util::string_encoding::latin1_decode("/a/\u{e9}\u{1F437}".as_bytes())
            .into_owned();
        path.push_str("\u{ff} \u{c3}\u{a0}");
        assert_eq!(
            FileUriPathConverter::path_to_uri_string(&path),
            "file:///a/\u{c3}\u{a9}\u{f0}%C2%9F%C2%90\u{b7}\u{ff}%20\u{c3}%C2%A0"
        );
    }
}

/// Port of `buildeventstream/LastBuildEventTest.java`.
mod last_build_event_test {
    use std::sync::Arc;

    use blaze_bep::buildeventstream::{
        BuildEvent, BuildEventContext, Interrupted, LastBuildEvent, LocalFile, LocalFileType,
    };
    use blaze_protos::build_event_stream::{self as bes, BuildEventId};
    use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};

    #[derive(Debug)]
    struct EventWithLocalFile {
        local_file: LocalFile,
    }

    impl BuildEvent for EventWithLocalFile {
        fn get_event_id(&self) -> BuildEventId {
            // Java returns null.
            BuildEventId::default()
        }

        fn get_children_events(&self) -> Vec<BuildEventId> {
            Vec::new()
        }

        fn referenced_local_files(&self) -> Vec<LocalFile> {
            vec![self.local_file.clone()]
        }

        fn as_stream_proto(
            &self,
            _context: &dyn BuildEventContext,
        ) -> Result<bes::BuildEvent, Interrupted> {
            // Java returns null; the test never calls it.
            unreachable!("asStreamProto is not called by this test")
        }
    }

    #[test]
    fn test_forwards_referenced_local_files_call() {
        let fs: FileSystemRef = Arc::new(UnixFileSystem::new(DigestHashFunction::Sha256));
        let local_file = LocalFile::new(
            Path::create_from_str("/some/file", &fs),
            LocalFileType::FailedTestOutput,
            /* artifact_metadata= */ None,
        );
        let event = LastBuildEvent::new(Arc::new(EventWithLocalFile {
            local_file: local_file.clone(),
        }));
        assert_eq!(event.referenced_local_files(), vec![local_file]);
    }
}

/// Port of `runtime/BuildEventStreamerUtilsTest.java`.
mod build_event_streamer_utils_test {
    use blaze_exec::analysis::test::test_attempt::bep_status;
    use blaze_protos::blaze::BlazeTestStatus;
    use blaze_protos::build_event_stream::TestStatus;

    /// `@TestParameter BlazeTestStatus status`: every value of the enum.
    const ALL_BLAZE_TEST_STATUSES: [BlazeTestStatus; 9] = [
        BlazeTestStatus::NoStatus,
        BlazeTestStatus::Passed,
        BlazeTestStatus::Flaky,
        BlazeTestStatus::Timeout,
        BlazeTestStatus::Failed,
        BlazeTestStatus::Incomplete,
        BlazeTestStatus::RemoteFailure,
        BlazeTestStatus::FailedToBuild,
        BlazeTestStatus::BlazeHaltedBeforeTesting,
    ];

    #[test]
    fn all_values_convert_to_real_status() {
        for status in ALL_BLAZE_TEST_STATUSES {
            let bep_status = bep_status(status);
            if status == BlazeTestStatus::NoStatus {
                assert_eq!(bep_status, TestStatus::NoStatus, "{status:?}");
            } else {
                assert_ne!(bep_status, TestStatus::NoStatus, "{status:?}");
            }
        }
    }
}
