//! `CommandManager`: helper class for commands that are currently running on the server.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use blaze_protos::command_server::{CancelRequest, TerminalSizeRequest};
use blaze_shell::thread_interrupt::{InterruptedException, ThreadInterrupt};
use blaze_util::check_state;
use parking_lot::Mutex;

use crate::idle_task::{IdleTask, IdleTaskResult};
use crate::idle_task_manager::IdleTaskManager;
use crate::interruptible::{self, Monitor};
use crate::terminal_size_monitor::TerminalSizeMonitor;

/// What the running-command map holds for each command (the fields of Java's `RunningCommand`
/// that other threads read).
struct RunningCommandEntry {
    /// `thread`: the interrupt status of the thread that created the command.
    thread: ThreadInterrupt,
    /// `thread.getName()`.
    thread_name: String,
    id: String,
    preemptible: bool,
    terminal_size_monitor: Arc<TerminalSizeMonitor>,
}

/// The state guarded by `synchronized (this)`.
#[derive(Default)]
struct IdleState {
    /// The current IdleTaskManager. `None` when a command is running or if idle tasks are
    /// disabled.
    idle_task_manager: Option<IdleTaskManager>,
    /// Idle task results from the most recent idle period following a command that registered
    /// idle tasks. `None` after a subsequent command retrieves them or if idle tasks are
    /// disabled.
    idle_task_results: Option<Vec<IdleTaskResult>>,
}

/// Helper class for commands that are currently running on the server.
// Java: com/google/devtools/build/lib/server/CommandManager.java:35 CommandManager
pub struct CommandManager {
    /// The list of currently running commands. Note that, even though most commands run serially
    /// because of the output base lock, they're registered here before blocking for the lock, so
    /// the map is effectively unbounded.
    running_commands_map: Monitor<HashMap<String, Arc<RunningCommandEntry>>>,
    /// Whether idle tasks are enabled.
    do_idle_server_tasks: bool,
    idle_state: Mutex<IdleState>,
    interrupt_counter: AtomicI64,
    slow_interrupt_message_suffix: Option<String>,
}

/// The parts of `CommandManager` that `ServerWatcherRunnable` uses.
///
/// PORT: a trait so that `ServerWatcherRunnableTest` can substitute a mock, as Java does with
/// Mockito.
pub trait CommandManagerWatch: Send + Sync {
    // Java: com/google/devtools/build/lib/server/CommandManager.java:129 CommandManager.isEmpty
    fn is_empty(&self) -> bool;
    // Java: com/google/devtools/build/lib/server/CommandManager.java:135 CommandManager.waitForChange
    fn wait_for_change(&self) -> Result<(), InterruptedException>;
    // Java: com/google/devtools/build/lib/server/CommandManager.java:141 CommandManager.waitForChange
    fn wait_for_change_with_timeout(&self, timeout: i64) -> Result<(), InterruptedException>;
}

