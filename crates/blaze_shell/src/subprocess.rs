//! Ports of the `Subprocess` and `SubprocessFactory` interfaces.

use std::io::{self, Read, Write};

use crate::subprocess_builder::SubprocessBuilder;
use crate::thread_interrupt::{self, InterruptedException};

/// A process started by Bazel.
// Java: com/google/devtools/build/lib/shell/Subprocess.java:23 Subprocess
pub trait Subprocess: Send + Sync {
    /// Kill the process.
    // Java: com/google/devtools/build/lib/shell/Subprocess.java:26 Subprocess.destroy
    fn destroy(&self) -> bool;

    /// Returns the exit value of the process. Panics (Java `IllegalThreadStateException`) if the
    /// process has not terminated yet.
    // Java: com/google/devtools/build/lib/shell/Subprocess.java:33 Subprocess.exitValue
    fn exit_value(&self) -> i32;

    /// Returns if the process has finished. This may cause the process to be destroyed as a side
    /// effect, for example due to a timeout.
    fn finished(&self) -> bool;

    /// Returns true if the process is still alive. Does not block or cause any side effects.
    // Java: com/google/devtools/build/lib/shell/Subprocess.java:43 Subprocess.isAlive
    fn is_alive(&self) -> bool;

    /// Returns if the process timed out.
    // Java: com/google/devtools/build/lib/shell/Subprocess.java:46 Subprocess.timedout
    fn timedout(&self) -> bool;

    /// Waits for the process to finish.
    fn wait_for(&self) -> Result<(), InterruptedException>;

    /// Returns a stream into which the stdin of the process can be written, or `None` if the
    /// stdin was redirected from a file. PORT: ownership moves to the caller on the first call.
    // Java: com/google/devtools/build/lib/shell/Subprocess.java:55 Subprocess.getOutputStream
    fn get_output_stream(&self) -> Option<Box<dyn Write + Send>>;

    /// Returns a stream from which the stdout of the process can be read, or `None` if the stdout
    /// was redirected to a file. PORT: ownership moves to the caller on the first call.
    // Java: com/google/devtools/build/lib/shell/Subprocess.java:62 Subprocess.getInputStream
    fn get_input_stream(&self) -> Option<Box<dyn Read + Send>>;

    /// Returns a stream from which the stderr of the process can be read, or `None` if the stderr
    /// was redirected to a file. PORT: ownership moves to the caller on the first call.
    // Java: com/google/devtools/build/lib/shell/Subprocess.java:69 Subprocess.getErrorStream
    fn get_error_stream(&self) -> Option<Box<dyn Read + Send>>;

    /// Returns the PID of the current process.
    // Java: com/google/devtools/build/lib/shell/Subprocess.java:73 Subprocess.getProcessId
    fn get_process_id(&self) -> i64;

    /// Terminates the process as thoroughly as the underlying implementation allows and releases
    /// native data structures associated with the process.
    // Java: com/google/devtools/build/lib/shell/Subprocess.java:79 Subprocess.close
    fn close(&self);

    /// Waits for the process to finish in a non-interruptible manner.
    // Java: com/google/devtools/build/lib/shell/Subprocess.java:83 Subprocess.waitForUninterruptibly
    fn wait_for_uninterruptibly(&self) {
        let mut was_interrupted = false;
        loop {
            match self.wait_for() {
                Ok(()) => break,
                Err(InterruptedException) => was_interrupted = true,
            }
        }
        if was_interrupted {
            thread_interrupt::interrupt_current(); // preserve interrupted status
        }
    }

    /// Kills the subprocess and awaits for its termination so that we know it has released any
    /// resources it may have held.
    // Java: com/google/devtools/build/lib/shell/Subprocess.java:106 Subprocess.destroyAndWait
    fn destroy_and_wait(&self) {
        self.destroy();
        self.wait_for_uninterruptibly();
    }
}

/// Something that can create subprocesses.
// Java: com/google/devtools/build/lib/shell/SubprocessFactory.java:19 SubprocessFactory
pub trait SubprocessFactory: Send + Sync {
    /// Create a subprocess according to the specified parameters. Fails if the underlying file is
    /// not executable or cannot be found.
    fn create(&self, params: &SubprocessBuilder) -> io::Result<Box<dyn Subprocess>>;
}
