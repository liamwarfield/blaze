//! Port of `com.google.devtools.build.lib.shell.JavaSubprocessFactory`.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use crate::java_file::JavaFile;
use crate::java_process::{JavaProcess, JavaProcessBuilder, Redirect};
use crate::subprocess::{Subprocess, SubprocessFactory};
use crate::subprocess_builder::{SubprocessBuilder, SubprocessBuilderStreamAction};
use crate::thread_interrupt::InterruptedException;

/// `System.currentTimeMillis()`.
fn current_time_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis() as i64
}

/// A subprocess backed by a [`JavaProcess`].
// Java: com/google/devtools/build/lib/shell/JavaSubprocessFactory.java:40 JavaSubprocessFactory.JavaSubprocess
struct JavaSubprocess {
    process: JavaProcess,
    deadline_millis: i64,
    deadline_exceeded: AtomicBool,
}

impl Subprocess for JavaSubprocess {
    fn destroy(&self) -> bool {
        self.process.destroy();
        true
    }

    fn exit_value(&self) -> i32 {
        self.process.exit_value()
    }

    // Java: com/google/devtools/build/lib/shell/JavaSubprocessFactory.java:62 JavaSubprocess.finished
    fn finished(&self) -> bool {
        if self.deadline_millis > 0
            && current_time_millis() > self.deadline_millis
            && self
                .deadline_exceeded
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            // We use compareAndSet here to avoid calling destroy multiple times. Note that destroy
            // returns immediately, and we don't want to wait in this method.
            self.process.destroy();
        }
        // this seems to be the only non-blocking call for checking liveness
        !self.process.is_alive()
    }

    fn is_alive(&self) -> bool {
        self.process.is_alive()
    }

    fn timedout(&self) -> bool {
        self.deadline_exceeded.load(Ordering::SeqCst)
    }

    // Java: com/google/devtools/build/lib/shell/JavaSubprocessFactory.java:85 JavaSubprocess.waitFor
    fn wait_for(&self) -> Result<(), InterruptedException> {
        let wait_time_millis = if self.deadline_millis > 0 {
            self.deadline_millis - current_time_millis()
        } else {
            i64::MAX
        };
        let exited_in_time = self.process.wait_for_millis(wait_time_millis)?;
        if !exited_in_time
            && self
                .deadline_exceeded
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            self.process.destroy();
            // The destroy call returns immediately, so we still need to wait for the actual exit.
            // The sole caller assumes that waitFor only exits when the process is gone (or throws).
            self.process.wait_for()?;
        }
        Ok(())
    }

    fn get_output_stream(&self) -> Option<Box<dyn Write + Send>> {
        self.process.get_output_stream()
    }

    fn get_input_stream(&self) -> Option<Box<dyn Read + Send>> {
        self.process.get_input_stream()
    }

    fn get_error_stream(&self) -> Option<Box<dyn Read + Send>> {
        self.process.get_error_stream()
    }

    fn get_process_id(&self) -> i64 {
        self.process.pid()
    }

    fn close(&self) {
        self.process.destroy_forcibly();
    }
}

/// A task submitted to a fork executor: starts a process.
pub type ForkTask = Box<dyn FnOnce() -> io::Result<JavaProcess> + Send>;

/// What a fork task produces: its result, or the payload of its panic.
pub type ForkOutcome = std::thread::Result<io::Result<JavaProcess>>;

type ForkRequest = (ForkTask, mpsc::Sender<ForkOutcome>);

/// The result of a submitted [`ForkTask`] (a `Future<Process>`).
pub struct ForkFuture(mpsc::Receiver<ForkOutcome>);

impl ForkFuture {
    /// Wraps a channel on which the executor sends the task's outcome.
    pub fn new(rx: mpsc::Receiver<ForkOutcome>) -> ForkFuture {
        ForkFuture(rx)
    }

