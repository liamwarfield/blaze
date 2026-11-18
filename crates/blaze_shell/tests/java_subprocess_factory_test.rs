//! Port of `src/test/java/com/google/devtools/build/lib/shell/JavaSubprocessFactoryTest.java`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use indexmap::IndexMap;
use parking_lot::Mutex;

use blaze_shell::java_subprocess_factory::{ForkExecutor, ForkFuture, ForkTask};
use blaze_shell::{JavaSubprocessFactory, SubprocessBuilder};

/// Observes the thread executing the submitted task, then delegates to the real executor.
struct RecordingExecutor {
    submits: AtomicUsize,
    fork_thread: Arc<Mutex<Option<String>>>,
}

impl ForkExecutor for RecordingExecutor {
    fn submit(&self, task: ForkTask) -> ForkFuture {
        self.submits.fetch_add(1, Ordering::SeqCst);
        let fork_thread = self.fork_thread.clone();
        JavaSubprocessFactory::default_fork_executor().submit(Box::new(move || {
            *fork_thread.lock() = std::thread::current().name().map(str::to_string);
            task()
        }))
    }
}

#[test]
fn create_from_virtual_thread_forks_on_platform_thread() {
    let fork_thread = Arc::new(Mutex::new(None));
    let executor = Arc::new(RecordingExecutor {
        submits: AtomicUsize::new(0),
        fork_thread: fork_thread.clone(),
    });
    let subprocess_factory = Arc::new(JavaSubprocessFactory::create_for_testing(executor.clone()));

    // PORT: Rust has no virtual threads; a short-lived thread stands in for one.
    let exit_code = std::thread::Builder::new()
        .name("test-virtual".to_string())
        .spawn(move || {
            let mut subprocess_builder =
                SubprocessBuilder::with_factory(&IndexMap::new(), subprocess_factory);
            subprocess_builder.set_argv(vec!["/bin/true".to_string()]);
            let subprocess = subprocess_builder.start().unwrap();
            subprocess.wait_for().unwrap();
            let exit = subprocess.exit_value();
            subprocess.close();
            exit
        })
        .unwrap()
        .join()
        .expect("Error running subprocess from virtual thread");

    assert_eq!(exit_code, 0);

    // The fork was routed through the correct executor and ran on the right thread
    assert_eq!(executor.submits.load(Ordering::SeqCst), 1);
    assert_eq!(fork_thread.lock().as_deref(), Some("subprocess-fork"));
}
