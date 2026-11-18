//! CLI evaluation of one file with Bazel-format errors (`starlark_conformance eval FILE`).
//! The rendering is blaze_starlark's (`EvalError::message_with_stack`).

use blaze_starlark::EvalError;
use starlark::PrintHandler;
use starlark::environment::Globals;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;

struct StdoutPrint;
impl PrintHandler for StdoutPrint {
    fn println(&self, text: &str) -> starlark::Result<()> {
        println!("{text}");
        Ok(())
    }
}

/// Evaluates `src` (a ScriptTest-like file) and returns the Bazel-format traceback on error.
pub fn eval_for_cli(path: &str, src: &str, globals: &Globals) -> Result<(), String> {
    let ast = AstModule::parse(path, src.to_owned(), &crate::dialect::bzl_dialect())
        .map_err(|e| EvalError::from_starlark(&e).message_with_stack())?;
    let reporter = crate::harness::Reporter::default();
    let res = Module::with_temp_heap(|module| {
        let mut eval = Evaluator::new(&module);
        eval.set_forbid_recursion(true);
        eval.extra = Some(&reporter);
        eval.set_print_handler(&StdoutPrint);
        eval.eval_module(ast, globals).map(|_| ()).map_err(|e| {
            format!(
                "{}\n--- starlark-rust native rendering ---\n{e}",
                EvalError::from_starlark(&e).message_with_stack()
            )
        })
    });
    for ev in reporter.events.borrow().iter() {
        if ev.outcome != crate::harness::AssertOutcome::Pass {
            println!("{}: {}", ev.line, ev.message);
        }
    }
    res
}
