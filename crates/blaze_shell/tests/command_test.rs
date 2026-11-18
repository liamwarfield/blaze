//! Port of `src/test/java/com/google/devtools/build/lib/shell/CommandTest.java`.

mod common;

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;
use parking_lot::Mutex;

use blaze_shell::{
    AbnormalTerminationException, Command, CommandException, CommandExecutionError, CommandResult,
    JavaFile, OutputStream,
};

fn empty() -> IndexMap<String, String> {
    IndexMap::new()
}

fn cmd(args: &[&str]) -> Command {
    Command::new(common::strings(args), &empty())
}

fn command_exception(r: Result<CommandResult, CommandExecutionError>) -> CommandException {
    match r {
        Err(CommandExecutionError::Command(e)) => e,
        Err(e) => panic!("unexpected {e}"),
        Ok(_) => panic!("expected a CommandException"),
    }
}

fn abnormal(e: &CommandException) -> &AbnormalTerminationException {
    e.as_abnormal_termination()
        .expect("expected an AbnormalTerminationException")
}

fn shared_buffer() -> (Arc<Mutex<Vec<u8>>>, OutputStream) {
    let b = Arc::new(Mutex::new(Vec::new()));
    (b.clone(), b)
}

#[test]
#[ignore = "null arguments are unrepresentable in Rust; the empty-command-line check is covered by test_empty_command_line"]
fn test_illegal_args() {}

#[test]
fn test_empty_command_line() {
    let msg = common::panic_message(|| Command::new(Vec::new(), &empty()));
    assert_eq!(
        msg,
        "IllegalArgumentException: cannot run an empty command line"
    );
}

#[test]
fn test_getters() {
    let working_dir = JavaFile::new(".");
    let env = common::map(&[("foo", "bar")]);
    let args = common::strings(&["command"]);
    let command = Command::with_env(
        args.clone(),
        Some(env.clone()),
        Some(working_dir.clone()),
        &empty(),
    );
    assert_eq!(command.get_arguments(), args);
    for (key, value) in &env {
        assert_eq!(command.get_environment().unwrap().get(key), Some(value));
    }
    assert_eq!(command.get_working_directory(), Some(&working_dir));
}

#[test]
fn test_simple_command() {
    let result = cmd(&["ls"]).execute().unwrap();
    assert!(result.termination_status().success());
    assert!(result.get_stderr().is_empty());
    assert!(!result.get_stdout().is_empty());
}

#[test]
fn test_arguments() {
    check_success(&cmd(&["echo", "foo"]).execute().unwrap(), "foo\n");
}

#[test]
fn test_non_empty_environment() {
    let env = common::map(&[("FOO", "abc"), ("BAR", "def")]);
    let command = Command::with_env(
        common::strings(&["/bin/sh", "-c", "echo $FOO $BAR"]),
        Some(env),
        None,
        &common::map(&[("FOO", "not abc"), ("BAR", "not def")]),
    );
    check_success(&command.execute().unwrap(), "abc def\n");
}

#[test]
fn test_empty_environment() {
    let command = Command::with_env(
        common::strings(&["/bin/sh", "-c", "echo $TZ"]),
        Some(empty()),
        None,
        &common::map(&[("TZ", "not empty")]),
    );
    check_success(&command.execute().unwrap(), "\n");
}

#[test]
fn test_inherited_environment() {
    let command = Command::with_env(
        common::strings(&["/bin/sh", "-c", "echo $TZ"]),
        None,
        None,
        &common::map(&[("TZ", "not empty")]),
    );
    check_success(&command.execute().unwrap(), "not empty\n");
}

#[test]
fn test_working_dir() {
    let command = Command::with_env(
        common::strings(&["pwd"]),
        None,
        Some(JavaFile::new("/")),
        &empty(),
    );
    check_success(&command.execute().unwrap(), "/\n");
}

#[test]
fn test_stdin() {
    let command = cmd(&["grep", "bar"]);
    let mut input: &[u8] = b"foobarbaz";
    check_success(
        &command
            .execute_async_with_input(&mut input, Command::KILL_SUBPROCESS_ON_INTERRUPT)
            .unwrap()
            .get()
            .unwrap(),
        "foobarbaz\n",
    );
}

