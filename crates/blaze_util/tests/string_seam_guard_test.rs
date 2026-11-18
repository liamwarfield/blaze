//! Guard of the string migration seam (review 2026-11-18-actions round 2, N3): the Starlark
//! source decoders and `string_encoding::STRINGS_ARE_INTERNAL` must say the same thing.
//!
//! `Fingerprint::add_string` and the other seam functions assume that Starlark strings are
//! internal strings exactly when the constant is `true`. Whether they are is decided by how the
//! source files are decoded, in other crates. If one side is switched without the other, every
//! non-ASCII action key is encoded once too often or once too few, and no unit test of either
//! side notices. A decoder that calls `string_encoding::parser_input_from_latin1` cannot
//! disagree; one that decodes by itself must be the UTF-8 decoder and the constant `false`.
//!
//! The decoders live in crates that depend on this one, so the test reads their source.

use std::path::{Path, PathBuf};

use blaze_util::string_encoding::{
    STRINGS_ARE_INTERNAL, parser_input_from_latin1, with_strings_are_internal_for_testing,
};

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

// The assertion is on the constant on purpose: it changes when the switch is made.
#[allow(clippy::assertions_on_constants)]
#[test]
fn source_decoders_agree_with_strings_are_internal() {
    for decoder in DECODERS {
        let code = code(&repo().join(decoder));
        assert!(
            code.contains("fn create_parser_input"),
            "{decoder} no longer defines create_parser_input: update DECODERS in this test"
        );
        if code.contains(SHARED) {
            // Tied to the constant by construction.
            continue;
        }
        // Not on the seam: this is only right as the UTF-8 decoder, before the switch.
        assert!(
            !STRINGS_ARE_INTERNAL,
            "STRINGS_ARE_INTERNAL is true, but {decoder} does not decode with {SHARED}: \
             Starlark strings would not be internal strings and every non-ASCII action key \
             would be wrong. See \"The switch\" in blaze_util/src/string_encoding.rs."
        );
        assert!(
            code.contains("String::from_utf8(") && !code.contains("latin1"),
            "{decoder} does not decode sources as UTF-8 any more, but not with {SHARED} either, \
             while STRINGS_ARE_INTERNAL is false: Starlark strings and the fingerprint bridge \
             would disagree. Decode with {SHARED} and switch the constant instead. See \"The \
             switch\" in blaze_util/src/string_encoding.rs."
        );
    }
}

/// No other file defines a source decoder that this guard does not know.
#[test]
fn no_unlisted_source_decoder() {
    fn visit(dir: &Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, found);
            } else if path.extension().is_some_and(|e| e == "rs")
                && code(&path).contains("fn create_parser_input")
            {
                found.push(path.to_string_lossy().into_owned());
            }
        }
    }
    let repo = repo();
    let mut found = Vec::new();
    for krate in std::fs::read_dir(repo.join("crates")).unwrap() {
        let src = krate.unwrap().path().join("src");
        if src.is_dir() {
            visit(&src, &mut found);
        }
    }
    let mut found: Vec<String> = found
        .iter()
        .map(|p| {
            p.split_once("/../../")
                .map(|(_, rest)| rest.to_owned())
                .unwrap_or_else(|| p.clone())
        })
        .collect();
    found.sort();
    let mut want: Vec<String> = DECODERS.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(
        found, want,
        "source decoders (fn create_parser_input) in crates/*/src"
    );
}

/// The shared decoder follows the constant: UTF-8 text before the switch, one char per byte
/// after it.
#[test]
fn parser_input_from_latin1_follows_the_seam() {
    let source = "x = \"é\"\n".as_bytes();
    with_strings_are_internal_for_testing(false, || {
        assert_eq!(parser_input_from_latin1(source).unwrap(), "x = \"é\"\n");
        assert!(parser_input_from_latin1(b"x = \"\xff\"\n").is_err());
    });
    with_strings_are_internal_for_testing(true, || {
        assert_eq!(
            parser_input_from_latin1(source).unwrap(),
            "x = \"\u{c3}\u{a9}\"\n"
        );
        assert_eq!(
            parser_input_from_latin1(b"x = \"\xff\"\n").unwrap(),
            "x = \"\u{ff}\"\n"
        );
    });
}
