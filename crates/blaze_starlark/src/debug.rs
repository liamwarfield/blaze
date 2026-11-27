//! Port of `net/starlark/java/eval/Debug.java`: the interpreter's hooks for a Starlark debugger
//! (`--experimental_skylark_debug`, `blaze_starlarkdebug`).
//!
//! Java checks for a debugger before every statement of a frame created while one is set
//! (`StarlarkThread.Frame.dbg`, `Eval.exec`). starlark-rust instruments a statement only when the
//! code is compiled with a hook in place, so (ruling of 2026-11-26, option b) every evaluator of
//! a command with the flag on installs the hook before it compiles anything
//! ([`install_hook`]), and a kept graph holding code compiled without it is discarded when the
//! flag turns on or off (`StarlarkDebuggerModule`, blaze_runtime). With no debugger set nothing
//! is installed, so evaluation pays nothing.

use starlark::codemap::FileSpanRef;
use starlark::environment::FrozenModule;
use starlark::eval::{BeforeStmtFunc, BeforeStmtFuncDyn, Evaluator};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

/// A source location as the debugger reports it (Java `Location`: file, 1-based line and
/// column).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DebugLocation {
    pub file: String,
    pub line: i32,
    pub column: i32,
}

impl DebugLocation {
    /// The start of a statement's span (Java `Statement.getStartLocation()`).
    pub fn of(span: FileSpanRef) -> DebugLocation {
        let resolved = span.resolve_span();
        DebugLocation {
            file: span.filename().to_owned(),
            line: resolved.begin.line as i32 + 1,
            column: resolved.begin.column as i32 + 1,
        }
    }
}

/// A debugger, called before each statement of instrumented code.
// Java: net/starlark/java/eval/Debug.java:34 Debug.Debugger
pub trait Debugger: Send + Sync {
    /// Called before each statement; may block (to pause the thread).
    // Java: net/starlark/java/eval/Debug.java:41 Debug.Debugger.before
    fn before(&self, eval: &mut Evaluator<'_, '_, '_>, location: &DebugLocation);

    /// Called when the debugger is replaced or removed.
    // Java: net/starlark/java/eval/Debug.java:44 Debug.Debugger.close
    fn close(&self);
}

// Java: net/starlark/java/eval/Debug.java:92 Debug.debugger
static DEBUGGER: LazyLock<RwLock<Option<Arc<dyn Debugger>>>> = LazyLock::new(|| RwLock::new(None));

/// Installs `dbg` as the debugger (or removes it), closing the previous one.
// Java: net/starlark/java/eval/Debug.java:101 Debug.setDebugger
pub fn set_debugger(dbg: Option<Arc<dyn Debugger>>) {
    let prev = std::mem::replace(&mut *DEBUGGER.write().unwrap(), dbg);
    // The modules recorded for the previous debugger belong to its command.
    MODULE_GLOBALS.write().unwrap().clear();
    if let Some(prev) = prev {
        prev.close();
    }
}

/// The current debugger, if any.
pub fn debugger() -> Option<Arc<dyn Debugger>> {
    DEBUGGER.read().unwrap().clone()
}

/// The hook installed in an evaluator: Java's `if (fr.dbg != null) fr.dbg.before(...)`.
struct Hook(Arc<dyn Debugger>);

impl<'e> BeforeStmtFuncDyn<'e> for Hook {
    fn call<'v>(
        &mut self,
        span: FileSpanRef,
        continued: bool,
        eval: &mut Evaluator<'v, '_, 'e>,
    ) -> starlark::Result<()> {
        // A "continued" instruction is the rest of the statement that is running (after a
        // call returned); Java calls the debugger once per statement.
        if !continued {
            let location = DebugLocation::of(span);
            self.0.before(eval, &location);
        }
        Ok(())
    }
}

/// Installs the debugger's hook in a fresh evaluator, before it compiles anything (the frames
/// of Java's thread capture the debugger when they are created). Does nothing without a
/// debugger.
pub fn install_hook(eval: &mut Evaluator<'_, '_, '_>) {
    if let Some(dbg) = debugger() {
        eval.before_stmt_for_dap(BeforeStmtFunc::Dyn(Box::new(Hook(dbg))));
    }
}