#[test]
fn test_raw_command() {
    let result = cmd(&["perl", "-e", "print 'a'x100000"]).execute().unwrap();
    assert!(result.termination_status().success());
    assert!(result.get_stderr().is_empty());
    assert!(!result.get_stdout().is_empty());
}

#[test]
fn test_raw_command_with_dir() {
    let command = Command::with_env(
        common::strings(&["pwd"]),
        None,
        Some(JavaFile::new("/")),
        &empty(),
    );
    check_success(&command.execute().unwrap(), "/\n");
}

#[test]
fn test_huge_output() {
    let result = cmd(&["perl", "-e", "print 'a'x100000"]).execute().unwrap();
    assert!(result.termination_status().success());
    assert!(result.get_stderr().is_empty());
    assert_eq!(result.get_stdout().len(), 100000);
}

#[test]
fn test_no_streaming_input_for_cat() {
    let command = cmd(&["/bin/cat"]);
    let mut empty_input: &[u8] = &[];
    let (out_buf, out) = shared_buffer();
    let (err_buf, err) = shared_buffer();
    let result = command
        .execute_async_with_input_streaming(
            &mut empty_input,
            out,
            err,
            Command::KILL_SUBPROCESS_ON_INTERRUPT,
        )
        .unwrap()
        .get()
        .unwrap();
    assert!(result.termination_status().success());
    assert!(out_buf.lock().is_empty());
    assert!(err_buf.lock().is_empty());
}

#[test]
fn test_no_input_for_cat() {
    let result = cmd(&["/bin/cat"]).execute().unwrap();
    assert!(result.termination_status().success());
    assert!(result.get_stdout().is_empty());
    assert!(result.get_stderr().is_empty());
}

#[test]
fn test_provided_output_stream_captures_hello_world() {
    let hello_world = "Hello, world.";
    let command = cmd(&["/bin/echo", hello_world]);
    let (out_buf, out) = shared_buffer();
    let (err_buf, err) = shared_buffer();
    command.execute_streaming(out, err).unwrap();
    assert_eq!(
        String::from_utf8(out_buf.lock().clone()).unwrap(),
        format!("{hello_world}\n")
    );
    assert!(err_buf.lock().is_empty());
}

#[test]
fn test_asynchronous() {
    let dir = std::env::temp_dir().join(format!("blaze_shell_async_{}", std::process::id()));
    let temp_file = dir.to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&temp_file);
    let command = cmd(&["touch", &temp_file]);
    let mut result = command.execute_async().unwrap();
    result.get().unwrap();
    assert!(std::path::Path::new(&temp_file).exists());
    assert!(result.is_done());
    let _ = std::fs::remove_file(&temp_file);
}

#[test]
fn test_asynchronous_with_output_streams() {
    let hello_world = "Hello, world.";
    let command = cmd(&["/bin/echo", hello_world]);
    let mut empty_input: &[u8] = &[];
    let (out_buf, out) = shared_buffer();
    let (err_buf, err) = shared_buffer();
    let mut result = command
        .execute_async_with_input_streaming(&mut empty_input, out, err, false)
        .unwrap();
    result.get().unwrap(); // Make sure the process actually finished
    assert_eq!(
        String::from_utf8(out_buf.lock().clone()).unwrap(),
        format!("{hello_world}\n")
    );
    assert!(err_buf.lock().is_empty());
}

#[test]
fn test_timeout() {
    // Sleep for 3 seconds, but timeout after 1 second.
    let command = Command::with_timeout(
        common::strings(&["sleep", "3"]),
        None,
        None,
        Duration::from_secs(1),
        &empty(),
    );
    let e = command_exception(command.execute());
    let ate = abnormal(&e);
    check_command_elements(&e, &["sleep", "3"]);
    assert!(!ate.get_result().termination_status().success());
}

