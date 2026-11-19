//! Guard of Starlark source decoding (review 2026-11-18-actions round 2, N3; byte-strings review
//! 2026-11-20 S2-7).
//!
//! Starlark strings are internal strings (one char per byte) because every source decoder
//! produces its text with `string_encoding::parser_input_from_latin1` (Java
//! `ParserInput.fromLatin1`). A decoder that decoded by itself (UTF-8, say) would make every
//! non-ASCII string, action key and output wrong, and no unit test of either side would notice.
//! The bundled builtins are source text that reaches the interpreter without a decoder
//! (`include_str!`); they are right only while they are ASCII, which is checked here too.
//!
//! The decoders live in crates that depend on this one, so the test reads their source.

use std::path::{Path, PathBuf};

use blaze_util::string_encoding::parser_input_from_latin1;

/// Java `StarlarkUtil.createParserInput` and its callers' ports: every function that turns the
/// bytes of a Starlark source file into the interpreter's text.
const DECODERS: &[&str] = &[
    "crates/blaze_sky_loading/src/starlark_util.rs",
    "crates/blaze_bzlmod/src/module_file/compiled_module_file.rs",
];

const SHARED: &str = "string_encoding::parser_input_from_latin1";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The file without its comment lines (a comment that names a function is not a call).
fn code(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn rust_sources(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// Every `.rs` file under `crates/*/src`.
fn all_crate_sources() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for krate in std::fs::read_dir(repo().join("crates")).unwrap() {
        let src = krate.unwrap().path().join("src");
        if src.is_dir() {
            rust_sources(&src, &mut found);
        }
    }
    found.sort();
    found
}

#[test]
fn source_decoders_decode_with_parser_input_from_latin1() {
    for decoder in DECODERS {
        let code = code(&repo().join(decoder));
        assert!(
            code.contains("fn create_parser_input"),
            "{decoder} no longer defines create_parser_input: update DECODERS in this test"
        );
        assert!(
            code.contains(SHARED),
            "{decoder} does not decode Starlark sources with {SHARED}: Starlark strings would \
             not be internal strings (PORTING.md \"Strings\")"
        );
    }
}

/// No other file defines a source decoder that this guard does not know.
#[test]
fn no_unlisted_source_decoder() {
    let repo = repo();
    let mut found: Vec<String> = all_crate_sources()
        .iter()
        .filter(|p| code(p).contains("fn create_parser_input"))
        .map(|p| {
            let p = p.to_string_lossy().into_owned();
            p.split_once("/../../")
                .map(|(_, rest)| rest.to_owned())
                .unwrap_or(p)
        })
        .collect();
    found.sort();
    let mut want: Vec<String> = DECODERS.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(
        found,
        want,
        "source decoders (fn create_parser_input) in {}/crates/*/src",
        repo.display()
    );
}

/// Starlark files compiled into the binary with `include_str!` reach `eval::compile` as Rust
/// (Unicode) strings without going through a decoder. That equals Java's `fromLatin1` only for
/// ASCII text, so every such file must stay ASCII.
#[test]
fn bundled_starlark_sources_are_ascii() {
    let mut checked = 0;
    for source in all_crate_sources() {
        let text = std::fs::read_to_string(&source).unwrap();
        let mut rest = text.as_str();
        while let Some(i) = rest.find("include_str!(\"") {
            rest = &rest[i + "include_str!(\"".len()..];
            let Some(end) = rest.find('"') else { break };
            let rel = &rest[..end];
            if rel.ends_with(".bzl") || rel.ends_with(".scl") || rel.ends_with(".bazel") {
                let path = source.parent().unwrap().join(rel);
                let bytes =
                    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                assert!(
                    bytes.is_ascii(),
                    "{} (included by {}) is not ASCII: decode it with {SHARED} before compiling",
                    path.display(),
                    source.display()
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked > 0,
        "no bundled Starlark file found: update this test"
    );
}

/// The shared decoder is one char per byte.
#[test]
fn parser_input_from_latin1_is_one_char_per_byte() {
    assert_eq!(
        parser_input_from_latin1("x = \"é\"\n".as_bytes()),
        "x = \"\u{c3}\u{a9}\"\n"
    );
    assert_eq!(
        parser_input_from_latin1(b"x = \"\xff\"\n"),
        "x = \"\u{ff}\"\n"
    );
}
