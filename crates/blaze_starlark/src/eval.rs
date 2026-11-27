//! Compiling and executing Starlark files and calling frozen functions.
//!
//! Java: com/google/devtools/build/lib/skyframe/BzlCompileFunction.java (compile),
//! BzlLoadFunction.executeBzlFile and PackageFunction's BUILD execution (exec), and
//! `Starlark.call` for rule implementations. Threading rules (docs/research/05-starlark-rust.md
//! 6.2):
//! - one `Module` per evaluation unit, created and dropped on one thread;
//! - only `FrozenModule` / `OwnedFrozenValue` or plain Rust data leave an evaluation;
//! - evaluation recurses on the native stack (calls, repr, freezing, GC of deep values), so run
//!   it on a thread from [`run_on_evaluation_thread`] or with at least [`EVAL_STACK_SIZE`].

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use starlark::environment::FrozenModule;
use starlark::environment::Globals;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;
use starlark::values::OwnedFrozenValue;
use starlark::values::Value;

use crate::error::EvalError;
use crate::error::EvalErrorKind;
use crate::error::Location;
use crate::error::SyntaxError;
use crate::file_kind::FileKind;
use crate::loader::LoadMap;
use crate::loader::LoadRequest;
use crate::loader::load_requests;
use crate::syntax::BuildFileFacts;
use crate::thread::ThreadContext;

/// Native stack size for evaluation threads. Virtual memory only; pages are committed on use.
pub const EVAL_STACK_SIZE: usize = 256 << 20;

/// Default Starlark call depth limit (frames, natives included). starlark-rust's default is 50;
/// Bazel has no fixed limit (only the JVM stack).
pub const DEFAULT_MAX_CALLSTACK: usize = 1000;

/// Knobs for one evaluation.
#[derive(Clone)]
pub struct EvalOptions {
    pub max_callstack_size: usize,
    /// Set to true to interrupt the evaluation (Java `InterruptedException`).
    pub cancelled: Option<Arc<AtomicBool>>,
}

impl Default for EvalOptions {
    fn default() -> Self {
        EvalOptions {
            max_callstack_size: DEFAULT_MAX_CALLSTACK,
            cancelled: None,
        }
    }
}

/// A parsed and statically checked file (Java `BzlCompileValue` / `CompiledBuildFile`).
pub struct CompiledFile {
    pub kind: FileKind,
    pub ast: AstModule,
    pub loads: Vec<LoadRequest>,
    /// BUILD only: glob/subpackages prefetch patterns and generator names.
    pub build_facts: BuildFileFacts,
    /// Java's syntax tree of the file, kept from [`compile_with_options`] for [`resolve`], which
    /// takes it (so that a compiled file Skyframe keeps does not pin it). Empty otherwise;
    /// `resolve` then parses again.
    pub java_tree: std::sync::Mutex<Option<Box<crate::syntax::java::ParsedFile>>>,
}

impl std::fmt::Debug for CompiledFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledFile")
            .field("kind", &self.kind)
            .field("loads", &self.loads)
            .field("build_facts", &self.build_facts)
            .finish_non_exhaustive()
    }
}

/// Parses `source` as a file of `kind` and runs Bazel's static checks.
pub fn compile(kind: FileKind, file_name: &str, source: String) -> Result<CompiledFile, EvalError> {
    compile_with_options(kind, kind.file_options(), file_name, source)
}

/// Java's static errors of a file before the resolver: the scanner's and parser's errors (every
/// one, with Java's recovery) and, for BUILD and the other `.bazel` files,
/// `DotBazelFileSyntaxChecker`'s. Also returns the parsed Java tree.
fn java_static_errors(
    kind: FileKind,
    options: crate::file_kind::FileOptions,
    file_name: &str,
    source: &str,
) -> (crate::syntax::java::ParsedFile, Vec<SyntaxError>) {
    use crate::syntax::java;
    let jopts = java::JavaOptions::from_file_options(options);
    let parsed = java::parse_file(file_name, source, jopts);
    if !parsed.errors.is_empty() {
        let errors = parsed.errors.clone();
        return (parsed, errors);
    }
    // Java: PackageFunction.java:1380 checkBuildSyntax (and the module/repo file checkers) run
    // after parsing, before compileFile.
    if let Some((where_, can_load_bzl, allow_literal_star_star_args)) =
        kind.dot_bazel_checker_config()
    {
        let checker = crate::syntax::dot_bazel::Checker {
            where_,
            can_load_bzl,
            allow_literal_star_star_args,
        };
        let errors: Vec<SyntaxError> = java::dot_bazel::check(&checker, &parsed.statements)
            .into_iter()
            .map(|e| SyntaxError {
                location: parsed.locs.location(e.offset),
                message: e.message,
            })
            .collect();
        if !errors.is_empty() {
            return (parsed, errors);
        }
    }
    (parsed, Vec::new())
}

