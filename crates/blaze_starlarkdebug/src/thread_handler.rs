//! Port of `ThreadHandler`: the paused and stepping threads of the debug server, its
//! breakpoints, and what a paused thread answers.
//!
//! PORT: Java's client thread reads a paused `StarlarkThread` directly (its call stack, its
//! module, `Starlark.execFile` on it) while the thread waits on a semaphore. A starlark-rust
//! `Evaluator` is borrowed by the thread that runs it, so here the paused thread waits on a
//! channel and answers the client's requests itself (list frames, evaluate, get children); the
//! client thread sends a request and waits for the answer. Requests are handled one at a time
//! in both, so the order of events and responses is Java's.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};
use std::sync::mpsc;

use blaze_protos::starlark_debugging::{
    Breakpoint, Error as ErrorProto, Frame, PauseReason, PausedThread, Stepping as SteppingProto,
    Value as ValueProto, breakpoint::Condition,
};
use blaze_starlark::debug::{self, DebugLocation, ReadyToPause};
use blaze_starlark::file_kind::FileKind;
use parking_lot::{Mutex, RwLock};
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;
use starlark::values::Value;

use crate::event_helper;
use crate::serialization::{DebugObject, ThreadObjectMap, get_children, get_value_proto};
use crate::transport::DebugServerTransport;

/// An error answering a client request (its message goes back in an `Error` event).
// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugRequestException.java:19 DebugRequestException
#[derive(Debug, Clone)]
pub struct DebugRequestException(pub String);

/// What the client asks of a paused thread.
enum Command {
    ListFrames(mpsc::Sender<Vec<Frame>>),
    Evaluate(
        String,
        mpsc::Sender<Result<ValueProto, DebugRequestException>>,
    ),
    GetChildren(
        i64,
        mpsc::Sender<Result<Vec<ValueProto>, DebugRequestException>>,
    ),
    Resume,
}

/// Information about a paused thread.
// Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:52 ThreadHandler.PausedThreadState
struct PausedThreadState {
    /// Java `thread.getCallStackSize()` at the pause, for `Debug.stepControl`.
    depth: usize,
    commands: mpsc::Sender<Command>,
}

/// The debugger's global state.
// Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:107 ThreadHandler.DebuggerState
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum DebuggerState {
    /// no StartDebuggingRequest has yet been received; all threads are paused
    Initializing = 0,
    /// all threads are paused in response to a PauseThreadRequest with id=0
    AllThreadsPaused = 1,
    /// normal running: threads are not globally paused
    Running = 2,
}

/// A breakpoint's location: path and line (all breakpoints cover the entire line).
type BreakpointKey = (String, u32);

#[derive(Default)]
struct Threads {
    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:120 ThreadHandler.pausedThreads
    paused_threads: HashMap<i64, PausedThreadState>,
    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:127 ThreadHandler.steppingThreads
    stepping_threads: HashMap<i64, ReadyToPause>,
}

/// Handles all thread-related operations of the debug server.
// Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:47 ThreadHandler
pub struct ThreadHandler {
    debugger_state: AtomicU8,
    threads: Mutex<Threads>,
    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:131 ThreadHandler.breakpoints
    breakpoints: RwLock<Arc<HashMap<BreakpointKey, Breakpoint>>>,
    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:149 ThreadHandler.threadsToPause
    threads_to_pause: Mutex<HashSet<i64>>,
}

thread_local! {
    /// Threads which are currently servicing a client request: no pausing inside an evaluation.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:146 ThreadHandler.servicingEvalRequest
    static SERVICING_EVAL_REQUEST: Cell<bool> = const { Cell::new(false) };
    static THREAD_ID: Cell<i64> = const { Cell::new(0) };
}

static NEXT_THREAD_ID: AtomicI64 = AtomicI64::new(1);

/// `Thread.currentThread().getId()`: a number unique to the OS thread in this process.
pub fn current_thread_id() -> i64 {
    THREAD_ID.with(|id| {
        if id.get() == 0 {
            id.set(NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed));
        }
        id.get()
    })
}

/// `Thread.currentThread().getName()`.
fn current_thread_name() -> String {
    std::thread::current().name().unwrap_or("").to_owned()
}

