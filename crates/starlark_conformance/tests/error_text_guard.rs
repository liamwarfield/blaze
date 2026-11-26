//! Guard for docs/decisions.md 2026-11-19 (Starlark core-language error texts are ported): every
//! string literal on an error path of the vendored interpreter (third_party/starlark-rust) is
//! one of Bazel's texts (a message of docs/starlark-errors.tsv, Java's error sites of
//! net.starlark.java at the pin) or is listed, with its reason, in
//! tests/data/rust_error_texts_allowed.tsv (internal errors, errors of starlark-rust features
//! Bazel does not have, texts Bazel's dialect cannot reach).
//!
//! Error paths: `#[error("...")]`, `anyhow!(`, `value_error!(`, `bail!(`, `format!(` given to
//! `ValueError::Bazel` / `FunctionError::Bazel`, `Error::new_other(`, `EvalSeverity` texts are not
//! scanned. A literal matches a Java text when its fixed pieces (the text between `{...}`
//! placeholders) occur, in order, in the Java message with its format specifiers removed.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

const PLACEHOLDER: char = '\u{1}';

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Parses the Rust string literal at the start of `s` (`"..."`, `r"..."`, `r#"..."#`); returns
/// its value.
fn parse_rust_literal(s: &str) -> Option<String> {
    let s = s.trim_start();
    if let Some(rest) = s.strip_prefix('r') {
        let hashes = rest.chars().take_while(|c| *c == '#').count();
        let rest = rest[hashes..].strip_prefix('"')?;
        let end = format!("\"{}", "#".repeat(hashes));
        let i = rest.find(&end)?;
        return Some(rest[..i].to_owned());
    }
    let rest = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                '\'' => out.push('\''),
                '0' => out.push('\0'),
                '\n' => {
                    // Line continuation: skip the next line's leading whitespace.
                    let skipped: String = chars.clone().take_while(|c| c.is_whitespace()).collect();
                    for _ in 0..skipped.chars().count() {
                        chars.next();
                    }
                }
                'x' => {
                    let h: String = chars.by_ref().take(2).collect();
                    out.push(u8::from_str_radix(&h, 16).ok()? as char);
                }
                'u' => {
                    let u: String = chars.by_ref().skip(1).take_while(|c| *c != '}').collect();
                    out.push(char::from_u32(u32::from_str_radix(&u, 16).ok()?)?);
                }
                c => out.push(c),
            },
            c => out.push(c),
        }
    }
    None
}

/// A Rust format string with each `{...}` placeholder replaced by [`PLACEHOLDER`].
fn rust_skeleton(format: &str) -> String {
    let mut out = String::new();
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                }
                out.push(PLACEHOLDER);
            }
            c => out.push(c),
        }
    }
    out
}

/// The error literals of one source file: (line, literal).
fn error_literals(text: &str) -> Vec<(usize, String)> {
    // Unit tests at the end of a file are not error paths.
    let text = match text.find("#[cfg(test)]\nmod ") {
        Some(i) => &text[..i],
        None => text,
    };
    const OPENERS: &[&str] = &[
        "#[error(",
        "anyhow!(",
        "value_error!(",
        "bail!(",
        "Bazel(format!(",
        "Error::new_other(format!(",
        // Patch 0061's markers of where Java raises an error.
        "bazel_argument_binding_error(format!(",
        "JavaNotCallable(format!(",
        "JavaStar(format!(",
        "JavaStarStar(format!(",
        "JavaBinding(format!(",
    ];
    let mut out = Vec::new();
    for opener in OPENERS {
        let mut from = 0;
        while let Some(i) = text[from..].find(opener) {
            let at = from + i + opener.len();
            from = at;
            if let Some(lit) = parse_rust_literal(&text[at..]) {
                let line = text[..at].matches('\n').count() + 1;
                out.push((line, lit));
            }
        }
    }
    out
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if p.file_name()
                .is_some_and(|n| n == "tests" || n == "testcases")
            {
                continue;
            }
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs")
            && !p.file_name().unwrap().to_string_lossy().contains("test")
        {
            out.push(p);
        }
    }
}