/// The Java resolver's errors of `parsed` (Program.compileFile after parsing).
fn java_resolver_errors(
    parsed: &crate::syntax::java::ParsedFile,
    module: &dyn crate::syntax::java::resolver::ResolverModule,
    options: crate::file_kind::FileOptions,
) -> Vec<SyntaxError> {
    use crate::syntax::java;
    java::resolver::resolve_file(
        &parsed.statements,
        module,
        java::JavaOptions::from_file_options(options),
    )
    .into_iter()
    .map(|e| SyntaxError {
        location: parsed.locs.location(e.offset),
        message: e.message,
    })
    .collect()
}

/// A resolver module in which every valid name is predeclared: the resolver's errors that do
/// not depend on the predeclared names. The identifiers the parser's recovery makes (not valid
/// identifiers) are undefined in every module, so Java reports them ("contains syntax errors",
/// Resolver.java:946) whatever the names are.
struct EveryNamePredeclared;

impl crate::syntax::java::resolver::ResolverModule for EveryNamePredeclared {
    fn resolve(
        &self,
        name: &str,
    ) -> Result<crate::syntax::java::resolver::Scope, crate::syntax::java::resolver::Undefined>
    {
        if !crate::syntax::java::ast::is_valid_identifier(name) {
            return Err(crate::syntax::java::resolver::Undefined {
                message: format!("name '{name}' is not defined"),
                candidates: Some(Vec::new()),
            });
        }
        Ok(crate::syntax::java::resolver::Scope::Predeclared)
    }
}

/// [`compile`] with explicit [`crate::file_kind::FileOptions`] (Java `FileOptions` whose values
/// depend on flags: `allowTypeSyntax`).
///
/// The errors are Bazel's, from Java's front end (`crate::syntax::java`): every scanner and
/// parser error of the file; for BUILD and the other `.bazel` files then
/// `DotBazelFileSyntaxChecker`'s; then the resolver's errors that do not depend on the
/// predeclared names (for a .bzl file also after its parse errors, on the recovered tree, as
/// `compile_and_resolve`). PORT: Java's list also has the resolver's undefined-name errors; they
/// need the predeclared names, which [`compile_and_resolve`] takes. Callers that have them
/// should use it.
// Java: com/google/devtools/build/lib/skyframe/BzlCompileFunction.java:216 compileInternal
// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:1366 compileBuildFile
pub fn compile_with_options(
    kind: FileKind,
    options: crate::file_kind::FileOptions,
    file_name: &str,
    source: String,
) -> Result<CompiledFile, EvalError> {
    let (parsed, mut errors) = java_static_errors(kind, options, file_name, &source);
    // Java: as compile_and_resolve; a .bzl file's parse errors are followed by the resolver's
    // errors on the recovered tree (BzlCompileFunction.java:234).
    let parse_failed = !parsed.errors.is_empty();
    if !errors.is_empty() && (kind.dot_bazel_checker_config().is_some() || !parse_failed) {
        return Err(EvalError::syntax(errors));
    }
    errors.extend(java_resolver_errors(
        &parsed,
        &EveryNamePredeclared,
        options,
    ));
    if !errors.is_empty() {
        return Err(EvalError::syntax(errors));
    }
    let compiled = compile_starlark_rust(kind, options, file_name, source)?;
    *compiled.java_tree.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(parsed));
    Ok(compiled)
}

