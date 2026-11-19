//! Runs one Starlark file the way `java/StarlarkProbe.java` runs it on Bazel 9.3.2's interpreter,
//! and prints the same lines, so the two outputs can be compared line by line.
//!
//! Java side: `java -cp <install_base>/A-server.jar java/StarlarkProbe.java latin1|unicode FILE`.
//! Rust side: [`run`] (CLI: `starlark_conformance probe [--unicode] FILE...`).
//!
//! Environment: Blaze's universe (`blaze_starlark::env::universe_builder`, so Blaze's `fail`),
//! `json`, `_utf8_byte_strings`, and a `print` that writes to the output (Java:
//! `thread.setPrintHandler`). The file is parsed with the `.bzl` dialect (Java:
//! `FileOptions.DEFAULT`), then resolved (patch 0029: every unresolved name), then executed.
//!
//! Output: each printed line escaped as `StarlarkProbe.out` does; a syntax error (lexer, parser
//! or resolver) as `SYNTAX file:line:col: message`, an evaluation error as `EVAL message`.
//! Java prints every syntax error of a file; starlark-rust stops at the first lexer or parser
//! error, so only the first `SYNTAX` line is comparable.

use std::sync::Mutex;

use blaze_starlark::EvalError;
use blaze_starlark::error::EvalErrorKind;
use starlark::PrintHandler;
use starlark::environment::GlobalsBuilder;
use starlark::environment::LibraryExtension;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;

use crate::dialect;
use crate::script_test::ScriptMode;

/// The probe's output escaping (StarlarkProbe.out).
pub fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let u = c as u32;
        if c == '\\' {
            out.push_str("\\\\");
        } else if u < 0x20 || (0x7f..=0xff).contains(&u) {
            out.push_str(&format!("\\x{u:02x}"));
        } else if u > 0xff {
            out.push_str(&format!("\\u{{{u:04x}}}"));
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Default)]
struct Collect(Mutex<Vec<String>>);

impl PrintHandler for Collect {
    fn println(&self, text: &str) -> starlark::Result<()> {
        self.0.lock().unwrap().push(escape(text));
        Ok(())
    }
}

/// `SYNTAX file:line:col: message` (Java `SyntaxError.toString`), or `EVAL message` when the
/// error is not a syntax error.
fn syntax_line(e: &starlark::Error) -> String {
    let err = EvalError::from_starlark(e);
    match &err.kind {
        EvalErrorKind::Syntax(errors) => escape(&format!("SYNTAX {}", errors[0])),
        _ => escape(&format!("EVAL {}", err.message)),
    }
}

/// Runs the file `name` with contents `bytes` (decoded per `mode`, as the probe's
/// `ParserInput.fromLatin1` / `fromUTF8`); returns the probe's output lines.
pub fn run(name: &str, bytes: &[u8], mode: ScriptMode) -> Vec<String> {
    let source = match mode {
        ScriptMode::Latin1 => blaze_util::string_encoding::latin1_decode(bytes).into_owned(),
        ScriptMode::Unicode => match String::from_utf8(bytes.to_vec()) {
            Ok(s) => s,
            Err(_) => return vec!["HARNESS source is not UTF-8".to_owned()],
        },
    };
    let name = name.to_owned();
    std::thread::Builder::new()
        .stack_size(1 << 28)
        .spawn(move || {
            let semantics = blaze_starlark::semantics::StarlarkSemantics::default();
            let mut builder: GlobalsBuilder = blaze_starlark::env::universe_builder(&semantics)
                .with(blaze_starlark::json::register_json)
                // The probe's print handler (Blaze's own `print` reports a DEBUG event).
                .with(|b| LibraryExtension::Print.add(b));
            builder.set("_utf8_byte_strings", mode.utf8_byte_strings());
            let globals = builder.build();
            let out = Collect::default();
            let ast = match AstModule::parse(&name, source, &dialect::bzl_dialect()) {
                Ok(ast) => ast,
                Err(e) => return vec![syntax_line(&e)],
            };
            if let Some(e) = starlark::eval::resolve_errors(&ast, &globals).first() {
                return vec![syntax_line(e)];
            }
            let result = Module::with_temp_heap(|module| {
                let mut eval = Evaluator::new(&module);
                eval.set_forbid_recursion(true);
                eval.set_utf8_byte_strings(mode.utf8_byte_strings());
                eval.set_print_handler(&out);
                eval.eval_module(ast, &globals).map(|_| ())
            });
            let mut lines = out.0.into_inner().unwrap();
            if let Err(e) = result {
                let err = EvalError::from_starlark(&e);
                lines.push(escape(&format!("EVAL {}", err.message)));
            }
            lines
        })
        .unwrap()
        .join()
        .unwrap_or_else(|_| vec!["HARNESS panic".to_owned()])
}
