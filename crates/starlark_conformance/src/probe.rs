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
//! or resolver) as `SYNTAX file:line:col: message` (every resolver error of the file; for the
//! lexer and parser, every error starlark-rust reports), an evaluation error as `EVAL message`
//! followed by one `TRACE line` per line of Bazel's traceback
//! (`EvalError::message_with_stack`, Java `EvalException.getMessageWithStack`).

use std::sync::Mutex;

use blaze_starlark::EvalError;
use blaze_starlark::error::EvalErrorKind;
use starlark::environment::GlobalsBuilder;
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

/// The probe's print handler: Bazel's own `print` (blaze_starlark's universe) reports a DEBUG
/// event; the probe writes its message as `StarlarkProbe`'s print handler does.
impl blaze_starlark::thread::StarlarkEvents for Collect {
    fn debug(&self, _location: Option<&blaze_starlark::error::Location>, message: &str) {
        self.0.lock().unwrap().push(escape(message));
    }

    fn warn(&self, _location: Option<&blaze_starlark::error::Location>, message: &str) {
        panic!("the probe got a warning: {message}");
    }
}

/// `SYNTAX file:line:col: message` (Java `SyntaxError.toString`) for an error of parsing or
/// resolving (Java reports every error before execution as a `SyntaxError`; starlark-rust's
/// dialect and resolver errors are not `EvalErrorKind::Syntax`, so they print without a
/// location).
fn syntax_line(e: &starlark::Error) -> String {
    let err = EvalError::from_starlark(e);
    match &err.kind {
        EvalErrorKind::Syntax(errors) => escape(&format!("SYNTAX {}", errors[0])),
        _ => match e.span() {
            Some(span) => escape(&format!(
                "SYNTAX {}: {}",
                blaze_starlark::error::Location::of_span_start(span),
                err.message
            )),
            None => escape(&format!("SYNTAX {}", err.message)),
        },
    }
}

/// The `TRACE` lines of an evaluation error.
fn trace_lines(err: &EvalError) -> Vec<String> {
    err.message_with_stack()
        .split('\n')
        .map(|l| escape(&format!("TRACE {l}")))
        .collect()
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
                .with(blaze_starlark::json::register_json);
            builder.set("_utf8_byte_strings", mode.utf8_byte_strings());
            let globals = builder.build();
            let out = std::sync::Arc::new(Collect::default());
            let cx = blaze_starlark::thread::ThreadContext::new(
                blaze_starlark::thread::Phase::BzlLoad,
                std::sync::Arc::new(semantics.clone()),
                out.clone(),
            );
            // Bazel's static errors (every one, in Java's order): Java's front end
            // (blaze_starlark::syntax::java), Program.compileFile with FileOptions.DEFAULT.
            let java_options = blaze_starlark::syntax::java::JavaOptions::DEFAULT;
            let parsed = blaze_starlark::syntax::java::parse_file(&name, &source, java_options);
            let no_guard = |_: &str| None;
            let module = blaze_starlark::syntax::java::PredeclaredModule::new(
                globals.names().map(|n| n.as_str().to_owned()).collect(),
                &no_guard,
            );
            let static_errors =
                blaze_starlark::syntax::java::compile_errors(&parsed, &module, java_options);
            if !static_errors.is_empty() {
                return static_errors
                    .iter()
                    .map(|e| escape(&format!("SYNTAX {e}")))
                    .collect();
            }
            let ast = match AstModule::parse(&name, source, &dialect::bzl_dialect()) {
                Ok(ast) => ast,
                Err(e) => return vec![syntax_line(&e)],
            };
            let errors = starlark::eval::resolve_errors(&ast, &globals);
            if !errors.is_empty() {
                return errors.iter().map(syntax_line).collect();
            }
            let result = Module::with_temp_heap(|module| {
                let mut eval = Evaluator::new(&module);
                eval.set_forbid_recursion(true);
                eval.set_utf8_byte_strings(mode.utf8_byte_strings());
                eval.extra = Some(&cx);
                eval.eval_module(ast, &globals).map(|_| ())
            });
            let mut lines = std::mem::take(&mut *out.0.lock().unwrap());
            if let Err(e) = result {
                let err = EvalError::from_starlark(&e);
                lines.push(escape(&format!("EVAL {}", err.message)));
                lines.extend(trace_lines(&err));
            }
            lines
        })
        .unwrap()
        .join()
        .unwrap_or_else(|_| vec!["HARNESS panic".to_owned()])
}

/// How a case of a probe directory compares with Bazel's recorded output (`NAME.latin1.txt`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseResult {
    /// Every line equal: printed output, every `SYNTAX` line, `EVAL` message and `TRACE` lines.
    Match,
    /// Printed output and the kind of the first error (`SYNTAX`/`EVAL`/none) are equal, the
    /// error text, location or traceback differ.
    TextDiffers,
    /// Printed output or the kind of error differ.
    Differs,
}

/// One compared case: its result, Java's lines and Blaze's lines.
pub struct CaseDiff {
    pub name: String,
    pub result: CaseResult,
    pub java: Vec<String>,
    pub rust: Vec<String>,
}

/// Runs `dir/NAME.star` (latin1 mode) and compares it with `dir/NAME.latin1.txt`.
pub fn compare_case(dir: &std::path::Path, name: &str) -> CaseDiff {
    let bytes = std::fs::read(dir.join(name)).unwrap_or_default();
    let rust = run(name, &bytes, ScriptMode::Latin1);
    let stem = name.strip_suffix(".star").unwrap_or(name);
    let java: Vec<String> = std::fs::read_to_string(dir.join(format!("{stem}.latin1.txt")))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    let result = if java == rust {
        CaseResult::Match
    } else {
        let is_error = |l: &String| l.starts_with("SYNTAX ") || l.starts_with("EVAL ");
        let split = |v: &[String]| {
            let n = v.iter().position(is_error).unwrap_or(v.len());
            (
                v[..n].to_vec(),
                v.get(n)
                    .map(|l| l.split(' ').next().unwrap_or("").to_owned()),
            )
        };
        if split(&java) == split(&rust) {
            CaseResult::TextDiffers
        } else {
            CaseResult::Differs
        }
    };
    CaseDiff {
        name: name.to_owned(),
        result,
        java,
        rust,
    }
}

/// The `.star` files of `dir`, sorted.
pub fn case_names(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let n = e.file_name().into_string().ok()?;
                    n.ends_with(".star").then_some(n)
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}
