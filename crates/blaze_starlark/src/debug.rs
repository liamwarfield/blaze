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
use starlark::environment::{FrozenModule, Globals};
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
// Java: net/starlark/java/eval/Debug.java:32 Debug.Debugger
pub trait Debugger: Send + Sync {
    /// Called before each statement; may block (to pause the thread).
    // Java: net/starlark/java/eval/Debug.java:34 Debug.Debugger.before
    fn before(&self, eval: &mut Evaluator<'_, '_, '_>, location: &DebugLocation);

    /// Called when the debugger is replaced or removed.
    // Java: net/starlark/java/eval/Debug.java:37 Debug.Debugger.close
    fn close(&self);
}

// Java: net/starlark/java/eval/Debug.java:90 Debug.debugger
static DEBUGGER: LazyLock<RwLock<Option<Arc<dyn Debugger>>>> = LazyLock::new(|| RwLock::new(None));

/// Installs `dbg` as the debugger (or removes it), closing the previous one.
// Java: net/starlark/java/eval/Debug.java:97 Debug.setDebugger
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
///
/// It calls the debugger set when the statement runs (Java: the one the frame captured when it
/// was created): code compiled with the hook can outlive its command in a kept graph, and must
/// not call a closed server of an earlier command.
///
/// starlark-rust compiles each top-level statement as a `PossibleGc` statement with the span of
/// the statement, followed by the statement itself: the hook is called twice, one call right
/// after the other, with the same span. Java calls the debugger once per statement, so the
/// second call is not passed on. (A top-level statement starts at column 1, and no statement
/// runs twice in a row with the same span at the top level: a loop body is indented.)
struct Hook {
    /// The span of the last call, when it was a top-level statement's first call.
    last_top_level: Option<(String, u32, u32)>,
    /// The evaluator executes a module's top level (its call stack starts with the module's
    /// frame), not a function called from Rust.
    executes_module: bool,
}

thread_local! {
    // Whether the evaluator whose hook is running on this thread executes a module (read by
    // `frames` while the debugger has the thread paused).
    static EXECUTES_MODULE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

impl<'e> BeforeStmtFuncDyn<'e> for Hook {
    fn call<'v>(
        &mut self,
        span: FileSpanRef,
        continued: bool,
        eval: &mut Evaluator<'v, '_, 'e>,
    ) -> starlark::Result<()> {
        // A "continued" instruction is the rest of the statement that is running (after a
        // call returned); Java calls the debugger once per statement.
        if continued {
            return Ok(());
        }
        let location = DebugLocation::of(span);
        let key = (
            span.filename().to_owned(),
            span.span.begin().get(),
            span.span.end().get(),
        );
        let top_level =
            self.executes_module && location.column == 1 && eval.call_stack_count() <= 1;
        if top_level && self.last_top_level.as_ref() == Some(&key) {
            // The statement after its `PossibleGc`.
            self.last_top_level = None;
            return Ok(());
        }
        self.last_top_level = if top_level { Some(key) } else { None };
        if let Some(dbg) = debugger() {
            let outer = EXECUTES_MODULE.with(|m| m.replace(self.executes_module));
            dbg.before(eval, &location);
            EXECUTES_MODULE.with(|m| m.set(outer));
        }
        Ok(())
    }
}

/// Installs the debugger's hook in a fresh evaluator that executes a module's top level,
/// before it compiles anything (the frames of Java's thread capture the debugger when they are
/// created). Does nothing without a debugger.
pub fn install_hook(eval: &mut Evaluator<'_, '_, '_>) {
    install(eval, true);
}

/// [`install_hook`] for an evaluator that calls a function from Rust (a rule or aspect
/// implementation, a callback): its call stack has no top-level frame, as in Java.
pub fn install_hook_for_call(eval: &mut Evaluator<'_, '_, '_>) {
    install(eval, false);
}

fn install(eval: &mut Evaluator<'_, '_, '_>, executes_module: bool) {
    if debugger().is_some() {
        eval.before_stmt_for_dap(BeforeStmtFunc::Dyn(Box::new(Hook {
            last_top_level: None,
            executes_module,
        })));
    }
}

/// How a resumed thread steps.
// Java: net/starlark/java/eval/Debug.java:67 Debug.Stepping
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stepping {
    None,
    Into,
    Over,
    Out,
}

/// Whether a stepping thread pauses at its next statement, given its call-stack depth then.
// Java: net/starlark/java/eval/Debug.java:61 Debug.ReadyToPause
pub type ReadyToPause = Box<dyn Fn(usize) -> bool + Send + Sync>;

/// The predicate that pauses a thread resumed with `stepping` at call-stack depth `depth`.
// Java: net/starlark/java/eval/Debug.java:126 Debug.stepControl
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
/// top-level frame. starlark-rust runs a module's top level in a frame of its own (`module`), so
/// its count is Java's; a function called from Rust (a rule implementation) has no top-level
/// frame in either.
// Java: net/starlark/java/eval/StarlarkThread.java getCallStackSize
pub fn call_stack_size(eval: &Evaluator<'_, '_, '_>) -> usize {
    eval.call_stack_count()
}

// The modules of the files evaluated while a debugger is set, by file name: the globals of a
// frame whose function was defined in a frozen module (Java `StarlarkFunction.getModule()`), and
// the predeclared names the file was evaluated with (Java `Module.getPredeclaredBindings`).
#[derive(Default)]
struct DebugModule {
    frozen: Option<FrozenModule>,
    predeclared: Option<Globals>,
    /// The names `load` binds file-locally (not module globals: Java `Module.getGlobals()`
    /// leaves them out); empty for a file whose loads bind globally (BUILD).
    load_bound: Vec<String>,
}

static MODULE_GLOBALS: LazyLock<RwLock<HashMap<String, DebugModule>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Records the predeclared names `file` is evaluated with (when a debugger is set), before it
/// runs.
pub fn register_predeclared(file: &str, globals: &Globals) {
    if DEBUGGER.read().unwrap().is_some() {
        MODULE_GLOBALS
            .write()
            .unwrap()
            .entry(file.to_owned())
            .or_default()
            .predeclared = Some(globals.clone());
    }
}

/// Records the names `file`'s `load` statements bind file-locally (when a debugger is set).
pub fn register_load_bound(file: &str, names: Vec<String>) {
    if DEBUGGER.read().unwrap().is_some() {
        MODULE_GLOBALS
            .write()
            .unwrap()
            .entry(file.to_owned())
            .or_default()
            .load_bound = names;
    }
}

/// The names `file`'s loads bind file-locally.
pub fn load_bound_of_file(file: &str) -> Vec<String> {
    MODULE_GLOBALS
        .read()
        .unwrap()
        .get(file)
        .map(|m| m.load_bound.clone())
        .unwrap_or_default()
}

/// Records the frozen module of `file` (when a debugger is set).
pub fn register_module(file: &str, module: &FrozenModule) {
    if DEBUGGER.read().unwrap().is_some() {
        MODULE_GLOBALS
            .write()
            .unwrap()
            .entry(file.to_owned())
            .or_default()
            .frozen = Some(module.clone());
    }
}

/// The frozen module of `file`, if it was evaluated while the debugger was set.
pub fn module_of_file(file: &str) -> Option<FrozenModule> {
    MODULE_GLOBALS
        .read()
        .unwrap()
        .get(file)
        .and_then(|m| m.frozen.clone())
}

/// The predeclared names of `file`, if it was evaluated while the debugger was set.
pub fn predeclared_of_file(file: &str) -> Option<Globals> {
    MODULE_GLOBALS
        .read()
        .unwrap()
        .get(file)
        .and_then(|m| m.predeclared.clone())
}

/// The front end of Java's `Starlark.execFile(input, FileOptions.DEFAULT, module, thread)` for
/// the debugger's statements: parses `content` (as `<debug eval>`) and resolves it in a module
/// whose globals are `globals` and whose predeclared and universal names are those of
/// `predeclared`. `Err` is `SyntaxError.Exception.getMessage()`.
// Java: net/starlark/java/eval/Starlark.java:1105 Starlark.execFile
pub fn check_statement(
    content: &str,
    globals: &[String],
    predeclared: Option<&Globals>,
) -> Result<(), String> {
    use crate::syntax::java;
    let options = java::JavaOptions::DEFAULT;
    let parsed = java::parse_file("<debug eval>", content, options);
    let errors: Vec<String> = if !parsed.errors.is_empty() {
        parsed.errors.iter().map(|e| e.message.clone()).collect()
    } else {
        let module = DebugResolverModule {
            globals: globals.iter().cloned().collect(),
            predeclared: predeclared
                .map(|g| g.names().map(|n| n.as_str().to_owned()).collect())
                .unwrap_or_default(),
        };
        java::resolver::resolve_file(&parsed.statements, &module, options)
            .into_iter()
            .map(|e| e.message)
            .collect()
    };
    match errors.as_slice() {
        [] => Ok(()),
        // Java: net/starlark/java/syntax/SyntaxError.java:89 SyntaxError.Exception.getMessage
        [first] => Err(first.clone()),
        [first, rest @ ..] => Err(format!("{first} (+ {} more)", rest.len())),
    }
}

/// Java `Module.resolve` for a module with these globals and predeclared (and universal) names.
// Java: net/starlark/java/eval/Module.java:230 Module.resolve
struct DebugResolverModule {
    globals: Vec<String>,
    predeclared: Vec<String>,
}

impl crate::syntax::java::resolver::ResolverModule for DebugResolverModule {
    fn resolve(
        &self,
        name: &str,
    ) -> Result<crate::syntax::java::resolver::Scope, crate::syntax::java::resolver::Undefined>
    {
        use crate::syntax::java::resolver::{Scope, Undefined};
        if self.globals.iter().any(|g| g == name) {
            return Ok(Scope::Global);
        }
        if self.predeclared.iter().any(|p| p == name) {
            return Ok(Scope::Predeclared);
        }
        let mut candidates = self.globals.clone();
        candidates.extend(self.predeclared.iter().cloned());
        Err(Undefined {
            message: format!("name '{name}' is not defined"),
            candidates: Some(candidates),
        })
    }
}

/// One frame of the paused thread, innermost first (Java `Debug.getCallStack`, reversed by the
/// debug server).
// Java: net/starlark/java/eval/Debug.java:144 Debug.Frame
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
        // A frame's recorded location is that of the call it made: where its caller is, at
        // the call's `(` (Java `CallExpression.getLparenLocation`, the frame's location while
        // the call runs).
        next = frame.location.as_ref().map(|l| {
            let at = crate::error::Location::bazel_position(l);
            DebugLocation {
                file: at.file,
                line: at.line as i32,
                column: at.column as i32,
            }
        });
    }
    // The top-level frame of a module being executed (starlark-rust's `module` frame, which
    // `into_frames` leaves out). A function called from Rust (a rule implementation) has none,
    // as in Java, where such a thread's stack starts with the function
    // (`install_hook_for_call`).
    if EXECUTES_MODULE.with(|m| m.get()) {
        result.push(DebugFrame {
            function_name: TOP_LEVEL.to_owned(),
            location: next,
            frame_index: None,
        });
    }
    result
}
