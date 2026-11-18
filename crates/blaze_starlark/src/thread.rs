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
use blaze_cmdline::RepositoryMapping;
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
}

/// One recorded event (see [`CollectingEvents`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// "DEBUG" or "WARNING".
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
