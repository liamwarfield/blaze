//! Error locations in tracebacks, compared with Bazel 9.3.2. Each `tests/data/locations/NAME.bzl`
//! fails during initialization; `NAME.oracle.txt` is the traceback the oracle printed for it
//! (recorded by `scripts/starlark-locations record`). Compared: every traceback line (files,
//! lines, columns, function names, source lines) and the kind of the last line (`Error: ` or
//! `Error in NAME: `). The message itself is starlark-rust's where Bazel's text differs
//! (decisions.md), so it is not compared.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use blaze_starlark::FileKind;
use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::env::NativeRegistry;
use blaze_starlark::eval;
use blaze_starlark::loader::LoadMap;
use blaze_starlark::thread::CollectingEvents;

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/locations")
}

/// Blaze's rendering of case `name`, evaluated as `<WS>/name/name.bzl`.
fn render(name: &str, src: String) -> String {
    let file = format!("<WS>/{name}/{name}.bzl");
    eval::run_on_evaluation_thread(move || {
        let semantics = StarlarkSemantics::default();
        let events = Arc::new(CollectingEvents::default());
        let ctx = common::context(Phase::BzlLoad, semantics.clone(), events);
        let globals =
            NativeRegistry::with_blaze_starlark_builtins().build(FileKind::Bzl, &semantics);
        match eval::compile(FileKind::Bzl, &file, src).and_then(|c| {
            eval::exec_module(c, &globals, &LoadMap::new(), &ctx, &Default::default())
        }) {
            Ok(_) => "NO ERROR".to_owned(),
            Err(e) => e.message_with_stack(),
        }
    })
}

/// The compared part of a rendering: traceback lines and the last line up to the message.
fn shape(text: &str) -> String {
    let text = text.trim_end();
    let (head, last) = text.rsplit_once('\n').unwrap_or(("", text));
    let kind = if let Some(rest) = last.strip_prefix("Error in ") {
        format!("Error in {}:", rest.split(':').next().unwrap_or(""))
    } else if last.starts_with("Error: ") {
        "Error:".to_owned()
    } else {
        // Syntax errors: `file:line:col: message`.
        last.rsplitn(2, ": ").last().unwrap_or(last).to_owned()
    };
    format!("{head}\n{kind}")
}

/// Cases that still differ, with the reason. A case that starts matching fails the test, so the
/// entry is removed when a fix lands.
const KNOWN_DEVIATIONS: &[(&str, &str)] = &[
    (
        "call_not_callable",
        "starlark-rust pushes a frame for the non-callable value and reports `Error in 1` at the \
         `(`; Java's Starlark.getStarlarkCallable fails before the call (no frame), located by \
         the enclosing statement (`=`)",
    ),
    (
        "star_not_iterable",
        "`f(*x)` with a non-iterable x fails inside the callee's argument binding (`Error in \
         len` at the `(`); Java fails before the call, at the `*` (Eval.evalCall)",
    ),
    (
        "starstar_not_dict",
        "`f(**x)` with a non-dict x: as star_not_iterable, Java at the `**`",
    ),
];

#[test]
fn error_locations_match_bazel() {
    let mut names: Vec<String> = std::fs::read_dir(data_dir())
        .unwrap()
        .filter_map(|e| {
            let n = e.unwrap().file_name().to_string_lossy().into_owned();
            n.strip_suffix(".bzl").map(str::to_owned)
        })
        .collect();
    names.sort();
    assert!(names.len() >= 50, "{} cases", names.len());
    let mut failures = Vec::new();
    for name in &names {
        let dir = data_dir();
        let src = std::fs::read_to_string(dir.join(format!("{name}.bzl"))).unwrap();
        let want = std::fs::read_to_string(dir.join(format!("{name}.oracle.txt"))).unwrap();
        let got = render(name, src);
        let known = KNOWN_DEVIATIONS.iter().any(|(n, _)| n == name);
        if known && shape(&got) == shape(&want) {
            failures.push(format!(
                "--- {name}\nmatches Bazel now: remove it from KNOWN_DEVIATIONS"
            ));
        } else if !known && shape(&got) != shape(&want) {
            failures.push(format!(
                "--- {name}\nwant:\n{}\ngot:\n{}\n(full: {got})",
                shape(&want),
                shape(&got)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ from Bazel:\n{}",
        failures.len(),
        names.len(),
        failures.join("\n")
    );
}
