//! Runner for Bazel's Starlark script tests on starlark-rust.
//!
//! Java: src/test/java/net/starlark/java/eval/ScriptTest.java `main`. Semantics kept:
//! - a file is split on "\n---\n"; each chunk is evaluated in a fresh Module, with blank lines
//!   prepended so line numbers match the file;
//! - `### regex` comments are expected-error expectations (Java `Pattern.find`);
//! - an evaluation error must match one expectation; every expectation must be matched;
//! - assert_ / assert_eq / assert_fails report without stopping the chunk.
//!
//! Difference: Java fails a chunk on any static (parse/resolve) error. starlark-rust resolves
//! some things statically that Java does dynamically, so a static error that matches an
//! expectation is accepted here and flagged `static` in the detail.

use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use starlark::PrintHandler;
use starlark::environment::Globals;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;
use starlark::syntax::ast::StmtP;

use crate::dialect;
use crate::harness;
use crate::harness::AssertOutcome;
use crate::harness::Reporter;

/// Failure categories used in the conformance report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    /// A builtin function, method or module that Bazel has and starlark-rust lacks.
    MissingBuiltin,
    /// Same behaviour (an error is raised), different error text.
    ErrorText,
    /// Different result value, or an error on one side only.
    Semantic,
    /// Parser/resolver (dialect) disagreement.
    DialectSyntax,
    /// The harness cannot express the test on starlark-rust (e.g. `freeze()`).
    Harness,
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Category::MissingBuiltin => "missing-builtin",
            Category::ErrorText => "error-text",
            Category::Semantic => "semantic",
            Category::DialectSyntax => "dialect-syntax",
            Category::Harness => "harness",
        })
    }
}

/// One failure in a chunk.
#[derive(Debug, Clone)]
pub struct Failure {
    pub line: usize,
    pub category: Category,
    pub detail: String,
}

/// Result of one chunk.
#[derive(Debug, Clone)]
pub struct ChunkResult {
    pub start_line: usize,
    pub failures: Vec<Failure>,
    pub asserts_passed: usize,
    pub asserts_run: usize,
    /// Assertion calls in the chunk source (static count; loops may run more, aborts fewer).
    pub asserts_in_source: usize,
    /// True when the chunk stopped on an unexpected error (later asserts never ran).
    pub aborted: bool,
    /// Statement-by-statement re-run of an aborted chunk (see `run_chunk_resilient`).
    pub resilient: Option<Resilient>,
}

impl ChunkResult {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Result of one file.
#[derive(Debug, Clone)]
pub struct FileResult {
    pub name: String,
    pub chunks: Vec<ChunkResult>,
}

impl FileResult {
    pub fn chunks_passed(&self) -> usize {
        self.chunks.iter().filter(|c| c.passed()).count()
    }
    pub fn asserts_passed(&self) -> usize {
        self.chunks.iter().map(|c| c.asserts_passed).sum()
    }
    pub fn asserts_run(&self) -> usize {
        self.chunks.iter().map(|c| c.asserts_run).sum()
    }
    pub fn asserts_in_source(&self) -> usize {
        self.chunks.iter().map(|c| c.asserts_in_source).sum()
    }
    pub fn failures(&self) -> impl Iterator<Item = &Failure> {
        self.chunks.iter().flat_map(|c| c.failures.iter())
    }
    /// Asserts passed, counting aborted chunks by their statement-by-statement re-run.
    pub fn asserts_passed_resilient(&self) -> usize {
        self.chunks
            .iter()
            .map(|c| {
                c.resilient
                    .as_ref()
                    .map_or(c.asserts_passed, |r| r.asserts_passed)
            })
            .sum()
    }
    pub fn asserts_run_resilient(&self) -> usize {
        self.chunks
            .iter()
            .map(|c| {
                c.resilient
                    .as_ref()
                    .map_or(c.asserts_run, |r| r.asserts_run)
            })
            .sum()
    }
    /// Failures, taking aborted chunks from their statement-by-statement re-run.
    pub fn failures_resilient(&self) -> impl Iterator<Item = &Failure> {
        self.chunks.iter().flat_map(|c| match &c.resilient {
            Some(r) => r.failures.iter(),
            None => c.failures.iter(),
        })
    }
}

/// Default location of Bazel's testdata at the pin.
pub fn default_testdata_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".cache/blaze/src/bazel-9.3.2/src/test/java/net/starlark/java/eval/testdata")
}

