//! Port of `src/test/java/com/google/devtools/build/lib/shell/InterruptibleTest.java`.
//!
//! Tests of the interaction of thread interruption and Command.execute. These depend on thread
//! timings.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::Duration;

use blaze_shell::thread_interrupt;
use blaze_shell::{Command, CommandExecutionError, ThreadInterrupt};

struct Fixture {
    command: Command,
    tmp_dir: PathBuf,
    interrupter: Option<JoinHandle<()>>,
}

fn start_interrupter(name: &str) -> Fixture {
    thread_interrupt::interrupted(); // side effect: clear interrupted status
    assert!(
        !ThreadInterrupt::current().is_interrupted(),
        "Unexpected interruption!"
    );

    // We interrupt after 1 sec, so this gives us plenty of time for the library to notice the
    // subprocess exit.
    let base = std::env::temp_dir().join(format!(
        "blaze_shell_interruptible_{}_{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    let tmp_dir = base.join("script_outs");
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let script = base.join("script.sh");
    std::fs::write(
        &script,
        format!(
            "echo start\nsleep 20\ntouch {}/endfile\necho end >&2\n",
            tmp_dir.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o777)).unwrap();
    let command = Command::new(
        vec![script.to_string_lossy().into_owned()],
        &common::getenv(),
    );

    // Interrupt main thread after 1 second. Hopefully by then /bin/sleep should be running.
    let main_thread = ThreadInterrupt::current();
    let interrupter = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1000));
        main_thread.interrupt();
    });
    Fixture {
        command,
        tmp_dir,
        interrupter: Some(interrupter),
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(i) = self.interrupter.take() {
            let _ = i.join();
        }
        thread_interrupt::interrupted(); // Clear interrupted status.
    }
}

/// Test that interrupting a thread in an "uninterruptible" Command.execute marks the thread as
/// interrupted, and does not terminate the subprocess.
#[test]
#[ignore = "takes 20 seconds (the script sleeps 20s); run with --ignored"]
fn uninterruptible_command_runs_to_completion() {
    let mut f = start_interrupter("uninterruptible");
    let result = f
        .command
        .execute_async_with_input(
            &mut Command::no_input(),
            Command::CONTINUE_SUBPROCESS_ON_INTERRUPT,
        )
        .unwrap()
        .get()
        .unwrap();
    assert!(result.termination_status().success());
    assert_eq!(String::from_utf8(result.get_stdout()).unwrap(), "start\n");
    assert_eq!(String::from_utf8(result.get_stderr()).unwrap(), "end\n");
    assert!(f.tmp_dir.join("endfile").exists());

    // The interrupter thread should have exited about 1000ms ago.
    let interrupter = f.interrupter.take().unwrap();
    assert!(
        interrupter.is_finished(),
        "Interrupter thread is still alive!"
    );
    interrupter.join().unwrap();

    // The interrupter thread should have set the main thread's interrupt flag.
    assert!(
        ThreadInterrupt::current().is_interrupted(),
        "Main thread was not interrupted during command execution!"
    );
}

/// Test that interrupting a thread in an "interruptible" Command.execute does terminate the
/// subprocess and throws an InterruptedException.
#[test]
fn interruptible_command_is_interrupted() {
    let f = start_interrupter("interruptible");
    let mut result = f.command.execute_async().unwrap();
    assert!(matches!(
        result.get(),
        Err(CommandExecutionError::Interrupted(_))
    ));
    assert!(!f.tmp_dir.join("endfile").exists());
    assert!(result.is_done());
}
