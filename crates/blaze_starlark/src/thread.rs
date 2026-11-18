//! Per-evaluation context, carried in `Evaluator::extra`.
//!
//! Java: the thread-locals of `net.starlark.java.eval.StarlarkThread` that Bazel installs per
//! evaluation: `com.google.devtools.build.lib.cmdline.StarlarkThreadContext` (main repo
//! mapping), `BazelStarlarkContext` (phase, symbol generator), `BazelModuleContext` (the .bzl's
//! label and repo mapping), and the phase-specific owners (`Package.Builder`, `RuleContext`,
//! `StarlarkRepositoryContext`, `ModuleExtensionContext`, `ModuleThreadContext`).
//!
//! The phase-specific state belongs to crates above this one, so it is an opaque payload:
//! a crate stores its own type with [`ThreadContext::with_payload`] and natives get it back with
//! [`ThreadContext::payload`].

use std::any::Any;
use std::any::TypeId;
use std::cell::Cell;
use std::cell::RefCell;
use std::cell::RefMut;
use std::sync::Arc;
use std::sync::Mutex;

use std::collections::HashMap;

use blaze_cmdline::Label;
use blaze_cmdline::LabelPackageContext;
use blaze_cmdline::LabelRepoContext;
use blaze_cmdline::LabelRepoContextImpl;
use blaze_cmdline::LabelRepoMappingRecorder;
use blaze_cmdline::LabelSimpleRepoMappingRecorder;
use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::RepositoryName;
use starlark::any::ProvidesStaticType;
use starlark::eval::Evaluator;

use crate::error::Location;
use crate::semantics::StarlarkSemantics;

/// Which Bazel evaluation this is (Java `BazelStarlarkContext.Phase` plus the thread owners).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Top-level evaluation of a .bzl / .scl file (BzlLoadFunction).
    BzlLoad,
    /// A BUILD file and the macros it calls (PackageFunction).
    Loading,
    /// A rule or aspect implementation function.
    Analysis,
    /// A repository rule implementation.
    Repository,
    /// A module extension implementation.
    ModuleExtension,
    /// MODULE.bazel evaluation.
    ModuleFile,
    /// REPO.bazel / VENDOR.bazel evaluation.
    RepoFile,
    /// Anything else (cquery output scripts, tests).
    Other,
}

/// Receives Starlark-originated events. Java routes these through the thread's `EventHandler`.
pub trait StarlarkEvents: Send + Sync {
    /// `print()` output: Bazel shows `DEBUG: <location>: <message>`.
    fn debug(&self, location: Option<&Location>, message: &str);
    /// A warning (e.g. deprecations).
    fn warn(&self, location: Option<&Location>, message: &str);
    /// An error event (Java `Event.error` posted to the thread's handler, e.g. by
    /// `StarlarkThread`-owned natives that report and continue). Java's BzlLoadFunction and
    /// PackageFunction wrap the handler to notice errors (`sawStarlarkError`).
    ///
    /// The default fails loudly: sinks that can receive errors must implement it.
    fn error(&self, location: Option<&Location>, message: &str) {
        let _ = location;
        panic!("StarlarkEvents::error is not implemented by this event sink: {message}");
    }
}

/// One recorded event (see [`CollectingEvents`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// "DEBUG", "WARNING" or "ERROR".
    pub kind: &'static str,
    pub location: Option<Location>,
    pub message: String,
}

impl Event {
    /// Bazel's console form: `DEBUG: <file>:<line>:<col>: <msg>`.
    pub fn to_console(&self) -> String {
        match &self.location {
            Some(l) => format!("{}: {}: {}", self.kind, l, self.message),
            None => format!("{}: {}", self.kind, self.message),
        }
    }
}

/// An event sink that records everything, in order.
#[derive(Default)]
pub struct CollectingEvents {
    pub events: Mutex<Vec<Event>>,
}

impl CollectingEvents {
    pub fn take(&self) -> Vec<Event> {
        std::mem::take(&mut self.events.lock().unwrap())
    }
}

impl StarlarkEvents for CollectingEvents {
    fn debug(&self, location: Option<&Location>, message: &str) {
        self.events.lock().unwrap().push(Event {
            kind: "DEBUG",
            location: location.cloned(),
            message: message.to_owned(),
        });
    }

    fn warn(&self, location: Option<&Location>, message: &str) {
        self.events.lock().unwrap().push(Event {
            kind: "WARNING",
            location: location.cloned(),
            message: message.to_owned(),
        });
    }

    fn error(&self, location: Option<&Location>, message: &str) {
        self.events.lock().unwrap().push(Event {
            kind: "ERROR",
            location: location.cloned(),
            message: message.to_owned(),
        });
    }
}

