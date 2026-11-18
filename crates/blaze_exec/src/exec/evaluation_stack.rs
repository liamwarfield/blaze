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
/// thread; a Java interrupt status that `f` leaves set (`Thread.currentThread().interrupt()`, to
/// preserve it) is set on the calling thread too.
pub fn on_evaluation_stack<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    if ON_EVALUATION_STACK.with(Cell::get) {
        return f();
    }
    let (result, interrupted) = blaze_starlark::eval::run_on_evaluation_thread(|| {
        ON_EVALUATION_STACK.with(|c| c.set(true));
        let result = f();
        (
            result,
            blaze_shell::thread_interrupt::is_current_interrupted(),
        )
    });
    if interrupted {
        blaze_shell::thread_interrupt::interrupt_current();
    }
    result
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
    #[should_panic(expected = "boom")]
    fn panics_are_resumed() {
        on_evaluation_stack(|| panic!("boom"));
    }
}
