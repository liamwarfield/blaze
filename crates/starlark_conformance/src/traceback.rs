//! CLI evaluation of one file with Bazel-format errors (`starlark_conformance eval FILE`).
//! The rendering is blaze_starlark's (`EvalError::message_with_stack`).

use blaze_starlark::EvalError;
use starlark::PrintHandler;
use starlark::environment::Globals;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;

use crate::script_test::ScriptMode;

/// `print` to stdout; in Latin-1 mode one byte per char, as Bazel's ISO-8859-1 console does.
struct StdoutPrint(ScriptMode);
impl PrintHandler for StdoutPrint {
    fn println(&self, text: &str) -> starlark::Result<()> {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let r = match self.0 {
            ScriptMode::Unicode => writeln!(out, "{text}"),
            ScriptMode::Latin1 => out
                .write_all(&blaze_util::string_encoding::latin1_encode_lossy(text))
                .and_then(|()| out.write_all(b"\n")),
        };
        r.map_err(|e| starlark::Error::new_other(anyhow::anyhow!("{e}")))
    }
}

/// Evaluates `src` (a ScriptTest-like file, already decoded for `mode`) and returns the
/// Bazel-format traceback on error.
pub fn eval_for_cli(
    path: &str,
    src: &str,
    globals: &Globals,
    mode: ScriptMode,
) -> Result<(), String> {
    let ast = AstModule::parse(path, src.to_owned(), &crate::dialect::bzl_dialect())
        .map_err(|e| EvalError::from_starlark(&e).message_with_stack())?;
    let print = StdoutPrint(mode);
    let reporter = crate::harness::Reporter::default();
    let res = Module::with_temp_heap(|module| {
        let mut eval = Evaluator::new(&module);
        eval.set_forbid_recursion(true);
        eval.extra = Some(&reporter);
        eval.set_print_handler(&print);
        eval.set_utf8_byte_strings(mode.utf8_byte_strings());
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
