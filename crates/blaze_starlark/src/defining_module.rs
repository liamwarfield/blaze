//! The module a Starlark function was defined in (Java `StarlarkFunction.getModule`,
//! `Module.ofInnermostEnclosingStarlarkFunction`).
//!
//! Java attaches a `BazelModuleContext` to each .bzl `Module` as client data and reads it back
//! with `BazelModuleContext.of(fn.getModule())`. starlark-rust (BLAZE PATCH 0017) identifies a
//! function's module by the file it was compiled from ([`DefModule`]); Blaze registers each
//! .bzl's `BazelModuleContext` under that compiled file name
//! (blaze_packages `BazelModuleContext::register` / `of_filename`). The helpers here return that
//! key; `BazelModuleContext::of_filename(key)` is Java's `BazelModuleContext.of(module)`.

use starlark::eval::Evaluator;
use starlark::values::Value;

pub use starlark::eval::DefModule;

/// Java `fn.getModule()` as its module key: the compiled file name of the module `function`
/// (a `def` or `lambda`) was defined in; `None` for other values (Java: not a
/// `StarlarkFunction`).
// Java: net/starlark/java/eval/StarlarkFunction.java:195 StarlarkFunction.getModule
pub fn module_file_of_function(function: Value) -> Option<String> {
    DefModule::of_function(function).map(|m| m.file_name().to_owned())
}

/// Java `Module.ofInnermostEnclosingStarlarkFunction(thread, depth)` as its module key: the
/// compiled file name of the module of the `depth`-th innermost Starlark function on the call
/// stack (`def`s and `lambda`s, inlined ones included, and the executing module's top level);
/// `None` when there is none.
// Java: net/starlark/java/eval/Module.java:121 Module.ofInnermostEnclosingStarlarkFunction
pub fn module_file_of_innermost_enclosing_starlark_function(
    eval: &Evaluator,
    depth: usize,
) -> Option<String> {
    eval.innermost_enclosing_def_module(depth)
        .map(|m| m.file_name().to_owned())
}

/// Java `fn.getModule().getGlobal(fn.getName()) == fn`: whether `function` is a `def` bound by
/// its own module's top level under its name (the check of `cc_internal.check_toplevel`,
/// StarlarkActionFactory.validateIsTopLevelStarlarkFunction and TemplateDict). False for
/// lambdas, nested defs, and values that are not Starlark functions.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:966 CcStarlarkInternal.checkToplevel
pub fn is_global_of_defining_module<'v>(function: Value<'v>, eval: &Evaluator<'v, '_, '_>) -> bool {
    let (Some(module), Some(name)) = (
        DefModule::of_function(function),
        DefModule::def_name(function),
    ) else {
        return false;
    };
    module
        .get_global(&name, eval)
        .is_some_and(|g| g.ptr_eq(function))
}
