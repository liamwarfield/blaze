//! .bzl module and initialization contexts.
//!
//! Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java and
//! com/google/devtools/build/lib/packages/BzlInitThreadContext.java.
//!
//! PORT: Java attaches a `BazelModuleContext` to each `Module` (client data) and finds the module
//! of the innermost Starlark function on the stack. starlark-rust exposes neither, so:
//! - BZL_LOAD stores the context as the frozen module's extra value AND registers it in a
//!   process-wide table keyed by the file name the module was compiled with
//!   ([`BazelModuleContext::register`]); single build per process, so the table never shrinks.
//! - [`BazelModuleContext::of_innermost_bzl`] maps the innermost frame's file name through it.
//!
//! Owner: agent B (see DESIGN.md); D constructs these in BZL_LOAD.

use std::cell::RefMut;
use std::collections::HashMap;
use std::sync::Arc;

use blaze_cmdline::EvalException;
use blaze_cmdline::Label;
use blaze_cmdline::LabelPackageContext;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::RepositoryName;
use blaze_starlark::ThreadContext;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use starlark::eval::Evaluator;

use crate::target::PackageSpecification;

/// Java: cmdline/BazelModuleContext.java (AutoValue).
#[derive(Debug)]
pub struct BazelModuleContext {
    /// Java `key().getLabel()`. `is_builtins` tells `@_builtins` keys apart.
    pub label: Label,
    pub is_builtins: bool,
    pub repo_mapping: RepositoryMapping,
    /// The file name the module was compiled with (Java `filename()`; used in locations).
    pub filename: String,
    /// The directly loaded modules, in load order (Java `loads()`).
    pub loads: Vec<Arc<BazelModuleContext>>,
    /// Java `bzlTransitiveDigest()`.
    pub bzl_transitive_digest: Vec<u8>,
}

static BY_FILENAME: Lazy<RwLock<HashMap<String, Arc<BazelModuleContext>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

impl BazelModuleContext {
    // Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:45 BazelModuleContext.label
    pub fn label(&self) -> &Label {
        &self.label
    }

    // Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:206 BazelModuleContext.packageContext
    pub fn package_context(&self) -> LabelPackageContext {
        LabelPackageContext::of(
            self.label.get_package_identifier(),
            self.repo_mapping.clone(),
        )
    }

    /// Registers `ctx` for [`BazelModuleContext::of_innermost_bzl`]. Called by BZL_LOAD before
    /// executing the module.
    pub fn register(ctx: Arc<BazelModuleContext>) {
        BY_FILENAME.write().insert(ctx.filename.clone(), ctx);
    }

    /// The package context of every registered .bzl file, by compiled file name, for threads
    /// that call functions of loaded modules without a BZL_LOAD thread context (Java finds the
    /// module of the innermost Starlark function from the function itself).
    pub fn all_package_contexts() -> Vec<(String, LabelPackageContext)> {
        BY_FILENAME
            .read()
            .iter()
            .map(|(f, c)| (f.clone(), c.package_context()))
            .collect()
    }

    /// The context registered for a compiled file name.
    pub fn of_filename(filename: &str) -> Option<Arc<BazelModuleContext>> {
        BY_FILENAME.read().get(filename).cloned()
    }

    /// Java `ofInnermostBzlOrThrow` / module of the innermost Starlark function: `None` when the
    /// innermost frame is not in a registered .bzl (e.g. a BUILD file).
    pub fn of_innermost_bzl(eval: &Evaluator) -> Option<Arc<BazelModuleContext>> {
        let loc = eval.call_stack_top_location()?;
        Self::of_filename(loc.filename())
    }

    // Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:174 BazelModuleContext.ofInnermostBzlOrFail
    pub fn of_innermost_bzl_or_fail(
        eval: &Evaluator,
        what: &str,
    ) -> Result<Arc<BazelModuleContext>, EvalException> {
        Self::of_innermost_bzl(eval).ok_or_else(|| {
            EvalException::new(format!(
                "{what} can only be used during .bzl initialization (top-level evaluation)"
            ))
        })
    }

    /// Java `visitLoadGraphRecursively`: every module reachable through loads, each once,
    /// depth-first in load order; `visitor` returns false to skip a module's loads.
    // Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:88 BazelModuleContext.visitLoadGraphRecursively
    pub fn visit_load_graph_recursively(
        loads: &[Arc<BazelModuleContext>],
        visitor: &mut dyn FnMut(&Arc<BazelModuleContext>) -> bool,
    ) {
        let mut seen = std::collections::HashSet::new();
        fn go(
            loads: &[Arc<BazelModuleContext>],
            seen: &mut std::collections::HashSet<*const BazelModuleContext>,
            visitor: &mut dyn FnMut(&Arc<BazelModuleContext>) -> bool,
        ) {
            for m in loads {
                if seen.insert(Arc::as_ptr(m)) && visitor(m) {
                    go(&m.loads, seen, visitor);
                }
            }
        }
        go(loads, &mut seen, visitor);
    }
}

