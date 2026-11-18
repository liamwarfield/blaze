//! Port of `src/test/java/com/google/devtools/build/lib/shell/ToTruncatedStringTest.java`.

use blaze_shell::LogUtil;

#[test]
fn test_truncating_null_yields_empty_string() {
    assert!(LogUtil::to_truncated_string(None).is_empty());
}

#[test]
fn test_truncating_empty_array_yields_empty_string() {
    assert!(LogUtil::to_truncated_string(Some(&[])).is_empty());
}

#[test]
fn test_truncating_sample_array_yields_truncated_string() {
    let sample_input = "Well, there could be a lot of output, but we want \
        to produce a useful log. A log is useful if it contains the \
        interesting information (like what the command was), and maybe \
        some of the output. However, too much is too much, so we just \
        cut it after 150 bytes ...";
    let expected_output = "Well, there could be a lot of output, but we \
        want to produce a useful log. A log is useful if it contains \
        the interesting information (like what the c[... truncated. \
        original size was 261 bytes.]";
    assert_eq!(
        LogUtil::to_truncated_string(Some(sample_input.as_bytes())),
        expected_output
    );
}

#[test]
fn test_truncating_hello_world_yields_hello_world() {
    let hello_world = "Hello, world.";
    assert_eq!(
        LogUtil::to_truncated_string(Some(hello_world.as_bytes())),
        hello_world
    );
}
