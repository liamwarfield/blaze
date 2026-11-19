//! Port of src/test/java/com/google/devtools/build/lib/worker/WorkerParserTest.java.

mod worker_common;

use std::sync::Arc;

use blaze_exec::actions::execution_requirements::SUPPORTS_MULTIPLEX_SANDBOXING;
use blaze_exec::exec::bin_tools::BinTools;
use blaze_exec::exec::local::local_env_provider::NoopLocalEnvProvider;
use blaze_exec::worker::worker_parser::WorkerParser;
use blaze_optdefs::misc::worker_options::WorkerOptions;
use blaze_vfs::FileSystemExt;
use indexmap::IndexMap;
use worker_common::*;

#[test]
fn worker_key_computation_check() {
    let fs = fs();
    let cases = [
        // (multiplex, sandboxed, dynamic) -> (isMultiplex, isSandboxed, type name)
        ((false, false, false), (false, false, "worker")),
        ((true, false, false), (true, false, "multiplex-worker")),
        ((false, true, false), (false, true, "worker")),
        ((true, true, false), (true, false, "multiplex-worker")),
        ((false, false, true), (false, true, "worker")),
        ((true, false, true), (false, true, "worker")),
        ((false, true, true), (false, true, "worker")),
        ((true, true, true), (false, true, "worker")),
    ];
    for ((m, s, d), (multiplex, sandboxed, name)) in cases {
        let key = create_worker_key(&fs, m, s, d);
        assert_eq!(key.is_multiplex(), multiplex, "{m} {s} {d}");
        assert_eq!(key.is_sandboxed(), sandboxed, "{m} {s} {d}");
        assert_eq!(key.get_worker_type_name(), name, "{m} {s} {d}");
    }
}

#[test]
fn create_worker_key_understands_multiplex_sandboxing() {
    let fs = fs();
    let output_base = fs.get_path("/outputbase");
    let mut options = worker_options();
    options.multiplex_sandboxing = false;
    options.worker_multiplex = true;

    let key = create_worker_key_with_requirements(&output_base, &options, "Nom", false, &[]);
    assert!(key.is_multiplex());
    assert!(!key.is_sandboxed());
    assert_eq!(key.get_worker_type_name(), "multiplex-worker");

    let key = create_worker_key_with_requirements(&output_base, &options, "Nom", true, &[]);
    assert!(!key.is_multiplex());
    assert!(key.is_sandboxed());
    assert_eq!(key.get_worker_type_name(), "worker");

    let key = create_worker_key_with_requirements(
        &output_base,
        &options,
        "Nom",
        true,
        &[SUPPORTS_MULTIPLEX_SANDBOXING],
    );
    assert!(!key.is_multiplex());
    assert!(key.is_sandboxed());
    assert_eq!(key.get_worker_type_name(), "worker");

    options.multiplex_sandboxing = true;

    let key = create_worker_key_with_requirements(&output_base, &options, "Nom", false, &[]);
    assert!(key.is_multiplex());
    assert!(!key.is_sandboxed());
    assert_eq!(key.get_worker_type_name(), "multiplex-worker");

    let key = create_worker_key_with_requirements(
        &output_base,
        &options,
        "Nom",
        false,
        &[SUPPORTS_MULTIPLEX_SANDBOXING],
    );
    assert!(key.is_multiplex());
    assert!(key.is_sandboxed());
    assert_eq!(key.get_worker_type_name(), "multiplex-worker");

    let key = create_worker_key_with_requirements(
        &output_base,
        &options,
        "Nom",
        true,
        &[SUPPORTS_MULTIPLEX_SANDBOXING],
    );
    assert!(key.is_multiplex());
    assert!(key.is_sandboxed());
    assert_eq!(key.get_worker_type_name(), "multiplex-worker");
}