/// Who may load a .bzl file (Java `packages/BzlVisibility`).
// Java: com/google/devtools/build/lib/packages/BzlVisibility.java:30 BzlVisibility
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BzlVisibility {
    /// Java `BzlVisibility.PUBLIC`: everyone may load the .bzl.
    Public,
    /// Java `BzlVisibility.PRIVATE`: only BUILD and .bzl files of the same package.
    Private,
    /// Java `ListBzlVisibility`: packages matching one of the specifications.
    PackageList(Vec<PackageSpecification>),
}

impl BzlVisibility {
    /// Returns a `BzlVisibility` that allows access as per the given package specifications.
    // Java: com/google/devtools/build/lib/packages/BzlVisibility.java:43 BzlVisibility.of
    pub fn of(specs: Vec<PackageSpecification>) -> BzlVisibility {
        if specs.is_empty()
            || (specs.len() == 1 && matches!(specs[0], PackageSpecification::NoPackages))
        {
            return BzlVisibility::Private;
        }
        for spec in &specs {
            if matches!(spec, PackageSpecification::AllPackages) {
                return BzlVisibility::Public;
            }
        }
        BzlVisibility::PackageList(specs)
    }

    /// Whether the given package's BUILD and .bzl files may load the .bzl. This does not include
    /// cases where `pkg` is the package containing the .bzl.
    // Java: com/google/devtools/build/lib/packages/BzlVisibility.java:40 BzlVisibility.allowsPackage
    pub fn allows_package(&self, pkg: &PackageIdentifier) -> bool {
        match self {
            BzlVisibility::Public => true,
            BzlVisibility::Private => false,
            // Java: com/google/devtools/build/lib/packages/BzlVisibility.java:93 BzlVisibility.ListBzlVisibility.allowsPackage
            BzlVisibility::PackageList(specs) => specs.iter().any(|s| s.contains_package(pkg)),
        }
    }
}

/// Java: packages/BzlInitThreadContext.java. The `ThreadContext` payload during .bzl top-level
/// evaluation.
#[derive(Debug)]
pub struct BzlInitThreadContext {
    pub bzl_file: Label,
    pub transitive_digest: Vec<u8>,
    pub bzl_visibility: Option<BzlVisibility>,
    pub tools_repository: RepositoryName,
    /// Java `fragmentNameToClass` keys (fragment names known to this Bazel).
    pub fragment_names: Vec<String>,
    pub main_repo_mapping: RepositoryMapping,
}

impl BzlInitThreadContext {
    // Java: com/google/devtools/build/lib/packages/BzlInitThreadContext.java:84 BzlInitThreadContext.fromOrFail
    pub fn from_or_fail<'a>(
        eval: &Evaluator<'_, 'a, '_>,
        what: &str,
    ) -> Result<RefMut<'a, BzlInitThreadContext>, EvalException> {
        Self::from_or_null(eval).ok_or_else(|| {
            EvalException::new(format!(
                "{what} can only be used during .bzl initialization (top-level evaluation)"
            ))
        })
    }

    // Java: com/google/devtools/build/lib/packages/BzlInitThreadContext.java:98 BzlInitThreadContext.fromOrNull
    pub fn from_or_null<'a>(
        eval: &Evaluator<'_, 'a, '_>,
    ) -> Option<RefMut<'a, BzlInitThreadContext>> {
        ThreadContext::get(eval)
            .ok()?
            .payload::<BzlInitThreadContext>()
    }

    /// The repo mapping entries recorded so far by the .bzl thread's
    /// `Label.SimpleRepoMappingRecorder` (load statements, transitively loaded .bzls and
    /// `Label()` calls), or `None` when the thread has no such recorder.
    ///
    /// Java: `(Label.SimpleRepoMappingRecorder) thread.getThreadLocal(Label.RepoMappingRecorder
    /// .class)` then `recordedEntries()` (StarlarkRepositoryModule.repositoryRule). The recorder
    /// is a thread-local next to this context; it lives in [`ThreadContext`].
    // Java: com/google/devtools/build/lib/cmdline/Label.java:275 Label.SimpleRepoMappingRecorder.recordedEntries
    pub fn recorded_repo_mapping_entries(
        eval: &Evaluator<'_, '_, '_>,
    ) -> Option<Vec<(RepositoryName, String, RepositoryName)>> {
        ThreadContext::get(eval)
            .ok()?
            .recorded_repo_mapping_entries()
    }
}
