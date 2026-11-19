//! BUILD-file globals, the non-rule members of `native`, `select()`, globbing and BUILD-file
//! execution (crates/blaze_packages/DESIGN.md, owner C).
//!
//! Ports of `packages/BuildGlobals.java`, `packages/PackageCallable.java`,
//! `packages/StarlarkNativeModule.java`, `SelectorList.SelectLibrary`, `packages/GlobCache.java`,
//! `packages/NonSkyframeGlobber.java`, `packages/GlobberUtils.java`, `vfs/UnixGlob.java` and the
//! non-Skyframe parts of `packages/PackageFactory.java`.
//!
//! Environment contract (DESIGN.md "Environment composition contract"): agent D composes the
//! per-file-kind environments from
//! - [`register_build_globals`]: BUILD-only functions (`package`, `licenses`,
//!   `environment_group`);
//! - [`register_native_module_functions`]: the non-rule members of `native` (`glob`,
//!   `existing_rule(s)`, `package_group`, `exports_files`, `package_name`, ...), which Java also
//!   adds to the BUILD toplevel;
//! - [`register_select`]: `select` (BUILD and .bzl);
//! - the native rule functions of [`crate::native_rules::RuleClassProvider`], added with
//!   [`crate::native_rules::builtin_rule_function::add_rule_functions`].

pub mod build_globals;
pub mod glob_cache;
#[cfg(test)]
mod loading_tests;
pub mod package_callable;
pub mod package_factory;
pub mod select;
pub mod starlark_native_module;
pub mod unix_glob;

use std::fmt;

use blaze_starlark::Location;
use blaze_starlark::error::StackEntry;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;

/// Adds the BUILD-only globals: Java `BuildGlobals` (`environment_group`, `licenses`) and
/// `PackageCallable` (`package`, added by `BazelRuleClassProvider.BAZEL_SETUP` as a BUILD
/// toplevel).
pub fn register_build_globals(builder: &mut GlobalsBuilder) {
    build_globals::build_globals(builder);
    package_callable::package_callable(builder);
}

/// Adds `select` (Java `SelectorList.SelectLibrary`, in BUILD and .bzl environments).
pub fn register_select(builder: &mut GlobalsBuilder) {
    select::select_library(builder);
}

/// Adds the non-rule members of `native` (Java `StarlarkNativeModule`; also BUILD toplevels).
///
/// `repository_name` is `enableOnlyWithFlag = INCOMPATIBLE_ENABLE_DEPRECATED_LABEL_APIS` in
/// Java. It is always bound, as here: `StarlarkNativeModule.BINDINGS_FOR_BUILD_FILES` comes
/// from `Starlark.addMethods(bindings, new StarlarkNativeModule())`, which selects methods
/// under the default semantics (the flag defaults to true). The flag is checked when the
/// function is called (`MethodDescriptor.checkEnabled`), which the function itself does.
// Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:86 StarlarkNativeModule.initializeBindings
// Java: net/starlark/java/eval/Starlark.java:1065 Starlark.addMethods
pub fn register_native_module_functions(builder: &mut GlobalsBuilder) {
    starlark_native_module::starlark_native_module(builder);
}

/// A Starlark error carrying `e`'s message (Java `new EvalException(e)` /
/// `Starlark.errorf("%s", ...)`).
pub(crate) fn eval_err(e: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{e}"))
}

/// The thread's call stack, outermost first, as Java's `StarlarkThread.getCallStack()` returns
/// it while a builtin runs: entry 0 is `<toplevel>` at the position of its current call, entry
/// i is the i-th called function at the position of its current call, and the last entry is
/// the running builtin (location `None`, Java `Location.BUILTIN`).
// PORT: move to blaze_starlark (StarlarkThread.getCallStack).
// Java: net/starlark/java/eval/StarlarkThread.java:603 StarlarkThread.getCallStack
pub fn thread_call_stack(eval: &Evaluator<'_, '_, '_>) -> Vec<StackEntry> {
    let frames = eval.call_stack().frames;
    let mut callstack = Vec::with_capacity(frames.len() + 1);
    let mut caller = "<toplevel>".to_owned();
    for (i, f) in frames.iter().enumerate() {
        // A function called from native code (a symbolic macro's implementation) is the
        // outermost frame and has no call site: Java's thread has no `<toplevel>` frame then.
        if i == 0 && f.location.is_none() {
            caller = f.name.clone();
            continue;
        }
        callstack.push(StackEntry {
            name: caller,
            location: f.location.as_ref().map(Location::bazel_position),
            source_line: None,
        });
        caller = f.name.clone();
    }
    callstack.push(StackEntry {
        name: caller,
        location: None,
        source_line: None,
    });
    callstack
}

/// Java `thread.getCallerLocation()` inside a builtin: the position of the innermost call.
pub(crate) fn caller_location(eval: &Evaluator<'_, '_, '_>) -> Option<Location> {
    blaze_starlark::env::caller_location(eval)
}
