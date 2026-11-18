//! Bazel-format error rendering from a starlark-rust error.
//!
//! Java: net/starlark/java/eval/EvalException.java:201 EvalException.formatCallStack.
//!
//! Bazel's stack is a list of (function name, current location in that function), outermost
//! first, where the outermost name is `<toplevel>`. A call's location is the position of its `(`
//! (Java `CallExpression.getLparenLocation`). If the innermost entry is a builtin, it is not shown
//! and the message is prefixed `Error in <builtin>: `; otherwise the prefix is `Error: `.
//!
//! starlark-rust's `CallStack` is a list of `Frame { name: callee, location: call-site span }`,
//! and `Error::span()` is the span of the failing expression. The mapping is:
//! - entry 0 = (`<toplevel>`, frames[0].location), entry i = (frames[i-1].name, frames[i].location);
//! - the innermost frame is a builtin iff the error has no span of its own beyond the call site
//!   (its span equals the last frame's call span) — starlark-rust attaches the call-site span to
//!   errors raised by native functions.

use std::fmt::Write;

use starlark::PrintHandler;
use starlark::codemap::FileSpan;
use starlark::environment::Globals;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;

use crate::harness::bare_message;

/// 1-based (line, column) Bazel would report for `span`. For a call expression `f(...)` this is
/// the column of the `(` that matches the final `)`.
pub fn bazel_position(span: &FileSpan, is_call: bool) -> (usize, usize) {
    let begin = span.resolve_span().begin;
    if is_call {
        let text = span.source_span();
        if let Some(off) = lparen_offset(text) {
            let before = &text[..off];
            let nl = before.matches('\n').count();
            let col = match before.rfind('\n') {
                Some(i) => before[i + 1..].chars().count(),
                None => begin.column + before.chars().count(),
            };
            return (begin.line + 1 + nl, col + 1);
        }
    }
    (begin.line + 1, begin.column + 1)
}

/// Byte offset of the `(` matching the trailing `)` of a call expression's source text.
/// String literals inside the argument list are skipped (simple lexer: quotes and escapes).
fn lparen_offset(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.last() != Some(&b')') {
        return None;
    }
    // Forward scan recording every top-level '(' that is closed at the very end.
    let mut stack: Vec<usize> = Vec::new();
    let mut i = 0;
    let mut last_open_closed_at_end = None;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                let q = bytes[i];
                let triple = i + 2 < bytes.len() && bytes[i + 1] == q && bytes[i + 2] == q;
                i += if triple { 3 } else { 1 };
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == q
                        && (!triple
                            || (i + 2 < bytes.len() && bytes[i + 1] == q && bytes[i + 2] == q))
                    {
                        i += if triple { 3 } else { 1 };
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            b'(' | b'[' | b'{' => stack.push(i),
            b')' | b']' | b'}' => {
                let open = stack.pop();
                if i == bytes.len() - 1 {
                    last_open_closed_at_end = open;
                }
            }
            _ => {}
        }
        i += 1;
    }
    last_open_closed_at_end
}

/// Renders `e` the way Bazel's `EvalException.getMessageWithStack` does.
pub fn render_bazel(e: &starlark::Error) -> String {
    let frames = &e.call_stack().frames;
    let message = bare_message(e);
    // (name, location, location-is-a-call)
    let mut entries: Vec<(String, Option<FileSpan>, bool)> = Vec::new();
    let mut prefix = "Error: ".to_owned();
    if frames.is_empty() {
        // Error at top level of the module, outside any call.
        entries.push(("<toplevel>".to_owned(), e.span().cloned(), false));
    } else {
        let mut caller = "<toplevel>".to_owned();
        for f in frames {
            entries.push((caller, f.location.clone(), true));
            caller = f.name.clone();
        }
        let last = frames.last().unwrap();
        let builtin = match (e.span(), &last.location) {
            (None, _) => true,
            (Some(s), Some(l)) => s == l,
            (Some(_), None) => false,
        };
        if builtin {
            prefix = format!("Error in {}: ", last.name);
        } else {
            entries.push((last.name.clone(), e.span().cloned(), false));
        }
    }
    let mut buf = String::new();
    if !entries.is_empty() {
        buf.push_str("Traceback (most recent call last):\n");
        for (name, loc, is_call) in &entries {
            match loc {
                Some(loc) => {
                    let (line, col) = bazel_position(loc, *is_call);
                    writeln!(
                        buf,
                        "\tFile \"{}\", line {line}, column {col}, in {name}",
                        loc.filename()
                    )
                    .unwrap();
                    let src = loc.file.source_line(line - 1).trim();
                    writeln!(buf, "\t\t{src}").unwrap();
                }
                None => writeln!(buf, "\tFile \"<builtin>\", in {name}").unwrap(),
            }
        }
    }
    buf.push_str(&prefix);
    buf.push_str(&message);
    buf
}

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
        .map_err(|e| format!("{e}"))?;
    let reporter = crate::harness::Reporter::default();
    let res = Module::with_temp_heap(|module| {
        let mut eval = Evaluator::new(&module);
        eval.extra = Some(&reporter);
        eval.set_print_handler(&StdoutPrint);
        eval.eval_module(ast, globals).map(|_| ()).map_err(|e| {
            format!(
                "{}\n--- starlark-rust native rendering ---\n{e}",
                render_bazel(&e)
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
