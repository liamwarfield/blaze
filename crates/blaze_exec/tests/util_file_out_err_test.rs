//! Port of `src/test/java/com/google/devtools/build/lib/util/io/FileOutErrTest.java`.
//!
//! PORT: Java uses an `InMemoryFileSystem` (not ported); these tests use the Unix file system in
//! a temporary directory. The permission tests need a user other than root.

use blaze_exec::util::file_out_err::FileRecordingOutputStream;
use blaze_vfs::{DigestHashFunction, FileSystemExt, Path, UnixFileSystem};
use parking_lot::Mutex;
use std::io::Write;

struct Fixture {
    _dir: tempfile::TempDir,
    root: Path,
}

fn set_up() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let root = fs.get_path(dir.path().to_str().unwrap());
    Fixture { _dir: dir, root }
}

fn new_file_recording_output_stream(f: &Fixture, path: &str) -> FileRecordingOutputStream {
    FileRecordingOutputStream::new(Some(f.root.get_relative_str(path)))
}

fn dump(os: &FileRecordingOutputStream) -> Vec<u8> {
    let recorder = Mutex::new(Vec::new());
    os.dump_out(&recorder);
    recorder.into_inner()
}

#[test]
fn test_file_recording_output_stream_does_not_exist_by_default() {
    let f = set_up();
    let os = new_file_recording_output_stream(&f, "some-file.txt");

    assert!(!os.has_recorded_output());
    assert!(os.get_recorded_output().is_empty());
    assert_eq!(os.get_recorded_output_size().unwrap(), 0);

    assert!(dump(&os).is_empty());

    // Existence and error checks must come last to ensure previous calls have no side-effects.
    assert!(!os.get_file_unsafe().unwrap().exists());
    assert!(!os.had_error());
}

#[test]
fn test_file_recording_output_stream_create_out_of_band_as_empty() {
    let f = set_up();
    let os = new_file_recording_output_stream(&f, "some-file.txt");
    let path = os.get_file().unwrap();
    drop(path.get_output_stream().unwrap());

    assert!(!os.has_recorded_output());
    assert!(os.get_recorded_output().is_empty());
    assert_eq!(os.get_recorded_output_size().unwrap(), 0);

    assert!(dump(&os).is_empty());

    // Existence and error checks must come last to ensure previous calls have no side-effects.
    assert!(os.get_file_unsafe().unwrap().exists());
    assert!(!os.had_error());
}

#[test]
fn test_file_recording_output_stream_create_out_of_band_with_contents() {
    let f = set_up();
    let os = new_file_recording_output_stream(&f, "some-file.txt");
    let path = os.get_file().unwrap();
    let data = b"12345";
    {
        let mut writer = path.get_output_stream().unwrap();
        writer.write_all(data).unwrap();
    }

    assert!(os.has_recorded_output());
    assert_eq!(os.get_recorded_output(), data);
    assert_eq!(os.get_recorded_output_size().unwrap(), data.len() as i64);

    assert_eq!(dump(&os), data);

    // Existence and error checks must come last to ensure previous calls have no side-effects.
    assert!(os.get_file_unsafe().unwrap().exists());
    assert!(!os.had_error());
}

#[test]
fn test_file_recording_output_stream_write() {
    let f = set_up();
    let os = new_file_recording_output_stream(&f, "some-file.txt");
    let data = b"12345";
    let result = os.write(data);
    os.close().unwrap();
    result.unwrap();

    assert!(os.has_recorded_output());
    assert_eq!(os.get_recorded_output(), data);
    assert_eq!(os.get_recorded_output_size().unwrap(), data.len() as i64);

    assert_eq!(dump(&os), data);

    // Existence and error checks must come last to ensure previous calls have no side-effects.
    assert!(os.get_file_unsafe().unwrap().exists());
    assert!(!os.had_error());
}

#[test]
fn test_file_recording_output_stream_clear_after_creation() {
    let f = set_up();
    let os = new_file_recording_output_stream(&f, "some-file.txt");
    let path = os.get_file().unwrap().clone();
    let result = os.write(b"12345");
    os.close().unwrap();
    result.unwrap();

    assert!(path.exists());
    assert!(os.get_recorded_output_size().unwrap() > 0);
    assert!(!os.had_error());
    os.clear().unwrap();
    assert!(!path.exists());
    assert_eq!(os.get_recorded_output_size().unwrap(), 0);
    assert!(!os.had_error());
}

#[test]
fn test_file_recording_output_stream_error_during_size_check() {
    let f = set_up();
    let dir = f.root.get_relative_str("dir");
    dir.create_directory().unwrap();
    let os = new_file_recording_output_stream(&f, "dir/some-file.txt");
    let path = os.get_file().unwrap();
    drop(path.get_output_stream().unwrap());
    dir.set_readable(false).unwrap();
    dir.set_executable(false).unwrap();

    let expected = os.get_recorded_output_size().unwrap_err();
    assert!(
        expected.to_string().contains("Permission denied"),
        "{expected}"
    );

    let recorded = dump(&os);
    assert!(
        String::from_utf8_lossy(&recorded).contains("Permission denied"),
        "{}",
        String::from_utf8_lossy(&recorded)
    );

    // Restore directory permissions so our existence check works.
    dir.set_readable(true).unwrap();
    dir.set_executable(true).unwrap();
    // Existence and error checks must come last to ensure previous calls have no side-effects.
    assert!(os.get_file_unsafe().unwrap().exists());
    assert!(os.had_error());
}

#[test]
fn test_file_recording_output_stream_error_during_read() {
    let f = set_up();
    let os = new_file_recording_output_stream(&f, "some-file.txt");
    let path = os.get_file().unwrap();
    drop(path.get_output_stream().unwrap());
    path.set_readable(false).unwrap();

    let error = blaze_util::string_encoding::latin1_decode(&os.get_recorded_output()).into_owned();
    // The error message comes from the system so we cannot be too specific about what we look for.
    assert!(error.contains("Permission denied"), "{error}");
    assert!(os.get_recorded_output_size().unwrap() > 0);

    assert_eq!(
        dump(&os),
        blaze_util::string_encoding::latin1_encode_lossy(&format!("{error}\n{error}")).as_ref()
    );

    // Existence and error checks must come last to ensure previous calls have no side-effects.
    assert!(os.get_file_unsafe().unwrap().exists());
    assert!(os.had_error());
    path.set_readable(true).unwrap();
}