/// Globals predeclared for a ScriptTest chunk: Bazel universe + harness builtins + json.
pub fn script_test_globals() -> Globals {
    let (builder, _, _) = dialect::bazel_universe_builder();
    builder
        .with(harness::script_test_builtins)
        .with_namespace("json", harness::json_shim)
        .build()
}

struct DiscardPrint;
impl PrintHandler for DiscardPrint {
    fn println(&self, _text: &str) -> starlark::Result<()> {
        Ok(())
    }
}

pub struct Expectation {
    pub line: usize,
    pub pattern: String,
    pub regex: Option<regex::Regex>,
}

fn count_asserts(chunk: &str) -> usize {
    let re = regex::Regex::new(r"\bassert_(eq|fails)?\(").unwrap();
    chunk
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(|l| re.find_iter(l).count())
        .sum()
}

fn classify_error(msg: &str) -> Category {
    if msg.starts_with("HARNESS:") {
        Category::Harness
    } else if msg.starts_with("SHIM:")
        || (msg.contains("Variable `") && msg.contains("` not found"))
        || msg.contains("has no attribute")
    {
        Category::MissingBuiltin
    } else {
        Category::Semantic
    }
}

/// Assertions that cannot be run safely on starlark-rust 0.14.2: they allocate gigabytes (no
/// "excessive repeat" / capacity guard) or panic. Each is blanked out and reported as a
/// semantic failure. (file, 1-based line, reason)
pub const HAZARDS: &[(&str, usize, &str)] = &[
    (
        "json.star",
        66,
        "json.encode of a 100000-deep list overflows the native stack in starlark-rust (Value::to_json has no depth limit; uncatchable abort); Bazel: nesting depth limit exceeded",
    ),
    (
        "list.star",
        66,
        "[1, 2, 3] * (1 << 30) panics in starlark-rust (AlignedSize must not exceed u32::MAX); Bazel: excessive repeat",
    ),
    (
        "list_mutation.star",
        54,
        "[].extend(range((1 << 31) - 1)) tries to allocate ~16 GiB; Bazel: excessive capacity requested",
    ),
    (
        "string_misc.star",
        180,
        "\"abc\" * (1 << 30) allocates 3 GiB; Bazel: excessive repeat",
    ),
];

/// Turns assertion events into failures; returns (passed, run).
fn record_events(reporter: &Reporter, failures: &mut Vec<Failure>) -> (usize, usize) {
    let mut passed = 0;
    let mut run = 0;
    for ev in reporter.events.borrow().iter() {
        run += 1;
        let cat = match ev.outcome {
            AssertOutcome::Pass => {
                passed += 1;
                continue;
            }
            // Depends on the harness's unsupported freeze(): nothing was frozen.
            AssertOutcome::NoError if ev.message.contains("frozen") => Category::Harness,
            AssertOutcome::ValueMismatch | AssertOutcome::NoError => Category::Semantic,
            AssertOutcome::ErrorTextMismatch
                if ev.message.contains("(trying to mutate a frozen") =>
            {
                Category::Harness
            }
            AssertOutcome::ErrorTextMismatch => {
                // "did not match error (<msg>)"
                let got = ev.message.rsplit_once("did not match error (").map(|x| x.1);
                match got.map(classify_error) {
                    Some(Category::MissingBuiltin) => Category::MissingBuiltin,
                    Some(Category::Harness) => Category::Harness,
                    _ => Category::ErrorText,
                }
            }
        };
        failures.push(Failure {
            line: ev.line,
            category: cat,
            detail: ev.message.clone(),
        });
    }
    (passed, run)
}

