//! `starlark-conformance`: runs Bazel's Starlark script tests on starlark-rust.

use std::path::PathBuf;

use starlark_conformance::script_test::ScriptMode;

const USAGE: &str = "\
usage: starlark-conformance <command> [options]

commands:
  help                 print this message
  run [-v] [--latin1] [--testdata DIR] [FILE.star...]
                       run Bazel's ScriptTest files (default: all *.star in DIR) and print a
                       per-file markdown table; -v also lists every failure with its category;
                       --latin1 imitates Bazel's ScriptTest_Latin1 target (sources decoded one
                       char per byte, the byte-strings semantics flag on, _utf8_byte_strings =
                       True), which is how Blaze reads Starlark files
  eval [--latin1] FILE evaluate one Starlark file with the ScriptTest globals (print enabled) and
                       show the Bazel-style traceback of any error
  probe [--unicode] FILE...
                       run each file as java/StarlarkProbe.java does on Bazel's interpreter
                       (Blaze's universe, json, print) and print the probe's lines; default
                       mode is latin1 (one char per byte, byte-strings flag on)
  probe-diff [-v] [--only SUBSTR] [DIR]
                       run every case of DIR (default tests/data/interp) as `probe` does and
                       compare all lines (printed output, every SYNTAX line, the EVAL message
                       and Bazel's traceback as TRACE lines) with NAME.latin1.txt, recorded on
                       Bazel's interpreter (scripts/starlark/interp-diff record-java). One line
                       per case: MATCH, TEXT (same output and error kind, different error
                       text/location/traceback) or DIFF; -v shows both sides of each difference
  parse PATH...         compile every Starlark file under PATH (.bzl, .scl, BUILD, *.BUILD,
                       MODULE.bazel, REPO.bazel) with Bazel's dialect and static checks
                       (blaze_starlark::eval::compile) and list failures
  failures [--latin1] [--testdata DIR]
                       list every failure as `file:line: [category] detail`

DIR defaults to ~/.cache/blaze/src/bazel-9.3.2/src/test/java/net/starlark/java/eval/testdata.
Categories: missing-builtin, error-text, semantic, dialect-syntax, harness.";

fn main() -> anyhow::Result<()> {
    // Starlark evaluation recurses on the native stack (calls, repr, json, GC tracing of deeply
    // nested values). Run on a thread with a large stack, as Blaze's evaluation threads do.
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(real_main)?
        .join()
        .map_err(|_| anyhow::anyhow!("main thread panicked"))?
}

fn real_main() -> anyhow::Result<()> {
    // Panics inside starlark-rust are caught per chunk and reported; keep stderr quiet.
    std::panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        println!("{USAGE}");
        return Ok(());
    };
    if cmd == "probe" {
        let mode = if args[1..].iter().any(|a| a == "--unicode") {
            ScriptMode::Unicode
        } else {
            ScriptMode::Latin1
        };
        for f in args[1..].iter().filter(|a| *a != "--unicode") {
            let name = PathBuf::from(f)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            for line in starlark_conformance::probe::run(&name, &std::fs::read(f)?, mode) {
                println!("{line}");
            }
        }
        return Ok(());
    }
    if cmd == "probe-diff" {
        let mut verbose = false;
        let mut only: Option<String> = None;
        let mut dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/interp");
        let mut it = args[1..].iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "-v" => verbose = true,
                "--only" => only = it.next().cloned(),
                d => dir = PathBuf::from(d),
            }
        }
        use starlark_conformance::probe::CaseResult;
        let (mut m, mut t, mut d) = (0, 0, 0);
        for name in starlark_conformance::probe::case_names(&dir) {
            if only.as_ref().is_some_and(|o| !name.contains(o.as_str())) {
                continue;
            }
            let c = starlark_conformance::probe::compare_case(&dir, &name);
            let tag = match c.result {
                CaseResult::Match => {
                    m += 1;
                    "MATCH"
                }
                CaseResult::TextDiffers => {
                    t += 1;
                    "TEXT"
                }
                CaseResult::Differs => {
                    d += 1;
                    "DIFF"
                }
            };
            println!("{tag} {}", c.name);
            if verbose && c.result != CaseResult::Match {
                for l in &c.java {
                    println!("    java {l}");
                }
                for l in &c.rust {
                    println!("    rust {l}");
                }
            }
        }
        println!("{} cases: {m} match, {t} differ in error text only, {d} differ", m + t + d);
        return Ok(());
    }
    if cmd == "parse" {
        let mut files = Vec::new();
        for a in &args[1..] {
            walk(&PathBuf::from(a), &mut files);
        }
        files.sort();
        files.dedup();
        let mut bad = 0;
        for f in &files {
            let kind = blaze_starlark::FileKind::from_file_name(
                &f.file_name().unwrap_or_default().to_string_lossy(),
            )
            .unwrap();
            let src = std::fs::read_to_string(f)?;
            if let Err(e) = blaze_starlark::eval::compile(kind, &f.to_string_lossy(), src) {
                bad += 1;
                println!("{}", e.message_with_stack().replace('\n', " | "));
            }
        }
        println!("parsed {} files, {} failed", files.len(), bad);
        return Ok(());
    }
    let mode = if args[1..].iter().any(|a| a == "--latin1") {
        ScriptMode::Latin1
    } else {
        ScriptMode::Unicode
    };
    if cmd == "eval" {
        let path = args[1..]
            .iter()
            .find(|a| *a != "--latin1")
            .expect("eval needs a FILE");
        let src = mode.source(&std::fs::read_to_string(path)?);
        let globals = starlark_conformance::script_test::script_test_globals(mode);
        match starlark_conformance::traceback::eval_for_cli(path, &src, &globals, mode) {
            Ok(()) => println!("OK"),
            Err(tb) => {
                println!("{tb}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let mut verbose = false;
    let mut dir = starlark_conformance::script_test::default_testdata_dir();
    let mut files = Vec::new();
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-v" => verbose = true,
            "--latin1" => {}
            "--testdata" => dir = PathBuf::from(it.next().expect("--testdata needs a value")),
            f => files.push(f.to_owned()),
        }
    }
    let results = if files.is_empty() {
        starlark_conformance::script_test::run_dir(&dir, mode)?
    } else {
        let globals = starlark_conformance::script_test::script_test_globals(mode);
        files
            .iter()
            .map(|f| {
                let p = PathBuf::from(f);
                let p = if p.exists() { p } else { dir.join(f) };
                starlark_conformance::script_test::run_file(&p, &globals, mode)
            })
            .collect::<anyhow::Result<Vec<_>>>()?
    };
    match cmd.as_str() {
        "help" | "--help" | "-h" => println!("{USAGE}"),
        "run" => {
            print!(
                "{}",
                starlark_conformance::script_test::summary_table(&results)
            );
            if verbose {
                println!();
                print!(
                    "{}",
                    starlark_conformance::script_test::failure_list(&results)
                );
            }
        }
        "failures" => print!(
            "{}",
            starlark_conformance::script_test::failure_list(&results)
        ),
        other => {
            eprintln!("unknown command: {other}\n\n{USAGE}");
            std::process::exit(2);
        }
    }
    Ok(())
}

fn walk(p: &std::path::Path, out: &mut Vec<PathBuf>) {
    if p.is_dir() {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                walk(&e.path(), out);
            }
        }
    } else if p
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(blaze_starlark::FileKind::from_file_name)
        .is_some()
    {
        out.push(p.to_owned());
    }
}
