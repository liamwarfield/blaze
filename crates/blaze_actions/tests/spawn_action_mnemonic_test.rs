//! `SpawnAction.Builder.setMnemonic`'s check of the mnemonic, against the oracle
//! (review 2026-11-18-actions round 2, N2).
//!
//! `data/map_each_probe/workspace/cases.bzl` (`MNEMONIC_CASES`) has the mnemonics;
//! `data/map_each_probe/expected.txt` is what the oracle did with `ctx.actions.run(mnemonic =
//! ...)` for each (`scripts/actions-probe/run`): `OK`, or `Error in run: mnemonic must only
//! contain letters and/or digits, and have non-zero length, was: "..."`.
//!
//! Java tests each char of the string as Bazel holds it (one Latin-1 char per byte), so the
//! result depends on the bytes: `Qé` (c3 a9) is rejected, `Qê` (c3 aa) is accepted, and `"Q\351"`
//! (the byte e9, not UTF-8 text) is accepted. Blaze decodes the sources as Bazel does (Latin-1).

use std::sync::Arc;

use blaze_actions::analysis::actions::spawn_action::is_valid_mnemonic;
use blaze_actions::analysis::actions::starlark_action::starlark_action_builder;
use blaze_util::string_encoding::latin1_decode;

const CASES_BZL: &[u8] = include_bytes!("data/map_each_probe/workspace/cases.bzl");
const EXPECTED: &[u8] = include_bytes!("data/map_each_probe/expected.txt");

/// What Bazel's source decoder gives (`ParserInput.fromLatin1`).
fn decode(bytes: &[u8]) -> String {
    latin1_decode(bytes).into_owned()
}

/// `MNEMONIC_CASES` of cases.bzl, evaluated: (case, mnemonic).
fn mnemonic_cases() -> Vec<(String, String)> {
    // Only the dict: the rest of the file needs `ctx.actions` natives.
    let source = decode(CASES_BZL);
    let start = source.find("MNEMONIC_CASES = {").unwrap();
    let semantics = Arc::new(blaze_starlark::StarlarkSemantics::default());
    let globals = blaze_starlark::env::universe_builder(&semantics).build();
    let compiled = blaze_starlark::eval::compile(
        blaze_starlark::FileKind::Bzl,
        "mnemonics.bzl",
        source[start..].to_owned(),
    )
    .unwrap();
    let ctx = blaze_starlark::thread::ThreadContext::new(
        blaze_starlark::thread::Phase::BzlLoad,
        semantics,
        Arc::new(blaze_starlark::thread::CollectingEvents::default()),
    );
    let module = blaze_starlark::eval::exec_module(
        compiled,
        &globals,
        &blaze_starlark::loader::LoadMap::new(),
        &ctx,
        &blaze_starlark::eval::EvalOptions::default(),
    )
    .unwrap();
    let dict = module.get("MNEMONIC_CASES").unwrap();
    let dict = starlark::values::dict::DictRef::from_value(dict.value()).unwrap();
    dict.iter()
        .map(|(k, v)| {
            (
                k.unpack_str().unwrap().to_owned(),
                v.unpack_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// (case, `None` for OK or the oracle's message without `Error in run: `).
fn expected() -> Vec<(String, Option<String>)> {
    decode(EXPECTED)
        .lines()
        .filter(|line| line.starts_with("mnemonic_"))
        .map(|line| {
            let (case, result) = line.split_once('\t').unwrap();
            let error = (result != "OK").then(|| {
                result
                    .strip_prefix("Error in run: ")
                    .expect("an error of ctx.actions.run")
                    .to_owned()
            });
            (case.to_owned(), error)
        })
        .collect()
}

fn check() -> usize {
    let cases = mnemonic_cases();
    let expected = expected();
    assert_eq!(cases.len(), 9);
    assert_eq!(
        cases.iter().map(|c| &c.0).collect::<Vec<_>>(),
        expected.iter().map(|c| &c.0).collect::<Vec<_>>()
    );
    let mut checked = 0;
    for ((case, mnemonic), (_, error)) in cases.iter().zip(&expected) {
        checked += 1;
        assert_eq!(is_valid_mnemonic(mnemonic), error.is_none(), "{case}");
        // The builder: Java's IllegalArgumentException with the same message, which
        // StarlarkActionFactory rethrows as the EvalException the oracle printed.
        let mnemonic = mnemonic.clone();
        let result = std::panic::catch_unwind(move || {
            starlark_action_builder().set_mnemonic(mnemonic);
        });
        match (result, error) {
            (Ok(()), None) => {}
            (Err(panic), Some(error)) => {
                let message = panic.downcast_ref::<String>().unwrap();
                assert_eq!(
                    message,
                    &format!("IllegalArgumentException: {error}"),
                    "{case}"
                );
            }
            (result, error) => {
                panic!(
                    "{case}: oracle {error:?}, builder accepted: {}",
                    result.is_ok()
                )
            }
        }
    }
    checked
}

#[test]
fn mnemonic_check() {
    assert_eq!(check(), 9);
}