fn panic_text(p: Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

/// Category of an evaluation error that matched no expectation.
fn classify_unexpected(e: &starlark::Error, msg: &str, error_expected: bool) -> Category {
    match classify_error(msg) {
        Category::Semantic if error_expected => Category::ErrorText,
        Category::Semantic if matches!(e.kind(), starlark::ErrorKind::Parser(_)) => {
            Category::DialectSyntax
        }
        c => c,
    }
}

/// Result of re-running an aborted chunk one top-level statement at a time.
#[derive(Debug, Clone)]
pub struct Resilient {
    pub asserts_passed: usize,
    pub asserts_run: usize,
    pub failures: Vec<Failure>,
}

/// Not a Java mode. When a chunk aborts on an unexpected error, every later assertion is lost.
/// This re-runs the chunk statement by statement in one Module (REPL-style), so an error in one
/// top-level statement is recorded and the rest still run. Expectations (`###`) apply to the
/// statement whose lines contain them.
pub fn run_chunk_resilient(
    file_name: &str,
    chunk: &str,
    linenum: usize,
    globals: &Globals,
    expectations: &[Expectation],
) -> Resilient {
    let mut failures = Vec::new();
    let mut asserts_passed = 0;
    let mut asserts_run = 0;
    let mut padded = "\n".repeat(linenum - 1);
    padded.push_str(chunk);
    let ast = match AstModule::parse(file_name, padded, &dialect::bzl_dialect()) {
        Ok(a) => a,
        Err(e) => {
            failures.push(Failure {
                line: linenum,
                category: Category::DialectSyntax,
                detail: format!("parse error: {}", harness::bare_message(&e)),
            });
            return Resilient {
                asserts_passed,
                asserts_run,
                failures,
            };
        }
    };
    // (first line 1-based, last line 1-based, source text)
    let stmts: Vec<(usize, usize, String)> = match &ast.statement().node {
        StmtP::Statements(xs) => xs.iter().collect::<Vec<_>>(),
        _ => vec![ast.statement()],
    }
    .into_iter()
    .map(|x| {
        let fs = ast.file_span(x.span);
        let r = fs.resolve_span();
        (
            r.begin.line + 1,
            r.end.line + 1,
            fs.source_span().to_owned(),
        )
    })
    .collect();
    Module::with_temp_heap(|module| {
        for (first, last, text) in stmts {
            let expected: Vec<&Expectation> = expectations
                .iter()
                .filter(|e| e.line >= first && e.line <= last)
                .collect();
            let src = format!("{}{}", "\n".repeat(first - 1), text);
            let ast = match AstModule::parse(file_name, src, &dialect::bzl_dialect()) {
                Ok(a) => a,
                Err(e) => {
                    failures.push(Failure {
                        line: first,
                        category: Category::DialectSyntax,
                        detail: format!("parse error: {}", harness::bare_message(&e)),
                    });
                    continue;
                }
            };
            let reporter = Reporter::default();
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut eval = Evaluator::new(&module);
                eval.extra = Some(&reporter);
                eval.set_print_handler(&DiscardPrint);
                eval.eval_module(ast, globals).map(|_| ())
            }));
            let (p, r) = record_events(&reporter, &mut failures);
            asserts_passed += p;
            asserts_run += r;
            match caught {
                Err(p) => failures.push(Failure {
                    line: first,
                    category: Category::Semantic,
                    detail: format!("PANIC in starlark-rust: {}", panic_text(p)),
                }),
                Ok(Ok(())) => {
                    for e in expected {
                        failures.push(Failure {
                            line: e.line,
                            category: Category::Semantic,
                            detail: format!(
                                "unmatched expectation (no error raised): {}",
                                e.pattern
                            ),
                        });
                    }
                }
                Ok(Err(e)) => {
                    let msg = harness::bare_message(&e);
                    let matched = expected
                        .iter()
                        .any(|x| x.regex.as_ref().is_some_and(|r| r.is_match(&msg)));
                    if !matched {
                        failures.push(Failure {
                            line: e
                                .span()
                                .map(|s| s.resolve_span().begin.line + 1)
                                .unwrap_or(first),
                            category: classify_unexpected(&e, &msg, !expected.is_empty()),
                            detail: format!("unexpected error: {msg}"),
                        });
                    }
                }
            }
        }
    });
    Resilient {
        asserts_passed,
        asserts_run,
        failures,
    }
}

