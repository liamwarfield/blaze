//! Native stacks for the Starlark code the execution phase runs.
//!
//! PORT: Blaze only. Java runs every Skyframe function on the JVM's thread stacks. Blaze's
//! execution phase evaluates its SkyFunctions on 16 MB evaluator stacks
//! (`blaze_buildtool::buildtool::execution_tool::evaluate_build_driver_keys`): spawns fork the
//! process, and the kernel's overcommit heuristic counts every writable private mapping of the
//! parent, including the reserved stacks of the evaluator's threads (up to
//! `EvaluationContext::max_threads`, 512, plus the forced ones), so 256 MB evaluator stacks make
//! `fork` fail with ENOMEM. Starlark still runs during execution: `map_each` callbacks of
//! `Args` (command lines expanded by `Action.execute`, action keys computed by the action cache
//! check and by `Actions.canBeShared`) and `ctx.actions.write(content = args)`. That code runs
//! here, on a thread with [`blaze_starlark::eval::EVAL_STACK_SIZE`], which is what evaluation
//! needs. Such threads live only while one action executes or computes its key, so at most about
//! one per action in flight (bounded by `--jobs`) exists when a spawn forks, not one per
//! evaluator thread.

use std::cell::Cell;

thread_local! {
    /// The current thread was started by [`on_evaluation_stack`].
    static ON_EVALUATION_STACK: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f` on a thread with an [`blaze_starlark::eval::EVAL_STACK_SIZE`] stack (or inline when
/// the current thread already is one) and waits for it. A panic in `f` is resumed on the calling
/// thread.
///
/// The thread stands in for the calling one: it adopts the caller's Java interrupt status
/// ([`blaze_shell::ThreadInterrupt::adopt`]). An interrupt of the caller (the evaluator
/// interrupting its in-flight work: `AbstractQueueVisitor.interruptInFlightTasks`, or the command
/// being interrupted) therefore reaches the waits of the spawn runners in `f`, which kill their
/// child and throw `InterruptedException` as in Java; and a status `f` leaves set
/// (`Thread.currentThread().interrupt()`, to preserve it) is the caller's.
pub fn on_evaluation_stack<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    if ON_EVALUATION_STACK.with(Cell::get) {
        return f();
    }
    let caller = blaze_shell::ThreadInterrupt::current();
    blaze_starlark::eval::run_on_evaluation_thread(move || {
        ON_EVALUATION_STACK.with(|c| c.set(true));
        let _adopted = caller.adopt();
        f()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recursion that needs far more than the 16 MB of an execution-phase evaluator thread.
    fn deep(n: u64) -> u64 {
        let pad = std::hint::black_box([0u8; 1024]);
        if n == 0 {
            return pad[0] as u64;
        }
        deep(n - 1) + std::hint::black_box(pad[1]) as u64
    }

    #[test]
    fn runs_with_the_evaluation_stack_size() {
        let r = std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(|| on_evaluation_stack(|| deep(64 * 1024)))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(r, 0);
    }

    #[test]
    fn nested_calls_run_inline() {
        let outer = on_evaluation_stack(|| {
            let id = std::thread::current().id();
            on_evaluation_stack(move || std::thread::current().id() == id)
        });
        assert!(outer);
    }

    #[test]
    fn interrupt_status_is_carried_back() {
        assert!(!blaze_shell::thread_interrupt::interrupted());
        on_evaluation_stack(blaze_shell::thread_interrupt::interrupt_current);
        assert!(blaze_shell::thread_interrupt::interrupted());
    }

    #[test]
    fn an_interrupt_of_the_caller_reaches_a_wait_on_the_evaluation_stack() {
        use blaze_shell::{Command, ThreadInterrupt};
        let caller = ThreadInterrupt::current();
        let interrupter = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            caller.interrupt();
        });
        let started = std::time::Instant::now();
        // Java: Command.execute() of a long child is interrupted: the child is killed and the
        // call throws InterruptedException.
        let result = on_evaluation_stack(|| {
            Command::new(
                vec!["/bin/sleep".to_owned(), "30".to_owned()],
                &indexmap::IndexMap::new(),
            )
            .execute()
        });
        interrupter.join().unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert!(matches!(
            result,
            Err(blaze_shell::CommandExecutionError::Interrupted(_))
        ));
    }

    #[test]
    #[should_panic(expected = "boom")]
    fn panics_are_resumed() {
        on_evaluation_stack(|| panic!("boom"));
    }
}