impl CollectingEvents {
    /// Whether an "ERROR" event was recorded (Java's `sawStarlarkError` wrapper in
    /// BzlLoadFunction.executeBzlFile).
    pub fn saw_error(&self) -> bool {
        self.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.kind == "ERROR")
    }
}

/// The `Evaluator::extra` payload of every Blaze evaluation.
#[derive(ProvidesStaticType)]
pub struct ThreadContext {
    pub phase: Phase,
    pub semantics: Arc<StarlarkSemantics>,
    /// Label of the file being evaluated (BUILD file label, .bzl label, …). Provider and rule
    /// identities are derived from it (Java `BazelModuleContext.label()`).
    pub file_label: Option<Label>,
    /// Repo context for `Label()` (Java: the defining module's repo mapping).
    pub label_repo_context: Option<LabelRepoContextImpl>,
    /// Package context per Starlark file name (as passed to `compile`), for `Label()` called
    /// from functions of loaded .bzl files (Java `BazelModuleContext.packageContext()` of the
    /// innermost .bzl on the stack).
    pub file_package_contexts: HashMap<String, LabelPackageContext>,
    /// Main repository mapping, for `print()` of labels (Java `StarlarkThreadContext`).
    pub main_repo_mapping: Option<RepositoryMapping>,
    pub events: Arc<dyn StarlarkEvents>,
    /// Java `SymbolGenerator`: identity tokens for unexported providers, rules, aspects.
    next_symbol: Cell<u64>,
    payload: RefCell<Option<Box<dyn Any>>>,
    /// Java: the other thread-locals of the `StarlarkThread`, keyed by class (see
    /// [`ThreadContext::set_thread_local`]).
    thread_locals: RefCell<HashMap<TypeId, Box<dyn Any>>>,
    /// Java: the `Label.RepoMappingRecorder` thread-local (see [`ThreadRepoMappingRecorder`]).
    repo_mapping_recorder: RefCell<Option<ThreadRepoMappingRecorder>>,
}

/// A callback recorder: receives the thread so it can write into the payload (Java
/// `StarlarkBaseExternalContext.repoMappingRecorder`, which records into the context's
/// recorded inputs).
pub type RepoMappingRecorderFn =
    dyn Fn(&ThreadContext, &RepositoryName, &str, &RepositoryName) + 'static;

/// The `Label.RepoMappingRecorder` a thread carries (Java
/// `thread.setThreadLocal(Label.RepoMappingRecorder.class, ...)`).
// Java: com/google/devtools/build/lib/cmdline/Label.java:248 Label.RepoMappingRecorder
pub enum ThreadRepoMappingRecorder {
    /// Java `Label.SimpleRepoMappingRecorder`: BUILD and .bzl load threads
    /// (BzlLoadFunction.executeBzlFile).
    Simple(LabelSimpleRepoMappingRecorder),
    /// Any other recorder (repository rules and module extensions:
    /// `StarlarkBaseExternalContext.storeRepoMappingRecorderInThread`).
    Callback(std::rc::Rc<RepoMappingRecorderFn>),
}

impl std::fmt::Debug for ThreadRepoMappingRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThreadRepoMappingRecorder::Simple(r) => f.debug_tuple("Simple").field(r).finish(),
            ThreadRepoMappingRecorder::Callback(_) => f.write_str("Callback(..)"),
        }
    }
}

/// Adapts the thread's recorder to blaze_cmdline's [`LabelRepoMappingRecorder`], for
/// [`blaze_cmdline::Label::parse_with_package_context_recording`].
pub struct ThreadRecorderAdapter<'a> {
    pub thread: &'a ThreadContext,
}

impl LabelRepoMappingRecorder for ThreadRecorderAdapter<'_> {
    fn record(
        &mut self,
        from_repo: &RepositoryName,
        apparent_repo_name: &str,
        canonical_repo_name: &RepositoryName,
    ) {
        self.thread
            .record_repo_mapping(from_repo, apparent_repo_name, canonical_repo_name);
    }
}

impl ThreadContext {
    pub fn new(
        phase: Phase,
        semantics: Arc<StarlarkSemantics>,
        events: Arc<dyn StarlarkEvents>,
    ) -> Self {
        ThreadContext {
            phase,
            semantics,
            file_label: None,
            label_repo_context: None,
            file_package_contexts: HashMap::new(),
            main_repo_mapping: None,
            events,
            next_symbol: Cell::new(0),
            payload: RefCell::new(None),
            thread_locals: RefCell::new(HashMap::new()),
            repo_mapping_recorder: RefCell::new(None),
        }
    }