/// Runs one chunk (already padded with leading newlines).
pub fn run_chunk(file_name: &str, chunk: &str, linenum: usize, globals: &Globals) -> ChunkResult {
    let mut failures = Vec::new();

    let mut lines: Vec<&str> = chunk.split('\n').collect();
    for (f, line, reason) in HAZARDS {
        if *f == file_name && *line >= linenum && *line < linenum + lines.len() {
            lines[*line - linenum] = "";
            failures.push(Failure {
                line: *line,
                category: Category::Semantic,
                detail: format!("SKIPPED (unsafe to run): {reason}"),
            });
        }
    }
    let chunk_owned = lines.join("\n");
    let chunk = chunk_owned.as_str();

    // Extract expectations: ### regex
    let mut expectations = Vec::new();
    let mut search = 0;
    while let Some(off) = chunk[search..].find("###") {
        let i = search + off;
        let j = chunk[i..].find('\n').map(|k| i + k).unwrap_or(chunk.len());
        let line = linenum + chunk[..i].matches('\n').count();
        let pattern = chunk[i + 3..j].trim().to_owned();
        let regex = regex::Regex::new(&pattern).ok();
        if regex.is_none() {
            failures.push(Failure {
                line,
                category: Category::Harness,
                detail: format!("expectation regex not valid in Rust regex: {pattern}"),
            });
        }
        expectations.push(Expectation {
            line,
            pattern,
            regex,
        });
        search = j;
    }

    let all_expectations: Vec<Expectation> = expectations
        .iter()
        .map(|e| Expectation {
            line: e.line,
            pattern: e.pattern.clone(),
            regex: e.regex.clone(),
        })
        .collect();
    let hazard_failures = failures.clone();

    let mut padded = "\n".repeat(linenum - 1);
    padded.push_str(chunk);

    let asserts_in_source = count_asserts(chunk);
    let mut asserts_passed = 0;
    let mut asserts_run = 0;
    let mut aborted = false;

    let mut match_expectation = |msg: &str| -> bool {
        if let Some(pos) = expectations
            .iter()
            .position(|e| e.regex.as_ref().is_some_and(|r| r.is_match(msg)))
        {
            expectations.remove(pos);
            true
        } else {
            false
        }
    };

    match AstModule::parse(file_name, padded, &dialect::bzl_dialect()) {
        Err(e) => {
            let msg = harness::bare_message(&e);
            if !match_expectation(&msg) {
                aborted = true;
                failures.push(Failure {
                    line: e
                        .span()
                        .map(|s| s.resolve_span().begin.line + 1)
                        .unwrap_or(linenum),
                    category: Category::DialectSyntax,
                    detail: format!("parse error: {msg}"),
                });
            }
        }
        Ok(ast) => {
            let reporter = Reporter::default();
            // starlark-rust panics (instead of erroring) on some inputs, e.g.
            // `[1, 2, 3] * (1 << 30)` ("AlignedSize must not exceed u32::MAX"). Contain it.
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Module::with_temp_heap(|module| {
                    let mut eval = Evaluator::new(&module);
                    eval.extra = Some(&reporter);
                    eval.set_print_handler(&DiscardPrint);
                    eval.eval_module(ast, globals).map(|_| ())
                })
            }));
            let result = match caught {
                Ok(r) => r,
                Err(p) => {
                    let what = panic_text(p);
                    let line = reporter
                        .events
                        .borrow()
                        .last()
                        .map(|e| e.line + 1)
                        .unwrap_or(linenum);
                    failures.push(Failure {
                        line,
                        category: Category::Semantic,
                        detail: format!("PANIC in starlark-rust (process abort in Blaze): {what}"),
                    });
                    aborted = true;
                    Ok(())
                }
            };
            let (p, r) = record_events(&reporter, &mut failures);
            asserts_passed += p;
            asserts_run += r;
            if let Err(e) = result {
                let msg = harness::bare_message(&e);
                if !match_expectation(&msg) {
                    aborted = true;
                    let line = e
                        .span()
                        .map(|s| s.resolve_span().begin.line + 1)
                        .unwrap_or(linenum);
                    let category = classify_unexpected(&e, &msg, !expectations.is_empty());
                    failures.push(Failure {
                        line,
                        category,
                        detail: format!("unexpected error: {msg}"),
                    });
                }
            }
        }
    }

    for e in expectations {
        // If the chunk already aborted with a categorised error, that error explains the miss.
        if aborted && failures.iter().any(|f| f.line == e.line) {
            continue;
        }
        failures.push(Failure {
            line: e.line,
            category: Category::Semantic,
            detail: format!("unmatched expectation (no error raised): {}", e.pattern),
        });
    }

    let resilient = aborted.then(|| {
        let mut r = run_chunk_resilient(file_name, chunk, linenum, globals, &all_expectations);
        let mut f = hazard_failures;
        f.append(&mut r.failures);
        r.failures = f;
        r
    });

    ChunkResult {
        start_line: linenum,
        failures,
        asserts_passed,
        asserts_run,
        asserts_in_source,
        aborted,
        resilient,
    }
}