    /// `Uninterruptibles.getUninterruptibly(future)`; a panic in the task is re-thrown here.
    pub fn get_uninterruptibly(self) -> io::Result<JavaProcess> {
        match self.0.recv() {
            Ok(Ok(result)) => result,
            Ok(Err(payload)) => std::panic::resume_unwind(payload),
            Err(_) => panic!("IllegalStateException: Unexpected error starting subprocess"),
        }
    }
}

/// The `ExecutorService` that subprocesses fork from.
pub trait ForkExecutor: Send + Sync {
    /// `ExecutorService.submit(Callable<Process>)`.
    fn submit(&self, task: ForkTask) -> ForkFuture;
}

/// A single long-lived thread named `subprocess-fork` (Java `Executors.newSingleThreadExecutor`).
pub struct SingleThreadForkExecutor {
    sender: Mutex<mpsc::Sender<ForkRequest>>,
}

impl SingleThreadForkExecutor {
    fn new(name: &str) -> SingleThreadForkExecutor {
        let (tx, rx) = mpsc::channel::<ForkRequest>();
        std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                for (task, reply) in rx {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
                    let _ = reply.send(result);
                }
            })
            .expect("failed to spawn subprocess-fork thread");
        SingleThreadForkExecutor {
            sender: Mutex::new(tx),
        }
    }
}

impl ForkExecutor for SingleThreadForkExecutor {
    fn submit(&self, task: ForkTask) -> ForkFuture {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.sender
            .lock()
            .send((task, reply_tx))
            .expect("subprocess-fork thread is gone");
        ForkFuture::new(reply_rx)
    }
}

/// A subprocess factory that uses `java.lang.ProcessBuilder` (here [`JavaProcessBuilder`]).
// Java: com/google/devtools/build/lib/shell/JavaSubprocessFactory.java:37 JavaSubprocessFactory
pub struct JavaSubprocessFactory {
    lock: Mutex<()>,
    fork_executor: Arc<dyn ForkExecutor>,
}

impl JavaSubprocessFactory {
    /// Subprocesses fork from this platform thread to ensure the subprocess's parent is long
    /// lived, which prevents actions from being killed erroneously (`linux-sandbox` and
    /// `process-wrapper` die with the thread that forked them, via `PR_SET_PDEATHSIG`).
    // Java: com/google/devtools/build/lib/shell/JavaSubprocessFactory.java:127 JavaSubprocessFactory.DEFAULT_FORK_EXECUTOR
    pub fn default_fork_executor() -> Arc<dyn ForkExecutor> {
        static EXECUTOR: OnceLock<Arc<dyn ForkExecutor>> = OnceLock::new();
        EXECUTOR
            .get_or_init(|| Arc::new(SingleThreadForkExecutor::new("subprocess-fork")))
            .clone()
    }

    /// `JavaSubprocessFactory.INSTANCE`.
    pub fn instance() -> Arc<dyn SubprocessFactory> {
        static INSTANCE: OnceLock<Arc<JavaSubprocessFactory>> = OnceLock::new();
        INSTANCE
            .get_or_init(|| {
                Arc::new(JavaSubprocessFactory::new(
                    JavaSubprocessFactory::default_fork_executor(),
                ))
            })
            .clone()
    }

    fn new(fork_executor: Arc<dyn ForkExecutor>) -> JavaSubprocessFactory {
        JavaSubprocessFactory {
            lock: Mutex::new(()),
            fork_executor,
        }
    }

    // Java: com/google/devtools/build/lib/shell/JavaSubprocessFactory.java:146 JavaSubprocessFactory.createForTesting
    pub fn create_for_testing(fork_executor: Arc<dyn ForkExecutor>) -> JavaSubprocessFactory {
        JavaSubprocessFactory::new(fork_executor)
    }