#[test]
fn test_timeout_doesnt_fire() {
    let command = Command::with_timeout(
        common::strings(&["cat"]),
        None,
        None,
        Duration::from_secs(2),
        &empty(),
    );
    let mut input: &[u8] = b"Hi!";
    command
        .execute_async_with_input(&mut input, Command::KILL_SUBPROCESS_ON_INTERRUPT)
        .unwrap()
        .get()
        .unwrap();
}

#[test]
fn test_command_does_not_exist() {
    let command = cmd(&["thisisnotreal"]);
    let e = command_exception(command.execute());
    assert!(matches!(e, CommandException::ExecFailed(_)));
    check_command_elements(&e, &["thisisnotreal"]);
    // The JDK's text for a failed exec.
    assert_eq!(
        e.get_message(),
        "java.io.IOException: Cannot run program \"thisisnotreal\": error=2, No such file or directory"
    );
}

#[test]
fn test_no_such_command() {
    let command = cmd(&["thisisnotreal"]);
    let e = command_exception(command.execute());
    assert!(matches!(e, CommandException::ExecFailed(_)));
}

#[test]
fn test_exit_codes() {
    // 0 => success
    {
        let result = cmd(&["/bin/sh", "-c", "exit 0"]).execute().unwrap();
        let status = result.termination_status();
        assert!(status.success());
        assert!(status.exited());
        assert_eq!(status.get_exit_code(), 0);
    }

    // Every exit value in range [1-255] is reported as such (except [129-191], which map to
    // signals).
    for exit in [1, 2, 3, 127, 128, 192, 255] {
        let script = format!("exit {exit}");
        let e = command_exception(cmd(&["/bin/sh", "-c", &script]).execute());
        assert!(
            matches!(e, CommandException::BadExitStatus(_)),
            "Should have exited with status {exit}"
        );
        assert_eq!(
            e.get_message(),
            format!("Process exited with status {exit}")
        );
        check_command_elements(&e, &["/bin/sh", "-c", &script]);
        let status = abnormal(&e).get_result().termination_status();
        assert!(!status.success());
        assert!(status.exited());
        assert_eq!(status.get_exit_code(), exit);
        assert_eq!(status.to_short_string(), format!("Exit {exit}"));
    }

    // negative exit values are modulo 256:
    for exit in [-1, -2, -3] {
        let expected = 256 + exit;
        let script = format!("exit {exit}");
        let e = command_exception(cmd(&["/bin/bash", "-c", &script]).execute());
        assert!(
            matches!(e, CommandException::BadExitStatus(_)),
            "Should have exited with status {expected}"
        );
        assert_eq!(
            e.get_message(),
            format!("Process exited with status {expected}")
        );
        check_command_elements(&e, &["/bin/bash", "-c", &script]);
        let status = abnormal(&e).get_result().termination_status();
        assert!(!status.success());
        assert!(status.exited());
        assert_eq!(status.get_exit_code(), expected);
        assert_eq!(status.to_short_string(), format!("Exit {expected}"));
    }
}

#[test]
fn test_failed_with_signal() {
    // SIGHUP, SIGINT, SIGKILL, SIGTERM
    for signal in [1, 2, 9, 15] {
        // PORT: Java runs killmyself.cc; a shell that kills itself with the signal is the same.
        let script = format!("kill -{signal} $$");
        let e = command_exception(cmd(&["/bin/sh", "-c", &script]).execute());
        assert!(
            matches!(e, CommandException::AbnormalTermination(_)),
            "Expected signal {signal}"
        );
        assert_eq!(
            e.get_message(),
            format!("Process terminated by signal {signal}")
        );
        check_command_elements(&e, &["/bin/sh", "-c", &script]);
        let status = abnormal(&e).get_result().termination_status();
        assert!(!status.success());
        assert!(!status.exited());
        assert_eq!(status.get_terminating_signal(), signal);

        match signal {
            1 => assert_eq!(status.to_short_string(), "Hangup"),
            2 => assert_eq!(status.to_short_string(), "Interrupt"),
            9 => assert_eq!(status.to_short_string(), "Killed"),
            15 => assert_eq!(status.to_short_string(), "Terminated"),
            _ => {}
        }
    }
}