/// Java's compile of a file against its predeclared names: every static error Bazel reports
/// for it, in Bazel's order (BzlCompileFunction / PackageFunction.compileBuildFile:
/// `StarlarkFile.parse`, for BUILD files `checkBuildSyntax`, `Program.compileFile`). `names` are
/// the predeclared and universal names; `guarded` gives the message of a predeclared name whose
/// `GuardedValue` is not accessible (Java `Module.resolve`).
// Java: net/starlark/java/syntax/Program.java:101 Program.compileFile
pub fn compile_and_resolve(
    kind: FileKind,
    options: crate::file_kind::FileOptions,
    file_name: &str,
    source: String,
    names: Vec<String>,
    guarded: &dyn Fn(&str) -> Option<String>,
) -> Result<CompiledFile, EvalError> {
    let module = crate::syntax::java::PredeclaredModule::new(names, guarded);
    let (parsed, mut errors) = java_static_errors(kind, options, file_name, &source);
    // Java: PackageFunction.java:1367 returns a BUILD file's parse errors alone; for a .bzl
    // file, compileFile resolves the partial tree and adds the resolver's errors
    // (BzlCompileFunction.java:234).
    let parse_failed = !parsed.errors.is_empty();
    if !errors.is_empty() && (kind.dot_bazel_checker_config().is_some() || !parse_failed) {
        return Err(EvalError::syntax(errors));
    }
    errors.extend(java_resolver_errors(&parsed, &module, options));
    if !errors.is_empty() {
        return Err(EvalError::syntax(errors));
    }
    compile_starlark_rust(kind, options, file_name, source)
}

/// starlark-rust's parse and Blaze's remaining checks of a file Java's front end accepts.
fn compile_starlark_rust(
    kind: FileKind,
    options: crate::file_kind::FileOptions,
    file_name: &str,
    source: String,
) -> Result<CompiledFile, EvalError> {
    let ast =
        AstModule::parse(file_name, source, &kind.dialect_with_options(options)).map_err(|e| {
            let location = e
                .span()
                .map(Location::of_span_start)
                .unwrap_or_else(|| Location {
                    file: file_name.to_owned(),
                    line: 0,
                    column: 0,
                });
            EvalError::syntax(vec![SyntaxError {
                location,
                message: crate::error::bare_message(&e),
            }])
        })?;
    let (errors, build_facts) = crate::syntax::check_file_with_options(kind, options, &ast);
    if !errors.is_empty() {
        return Err(EvalError::syntax(errors));
    }
    let loads = load_requests(&ast);
    Ok(CompiledFile {
        kind,
        ast,
        loads,
        build_facts,
        java_tree: std::sync::Mutex::new(None),
    })
}

/// Resolves every name of `compiled` against `globals` without executing it and returns all
/// resolution errors as one syntax error list, in Java's order (Java resolves when compiling:
/// `Program.compileFile` → `Resolver.resolveFile`). `Ok` when every name resolves. Uses Java's
/// resolver (`crate::syntax::java::resolver`) on the file's source; [`compile`] has already
/// reported every error that does not depend on the names, so these are the undefined names
/// ("name 'x' is not defined[ (did you mean 'y'?)]"), located at the identifier.
// Java: net/starlark/java/syntax/Resolver.java:922 Resolver.use
// Java: net/starlark/java/syntax/Program.java:101 Program.compileFile
pub fn resolve(compiled: &CompiledFile, globals: &Globals) -> Result<(), EvalError> {
    let kept = compiled
        .java_tree
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    let mut options = compiled.kind.file_options();
    let parsed = match kept {
        Some(p) => *p,
        None => {
            let ast = &compiled.ast;
            let span = ast.file_span(ast.statement().span);
            let source = span.file.source().to_owned();
            let file_name = span.filename().to_owned();
            // The type syntax setting the file was compiled with (`compile_with_options`).
            {
                use starlark_syntax::syntax::module::AstModuleFields;
                options.allow_type_syntax =
                    ast.dialect().enable_types == starlark::syntax::DialectTypes::ParseOnly;
            }
            crate::syntax::java::parse_file(
                &file_name,
                &source,
                crate::syntax::java::JavaOptions::from_file_options(options),
            )
        }
    };
    let no_guard = |_: &str| None;
    let module = crate::syntax::java::PredeclaredModule::new(
        globals.names().map(|n| n.as_str().to_owned()).collect(),
        &no_guard,
    );
    let errors = java_resolver_errors(&parsed, &module, options);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(EvalError::syntax(errors))
    }
}

fn panic_error(p: Box<dyn std::any::Any + Send>) -> EvalError {
    let what = p
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    EvalError {
        kind: EvalErrorKind::Panic,
        message: format!("internal error: Starlark evaluation panicked: {what}"),
        callstack: Vec::new(),
        include_stack: true,
    }
}