/// How a resumed thread steps.
// Java: net/starlark/java/eval/Debug.java:73 Debug.Stepping
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stepping {
    None,
    Into,
    Over,
    Out,
}

/// Whether a stepping thread pauses at its next statement, given its call-stack depth then.
// Java: net/starlark/java/eval/Debug.java:63 Debug.ReadyToPause
pub type ReadyToPause = Box<dyn Fn(usize) -> bool + Send + Sync>;

/// The predicate that pauses a thread resumed with `stepping` at call-stack depth `depth`.
// Java: net/starlark/java/eval/Debug.java:120 Debug.stepControl
pub fn step_control(depth: usize, stepping: Stepping) -> Option<ReadyToPause> {
    match stepping {
        Stepping::None => None,
        // pause at the very next statement
        Stepping::Into => Some(Box::new(|_| true)),
        Stepping::Over => Some(Box::new(move |d| d <= depth)),
        // if we're at the outermost frame, same as NONE
        Stepping::Out => {
            if depth == 0 {
                None
            } else {
                Some(Box::new(move |d| d < depth))
            }
        }
    }
}

/// The call-stack depth Java's `getCallStackSize` gives: the frames of the calls plus the
/// top-level frame.
pub fn call_stack_size(eval: &Evaluator<'_, '_, '_>) -> usize {
    eval.call_stack_count() + 1
}

// The modules of the files evaluated while a debugger is set, by file name: the globals of a
// frame whose function was defined in a frozen module (Java `StarlarkFunction.getModule()`).
static MODULE_GLOBALS: LazyLock<RwLock<HashMap<String, FrozenModule>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Records the frozen module of `file` (when a debugger is set).
pub fn register_module(file: &str, module: &FrozenModule) {
    if DEBUGGER.read().unwrap().is_some() {
        MODULE_GLOBALS
            .write()
            .unwrap()
            .insert(file.to_owned(), module.clone());
    }
}

/// The frozen module of `file`, if it was evaluated while the debugger was set.
pub fn module_of_file(file: &str) -> Option<FrozenModule> {
    MODULE_GLOBALS.read().unwrap().get(file).cloned()
}

/// One frame of the paused thread, innermost first (Java `Debug.getCallStack`, reversed by the
/// debug server).
// Java: net/starlark/java/eval/Debug.java:139 Debug.Frame
#[derive(Clone, Debug)]
pub struct DebugFrame {
    /// `getFunction().getName()` (`<toplevel>` for the file's top level).
    pub function_name: String,
    /// `getLocation()`: the statement running in the frame.
    pub location: Option<DebugLocation>,
    /// The index for [`Evaluator::local_variables_at_frame`] (`None`: the top level, which has
    /// no locals).
    pub frame_index: Option<usize>,
}

/// `StarlarkThread.TOP_LEVEL`.
pub const TOP_LEVEL: &str = "<toplevel>";

/// The frames of `eval`, innermost first, with `current` the location of the statement about
/// to run.
// Java: net/starlark/java/eval/StarlarkThread.java:525 StarlarkThread.getDebugCallStack
pub fn frames(eval: &Evaluator<'_, '_, '_>, current: &DebugLocation) -> Vec<DebugFrame> {
    let stack = eval.call_stack().into_frames();
    let mut next = Some(current.clone());
    let mut result = Vec::with_capacity(stack.len() + 1);
    for (i, frame) in stack.iter().rev().enumerate() {
        result.push(DebugFrame {
            function_name: frame.name.clone(),
            location: next,
            frame_index: Some(i),
        });
        // A frame's recorded location is that of the call it made: where its caller is.
        next = frame
            .location
            .as_ref()
            .map(|l| DebugLocation::of(l.as_ref()));
    }
    result.push(DebugFrame {
        function_name: TOP_LEVEL.to_owned(),
        location: next,
        frame_index: None,
    });
    result
}