impl Default for ThreadHandler {
    fn default() -> Self {
        ThreadHandler {
            debugger_state: AtomicU8::new(DebuggerState::Initializing as u8),
            threads: Mutex::new(Threads::default()),
            breakpoints: RwLock::new(Arc::new(HashMap::new())),
            threads_to_pause: Mutex::new(HashSet::new()),
        }
    }
}

impl ThreadHandler {
    fn state(&self) -> DebuggerState {
        match self.debugger_state.load(Ordering::SeqCst) {
            0 => DebuggerState::Initializing,
            1 => DebuggerState::AllThreadsPaused,
            _ => DebuggerState::Running,
        }
    }

    fn set_state(&self, state: DebuggerState) {
        self.debugger_state.store(state as u8, Ordering::SeqCst);
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:151 ThreadHandler.pauseAllThreads
    pub fn pause_all_threads(&self) {
        self.set_state(DebuggerState::AllThreadsPaused);
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:156 ThreadHandler.pauseThread
    pub fn pause_thread(&self, thread_id: i64) -> Result<(), DebugRequestException> {
        let threads = self.threads.lock();
        if !threads.stepping_threads.contains_key(&thread_id) {
            let error = if threads.paused_threads.contains_key(&thread_id) {
                "Thread is already paused"
            } else {
                "Unknown thread: only threads which are currently stepping can be paused"
            };
            return Err(DebugRequestException(error.to_owned()));
        }
        self.threads_to_pause.lock().insert(thread_id);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:170 ThreadHandler.setBreakpoints
    pub fn set_breakpoints(&self, breakpoints: &[Breakpoint]) {
        let mut map = HashMap::new();
        for breakpoint in breakpoints {
            let Some(Condition::Location(location)) = &breakpoint.condition else {
                continue;
            };
            // all breakpoints cover the entire line, so unset the column number
            map.insert(
                (location.path.clone(), location.line_number),
                breakpoint.clone(),
            );
        }
        *self.breakpoints.write() = Arc::new(map);
    }

    /// Returns a set of paths of all the files that currently have breakpoints.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:187 ThreadHandler.getBreakpointFilePaths
    pub fn get_breakpoint_file_paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = Vec::new();
        for (path, _) in self.breakpoints.read().keys() {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
        paths
    }

    /// Resumes all threads. Any currently stepping threads have their stepping behavior cleared,
    /// so will run unconditionally.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:197 ThreadHandler.resumeAllThreads
    pub fn resume_all_threads(&self) {
        self.threads_to_pause.lock().clear();
        self.set_state(DebuggerState::Running);
        let mut threads = self.threads.lock();
        let ids: Vec<i64> = threads.paused_threads.keys().copied().collect();
        for id in ids {
            // continue-all doesn't support stepping.
            Self::resume_paused_thread(&mut threads, id, SteppingProto::None);
        }
        threads.stepping_threads.clear();
    }

    /// Unpauses the given thread if it is currently paused. Also unsets {@link
    /// #pauseAllThreads}.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:213 ThreadHandler.resumeThread
    pub fn resume_thread(
        &self,
        thread_id: i64,
        stepping: SteppingProto,
    ) -> Result<(), DebugRequestException> {
        // once the user has requested any thread be resumed, don't continue pausing future threads
        self.set_state(DebuggerState::Running);
        let mut threads = self.threads.lock();
        self.threads_to_pause.lock().remove(&thread_id);
        if threads.stepping_threads.remove(&thread_id).is_some() {
            return Ok(());
        }
        if !threads.paused_threads.contains_key(&thread_id) {
            return Err(DebugRequestException(format!(
                "Unknown thread {thread_id}: cannot resume."
            )));
        }
        Self::resume_paused_thread(&mut threads, thread_id, stepping);
        Ok(())
    }

    /// Unpauses a currently-paused thread.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:233 ThreadHandler.resumePausedThread
    fn resume_paused_thread(threads: &mut Threads, thread_id: i64, stepping: SteppingProto) {
        let Some(thread) = threads.paused_threads.remove(&thread_id) else {
            return;
        };
        if let Some(ready_to_pause) =
            debug::step_control(thread.depth, event_helper::convert_stepping_enum(stepping))
        {
            threads.stepping_threads.insert(thread_id, ready_to_pause);
        }
        let _ = thread.commands.send(Command::Resume);
    }

    /// Handles a paused thread's frame, evaluation and children requests.
    fn ask_paused<T>(
        &self,
        thread_id: i64,
        make: impl FnOnce(mpsc::Sender<T>) -> Command,
    ) -> Result<T, DebugRequestException> {
        let (reply, answer) = mpsc::channel();
        {
            let threads = self.threads.lock();
            let Some(thread) = threads.paused_threads.get(&thread_id) else {
                return Err(DebugRequestException(format!(
                    "Thread {thread_id} is not paused or does not exist."
                )));
            };
            thread.commands.send(make(reply)).map_err(|_| {
                DebugRequestException(format!(
                    "Thread {thread_id} is not paused or does not exist."
                ))
            })?;
        }
        answer.recv().map_err(|_| {
            DebugRequestException(format!(
                "Thread {thread_id} is not paused or does not exist."
            ))
        })
    }

    /// Pauses the current thread if there's an unresolved pause request, or the thread hit a
    /// breakpoint or a stepping condition.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:248 ThreadHandler.pauseIfNecessary
    pub fn pause_if_necessary(
        &self,
        eval: &mut Evaluator<'_, '_, '_>,
        location: &DebugLocation,
        transport: &DebugServerTransport,
    ) {
        if SERVICING_EVAL_REQUEST.with(|s| s.get()) {
            return;
        }
        let (pause_reason, error) = match self.should_pause_current_thread(eval, location) {
            Ok(None) => return,
            Ok(Some(reason)) => (reason, None),
            Err(message) => (
                PauseReason::ConditionalBreakpointError,
                Some(ErrorProto { message }),
            ),
        };
        let thread_id = current_thread_id();
        self.threads_to_pause.lock().remove(&thread_id);
        self.threads.lock().stepping_threads.remove(&thread_id);
        self.pause_current_thread(eval, location, transport, pause_reason, error);
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:273 ThreadHandler.listFrames
    pub fn list_frames(&self, thread_id: i64) -> Result<Vec<Frame>, DebugRequestException> {
        self.ask_paused(thread_id, Command::ListFrames)
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:288 ThreadHandler.getChildrenForValue
    pub fn get_children_for_value(
        &self,
        thread_id: i64,
        value_id: i64,
    ) -> Result<Vec<ValueProto>, DebugRequestException> {
        self.ask_paused(thread_id, |reply| Command::GetChildren(value_id, reply))?
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:306 ThreadHandler.evaluate
    pub fn evaluate(
        &self,
        thread_id: i64,
        statement: &str,
    ) -> Result<ValueProto, DebugRequestException> {
        self.ask_paused(thread_id, |reply| {
            Command::Evaluate(statement.to_owned(), reply)
        })?
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:357 ThreadHandler.pauseCurrentThread
    fn pause_current_thread<'v>(
        &self,
        eval: &mut Evaluator<'v, '_, '_>,
        location: &DebugLocation,
        transport: &DebugServerTransport,
        pause_reason: PauseReason,
        conditional_breakpoint_error: Option<ErrorProto>,
    ) {
        let thread_id = current_thread_id();
        let (commands, requests) = mpsc::channel();
        self.threads.lock().paused_threads.insert(
            thread_id,
            PausedThreadState {
                depth: debug::call_stack_size(eval),
                commands,
            },
        );
        // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:405 ThreadHandler.getPausedThreadProto
        let thread_proto = PausedThread {
            id: thread_id,
            name: current_thread_name(),
            pause_reason: pause_reason as i32,
            location: event_helper::get_location_proto(Some(location)),
            conditional_breakpoint_error,
        };
        transport.post_event(&event_helper::thread_paused_event(thread_proto));
        // The values shown to the client while this thread is paused.
        let mut object_map: ThreadObjectMap<'v> = ThreadObjectMap::default();
        while let Ok(command) = requests.recv() {
            match command {
                Command::Resume => break,
                Command::ListFrames(reply) => {
                    let frames = list_frames_of(eval, location, &mut object_map);
                    let _ = reply.send(frames);
                }
                Command::GetChildren(value_id, reply) => {
                    let result = match object_map.get_value(value_id) {
                        None => Err(DebugRequestException(
                            "Couldn't retrieve children; object not found.".to_owned(),
                        )),
                        Some(value) => Ok(get_children(&mut object_map, value, eval.heap())),
                    };
                    let _ = reply.send(result);
                }
                Command::Evaluate(statement, reply) => {
                    let result = match do_evaluate(eval, location, &statement) {
                        Ok(value) => Ok(get_value_proto(
                            &mut object_map,
                            "Evaluation result",
                            DebugObject::Value(value),
                        )),
                        Err(message) => Err(DebugRequestException(message)),
                    };
                    let _ = reply.send(result);
                }
            }
        }
        transport.post_event(&event_helper::thread_continued_event(thread_id));
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:377 ThreadHandler.shouldPauseCurrentThread
    fn should_pause_current_thread(
        &self,
        eval: &mut Evaluator<'_, '_, '_>,
        location: &DebugLocation,
    ) -> Result<Option<PauseReason>, String> {
        let thread_id = current_thread_id();
        match self.state() {
            DebuggerState::AllThreadsPaused => return Ok(Some(PauseReason::AllThreadsPaused)),
            DebuggerState::Initializing => return Ok(Some(PauseReason::Initializing)),
            DebuggerState::Running => {}
        }
        if self.threads_to_pause.lock().contains(&thread_id) {
            return Ok(Some(PauseReason::PauseThreadRequest));
        }
        if self.has_breakpoint_matched_at_location(eval, location)? {
            return Ok(Some(PauseReason::HitBreakpoint));
        }
        let depth = debug::call_stack_size(eval);
        let threads = self.threads.lock();
        if let Some(ready_to_pause) = threads.stepping_threads.get(&thread_id) {
            if ready_to_pause(depth) {
                return Ok(Some(PauseReason::Stepping));
            }
        }
        Ok(None)
    }

    /// Returns true if there's a breakpoint at the current location, with a satisfied condition
    /// if relevant.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:403 ThreadHandler.hasBreakpointMatchedAtLocation
    fn has_breakpoint_matched_at_location(
        &self,
        eval: &mut Evaluator<'_, '_, '_>,
        location: &DebugLocation,
    ) -> Result<bool, String> {
        // breakpoints is volatile, so taking a local copy
        let breakpoints = self.breakpoints.read().clone();
        if breakpoints.is_empty() {
            return Ok(false);
        }
        let Some(breakpoint) = breakpoints.get(&(location.file.clone(), location.line as u32))
        else {
            return Ok(false);
        };
        let condition = &breakpoint.expression;
        if condition.is_empty() {
            return Ok(true);
        }
        // Starlark.truth(doEvaluate(thread, condition)); an error is a ConditionalBreakpointException.
        do_evaluate(eval, location, condition).map(|v| v.to_bool())
    }
}

/// The frames of the paused thread, innermost first.
// Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:273 ThreadHandler.listFrames
fn list_frames_of<'v>(
    eval: &mut Evaluator<'v, '_, '_>,
    current: &DebugLocation,
    object_map: &mut ThreadObjectMap<'v>,
) -> Vec<Frame> {
    let frames = debug::frames(eval, current);
    let live_file = frames
        .last()
        .and_then(|f| f.location.as_ref())
        .map(|l| l.file.clone());
    let mut result = Vec::with_capacity(frames.len());
    for frame in &frames {
        let locals: Vec<(String, Value<'v>)> = match frame.frame_index {
            Some(i) => eval.local_variables_at_frame(i).into_iter().collect(),
            None => Vec::new(),
        };
        let file = frame.location.as_ref().map(|l| l.file.as_str());
        let module_vars = module_globals(eval, file, live_file.as_deref());
        result.push(event_helper::get_frame_proto(
            object_map,
            &frame.function_name,
            frame.location.as_ref(),
            locals,
            module_vars,
        ));
    }
    result
}

/// `((StarlarkFunction) frame.getFunction()).getModule().getGlobals()`: the bindings of the
/// module the frame's code belongs to; the module being executed when that is the file at the
/// bottom of the stack, else the frozen module recorded for the file.
fn module_globals<'v>(
    eval: &Evaluator<'v, '_, '_>,
    file: Option<&str>,
    live_file: Option<&str>,
) -> Vec<(String, Value<'v>)> {
    let Some(file) = file else {
        return Vec::new();
    };
    if Some(file) == live_file {
        let module = eval.module();
        return module
            .names()
            .filter_map(|name| {
                module
                    .get(name.as_str())
                    .map(|v| (name.as_str().to_owned(), v))
            })
            .collect();
    }
    match debug::module_of_file(file) {
        Some(frozen) => frozen
            .names()
            .filter_map(|name| {
                frozen.get_option(name.as_str()).ok().flatten().map(|v| {
                    (
                        name.as_str().to_owned(),
                        blaze_starlark::eval::frozen_into(eval.module(), &v),
                    )
                })
            })
            .collect(),
        None => Vec::new(),
    }
}

/// Evaluates `content` in the module of the innermost Starlark function (its globals; the
/// frame's locals are not visible, as `Starlark.execFile` of a new file in that module).
///
/// PORT: starlark-rust evaluates debugger statements in the paused frame
/// (`Evaluator::eval_statements`), where the frame's locals are visible too; a statement that
/// reads a name bound only as a local of the frame fails with Java's resolver error instead. A
/// local that shadows a global is read as the local (Java reads the global).
// Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadHandler.java:338 ThreadHandler.doEvaluate
fn do_evaluate<'v>(
    eval: &mut Evaluator<'v, '_, '_>,
    current: &DebugLocation,
    content: &str,
) -> Result<Value<'v>, String> {
    struct Servicing;
    impl Drop for Servicing {
        fn drop(&mut self) {
            SERVICING_EVAL_REQUEST.with(|s| s.set(false));
        }
    }
    SERVICING_EVAL_REQUEST.with(|s| s.set(true));
    let _servicing = Servicing;
    // TODO(adonovan): opt: don't parse and resolve the expression every time we hit a breakpoint
    let ast = AstModule::parse("<debug eval>", content.to_owned(), &FileKind::Bzl.dialect())
        .map_err(|e| blaze_starlark::error::EvalError::from_starlark(&e).message)?;
    if eval.call_stack_count() > 0 {
        let frames = debug::frames(eval, current);
        let live_file = frames
            .last()
            .and_then(|f| f.location.as_ref())
            .map(|l| l.file.clone());
        let globals: HashSet<String> =
            module_globals(eval, Some(&current.file), live_file.as_deref())
                .into_iter()
                .map(|(n, _)| n)
                .collect();
        let locals: HashSet<String> = eval.local_variables().keys().cloned().collect();
        for name in free_identifiers(content) {
            if locals.contains(&name) && !globals.contains(&name) {
                return Err(format!("name '{name}' is not defined"));
            }
        }
    }
    eval.eval_statements(ast)
        .map_err(|e| blaze_starlark::error::EvalError::from_starlark(&e).message_with_stack())
}

/// The identifiers of a statement that are references (not attribute names after `.`, not
/// keyword-argument names, not inside strings or comments).
fn free_identifiers(content: &str) -> Vec<String> {
    let chars: Vec<char> = content.chars().collect();
    let mut result = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '"' || c == '\'' {
            let triple = i + 2 < chars.len() && chars[i + 1] == c && chars[i + 2] == c;
            let quote_len = if triple { 3 } else { 1 };
            i += quote_len;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == c
                    && (!triple || (i + 2 < chars.len() && chars[i + 1] == c && chars[i + 2] == c))
                {
                    i += quote_len;
                    break;
                }
                i += 1;
            }
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let prev = chars[..start].iter().rev().find(|c| !c.is_whitespace());
            let next = chars[i..]
                .iter()
                .position(|c| !c.is_whitespace())
                .map(|p| i + p);
            let is_attribute = prev == Some(&'.');
            let is_kwarg = next.is_some_and(|n| chars[n] == '=' && chars.get(n + 1) != Some(&'='))
                && prev.is_some_and(|p| *p == '(' || *p == ',');
            // A string prefix (r"...", b'...').
            let is_prefix = matches!(word.as_str(), "r" | "b" | "rb" | "br")
                && chars.get(i).is_some_and(|c| *c == '"' || *c == '\'');
            if !is_attribute && !is_kwarg && !is_prefix {
                result.push(word);
            }
            continue;
        } else {
            i += 1;
            continue;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::free_identifiers;

    #[test]
    fn free_identifiers_skip_strings_attributes_and_keywords() {
        assert_eq!(
            free_identifiers("f(x.y, 'z', k = w) # c"),
            vec!["f", "x", "w"]
        );
        assert_eq!(free_identifiers("my_arg[1]"), vec!["my_arg"]);
        assert_eq!(free_identifiers("a == b"), vec!["a", "b"]);
    }
}