/// Runs `f` with a fresh module and an evaluator configured for Blaze: `ctx` in
/// `Evaluator::extra`, `loads` as the loader, recursion forbidden unless the semantics allow it,
/// the byte-strings flag from the semantics (`-internal_bazel_only_utf_8_byte_strings`),
/// the call-depth limit and cancellation hook from `opts`. Panics inside starlark-rust become
/// [`EvalErrorKind::Panic`].
pub fn with_evaluator<R>(
    ctx: &ThreadContext,
    loads: Option<&LoadMap>,
    opts: &EvalOptions,
    f: impl for<'v, 'a> FnOnce(&'a Module<'v>, &mut Evaluator<'v, 'a, '_>) -> Result<R, EvalError>,
) -> Result<R, EvalError> {
    let run = AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            let mut eval = Evaluator::new(&module);
            crate::debug::install_hook(&mut eval);
            eval.extra = Some(ctx);
            if let Some(l) = loads {
                eval.set_loader(l);
            }
            eval.set_forbid_recursion(!ctx.semantics.allow_recursion);
            eval.set_utf8_byte_strings(ctx.semantics.utf8_byte_strings());
            eval.set_max_callstack_size(opts.max_callstack_size.max(1))
                .expect("fresh evaluator");
            if let Some(c) = &opts.cancelled {
                let c = c.clone();
                eval.set_check_cancelled(Box::new(move || c.load(Ordering::Relaxed)));
            }
            f(&module, &mut eval)
        })
    });
    std::panic::catch_unwind(run).unwrap_or_else(|p| Err(panic_error(p)))
}

fn freeze(module: Module) -> Result<FrozenModule, EvalError> {
    module.freeze().map_err(|e| EvalError {
        kind: EvalErrorKind::Eval,
        message: format!("{e:?}"),
        callstack: Vec::new(),
        include_stack: true,
    })
}

/// Executes a compiled file and freezes its module. `loads` must hold every load string of
/// `compiled.loads`.
pub fn exec_module(
    compiled: CompiledFile,
    globals: &Globals,
    loads: &LoadMap,
    ctx: &ThreadContext,
    opts: &EvalOptions,
) -> Result<FrozenModule, EvalError> {
    let ast = compiled.ast;
    let debug_file_name = ast.file_span(Default::default()).filename().to_owned();
    let run = AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            {
                let mut eval = Evaluator::new(&module);
                crate::debug::install_hook(&mut eval);
                eval.extra = Some(ctx);
                eval.set_loader(loads);
                eval.set_forbid_recursion(!ctx.semantics.allow_recursion);
                eval.set_utf8_byte_strings(ctx.semantics.utf8_byte_strings());
                eval.set_max_callstack_size(opts.max_callstack_size.max(1))
                    .expect("fresh evaluator");
                if let Some(c) = &opts.cancelled {
                    let c = c.clone();
                    eval.set_check_cancelled(Box::new(move || c.load(Ordering::Relaxed)));
                }
                eval.eval_module(ast, globals)
                    .map_err(|e| EvalError::from_starlark(&e))?;
            }
            let frozen = freeze(module)?;
            crate::debug::register_module(&debug_file_name, &frozen);
            Ok(frozen)
        })
    });
    std::panic::catch_unwind(run).unwrap_or_else(|p| Err(panic_error(p)))
}

/// Like [`exec_module`], but first sets the module's extra value to `extra_value(module)`
/// (starlark-rust `Module::set_extra_value`), so it is frozen with the module. Blaze stores the
/// .bzl's `BazelModuleContext` there (Java `Module.setClientData`).
pub fn exec_module_with_extra_value(
    compiled: CompiledFile,
    globals: &Globals,
    loads: &LoadMap,
    ctx: &ThreadContext,
    opts: &EvalOptions,
    extra_value: impl for<'v> FnOnce(&Module<'v>) -> Value<'v>,
) -> Result<FrozenModule, EvalError> {
    let ast = compiled.ast;
    let debug_file_name = ast.file_span(Default::default()).filename().to_owned();
    let run = AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            let extra = extra_value(&module);
            module.set_extra_value(extra);
            {
                let mut eval = Evaluator::new(&module);
                crate::debug::install_hook(&mut eval);
                eval.extra = Some(ctx);
                eval.set_loader(loads);
                eval.set_forbid_recursion(!ctx.semantics.allow_recursion);
                eval.set_utf8_byte_strings(ctx.semantics.utf8_byte_strings());
                eval.set_max_callstack_size(opts.max_callstack_size.max(1))
                    .expect("fresh evaluator");
                if let Some(c) = &opts.cancelled {
                    let c = c.clone();
                    eval.set_check_cancelled(Box::new(move || c.load(Ordering::Relaxed)));
                }
                eval.eval_module(ast, globals)
                    .map_err(|e| EvalError::from_starlark(&e))?;
            }
            let frozen = freeze(module)?;
            crate::debug::register_module(&debug_file_name, &frozen);
            Ok(frozen)
        })
    });
    std::panic::catch_unwind(run).unwrap_or_else(|p| Err(panic_error(p)))
}