    /// Stores a `Label.SimpleRepoMappingRecorder` in the thread (Java
    /// `thread.setThreadLocal(Label.RepoMappingRecorder.class, repoMappingRecorder)` in
    /// BzlLoadFunction.executeBzlFile).
    // Java: com/google/devtools/build/lib/cmdline/Label.java:257 Label.RepoMappingRecorder.storeInThread
    pub fn with_simple_repo_mapping_recorder(
        self,
        recorder: LabelSimpleRepoMappingRecorder,
    ) -> Self {
        *self.repo_mapping_recorder.borrow_mut() =
            Some(ThreadRepoMappingRecorder::Simple(recorder));
        self
    }

    /// Stores a callback recorder in the thread (Java
    /// `StarlarkBaseExternalContext.storeRepoMappingRecorderInThread`).
    pub fn with_repo_mapping_recorder_fn(
        self,
        recorder: impl Fn(&ThreadContext, &RepositoryName, &str, &RepositoryName) + 'static,
    ) -> Self {
        *self.repo_mapping_recorder.borrow_mut() = Some(ThreadRepoMappingRecorder::Callback(
            std::rc::Rc::new(recorder),
        ));
        self
    }

    /// Whether the thread has a repo mapping recorder (Java
    /// `thread.getThreadLocal(Label.RepoMappingRecorder.class) != null`).
    pub fn has_repo_mapping_recorder(&self) -> bool {
        self.repo_mapping_recorder.borrow().is_some()
    }

    /// Records an apparent-name lookup in the thread's recorder, if any (Java
    /// `RepoMappingRecorder.record`).
    // Java: com/google/devtools/build/lib/cmdline/Label.java:249 Label.RepoMappingRecorder.record
    pub fn record_repo_mapping(
        &self,
        from_repo: &RepositoryName,
        apparent_repo_name: &str,
        canonical_repo_name: &RepositoryName,
    ) {
        let callback = {
            let mut slot = self.repo_mapping_recorder.borrow_mut();
            match slot.as_mut() {
                None => return,
                Some(ThreadRepoMappingRecorder::Simple(r)) => {
                    r.record(from_repo, apparent_repo_name, canonical_repo_name);
                    return;
                }
                Some(ThreadRepoMappingRecorder::Callback(f)) => f.clone(),
            }
        };
        // The slot is released first: the callback may borrow the payload or the thread.
        callback(self, from_repo, apparent_repo_name, canonical_repo_name);
    }