impl CommandManager {
    // Java: com/google/devtools/build/lib/server/CommandManager.java:65 CommandManager.<init>
    pub fn new(
        do_idle_server_tasks: bool,
        slow_interrupt_message_suffix: Option<String>,
    ) -> Arc<CommandManager> {
        let manager = Arc::new(CommandManager {
            running_commands_map: Monitor::new(HashMap::new()),
            do_idle_server_tasks,
            idle_state: Mutex::new(IdleState::default()),
            interrupt_counter: AtomicI64::new(0),
            slow_interrupt_message_suffix,
        });
        manager.idle(None);
        manager
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:71 CommandManager.preemptEligibleCommands
    pub fn preempt_eligible_commands(self: &Arc<Self>) {
        let map = self.running_commands_map.lock();
        let mut commands_to_interrupt = HashSet::new();

        for command in map.values() {
            if command.preemptible {
                command.thread.interrupt();
                commands_to_interrupt.insert(command.id.clone());
            }
        }

        if !commands_to_interrupt.is_empty() {
            self.start_slow_interrupt_watcher(commands_to_interrupt);
        }
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:89 CommandManager.interruptInflightCommands
    pub fn interrupt_inflight_commands(self: &Arc<Self>) {
        let map = self.running_commands_map.lock();
        for command in map.values() {
            command.thread.interrupt();
        }

        self.start_slow_interrupt_watcher(map.keys().cloned().collect());
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:99 CommandManager.doCancel
    pub fn do_cancel(self: &Arc<Self>, request: &CancelRequest) {
        let _cancel_command = self.create_command();
        let map = self.running_commands_map.lock();
        match map.get(&request.command_id) {
            Some(pending_command) => {
                log_info(
                    "doCancel",
                    &format!(
                        "Interrupting command {} on thread {}",
                        request.command_id, pending_command.thread_name
                    ),
                );
                pending_command.thread.interrupt();
                self.start_slow_interrupt_watcher(HashSet::from([request.command_id.clone()]));
            }
            None => {
                log_info(
                    "doCancel",
                    &format!("Cannot find command {} to interrupt", request.command_id),
                );
            }
        }
        drop(map);
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:116 CommandManager.doUpdateTerminalSize
    pub fn do_update_terminal_size(&self, request: &TerminalSizeRequest) {
        let map = self.running_commands_map.lock();
        match map.get(&request.command_id) {
            Some(pending_command) => {
                pending_command
                    .terminal_size_monitor
                    .update_terminal_size(request.columns, request.rows);
            }
            None => {
                log_info(
                    "doUpdateTerminalSize",
                    &format!(
                        "Cannot find command {} to update terminal size",
                        request.command_id
                    ),
                );
            }
        }
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:147 CommandManager.createPreemptibleCommand
    pub fn create_preemptible_command(self: &Arc<Self>) -> RunningCommand {
        let command = RunningCommand::new(self.clone(), true);
        self.register_command(&command);
        command
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:153 CommandManager.createCommand
    pub fn create_command(self: &Arc<Self>) -> RunningCommand {
        let command = RunningCommand::new(self.clone(), false);
        self.register_command(&command);
        command
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:159 CommandManager.registerCommand
    fn register_command(&self, command: &RunningCommand) {
        {
            let mut map = self.running_commands_map.lock();
            if map.is_empty() {
                self.busy();
            }
            map.insert(command.entry.id.clone(), command.entry.clone());
            self.running_commands_map.notify_all();
        }
        log_info(
            "registerCommand",
            &format!(
                "Starting command {} on thread {}",
                command.entry.id, command.entry.thread_name
            ),
        );
    }

    /// Enters an idle period.
    ///
    /// Called when the set of running commands becomes empty.
    // Java: com/google/devtools/build/lib/server/CommandManager.java:177 CommandManager.idle
    fn idle(&self, idle_tasks: Option<Vec<Arc<dyn IdleTask>>>) {
        if self.do_idle_server_tasks
            && let Some(idle_tasks) = idle_tasks
        {
            let mut state = self.idle_state.lock();
            check_state!(state.idle_task_manager.is_none());
            let manager = IdleTaskManager::new(idle_tasks);
            manager.idle();
            state.idle_task_manager = Some(manager);
        }
    }

    /// Leaves an idle period.
    ///
    /// Called when the set of running commands becomes non-empty.
    // Java: com/google/devtools/build/lib/server/CommandManager.java:192 CommandManager.busy
    fn busy(&self) {
        let mut state = self.idle_state.lock();
        if let Some(manager) = state.idle_task_manager.take() {
            state.idle_task_results = Some(manager.busy());
        }
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:201 CommandManager.startSlowInterruptWatcher
    fn start_slow_interrupt_watcher(self: &Arc<Self>, command_ids: HashSet<String>) {
        if command_ids.is_empty() {
            return;
        }

        let this = self.clone();
        let interrupt_watcher = move || {
            if interruptible::sleep(Duration::from_secs(10)).is_err() {
                // Ignore.
                return;
            }
            let ok = {
                let map = this.running_commands_map.lock();
                command_ids.iter().all(|id| !map.contains_key(id))
            };
            if !ok {
                // At least one command was not interrupted. Interrupt took too long.
                warn_about_slow_interrupt(this.slow_interrupt_message_suffix.as_deref());
            }
        };

        let n = self.interrupt_counter.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::Builder::new()
            .name(format!("interrupt-watcher-{n}"))
            .spawn(move || crate::uncaught_exception::run_catching(interrupt_watcher))
            .expect("failed to start an interrupt watcher thread");
    }

    /// Returns idle task results returned by [`IdleTaskManager`] during a previous idle period,
    /// if available and not yet retrieved.
    ///
    /// Clears the stored idle task results as a side effect.
    // Java: com/google/devtools/build/lib/server/CommandManager.java:236 CommandManager.getIdleTaskResults
    pub fn get_idle_task_results(&self) -> Option<Vec<IdleTaskResult>> {
        self.idle_state.lock().idle_task_results.take()
    }
}

/// `logger.atInfo().log(message)` of `CommandManager.<method>`.
fn log_info(method: &str, message: &str) {
    blaze_util::server_logger::info(
        "com.google.devtools.build.lib.server.CommandManager",
        method,
        message,
    );
}

/// `Thread.currentThread().getName()`.
///
/// PORT: a Rust thread started without a name has none; Java names such a thread `Thread-<n>`.
/// Every thread Blaze runs commands on is named (`main`, `grpc-command-<n>`, the gRPC workers).
pub fn current_thread_name() -> String {
    std::thread::current()
        .name()
        .unwrap_or("<unnamed>")
        .to_string()
}

/// `ThreadUtils.warnAboutSlowInterrupt`.
///
/// PORT: skipped, JVM-only: the dump of every Java thread's stack (`Thread.getAllStackTraces`,
/// `ThreadDumper`) and the `SlowInterruptException` bug report (logged only, not shown to the
/// user). The first log line is written.
// Java: com/google/devtools/build/lib/util/ThreadUtils.java:55 ThreadUtils.warnAboutSlowInterrupt
// Java: com/google/devtools/build/lib/util/ThreadUtils.java:60 ThreadUtils.warnAboutSlowInterrupt
// PORT: skipped, JVM-only (Java thread dump): ThreadUtils.makeString, ThreadUtils.makeThreadInfoString
// PORT: skipped, JVM-only (Java thread dump): ThreadUtils.hashCode, ThreadUtils.equals, ThreadUtils.compareTo
fn warn_about_slow_interrupt(_slow_interrupt_message_suffix: Option<&str>) {
    blaze_util::server_logger::warning(
        "com.google.devtools.build.lib.util.ThreadUtils",
        "warnAboutSlowInterrupt",
        "Interrupt took too long. Dumping thread state.",
        None,
    );
}

impl CommandManagerWatch for CommandManager {
    fn is_empty(&self) -> bool {
        self.running_commands_map.lock().is_empty()
    }

    fn wait_for_change(&self) -> Result<(), InterruptedException> {
        let mut map = self.running_commands_map.lock();
        self.running_commands_map.wait(&mut map, None)
    }

    fn wait_for_change_with_timeout(&self, timeout: i64) -> Result<(), InterruptedException> {
        let mut map = self.running_commands_map.lock();
        // Object.wait(0) waits without a timeout.
        let timeout = if timeout == 0 {
            None
        } else {
            Some(Duration::from_millis(timeout.max(0) as u64))
        };
        self.running_commands_map.wait(&mut map, timeout)
    }
}

impl CommandManager {
    // Java: com/google/devtools/build/lib/server/CommandManager.java:129 CommandManager.isEmpty
    pub fn is_empty(&self) -> bool {
        CommandManagerWatch::is_empty(self)
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:135 CommandManager.waitForChange
    pub fn wait_for_change(&self) -> Result<(), InterruptedException> {
        CommandManagerWatch::wait_for_change(self)
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:141 CommandManager.waitForChange
    pub fn wait_for_change_with_timeout(&self, timeout: i64) -> Result<(), InterruptedException> {
        CommandManagerWatch::wait_for_change_with_timeout(self, timeout)
    }
}

/// A command registered with the [`CommandManager`]; dropping it is Java's `close()`.
// Java: com/google/devtools/build/lib/server/CommandManager.java:242 CommandManager.RunningCommand
pub struct RunningCommand {
    manager: Arc<CommandManager>,
    entry: Arc<RunningCommandEntry>,
    idle_tasks: Option<Vec<Arc<dyn IdleTask>>>,
}

impl RunningCommand {
    // Java: com/google/devtools/build/lib/server/CommandManager.java:249 RunningCommand.<init>
    fn new(manager: Arc<CommandManager>, preemptible: bool) -> RunningCommand {
        RunningCommand {
            manager,
            entry: Arc::new(RunningCommandEntry {
                thread: ThreadInterrupt::current(),
                thread_name: current_thread_name(),
                id: uuid::Uuid::new_v4().to_string(),
                preemptible,
                terminal_size_monitor: Arc::new(TerminalSizeMonitor::new()),
            }),
            idle_tasks: None,
        }
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:268 RunningCommand.getId
    pub fn get_id(&self) -> &str {
        &self.entry.id
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:272 RunningCommand.isPreemptible
    pub fn is_preemptible(&self) -> bool {
        self.entry.preemptible
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:276 RunningCommand.getTerminalSizeMonitor
    pub fn get_terminal_size_monitor(&self) -> Arc<TerminalSizeMonitor> {
        self.entry.terminal_size_monitor.clone()
    }

    /// Set idle tasks to be run by [`IdleTaskManager`] during an idle period immediately
    /// following this command, if one occurs and idle tasks are enabled.
    // Java: com/google/devtools/build/lib/server/CommandManager.java:284 RunningCommand.setIdleTasks
    pub fn set_idle_tasks(&mut self, idle_tasks: Vec<Arc<dyn IdleTask>>) {
        self.idle_tasks = Some(idle_tasks);
    }
}

impl Drop for RunningCommand {
    // Java: com/google/devtools/build/lib/server/CommandManager.java:256 RunningCommand.close
    fn drop(&mut self) {
        {
            let mut map = self.manager.running_commands_map.lock();
            map.remove(&self.entry.id);
            if map.is_empty() {
                self.manager.idle(self.idle_tasks.take());
            }
            self.manager.running_commands_map.notify_all();
        }

        blaze_util::server_logger::info(
            "com.google.devtools.build.lib.server.CommandManager$RunningCommand",
            "close",
            &format!(
                "Finished command {} on thread {}",
                self.entry.id, self.entry.thread_name
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/server/CommandManagerTest.java
    use super::*;
    use crate::idle_task::{IdleTaskError, IdleTaskStatus};
    use std::sync::Barrier;
    use std::sync::atomic::AtomicUsize;

    fn is_valid_uuid(uuid_string: &str) -> bool {
        uuid::Uuid::parse_str(uuid_string).is_ok()
    }

    #[test]
    fn test_basic_operations_on_single_thread() {
        let under_test =
            CommandManager::new(false, Some("slow interrupt message suffix".to_string()));
        assert!(under_test.is_empty());
        {
            let first_command = under_test.create_command();
            assert!(!under_test.is_empty());
            assert!(is_valid_uuid(first_command.get_id()));
            {
                let second_command = under_test.create_command();
                assert!(!under_test.is_empty());
                assert!(is_valid_uuid(second_command.get_id()));
                assert_ne!(first_command.get_id(), second_command.get_id());
            }
            assert!(!under_test.is_empty());
        }
        assert!(under_test.is_empty());
    }

    /// `waitForThreadWaiting`: the waiting flag is set and the thread is in `wait`.
    fn wait_for_thread_waiting(ready_to_wait_for_change: &AtomicBool, manager: &CommandManager) {
        while !(ready_to_wait_for_change.load(Ordering::SeqCst)
            && manager.running_commands_map.waiting_threads() == 1)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    use std::sync::atomic::AtomicBool;

    #[test]
    fn test_notifies_on_busy_and_idle() {
        let notification_counter = Arc::new(AtomicUsize::new(0));
        let under_test =
            CommandManager::new(false, Some("slow interrupt message suffix".to_string()));
        let waiting = Arc::new(AtomicBool::new(false));
        let cyclic_barrier = Arc::new(Barrier::new(2));

        let (tx, rx) = std::sync::mpsc::channel();
        let thread = {
            let (under_test, waiting, counter, barrier) = (
                under_test.clone(),
                waiting.clone(),
                notification_counter.clone(),
                cyclic_barrier.clone(),
            );
            std::thread::spawn(move || {
                tx.send(ThreadInterrupt::current()).unwrap();
                loop {
                    waiting.store(true, Ordering::SeqCst);
                    if under_test.wait_for_change().is_err() {
                        // Used to terminate the thread.
                        return;
                    }
                    waiting.store(false, Ordering::SeqCst);
                    counter.fetch_add(1, Ordering::SeqCst);
                    barrier.wait();
                }
            })
        };
        let thread_interrupt = rx.recv().unwrap();

        // We want to ensure at each step that we are actively awaiting notification.
        wait_for_thread_waiting(&waiting, &under_test);
        {
            let _first_command = under_test.create_command();
            cyclic_barrier.wait();
            assert_eq!(notification_counter.load(Ordering::SeqCst), 1);
            wait_for_thread_waiting(&waiting, &under_test);
            {
                let _second_command = under_test.create_command();
                cyclic_barrier.wait();
                assert_eq!(notification_counter.load(Ordering::SeqCst), 2);
                wait_for_thread_waiting(&waiting, &under_test);
            }
            cyclic_barrier.wait();
            assert_eq!(notification_counter.load(Ordering::SeqCst), 3);
            wait_for_thread_waiting(&waiting, &under_test);
        }
        cyclic_barrier.wait();
        assert_eq!(notification_counter.load(Ordering::SeqCst), 4);

        thread_interrupt.interrupt();
        thread.join().unwrap();
    }

    struct FnTask<F>(&'static str, F);

    impl<F: Fn() + Send + Sync> IdleTask for FnTask<F> {
        fn display_name(&self) -> String {
            self.0.to_string()
        }
        fn run(&self) -> Result<(), IdleTaskError> {
            (self.1)();
            Ok(())
        }
    }

    #[test]
    fn test_idle_tasks_enabled() {
        let under_test =
            CommandManager::new(true, Some("slow interrupt message suffix".to_string()));

        let (task_running_tx, task_running) = std::sync::mpsc::channel();
        let task_running_tx = Mutex::new(task_running_tx);
        let idle_task: Arc<dyn IdleTask> = Arc::new(FnTask("my idle task", move || {
            let _ = task_running_tx.lock().send(());
        }));

        // The 1st command collects no results and registers a task.
        {
            let mut c1 = under_test.create_command();
            assert_eq!(under_test.get_idle_task_results(), None);
            c1.set_idle_tasks(vec![idle_task]);
        }

        task_running.recv().unwrap();

        // The 2nd command does not attempt to collect results and registers no tasks.
        {
            let _c2 = under_test.create_command();
        }

        // The 3rd command collects results from the 1st command and registers no tasks.
        {
            let _c3 = under_test.create_command();
            let results: Vec<IdleTaskResult> = under_test
                .get_idle_task_results()
                .unwrap()
                .into_iter()
                .map(|r| IdleTaskResult::new(r.name(), r.status(), Duration::ZERO))
                .collect();
            assert_eq!(
                results,
                vec![IdleTaskResult::new(
                    "my idle task",
                    IdleTaskStatus::SUCCESS,
                    Duration::ZERO
                )]
            );
        }

        // The 4th command collects no results.
        {
            let _c4 = under_test.create_command();
            assert_eq!(under_test.get_idle_task_results(), None);
        }
    }

    #[test]
    fn test_idle_tasks_disabled() {
        let under_test =
            CommandManager::new(false, Some("slow interrupt message suffix".to_string()));

        let idle_task: Arc<dyn IdleTask> = Arc::new(FnTask("my idle task", || {}));

        {
            let mut c1 = under_test.create_command();
            c1.set_idle_tasks(vec![idle_task]);
        }

        {
            let _c2 = under_test.create_command();
            assert_eq!(under_test.get_idle_task_results(), None);
        }
    }
}
