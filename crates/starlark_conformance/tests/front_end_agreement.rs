//! Blaze reads every Starlark file with two front ends: the port of Bazel's (for the error list,
//! blaze_starlark::syntax::java) and starlark-rust's (for execution). They must agree on every
//! real file: both accept or both reject, and where both accept, the same number of top-level
//! statements binding the same top-level names.
//!
//! Files: the corpus checkouts (brotli, re2), the external repositories of the oracle's
//! recording of them (`~/.cache/blaze/oracle-out/rec-oracle-9.3.2/*/external`), the oracle's
//! embedded `@bazel_tools`, and Bazel's own builtins (`src/main/starlark`). A root that does not
//! exist on this machine is skipped and named in the output. A file that no longer exists (a
//! symlink in an output base whose target was deleted) is not an input: it is skipped and
//! counted. The test fails if no file at all could be read.

use std::path::Path;
use std::path::PathBuf;

use blaze_starlark::FileKind;

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn roots() -> Vec<PathBuf> {
    let c = home().join(".cache/blaze");
    let mut roots = vec![
        c.join("corpus/brotli"),
        c.join("corpus/re2"),
        c.join("oracle-out/ur/install/58cf1ecbff441bac762b0050d6da21e9/embedded_tools"),
        c.join("src/bazel-9.3.2/src/main/starlark"),
    ];
    if let Ok(rd) = std::fs::read_dir(c.join("oracle-out/rec-oracle-9.3.2")) {
        for e in rd.flatten() {
            let ext = e.path().join("external");
            if ext.is_dir() {
                roots.push(ext);
            }
        }
    }
    roots
}

fn kind_of(name: &str) -> Option<FileKind> {
    if name.ends_with(".BUILD") || name.starts_with("BUILD.") && name != "BUILD.bazel" {
        // e.g. `foo.BUILD` build files of repository rules.
        return Some(FileKind::Build);
    }
    FileKind::from_file_name(name)
}

fn walk(p: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 40 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(p) else {
        return;
    };
    for e in rd.flatten() {
        let path = e.path();
        // Follow symlinked directories (repositories are symlinks in some output bases).
        if path.is_dir() {
            walk(&path, out, depth + 1);
        } else if let Some(n) = path.file_name().and_then(|n| n.to_str())
            && kind_of(n).is_some()
        {
            out.push(path);
        }
    }
}

#[test]
fn java_and_starlark_rust_front_ends_agree_on_real_files() {
    let mut files = Vec::new();
    for r in roots() {
        if r.is_dir() {
            walk(&r, &mut files, 0);
        } else {
            eprintln!("skipped (not on this machine): {}", r.display());
        }
    }
    files.sort();
    files.dedup();
    let mut disagreements = Vec::new();
    let mut accepted = 0;
    let mut rejected = 0;
    let mut missing = 0;
    for f in &files {
        let name = f.file_name().unwrap().to_str().unwrap();
        let kind = kind_of(name).unwrap();
        let mut options = kind.file_options();
        // Bazel 9.3.2's default `--experimental_starlark_type_syntax` (BzlCompileFunction
        // .shouldUseTypeSyntax: .bzl files other than the prelude and .scl).
        options.allow_type_syntax = kind == FileKind::Bzl || kind == FileKind::BuiltinsBzl;
        let bytes = match std::fs::read(f) {
            Ok(bytes) => bytes,
            // A dangling symlink: the file it named no longer exists.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                missing += 1;
                continue;
            }
            Err(e) => panic!("{}: {e}", f.display()),
        };
        let source = blaze_util::string_encoding::latin1_decode(&bytes).into_owned();
        let display = f.display().to_string();
        let (java, rust) = blaze_starlark::eval::front_ends(kind, options, &display, &source);
        match (&java, &rust) {
            (Ok(j), Ok(r)) => {
                accepted += 1;
                if j != r {
                    disagreements.push(format!("{display}: java {j:?} | starlark-rust {r:?}"));
                }
            }
            (Err(_), Err(_)) => rejected += 1,
            (Ok(_), Err(e)) => disagreements.push(format!(
                "{display}: java accepts, starlark-rust rejects: {e:?}"
            )),
            (Err(e), Ok(_)) => disagreements.push(format!(
                "{display}: starlark-rust accepts, java rejects: {e:?}"
            )),
        }
    }
    eprintln!(
        "{} files: {accepted} accepted by both, {rejected} rejected by both, {} disagreements, \
         {missing} skipped (the file no longer exists)",
        files.len(),
        disagreements.len()
    );
    assert!(
        accepted + rejected > 0,
        "no file could be read ({} found, {missing} no longer exist)",
        files.len()
    );
    assert!(
        disagreements.is_empty(),
        "{} disagreements:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}