    /// The recorder for `parse_with_package_context_recording`, or `None` when the thread has
    /// none (Java passes the possibly-null thread-local).
    pub fn repo_mapping_recorder(&self) -> Option<ThreadRecorderAdapter<'_>> {
        if self.has_repo_mapping_recorder() {
            Some(ThreadRecorderAdapter { thread: self })
        } else {
            None
        }
    }

    /// The entries of the thread's `SimpleRepoMappingRecorder` so far (Java
    /// `((Label.SimpleRepoMappingRecorder) thread.getThreadLocal(...)).recordedEntries()`);
    /// `None` if the thread has no recorder or a recorder of another kind.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:275 Label.SimpleRepoMappingRecorder.recordedEntries
    pub fn recorded_repo_mapping_entries(
        &self,
    ) -> Option<Vec<(RepositoryName, String, RepositoryName)>> {
        match self.repo_mapping_recorder.borrow().as_ref() {
            Some(ThreadRepoMappingRecorder::Simple(r)) => Some(r.recorded_entries()),
            _ => None,
        }
    }

    /// Takes the `SimpleRepoMappingRecorder` back after evaluation.
    pub fn take_simple_repo_mapping_recorder(&self) -> Option<LabelSimpleRepoMappingRecorder> {
        let mut slot = self.repo_mapping_recorder.borrow_mut();
        match slot.take() {
            Some(ThreadRepoMappingRecorder::Simple(r)) => Some(r),
            other => {
                *slot = other;
                None
            }
        }
    }

    pub fn with_file_label(mut self, label: Label) -> Self {
        self.file_label = Some(label);
        self
    }

    pub fn with_label_repo_context(mut self, ctx: LabelRepoContextImpl) -> Self {
        self.label_repo_context = Some(ctx);
        self
    }

    /// Registers the package context of a Starlark file (by the name it was compiled with).
    pub fn with_file_package_context(
        mut self,
        file: impl Into<String>,
        ctx: LabelPackageContext,
    ) -> Self {
        self.file_package_contexts.insert(file.into(), ctx);
        self
    }

    /// The package context `Label()` uses for code in `file`: the registered one, else the
    /// package of [`ThreadContext::file_label`] under [`ThreadContext::label_repo_context`].
    pub fn package_context_for_file(&self, file: &str) -> Option<LabelPackageContext> {
        if let Some(c) = self.file_package_contexts.get(file) {
            return Some(c.clone());
        }
        let repo = self.label_repo_context.as_ref()?;
        let label = self.file_label.as_ref()?;
        Some(LabelPackageContext::of(
            label.get_package_identifier(),
            repo.repo_mapping().clone(),
        ))
    }

    pub fn with_main_repo_mapping(mut self, mapping: RepositoryMapping) -> Self {
        self.main_repo_mapping = Some(mapping);
        self
    }

    /// Attaches phase-specific state owned by a higher crate (e.g. a package builder).
    pub fn with_payload<T: Any>(self, payload: T) -> Self {
        *self.payload.borrow_mut() = Some(Box::new(payload));
        self
    }

    /// Replaces the payload of a live thread (Java code that swaps a thread-local, e.g.
    /// `StarlarkThreadContext.storeInThread` on an existing thread).
    pub fn set_payload<T: Any>(&self, payload: T) {
        *self.payload.borrow_mut() = Some(Box::new(payload));
    }

    /// Removes and returns the payload whatever its type.
    pub fn clear_payload(&self) -> Option<Box<dyn Any>> {
        self.payload.borrow_mut().take()
    }

    /// Puts back a payload removed with [`ThreadContext::clear_payload`] (Java
    /// `StarlarkThreadContext.storeInThread` on a thread whose context was set to null for a
    /// while, e.g. around rule initializers). `None` leaves the thread without a payload.
    // Java: com/google/devtools/build/lib/cmdline/StarlarkThreadContext.java:48 StarlarkThreadContext.storeInThread
    pub fn restore_payload(&self, payload: Option<Box<dyn Any>>) {
        *self.payload.borrow_mut() = payload;
    }

    /// Java `StarlarkThread.setThreadLocal(T.class, value)` for the thread-locals that are not
    /// the phase payload (e.g. the `LabelConverter` of a running rule initializer): `Some`
    /// stores the value under the type `T`, `None` removes it.
    // Java: net/starlark/java/eval/StarlarkThread.java:130 StarlarkThread.setThreadLocal
    pub fn set_thread_local<T: Any>(&self, value: Option<T>) {
        let mut thread_locals = self.thread_locals.borrow_mut();
        match value {
            Some(value) => {
                thread_locals.insert(TypeId::of::<T>(), Box::new(value));
            }
            None => {
                thread_locals.remove(&TypeId::of::<T>());
            }
        }
    }

    /// Java `StarlarkThread.getThreadLocal(T.class)`: a clone of the value stored with
    /// [`ThreadContext::set_thread_local`], or `None`.
    // Java: net/starlark/java/eval/StarlarkThread.java:138 StarlarkThread.getThreadLocal
    pub fn get_thread_local<T: Any + Clone>(&self) -> Option<T> {
        self.thread_locals
            .borrow()
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref::<T>())
            .cloned()
    }

    /// Borrows the payload as `T`. `None` if there is none or it is another type: that is how a
    /// native detects it was called in the wrong phase.
    pub fn payload<T: Any>(&self) -> Option<RefMut<'_, T>> {
        let b = self.payload.borrow_mut();
        if b.as_ref().is_some_and(|p| p.is::<T>()) {
            Some(RefMut::map(b, |p| {
                p.as_mut().unwrap().downcast_mut::<T>().unwrap()
            }))
        } else {
            None
        }
    }

    /// Takes the payload back after evaluation.
    pub fn take_payload<T: Any>(&self) -> Option<T> {
        let mut b = self.payload.borrow_mut();
        if b.as_ref().is_some_and(|p| p.is::<T>()) {
            b.take().map(|p| *p.downcast::<T>().unwrap())
        } else {
            None
        }
    }

    /// Next identity token (Java `SymbolGenerator.generate`).
    pub fn next_symbol(&self) -> u64 {
        let n = self.next_symbol.get();
        self.next_symbol.set(n + 1);
        n
    }

    /// The context of the running evaluation.
    pub fn get<'a>(eval: &Evaluator<'_, 'a, '_>) -> anyhow::Result<&'a ThreadContext> {
        eval.extra
            .and_then(|e| e.downcast_ref::<ThreadContext>())
            .ok_or_else(|| anyhow::anyhow!("internal error: no Blaze thread context"))
    }

    /// Like [`ThreadContext::get`], failing with `msg` unless the phase is one of `phases`
    /// (e.g. `"'native.glob' can only be called during the loading phase"`).
    pub fn require<'a>(
        eval: &Evaluator<'_, 'a, '_>,
        phases: &[Phase],
        msg: impl FnOnce() -> String,
    ) -> anyhow::Result<&'a ThreadContext> {
        let cx = ThreadContext::get(eval)?;
        if phases.contains(&cx.phase) {
            Ok(cx)
        } else {
            Err(anyhow::anyhow!("{}", msg()))
        }
    }
}
