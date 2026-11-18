//! The module a Starlark function was defined in (Java `StarlarkFunction.getModule`,
//! `Module.ofInnermostEnclosingStarlarkFunction`).
//!
//! Java attaches a `BazelModuleContext` to each .bzl `Module` as client data and reads it back
//! with `BazelModuleContext.of(fn.getModule())`. Blaze stores it as the module's extra value
//! (`eval::exec_module_with_extra_value`), and [`module_extra_value_of_function`] /
//! [`module_extra_value_of_innermost_enclosing_starlark_function`] read it back from the
//! function's own module (BLAZE PATCH 0017 and 0022). That is the module's identity: the same
//! file compiled once and executed for several BZL_LOAD keys (BUILD-loaded, bzlmod bootstrap)
//! gives several modules, each with its own context.
//!
//! The `module_file_*` helpers return the compiled file name instead, which does not tell such
//! modules apart; they remain for callers that need the name.

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

/// Java `fn.getModule().getClientData()`: the extra value of the module `function` (a `def` or
/// `lambda`) was defined in. `None` for other values and for modules without an extra value.
// Java: net/starlark/java/eval/StarlarkFunction.java:195 StarlarkFunction.getModule
// Java: net/starlark/java/eval/Module.java Module.getClientData
pub fn module_extra_value_of_function<'v>(
    function: Value<'v>,
    eval: &Evaluator<'v, '_, '_>,
) -> Option<Value<'v>> {
    extra_value_of(&DefModule::of_function(function)?, eval)
}

/// Java `Module.ofInnermostEnclosingStarlarkFunction(thread, depth).getClientData()`: the extra
/// value of the module of the `depth`-th innermost Starlark function on the call stack.
// Java: net/starlark/java/eval/Module.java:121 Module.ofInnermostEnclosingStarlarkFunction
pub fn module_extra_value_of_innermost_enclosing_starlark_function<'v>(
    eval: &Evaluator<'v, '_, '_>,
    depth: usize,
) -> Option<Value<'v>> {
    extra_value_of(&eval.innermost_enclosing_def_module(depth)?, eval)
}

fn extra_value_of<'v>(module: &DefModule<'v>, eval: &Evaluator<'v, '_, '_>) -> Option<Value<'v>> {
    if module.is_frozen() {
        module.frozen_extra_value().map(|v| v.to_value())
    } else {
        // A function whose module is not frozen yet lives in the one mutable module of the
        // evaluator that runs it.
        eval.module().extra_value()
    }
}

/// What a module's extra value provides (`StarlarkValue::provide`) for `Label()`: Java
/// `BazelModuleContext.packageContext()` of the module.
#[derive(Debug, Clone, starlark::any::ProvidesStaticType)]
pub struct ModulePackageContext(pub blaze_cmdline::LabelPackageContext);

/// Java `BazelModuleContext.ofInnermostBzlOrFail(thread, what).packageContext()`, as far as
/// blaze_starlark can see it: the package context provided by the extra value of the innermost
/// Starlark function's module.
pub fn package_context_of_innermost_enclosing_starlark_function(
    eval: &Evaluator,
) -> Option<blaze_cmdline::LabelPackageContext> {
    module_extra_value_of_innermost_enclosing_starlark_function(eval, 0)?
        .request_value::<ModulePackageContext>()
        .map(|c| c.0)
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
