//! Port of `src/test/java/com/google/devtools/build/lib/exec/TestLogHelperTest.java`.

use blaze_exec::exec::test_log_helper::{self, HEADER_DELIMITER};
use blaze_optdefs::command::execution_options::TestOutputFormat;
use blaze_vfs::{DigestHashFunction, FileSystemExt, FileSystemRef, Path, UnixFileSystem};

struct Scratch {
    _tmp: tempfile::TempDir,
    dir: Path,
}

impl Scratch {
    fn new() -> Scratch {
        let tmp = tempfile::tempdir().unwrap();
        let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
        let dir = fs.get_path(tmp.path().to_str().unwrap());
        Scratch { _tmp: tmp, dir }
    }

    /// `scratch.file(path, lines...)`: each line followed by a newline.
    fn file_lines(&self, name: &str, lines: &[&str]) -> Path {
        let mut content = String::new();
        for l in lines {
            content.push_str(l);
            content.push('\n');
        }
        self.file_bytes(name, content.as_bytes())
    }

    /// `scratch.file(path, bytes)`.
    fn file_bytes(&self, name: &str, bytes: &[u8]) -> Path {
        let p = self.dir.get_child(name);
        std::fs::write(p.get_path_string(), bytes).unwrap();
        p
    }
}

fn get_test_log(path: &Path, name: &str) -> String {
    let mut output = Vec::new();
    test_log_helper::write_test_log(
        path,
        name,
        &mut output,
        /* max_test_output_bytes= */ -1,
    )
    .unwrap();
    blaze_protos::java::decode_latin1(&output)
}

#[test]
fn test_should_output_test_log() {
    assert!(test_log_helper::should_output_test_log(
        TestOutputFormat::ALL,
        true
    ));
    assert!(test_log_helper::should_output_test_log(
        TestOutputFormat::ALL,
        false
    ));
    assert!(test_log_helper::should_output_test_log(
        TestOutputFormat::ERRORS,
        false
    ));
    assert!(!test_log_helper::should_output_test_log(
        TestOutputFormat::ERRORS,
        true
    ));
}

#[test]
fn test_format_empty_test_log() {
    let s = Scratch::new();
    let log_path = s.file_lines("test.log", &[""]);
    let result = get_test_log(&log_path, "testFormatEmptyTestLog");
    assert_eq!(
        result,
        "==================== Test output for testFormatEmptyTestLog:\n\
         \n\
         ================================================================================\n"
    );
}

#[test]
fn test_format_test_log_with_header() {
    let s = Scratch::new();
    let log_path = s.file_lines("test.log", &["Header", HEADER_DELIMITER, "Empty line"]);
    let result = get_test_log(&log_path, "testFormatTestLogWithHeader");
    assert_eq!(
        result,
        "==================== Test output for testFormatTestLogWithHeader:\n\
         Empty line\n\
         ================================================================================\n"
    );
}

#[test]
fn test_format_test_log_with_no_header() {
    let s = Scratch::new();
    let log_path = s.file_lines("test.log", &["Line 1", "Line 2"]);
    let result = get_test_log(&log_path, "testFormatTestLogWithNoHeader");
    assert_eq!(
        result,
        "==================== Test output for testFormatTestLogWithNoHeader:\n\
         Line 1\n\
         Line 2\n\
         ================================================================================\n"
    );
}

#[test]
fn test_format_test_log_too_large() {
    let s = Scratch::new();
    let log_path = s.file_bytes("test.log", &[0u8; 1000]);

    let mut bytes_out = Vec::new();
    test_log_helper::write_test_log(&log_path, "myTest", &mut bytes_out, 10).unwrap();
    assert_eq!(
        blaze_protos::java::decode_latin1(&bytes_out),
        "==================== Test output for myTest:\n\
         Test log too large (1000 > 10), skipping...\n\
         ================================================================================\n"
    );
}

#[test]
fn test_format_test_log0_byte_max0_byte_file_prints_nothing() {
    let s = Scratch::new();
    let log_path = s.file_bytes("test.log", &[]);

    let mut bytes_out = Vec::new();
    test_log_helper::write_test_log(&log_path, "myTest", &mut bytes_out, 0).unwrap();
    assert_eq!(
        blaze_protos::java::decode_latin1(&bytes_out),
        "==================== Test output for myTest:\n\
         ================================================================================\n"
    );
}

#[test]
fn test_format_test_log_max_bytes_includes_header() {
    let s = Scratch::new();
    let log_path = s.file_bytes("test.log", HEADER_DELIMITER.as_bytes());

    let test_log_header_length = HEADER_DELIMITER.len() as i32;
    let mut bytes_out = Vec::new();
    test_log_helper::write_test_log(
        &log_path,
        "myTest",
        &mut bytes_out,
        test_log_header_length - 1,
    )
    .unwrap();
    assert_eq!(
        blaze_protos::java::decode_latin1(&bytes_out),
        format!(
            "==================== Test output for myTest:\n\
             Test log too large ({} > {}), skipping...\n\
             ================================================================================\n",
            test_log_header_length,
            test_log_header_length - 1
        )
    );
}