/// Brings a value frozen elsewhere (a function from a loaded .bzl, a provider from a dependency's
/// analysis) into `module`'s heap.
///
/// starlark-rust's `OwnedFrozenValue::owned_value(&FrozenHeap)` cannot be used with
/// `Module::with_temp_heap`: the returned value's lifetime is the heap borrow, not the module's
/// invariant brand `'v`. This is Blaze's one audited `unsafe`.
pub fn frozen_into<'v>(module: &Module<'v>, v: &OwnedFrozenValue) -> Value<'v> {
    // SAFETY: `owned_frozen_value` adds `v`'s heap as a reference of `module`'s frozen heap, so
    // the value lives at least as long as the module and anything frozen from it.
    Value::new_frozen(unsafe { v.owned_frozen_value(module.frozen_heap()) })
}

/// Calls a frozen function in a fresh module (Java `Starlark.call` on a rule implementation).
/// `args` builds the arguments on the new heap; `extract` turns the result into Rust data (or
/// or use [`call_frozen_and_freeze`] to keep the result as a frozen value).
pub fn call_frozen<R>(
    function: &OwnedFrozenValue,
    ctx: &ThreadContext,
    opts: &EvalOptions,
    args: impl for<'v> FnOnce(&Module<'v>) -> (Vec<Value<'v>>, Vec<(String, Value<'v>)>),
    extract: impl for<'v> FnOnce(
        Value<'v>,
        &Module<'v>,
        &mut Evaluator<'v, '_, '_>,
    ) -> Result<R, EvalError>,
) -> Result<R, EvalError> {
    with_evaluator(ctx, None, opts, |module, eval| {
        let f = frozen_into(module, function);
        let (pos, named) = args(module);
        let named: Vec<(&str, Value)> = named.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        let res = eval
            .eval_function(f, &pos, &named)
            .map_err(|e| EvalError::from_starlark(&e))?;
        extract(res, module, eval)
    })
}

/// Like [`call_frozen`], but stores the result as `result` in the call's module and freezes it,
/// so the value outlives the call (Bazel keeps providers returned by analysis).
pub fn call_frozen_and_freeze(
    function: &OwnedFrozenValue,
    ctx: &ThreadContext,
    opts: &EvalOptions,
    args: impl for<'v> FnOnce(&Module<'v>) -> (Vec<Value<'v>>, Vec<(String, Value<'v>)>),
) -> Result<OwnedFrozenValue, EvalError> {
    let run = AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            {
                let mut eval = Evaluator::new(&module);
                crate::debug::install_hook(&mut eval);
                eval.extra = Some(ctx);
                eval.set_forbid_recursion(!ctx.semantics.allow_recursion);
                eval.set_utf8_byte_strings(ctx.semantics.utf8_byte_strings());
                eval.set_max_callstack_size(opts.max_callstack_size.max(1))
                    .expect("fresh evaluator");
                if let Some(c) = &opts.cancelled {
                    let c = c.clone();
                    eval.set_check_cancelled(Box::new(move || c.load(Ordering::Relaxed)));
                }
                let f = frozen_into(&module, function);
                let (pos, named) = args(&module);
                let named: Vec<(&str, Value)> =
                    named.iter().map(|(k, v)| (k.as_str(), *v)).collect();
                let res = eval
                    .eval_function(f, &pos, &named)
                    .map_err(|e| EvalError::from_starlark(&e))?;
                module.set("result", res);
            }
            let frozen = freeze(module)?;
            frozen.get("result").map_err(|e| EvalError {
                kind: EvalErrorKind::Eval,
                message: e.to_string(),
                callstack: Vec::new(),
                include_stack: true,
            })
        })
    });
    std::panic::catch_unwind(run).unwrap_or_else(|p| Err(panic_error(p)))
}