/// An input stream that writes an unbounded amount of zeros.
struct Zeros;

impl Read for Zeros {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        buf.fill(0);
        Ok(buf.len())
    }
}

#[test]
fn test_only_reads_partial_input() {
    // -c == --bytes, but -c also works on Darwin.
    let command = cmd(&["head", "-c", "500"]);
    let (_, out) = shared_buffer();
    let result = command
        .execute_async_with_input_streaming(
            &mut Zeros,
            out.clone(),
            out,
            Command::KILL_SUBPROCESS_ON_INTERRUPT,
        )
        .unwrap()
        .get()
        .unwrap();
    assert!(result.termination_status().success());
}

/// Records when each flush() occurs.
struct FlushRecorder {
    count: usize,
    flushed: Arc<Mutex<[bool; 8]>>,
}

impl Write for FlushRecorder {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.count += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushed.lock()[self.count] = true;
        Ok(())
    }
}

#[test]
fn test_flushing() {
    let command = cmd(&["/bin/bash", "-c", "echo -n Foo; sleep 0.1; echo Bar"]);
    // We run this command, passing in a special output stream that records when each flush()
    // occurs. We test that a flush occurs after writing "Foo" and that another flush occurs after
    // writing "Bar\n".
    let flushed = Arc::new(Mutex::new([false; 8]));
    let out: OutputStream = Arc::new(Mutex::new(FlushRecorder {
        count: 0,
        flushed: flushed.clone(),
    }));
    let err: OutputStream = Arc::new(Mutex::new(io::stderr()));
    command.execute_streaming(out, err).unwrap();
    let flushed = *flushed.lock();
    assert!(!flushed[0]);
    assert!(!flushed[1]); // 'F'
    assert!(!flushed[2]); // 'o'
    assert!(flushed[3]); // 'o'   <- expect flush here.
    assert!(!flushed[4]); // 'B'
    assert!(!flushed[5]); // 'a'
    assert!(!flushed[6]); // 'r'
    assert!(flushed[7]); // '\n'
}

/// An output stream whose writes fail with an `IOException` without a message.
struct Failing;

impl Write for Failing {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::other(""))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn test_output_stream_throws_exception() {
    let out: OutputStream = Arc::new(Mutex::new(Failing));
    let command = cmd(&["/bin/echo", "foo"]);
    let e = command_exception(command.execute_streaming(out.clone(), out));
    abnormal(&e);
    check_command_elements(&e, &["/bin/echo", "foo"]);
    assert_eq!(e.get_message(), "java.io.IOException");
}

#[test]
fn test_output_stream_throws_exception_and_command_fails() {
    let out: OutputStream = Arc::new(Mutex::new(Failing));
    let command = cmd(&["cat", "/dev/thisisnotreal"]);
    let e = command_exception(command.execute_streaming(out.clone(), out));
    check_command_elements(&e, &["cat", "/dev/thisisnotreal"]);
    let status = abnormal(&e).get_result().termination_status();
    // Subprocess either gets a SIGPIPE trying to write to our output stream, or it exits with
    // failure. Both are observed, nondetermistically.
    assert!(if status.exited() {
        status.get_exit_code() == 1
    } else {
        status.get_terminating_signal() == 13
    });
    assert!(
        e.get_message()
            .ends_with("also encountered an error while attempting to retrieve output"),
        "{}",
        e.get_message()
    );
}

fn check_command_elements(e: &CommandException, expected: &[&str]) {
    assert_eq!(e.get_command().get_arguments(), common::strings(expected));
}

fn check_success(result: &CommandResult, expected_output: &str) {
    assert!(result.termination_status().success());
    assert!(result.get_stderr().is_empty());
    assert_eq!(
        String::from_utf8(result.get_stdout()).unwrap(),
        expected_output
    );
}

#[test]
fn test_relative_path() {
    let command = Command::with_env(
        common::strings(&["relative/path/to/binary"]),
        Some(empty()),
        Some(JavaFile::new("/working/directory")),
        &empty(),
    );
    assert_eq!(
        command.get_arguments()[0],
        "/working/directory/relative/path/to/binary"
    );
}