/// `new WorkerParser(null, options, null, null)`.
fn parser(options: WorkerOptions) -> WorkerParser {
    let fs = fs();
    WorkerParser::new(
        fs.get_path("/"),
        Arc::new(options),
        Arc::new(NoopLocalEnvProvider),
        Some(Arc::new(BinTools::for_unit_testing(
            &fs.get_path("/"),
            Vec::new(),
        ))),
    )
}

fn split(parser: &WorkerParser, args: &[&str]) -> (Vec<String>, Vec<String>) {
    let spawn = create_spawn(args, IndexMap::new());
    let mut flag_files = Vec::new();
    let args = parser
        .split_spawn_args_into_worker_args_and_flag_files(&spawn, &mut flag_files)
        .unwrap();
    (args, flag_files)
}

#[test]
fn split_spawn_args_into_worker_args_and_flag_files_splits_args_basic_case() {
    let mut options = worker_options();
    options.worker_extra_flags = Vec::new();
    let (args, flag_files) = split(&parser(options), &["--foo", "@bar"]);
    assert_eq!(args, ["--foo", "--persistent_worker"]);
    assert_eq!(flag_files, ["@bar"]);
}

#[test]
fn split_spawn_args_into_worker_args_and_flag_files_adds_extras() {
    let mut options = worker_options();
    options.worker_extra_flags = vec![
        ("Null".to_string(), "--qux".to_string()),
        (
            "Other action".to_string(),
            "--should_not_appear".to_string(),
        ),
        ("Null".to_string(), "--quxify".to_string()),
    ];
    let (args, flag_files) = split(&parser(options), &["--foo", "@bar"]);
    assert_eq!(args, ["--foo", "--persistent_worker", "--qux", "--quxify"]);
    assert_eq!(flag_files, ["@bar"]);
}

#[test]
fn split_spawn_args_into_worker_args_and_flag_files_adds_flag_files() {
    let mut options = worker_options();
    options.worker_extra_flags = Vec::new();
    options.strict_flagfiles = false;
    let (args, flag_files) = split(
        &parser(options),
        &[
            "--foo",
            "--flagfile=bar",
            "@@escaped",
            "@bar",
            "@bartoo",
            "--final",
        ],
    );
    assert_eq!(args, ["--foo", "--final", "--persistent_worker"]);
    // Yes, the legacy implementation allows multiple flagfiles and ignores escape sequences.
    assert_eq!(
        flag_files,
        ["--flagfile=bar", "@@escaped", "@bar", "@bartoo"]
    );
}

#[test]
fn split_spawn_args_into_worker_args_and_flag_files_adds_flag_files_strict() {
    let mut options = worker_options();
    options.worker_extra_flags = Vec::new();
    options.strict_flagfiles = true;
    let (args, flag_files) = split(&parser(options), &["--foo", "@@escaped", "--final", "@bar"]);
    assert_eq!(
        args,
        ["--foo", "@@escaped", "--final", "--persistent_worker"]
    );
    assert_eq!(flag_files, ["@bar"]);
}

fn assert_illegal_flags(message: &str, args: &[&str]) {
    let mut options = worker_options();
    options.worker_extra_flags = Vec::new();
    options.strict_flagfiles = true;
    let parser = parser(options);
    let spawn = create_spawn(args, IndexMap::new());
    let r = parser.split_spawn_args_into_worker_args_and_flag_files(&spawn, &mut Vec::new());
    assert!(
        matches!(r, Err(blaze_exec::actions::ExecException::User(_))),
        "{message}"
    );
}

#[test]
fn split_spawn_args_into_worker_args_and_flag_files_strict_flag_files() {
    assert_illegal_flags("Must have args", &[]);
    assert_illegal_flags("Must have a flagfile", &["--foo", "--final"]);
    assert_illegal_flags("Flagfile must be at the end", &["@earlyFile", "--final"]);
    assert_illegal_flags(
        "Only one flagfile allowed",
        &["@earlyFile", "--final", "@lateFile"],
    );
    assert_illegal_flags(
        "Only one flagfile allowed, regardless of syntax",
        &["--flagfile=foo", "--final", "@lateFile"],
    );
}