    // since we are a singleton, we represent an ideal global lock for process invocations, which
    // is required due to the following race condition:
    //
    // Linux does not provide a safe API for a multi-threaded program to fork a subprocess.
    // Consider the case where two threads both write an executable file and then try to execute
    // it. It can happen that the first thread writes its executable file, with the file
    // descriptor still being open when the second thread forks, with the fork inheriting a copy
    // of the file descriptor. Then the first thread closes the original file descriptor, and
    // proceeds to execute the file. At that point Linux sees an open file descriptor to the file
    // and returns ETXTBSY (Text file busy) as an error. This race is inherent in the fork / exec
    // duality, with fork always inheriting a copy of the file descriptor table; if there was a
    // way to fork without copying the entire file descriptor table (e.g., only copy specific
    // entries), we could avoid this race.
    //
    // As a workaround, we use a lock around the fork.
    // Java: com/google/devtools/build/lib/shell/JavaSubprocessFactory.java:168 JavaSubprocessFactory.start
    fn start(&self, builder: JavaProcessBuilder) -> io::Result<JavaProcess> {
        let _guard = self.lock.lock();
        // PORT: the JDK-upgrade "Failed to exec spawn helper" check does not apply (no
        // jspawnhelper).
        self.start_on_platform_thread(builder)
    }

    /// Starts a subprocess by forking from a long-lived thread.
    // Java: com/google/devtools/build/lib/shell/JavaSubprocessFactory.java:201 JavaSubprocessFactory.startOnPlatformThread
    fn start_on_platform_thread(&self, builder: JavaProcessBuilder) -> io::Result<JavaProcess> {
        let future = self.fork_executor.submit(Box::new(move || builder.start()));
        future.get_uninterruptibly()
    }

    /// Returns a redirect appropriate for the parameters. If a file redirected to exists, deletes
    /// the file before redirecting to it.
    // Java: com/google/devtools/build/lib/shell/JavaSubprocessFactory.java:243 JavaSubprocessFactory.getRedirect
    fn get_redirect(action: SubprocessBuilderStreamAction, file: Option<&JavaFile>) -> Redirect {
        match action {
            SubprocessBuilderStreamAction::Discard => Redirect::To(JavaFile::new("/dev/null")),
            SubprocessBuilderStreamAction::Redirect => {
                // We need to use Redirect.appendTo() here, because on older Linux kernels writes
                // are otherwise not atomic and might result in lost log messages:
                // https://lkml.org/lkml/2014/3/3/308
                let file = file.expect("NullPointerException: file");
                if file.exists() {
                    file.delete();
                }
                Redirect::AppendTo(file.clone())
            }
            SubprocessBuilderStreamAction::Stream => Redirect::Pipe,
        }
    }
}

impl SubprocessFactory for JavaSubprocessFactory {
    // Java: com/google/devtools/build/lib/shell/JavaSubprocessFactory.java:213 JavaSubprocessFactory.create
    fn create(&self, params: &SubprocessBuilder) -> io::Result<Box<dyn Subprocess>> {
        let mut builder = JavaProcessBuilder::new();
        // PORT: StringEncoding identity (internalToPlatform).
        builder.command(params.get_argv().to_vec());
        let env = match params.get_env() {
            Some(env) => env,
            None => params.get_client_env(),
        };
        for (key, value) in env {
            builder.environment_put(key, value);
        }

        builder.redirect_output(JavaSubprocessFactory::get_redirect(
            params.get_stdout(),
            params.get_stdout_file(),
        ));
        builder.redirect_error(JavaSubprocessFactory::get_redirect(
            params.get_stderr(),
            params.get_stderr_file(),
        ));
        builder.redirect_error_stream(params.redirect_error_stream());
        builder.directory(params.get_working_directory().cloned());

        // Deadline is now + given timeout.
        let deadline_millis = if params.get_timeout_millis() > 0 {
            current_time_millis()
                .checked_add(params.get_timeout_millis())
                .unwrap_or_else(|| panic!("ArithmeticException: long overflow"))
        } else {
            0
        };
        Ok(Box::new(JavaSubprocess {
            process: self.start(builder)?,
            deadline_millis,
            deadline_exceeded: AtomicBool::new(false),
        }))
    }
}
