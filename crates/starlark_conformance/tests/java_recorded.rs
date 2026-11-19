//! Runs the Starlark files in `tests/data/*.star` on starlark-rust and compares what they print
//! with what Bazel 9.3.2's own interpreter printed (`<name>.latin1.txt`, `<name>.unicode.txt`,
//! recorded with `java/StarlarkProbe.java` on the oracle's A-server.jar; see its header).
//!
//! `latin1` is how Bazel and Blaze read every Starlark file (one char per byte, the byte-strings
//! semantics flag on); `unicode` (the flag off) is only compared for `latin1_chars.star`, whose
//! source is ASCII, so its strings are the same chars in both interpreters.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;

use blaze_starlark::EvalError;
use blaze_starlark::error::EvalErrorKind;
use starlark::PrintHandler;
use starlark::environment::GlobalsBuilder;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;
use starlark_conformance::dialect;
use starlark_conformance::harness::bare_message;
use starlark_conformance::script_test::ScriptMode;

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// The probe's output escaping (StarlarkProbe.out).
fn escape(s: &str) -> String {
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

/// Runs `name` (a file in tests/data) as the probe does; returns the printed lines.
fn run(name: &str, mode: ScriptMode) -> Vec<String> {
    let bytes = std::fs::read(data_dir().join(name)).unwrap();
    let source = match mode {
        ScriptMode::Latin1 => blaze_util::string_encoding::latin1_decode(&bytes).into_owned(),
        ScriptMode::Unicode => String::from_utf8(bytes).unwrap(),
    };
    let name = name.to_owned();
    std::thread::Builder::new()
        .stack_size(1 << 28)
        .spawn(move || {
            let (builder, _, _) = dialect::bazel_universe_builder();
            let mut builder: GlobalsBuilder = builder.with(blaze_starlark::json::register_json);
            builder.set("_utf8_byte_strings", mode.utf8_byte_strings());
            let globals = builder.build();
            let out = Collect::default();
            let ast = match AstModule::parse(&name, source, &dialect::bzl_dialect()) {
                Ok(ast) => ast,
                Err(e) => return vec![syntax_line(&e)],
            };
            let result = Module::with_temp_heap(|module| {
                let mut eval = Evaluator::new(&module);
                eval.set_forbid_recursion(true);
                eval.set_utf8_byte_strings(mode.utf8_byte_strings());
                eval.set_print_handler(&out);
                eval.eval_module(ast, &globals).map(|_| ())
            });
            let mut lines = out.0.into_inner().unwrap();
            if let Err(e) = result {
                lines.push(escape(&format!("EVAL {}", bare_message(&e))));
            }
            lines
        })
        .unwrap()
        .join()
        .unwrap()
}

/// `SYNTAX file:line:col: message` (Java `SyntaxError.toString`).
fn syntax_line(e: &starlark::Error) -> String {
    let err = EvalError::from_starlark(e);
    match &err.kind {
        EvalErrorKind::Syntax(errors) => escape(&format!("SYNTAX {}", errors[0])),
        _ => escape(&format!("EVAL {}", err.message)),
    }
}

fn expected(name: &str, mode: ScriptMode) -> Vec<String> {
    let stem = name.strip_suffix(".star").unwrap();
    let suffix = match mode {
        ScriptMode::Latin1 => "latin1",
        ScriptMode::Unicode => "unicode",
    };
    std::fs::read_to_string(data_dir().join(format!("{stem}.{suffix}.txt")))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Every differing line, as `file:line: java | rust`.
fn compare(name: &str, mode: ScriptMode) -> Vec<String> {
    let want = expected(name, mode);
    let got = run(name, mode);
    let mut bad = Vec::new();
    for i in 0..want.len().max(got.len()) {
        let w = want.get(i).map(String::as_str).unwrap_or("<none>");
        let g = got.get(i).map(String::as_str).unwrap_or("<none>");
        if w != g {
            bad.push(format!(
                "{name}:{}: java {w}\n{:>width$} rust {g}",
                i + 1,
                "",
                width = name.len()
            ));
        }
    }
    bad
}

fn star_files() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(data_dir())
        .unwrap()
        .filter_map(|e| {
            let n = e.unwrap().file_name().into_string().unwrap();
            n.ends_with(".star").then_some(n)
        })
        .collect();
    v.sort();
    v
}

#[test]
fn java_recorded_latin1() {
    let files = star_files();
    assert!(files.len() >= 13, "{files:?}");
    let bad: Vec<String> = files
        .iter()
        .flat_map(|f| compare(f, ScriptMode::Latin1))
        .collect();
    assert!(bad.is_empty(), "differences from Java:\n{}", bad.join("\n"));
}

#[test]
fn java_recorded_unicode_latin1_chars() {
    let bad = compare("latin1_chars.star", ScriptMode::Unicode);
    assert!(bad.is_empty(), "differences from Java:\n{}", bad.join("\n"));
    assert!(Path::new(&data_dir().join("latin1_chars.unicode.txt")).exists());
}