/// Runs `f` on a new thread with an [`EVAL_STACK_SIZE`] stack and waits for it.
pub fn run_on_evaluation_thread<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .name("starlark-eval".to_owned())
            .stack_size(EVAL_STACK_SIZE)
            .spawn_scoped(s, f)
            .expect("spawn evaluation thread")
            .join()
            .unwrap_or_else(|p| std::panic::resume_unwind(p))
    })
}

/// What one front end makes of a file: its top-level statement count and the names it binds at
/// top level (in order of first binding), or its errors.
pub type FrontEndResult = Result<(usize, Vec<String>), Vec<String>>;

/// Runs both front ends on a file, for the differential test of crates/starlark_conformance:
/// Java's port (parse, the `.bazel` checks, the resolver's name-independent errors) and
/// starlark-rust's parse with Blaze's checks. A file one accepts and the other rejects, or with
/// different top-level statements or bindings, is a disagreement.
pub fn front_ends(
    kind: FileKind,
    options: crate::file_kind::FileOptions,
    file_name: &str,
    source: &str,
) -> (FrontEndResult, FrontEndResult) {
    let (parsed, mut errors) = java_static_errors(kind, options, file_name, source);
    if errors.is_empty() {
        errors = java_resolver_errors(&parsed, &EveryNamePredeclared, options);
    }
    let java = if errors.is_empty() {
        let mut names: Vec<String> = Vec::new();
        let mut add = |n: &str| {
            if !names.iter().any(|x| x == n) {
                names.push(n.to_owned());
            }
        };
        fn bound(e: &crate::syntax::java::ast::Expr, add: &mut dyn FnMut(&str)) {
            use crate::syntax::java::ast::Expr;
            match e {
                Expr::Ident(id) => add(&id.name),
                Expr::List { elems, .. } => elems.iter().for_each(|x| bound(x, add)),
                _ => {}
            }
        }
        for s in &parsed.statements {
            use crate::syntax::java::ast::Stmt;
            match s {
                Stmt::Assign { lhs, .. } => bound(lhs, &mut add),
                Stmt::Def { name, .. } => add(&name.name),
                Stmt::Load { bindings, .. } => bindings.iter().for_each(|(l, _)| add(&l.name)),
                Stmt::Var { name, .. } => add(&name.name),
                _ => {}
            }
        }
        Ok((parsed.statements.len(), names))
    } else {
        Err(errors.iter().map(|e| e.to_string()).collect())
    };
    let rust = match compile_starlark_rust(kind, options, file_name, source.to_owned()) {
        Err(e) => Err(vec![e.message_with_stack()]),
        Ok(compiled) => {
            use starlark::syntax::ast::AssignTargetP;
            use starlark::syntax::ast::StmtP;
            let mut names: Vec<String> = Vec::new();
            let mut add = |n: &str| {
                if !names.iter().any(|x| x == n) {
                    names.push(n.to_owned());
                }
            };
            fn bound(
                t: &starlark::syntax::ast::AstAssignTargetP<starlark::syntax::ast::AstNoPayload>,
                add: &mut dyn FnMut(&str),
            ) {
                match &t.node {
                    AssignTargetP::Identifier(id) => add(&id.node.ident),
                    AssignTargetP::Tuple(xs) => xs.iter().for_each(|x| bound(x, add)),
                    _ => {}
                }
            }
            let top = compiled.ast.statement();
            let stmts: Vec<_> = match &top.node {
                StmtP::Statements(xs) => xs.iter().collect(),
                _ => vec![top],
            };
            for s in &stmts {
                if let Some(id) = s.blaze_var_statement() {
                    add(&id.node.ident);
                    continue;
                }
                match &s.node {
                    StmtP::Assign(a) => bound(&a.lhs, &mut add),
                    StmtP::AssignModify(lhs, _, _) => bound(lhs, &mut add),
                    StmtP::Def(d) => add(&d.name.node.ident),
                    StmtP::Load(l) => l.args.iter().for_each(|a| add(&a.local.node.ident)),
                    _ => {}
                }
            }
            Ok((stmts.len(), names))
        }
    };
    (java, rust)
}
