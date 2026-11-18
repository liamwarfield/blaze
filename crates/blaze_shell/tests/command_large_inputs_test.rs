//! Port of `src/test/java/com/google/devtools/build/lib/shell/CommandLargeInputsTest.java`.

mod common;

use std::sync::Arc;

use parking_lot::Mutex;

use blaze_shell::{Command, CommandResult, OutputStream};

/// PORT: Java uses `new Random(0xdeadbeef).nextBytes`; any fixed pseudo-random bytes serve.
fn get_random_bytes() -> Vec<u8> {
    let mut state: u64 = 0xdeadbeef;
    (0..10000)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u8
        })
        .collect()
}

fn get_all_byte_values() -> Vec<u8> {
    // Java: new byte[Byte.MAX_VALUE - Byte.MIN_VALUE] = 255 values, from -128.
    (0..255)
        .map(|i: i32| (i + i8::MIN as i32) as i8 as u8)
        .collect()
}

fn shared_buffer() -> (Arc<Mutex<Vec<u8>>>, OutputStream) {
    let b = Arc::new(Mutex::new(Vec::new()));
    (b.clone(), b)
}

#[test]
fn test_cat_random_binary_to_output_stream() {
    let command = Command::new(common::strings(&["cat"]), &common::getenv());
    let random_bytes = get_random_bytes();
    let mut input: &[u8] = &random_bytes;
    let result = command
        .execute_async_with_input(&mut input, Command::KILL_SUBPROCESS_ON_INTERRUPT)
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(result.termination_status().get_raw_exit_code(), 0);
    assert_eq!(result.get_stdout(), random_bytes);
    assert!(result.get_stderr().is_empty());
}

#[test]
fn test_cat_random_binary_to_error_stream() {
    let command = Command::new(
        common::strings(&["/bin/sh", "-c", "cat >&2"]),
        &common::getenv(),
    );
    let random_bytes = get_random_bytes();
    let mut input: &[u8] = &random_bytes;
    let result = command
        .execute_async_with_input(&mut input, Command::KILL_SUBPROCESS_ON_INTERRUPT)
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(result.termination_status().get_raw_exit_code(), 0);
    assert_eq!(result.get_stderr(), random_bytes);
    assert!(result.get_stdout().is_empty());
}

#[test]
fn test_cat_random_binary_from_input_stream_to_output_stream() {
    let command = Command::new(common::strings(&["cat"]), &common::getenv());
    let (out_buf, out) = shared_buffer();
    let (err_buf, err) = shared_buffer();
    let random_bytes = get_random_bytes();
    let mut input: &[u8] = &random_bytes;
    let result = command
        .execute_async_with_input_streaming(
            &mut input,
            out,
            err,
            Command::KILL_SUBPROCESS_ON_INTERRUPT,
        )
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(result.termination_status().get_raw_exit_code(), 0);
    assert!(err_buf.lock().is_empty());
    assert_eq!(*out_buf.lock(), random_bytes);
    assert_out_and_err_not_available(&result);
}

#[test]
fn test_cat_random_binary_from_input_stream_to_error_stream() {
    let command = Command::new(
        common::strings(&["/bin/sh", "-c", "cat >&2"]),
        &common::getenv(),
    );
    let (out_buf, out) = shared_buffer();
    let (err_buf, err) = shared_buffer();
    let random_bytes = get_random_bytes();
    let mut input: &[u8] = &random_bytes;
    let result = command
        .execute_async_with_input_streaming(
            &mut input,
            out,
            err,
            Command::KILL_SUBPROCESS_ON_INTERRUPT,
        )
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(result.termination_status().get_raw_exit_code(), 0);
    assert!(out_buf.lock().is_empty());
    assert_eq!(*err_buf.lock(), random_bytes);
    assert_out_and_err_not_available(&result);
}

#[test]
fn test_stdout_interleaved_with_std_err() {
    let command = Command::new(
        common::strings(&[
            "/bin/bash",
            "-c",
            "for i in $( seq 0 999); do (echo OUT$i >&1) && (echo ERR$i  >&2); done",
        ]),
        &common::getenv(),
    );
    let (out_buf, out) = shared_buffer();
    let (err_buf, err) = shared_buffer();
    command.execute_streaming(out, err).unwrap();
    let mut expected_out = String::new();
    let mut expected_err = String::new();
    for i in 0..1000 {
        expected_out.push_str(&format!("OUT{i}\n"));
        expected_err.push_str(&format!("ERR{i}\n"));
    }
    assert_eq!(
        String::from_utf8(out_buf.lock().clone()).unwrap(),
        expected_out
    );
    assert_eq!(
        String::from_utf8(err_buf.lock().clone()).unwrap(),
        expected_err
    );
}

fn assert_out_and_err_not_available(result: &CommandResult) {
    let msg = common::panic_message(|| result.get_stdout());
    assert!(msg.starts_with("IllegalStateException"), "{msg}");
    let msg = common::panic_message(|| result.get_stderr());
    assert!(msg.starts_with("IllegalStateException"), "{msg}");
}

#[test]
fn test_cat_all_byte_values() {
    let command = Command::new(common::strings(&["cat"]), &common::getenv());
    let all_byte_values = get_all_byte_values();
    let mut input: &[u8] = &all_byte_values;
    let result = command
        .execute_async_with_input(&mut input, Command::KILL_SUBPROCESS_ON_INTERRUPT)
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(result.termination_status().get_raw_exit_code(), 0);
    assert!(result.get_stderr().is_empty());
    assert_eq!(result.get_stdout(), all_byte_values);
}