/// The Java messages of docs/starlark-errors.tsv: string literals of the `message` column
/// joined (Java concatenation), format specifiers as [`PLACEHOLDER`].
fn java_texts() -> Vec<String> {
    let tsv = std::fs::read_to_string(root().join("docs/starlark-errors.tsv")).unwrap();
    let mut out = Vec::new();
    for line in tsv.lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 6 {
            continue;
        }
        let message = cols[5];
        let mut text = String::new();
        let mut rest = message;
        while let Some(i) = rest.find('"') {
            let Some(lit) = parse_rust_literal(&rest[i..]) else {
                break;
            };
            // Advance past the literal (its source length: find the closing quote).
            let src = &rest[i + 1..];
            let mut end = 0;
            let mut esc = false;
            for (j, c) in src.char_indices() {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    end = j;
                    break;
                }
            }
            text.push_str(&lit);
            text.push(PLACEHOLDER);
            rest = &src[end + 1..];
        }
        let mut skel = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%' {
                match chars.peek() {
                    Some('%') => {
                        chars.next();
                        skel.push('%');
                    }
                    Some(_) => {
                        // %s, %d, %r, %02x, %,d ...
                        while let Some(c) = chars.next() {
                            if c.is_ascii_alphabetic() {
                                break;
                            }
                        }
                        skel.push(PLACEHOLDER);
                    }
                    None => skel.push('%'),
                }
            } else {
                skel.push(c);
            }
        }
        out.push(skel);
    }
    out
}

/// Whether the fixed pieces of `rust` occur in order in `java`.
fn matches(rust: &str, java: &str) -> bool {
    let pieces: Vec<&str> = rust
        .split(PLACEHOLDER)
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if pieces.is_empty() {
        return false;
    }
    let mut at = 0;
    for p in pieces {
        match java[at..].find(p) {
            Some(i) => at += i + p.len(),
            None => return false,
        }
    }
    true
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

#[test]
fn every_vendored_error_text_is_bazels_or_listed() {
    let java = java_texts();
    let allowed_path =
        root().join("crates/starlark_conformance/tests/data/rust_error_texts_allowed.tsv");
    let allowed_text = std::fs::read_to_string(&allowed_path).unwrap_or_default();
    // FILE<TAB>LITERAL<TAB>REASON; the reason is required.
    let mut allowed = BTreeSet::new();
    for line in allowed_text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let cols: Vec<&str> = line.splitn(3, '\t').collect();
        assert!(
            cols.len() == 3 && !cols[2].trim().is_empty(),
            "{}: a line without a reason: {line}",
            allowed_path.display()
        );
        allowed.insert((cols[0].to_owned(), cols[1].to_owned()));
    }
    let vendored = root().join("third_party/starlark-rust");
    let mut files = Vec::new();
    for c in ["starlark/src", "starlark_syntax/src"] {
        rust_files(&vendored.join(c), &mut files);
    }
    let mut unlisted = Vec::new();
    let mut seen = BTreeSet::new();
    for f in &files {
        let rel = f
            .strip_prefix(&vendored)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let text = std::fs::read_to_string(f).unwrap();
        for (line, lit) in error_literals(&text) {
            let skel = rust_skeleton(&lit);
            let key = (rel.clone(), escape(&lit));
            seen.insert(key.clone());
            if java.iter().any(|j| matches(&skel, j)) || allowed.contains(&key) {
                continue;
            }
            unlisted.push(format!("{rel}:{line}\t{}", escape(&lit)));
        }
    }
    let stale: Vec<_> = allowed.difference(&seen).collect();
    assert!(
        unlisted.is_empty() && stale.is_empty(),
        "error texts of the vendored interpreter that are none of Bazel's (docs/starlark-errors.tsv) \
         and not listed in {}:\n{}\nlisted but gone:\n{:?}",
        allowed_path.display(),
        unlisted.join("\n"),
        stale
    );
}