/// Runs one testdata file.
pub fn run_file(path: &Path, globals: &Globals) -> anyhow::Result<FileResult> {
    let content = std::fs::read_to_string(path)?;
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut linenum = 1;
    let mut chunks = Vec::new();
    for chunk in content.split("\n---\n") {
        chunks.push(run_chunk(&name, chunk, linenum, globals));
        linenum += chunk.matches('\n').count() + 2;
    }
    Ok(FileResult { name, chunks })
}

/// Runs every `*.star` file in `dir`, sorted by name.
pub fn run_dir(dir: &Path) -> anyhow::Result<Vec<FileResult>> {
    let globals = script_test_globals();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "star"))
        .collect();
    paths.sort();
    paths.iter().map(|p| run_file(p, &globals)).collect()
}

/// Markdown summary table.
pub fn summary_table(results: &[FileResult]) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;
    let cats = [
        Category::MissingBuiltin,
        Category::ErrorText,
        Category::Semantic,
        Category::DialectSyntax,
        Category::Harness,
    ];
    let mut out = String::new();
    writeln!(
        out,
        "| file | chunks pass/total | asserts pass/run (in source) | resilient asserts pass/run | missing-builtin | error-text | semantic | dialect-syntax | harness |"
    )
    .unwrap();
    writeln!(out, "|---|---|---|---|---|---|---|---|---|").unwrap();
    let mut tot = (0, 0, 0, 0, 0, 0, 0);
    let mut tot_cat: BTreeMap<Category, usize> = BTreeMap::new();
    for r in results {
        let mut by: BTreeMap<Category, usize> = BTreeMap::new();
        for f in r.failures_resilient() {
            *by.entry(f.category).or_default() += 1;
            *tot_cat.entry(f.category).or_default() += 1;
        }
        tot.0 += r.chunks_passed();
        tot.1 += r.chunks.len();
        tot.2 += r.asserts_passed();
        tot.3 += r.asserts_run();
        tot.4 += r.asserts_in_source();
        tot.5 += r.asserts_passed_resilient();
        tot.6 += r.asserts_run_resilient();
        let cells: Vec<String> = cats
            .iter()
            .map(|c| by.get(c).map(|n| n.to_string()).unwrap_or_default())
            .collect();
        writeln!(
            out,
            "| {} | {}/{} | {}/{} ({}) | {}/{} | {} |",
            r.name,
            r.chunks_passed(),
            r.chunks.len(),
            r.asserts_passed(),
            r.asserts_run(),
            r.asserts_in_source(),
            r.asserts_passed_resilient(),
            r.asserts_run_resilient(),
            cells.join(" | ")
        )
        .unwrap();
    }
    let cells: Vec<String> = cats
        .iter()
        .map(|c| tot_cat.get(c).copied().unwrap_or(0).to_string())
        .collect();
    writeln!(
        out,
        "| **total** | {}/{} | {}/{} ({}) | {}/{} | {} |",
        tot.0,
        tot.1,
        tot.2,
        tot.3,
        tot.4,
        tot.5,
        tot.6,
        cells.join(" | ")
    )
    .unwrap();
    out
}

/// Every failure, one per line: `file:line: [category] detail`. Aborted chunks are listed from
/// their statement-by-statement re-run (the category columns of `summary_table` count these).
pub fn failure_list(results: &[FileResult]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for r in results {
        for f in r.failures_resilient() {
            writeln!(out, "{}:{}: [{}] {}", r.name, f.line, f.category, f.detail).unwrap();
        }
    }
    out
}
