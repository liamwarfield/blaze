//! Test helpers: evaluate Starlark expressions in a temporary module.

use starlark::environment::Globals;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;
use starlark::syntax::Dialect;
use starlark::values::Value;

/// Runs `f` with a fresh module and evaluator.
pub(crate) fn with_eval<R>(f: impl for<'v, 'a> FnOnce(&mut Evaluator<'v, 'a, '_>) -> R) -> R {
    Module::with_temp_heap(|module| {
        let mut eval = Evaluator::new(&module);
        f(&mut eval)
    })
}

/// Evaluates `code` (a Starlark expression or statements whose last is an expression) with
/// the standard globals.
pub(crate) fn eval_expr<'v>(eval: &mut Evaluator<'v, '_, '_>, code: &str) -> Value<'v> {
    let ast = AstModule::parse("test.bzl", code.to_owned(), &Dialect::Extended)
        .unwrap_or_else(|e| panic!("parse {code}: {e}"));
    eval.eval_module(ast, &Globals::standard())
        .unwrap_or_else(|e| panic!("eval {code}: {e}"))
}
