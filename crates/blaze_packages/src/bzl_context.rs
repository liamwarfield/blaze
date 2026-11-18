//! .bzl module and initialization contexts.
//!
//! Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java and
//! com/google/devtools/build/lib/packages/BzlInitThreadContext.java.
//!
//! PORT: Java attaches a `BazelModuleContext` to each `Module` (client data) and finds the module
//! of the innermost Starlark function on the stack. Here BZL_LOAD stores the context as the
//! module's extra value ([`BazelModuleContextValue`]), and
//! [`BazelModuleContext::of_innermost_bzl`], [`BazelModuleContext::of_function`] and
//! [`BazelModuleContext::of_innermost_enclosing_starlark_function`] read it from the function's
//! own module (starlark-rust patches 0017 and 0022, blaze_starlark `defining_module`). The same
//! .bzl file executed for several BZL_LOAD keys (BUILD-loaded, bzlmod bootstrap) gives several
//! modules, each with its own context.
//!
//! The process-wide table keyed by compiled file name ([`BazelModuleContext::register`],
//! [`BazelModuleContext::of_filename`], [`BazelModuleContext::all_package_contexts`]) cannot
//! tell those modules apart (last writer wins). Nothing in this crate reads it any more; it
//! stays for callers in other crates until they move to the functions above.
//!
//! Owner: agent B (see DESIGN.md); D constructs these in BZL_LOAD.

use std::cell::RefMut;
use std::collections::HashMap;
use std::sync::Arc;

use allocative::Allocative;
use blaze_cmdline::EvalException;
use blaze_cmdline::Label;
use blaze_cmdline::LabelPackageContext;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::RepositoryName;
use blaze_starlark::ThreadContext;
use blaze_starlark::defining_module;
use blaze_starlark::defining_module::ModulePackageContext;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use starlark::any::ProvidesStaticType;
use starlark::eval::Evaluator;
use starlark::starlark_simple_value;
use starlark::values::Demand;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

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

/// The `BazelModuleContext` as a Starlark value: the extra value of every .bzl module (Java
/// `Module.setClientData(BazelModuleContext)`), set before the module executes and frozen with
/// it.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BazelModuleContextValue(#[allocative(skip)] pub Arc<BazelModuleContext>);

starlark_simple_value!(BazelModuleContextValue);

/// Java prints the label (a def's repr is `<function f from //pkg:file.bzl>`, patch 0009).
// Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:133 BazelModuleContext.toString
impl std::fmt::Display for BazelModuleContextValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.label)
    }
}

#[starlark_value(type = "BazelModuleContext")]
impl<'v> StarlarkValue<'v> for BazelModuleContextValue {
    // `Label()` (blaze_starlark) reads the module's package context through this.
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(ModulePackageContext(self.0.package_context()));
    }
}

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

    /// Registers `ctx` in the table keyed by compiled file name (see the module comment: the
    /// table is ambiguous; lookups in this crate do not use it).
    pub fn register(ctx: Arc<BazelModuleContext>) {
        BY_FILENAME.write().insert(ctx.filename.clone(), ctx);
    }

    /// The package context of every registered .bzl file, by compiled file name. Ambiguous for
    /// a file executed under several BZL_LOAD keys (see the module comment); `Label()` no longer
    /// needs it, since it reads the calling function's own module.
    pub fn all_package_contexts() -> Vec<(String, LabelPackageContext)> {
        BY_FILENAME
            .read()
            .iter()
            .map(|(f, c)| (f.clone(), c.package_context()))
            .collect()
    }

    /// The context last registered for a compiled file name. Ambiguous for a file executed
    /// under several BZL_LOAD keys; use [`BazelModuleContext::of_function`] or
    /// [`BazelModuleContext::of_innermost_enclosing_starlark_function`].
    pub fn of_filename(filename: &str) -> Option<Arc<BazelModuleContext>> {
        BY_FILENAME.read().get(filename).cloned()
    }

    /// Java `BazelModuleContext.of(module)` for a module's extra value (client data): `None`
    /// when it is not a `BazelModuleContext` (a BUILD file's module, for instance).
    // Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:138 BazelModuleContext.of
    pub fn of_module_extra_value(extra_value: Value) -> Option<Arc<BazelModuleContext>> {
        extra_value
            .downcast_ref::<BazelModuleContextValue>()
            .map(|v| v.0.clone())
    }

    /// Java `BazelModuleContext.of(fn.getModule())` for a `def` or `lambda`.
    // Java: net/starlark/java/eval/StarlarkFunction.java:195 StarlarkFunction.getModule
    pub fn of_function<'v>(
        function: Value<'v>,
        eval: &Evaluator<'v, '_, '_>,
    ) -> Option<Arc<BazelModuleContext>> {
        defining_module::module_extra_value_of_function(function, eval)
            .and_then(Self::of_module_extra_value)
    }

    /// Java `BazelModuleContext.of(Module.ofInnermostEnclosingStarlarkFunction(thread, depth))`:
    /// `None` when there is no such function or its module has no `BazelModuleContext`.
    // Java: net/starlark/java/eval/Module.java:121 Module.ofInnermostEnclosingStarlarkFunction
    pub fn of_innermost_enclosing_starlark_function<'v>(
        eval: &Evaluator<'v, '_, '_>,
        depth: usize,
    ) -> Option<Arc<BazelModuleContext>> {
        defining_module::module_extra_value_of_innermost_enclosing_starlark_function(eval, depth)
            .and_then(Self::of_module_extra_value)
    }

    /// Java `ofInnermostBzlOrThrow` / module of the innermost Starlark function: `None` when the
    /// innermost Starlark function's module has no `BazelModuleContext` (e.g. a BUILD file).
    // Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:163 BazelModuleContext.ofInnermostBzlOrThrow
    pub fn of_innermost_bzl<'v>(eval: &Evaluator<'v, '_, '_>) -> Option<Arc<BazelModuleContext>> {
        Self::of_innermost_enclosing_starlark_function(eval, 0)
    }

    // Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:174 BazelModuleContext.ofInnermostBzlOrFail
    pub fn of_innermost_bzl_or_fail<'v>(
        eval: &Evaluator<'v, '_, '_>,
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
