//! End-to-end tests of what `repository_ctx` / `module_ctx` report: the text of failed context
//! methods, the order of `print()` output, `repository_ctx.attr`, and `--nofetch`.
//!
//! A repository rule (or module extension) in a scratch workspace runs through the real
//! REPOSITORY_DIRECTORY / REPO_DEFINITION / SINGLE_EXTENSION_EVAL functions. Every expected
//! string is derived from the Java at the pin (cited at each test) or from the recorded oracle
//! output (`oracle-9.3.2/re2/raw/query_deps_label_kind.stderr`: `Error in download_and_extract:
//! java.io.IOException: Error downloading [...`), never from running Blaze.

mod common;

use std::path::PathBuf;

use blaze_repo_rules::registration::RepoPrecomputedValues;
use common::Scratch;
use common::run;

/// The canonical name of the repo `my_rule(name = 'foo')` in the root module.
const FOO: &str = "+my_rule+foo";

/// A scratch workspace whose root module instantiates `my_rule(name = 'foo', <kwargs>)`, the
/// rule being `repository_rule(implementation = _impl, <rule_args>)` with the given body.
fn scratch_with_rule(impl_body: &str, rule_args: &str, kwargs: &str) -> Scratch {
    let s = Scratch::new();
    s.file(
        "MODULE.bazel",
        &format!(
            "my_rule = use_repo_rule('//:rule.bzl', 'my_rule')\nmy_rule(name = 'foo'{kwargs})\n"
        ),
    );
    s.file("BUILD", "");
    s.file(
        "rule.bzl",
        &format!(
            "def _impl(rctx):\n{impl_body}\n\
             my_rule = repository_rule(implementation = _impl{rule_args})\n"
        ),
    );
    s
}

/// `<output_base>/external/+my_rule+foo`.
fn repo_dir(s: &Scratch) -> PathBuf {
    s.output_base.join("external").join(FOO)
}

/// Fetches `@@+my_rule+foo`, which must fail, and returns the error of REPOSITORY_DIRECTORY: the
/// message of the `EvalException` the implementation function failed with (Java rethrows it as
/// `AlreadyReportedRepositoryAccessException(e)`, whose message is `e.getMessage()`).
fn fetch_error(s: &Scratch, extra_options: &[&str]) -> String {
    let ev = s.evaluator(extra_options);
    match s.fetch(&ev, FOO) {
        Ok(dir) => panic!(
            "expected the fetch to fail, got {}\n{:?}",
            dir.display(),
            s.events()
        ),
        Err(e) => e,
    }
}

/// Asserts that the fetch error event ends with `expected_last_line`, the last line of Java's
/// `EvalException.getMessageWithStack()` (`Error in <builtin>: <message>`).
fn assert_reported(s: &Scratch, expected_last_line: &str) {
    let events = s.events();
    assert!(
        events.iter().any(|e| e.ends_with(expected_last_line)),
        "no event ends with {expected_last_line:?}: {events:#?}"
    );
}

// ---------------------------------------------------------------------------------------------
// S1-5: RepositoryFunctionException(IOException) keeps the cause's class name.

/// Java: src/test/shell/bazel/starlark_repository_test.sh:1993
/// test_disable_download_should_prevent_downloading (adapted: `rctx.download` called directly
/// instead of through `http_archive`, and the whole message is checked).
///
/// `DownloadManager` throws `new IOException("Failed to download %s: download is disabled.")`
/// with the context `"repository " + repoName`; `completeDownload` rethrows it as
/// `RepositoryFunctionException(e, TRANSIENT)`, whose message is `e.toString()`.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:332 DownloadManager.downloadInExecutor
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:671 StarlarkBaseExternalContext.completeDownload
#[test]
fn test_disable_download_should_prevent_downloading() {
    run(|| {
        let s = scratch_with_rule(
            "  rctx.download(url = 'https://example.invalid/x.txt', output = 'x.txt')",
            "",
            "",
        );
        assert_eq!(
            fetch_error(&s, &["--repository_disable_download"]),
            "java.io.IOException: Failed to download repository @@+my_rule+foo: download is \
             disabled."
        );
        assert_reported(
            &s,
            "Error in download: java.io.IOException: Failed to download repository \
             @@+my_rule+foo: download is disabled.",
        );
    })
}

/// The same failure through `download_and_extract` (the oracle's re2 record has `Error in
/// download_and_extract: java.io.IOException: Error downloading [...`).
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1165 StarlarkBaseExternalContext.downloadAndExtract
#[test]
fn download_and_extract_failure_keeps_io_exception_class_name() {
    run(|| {
        let s = scratch_with_rule(
            "  rctx.download_and_extract(url = 'https://example.invalid/x.tar.gz')",
            "",
            "",
        );
        assert_eq!(
            fetch_error(&s, &["--repository_disable_download"]),
            "java.io.IOException: Failed to download repository @@+my_rule+foo: download is \
             disabled."
        );
        assert_reported(
            &s,
            "Error in download_and_extract: java.io.IOException: Failed to download repository \
             @@+my_rule+foo: download is disabled.",
        );
    })
}

/// `allow_fail = True` turns the same `IOException` into `struct(success = False)`.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:667 StarlarkBaseExternalContext.completeDownload
#[test]
fn download_failure_with_allow_fail_returns_success_false() {
    run(|| {
        let s = scratch_with_rule(
            "  r = rctx.download(url = 'https://example.invalid/x.txt', output = 'x.txt', \
             allow_fail = True)\n\
             \x20 rctx.file('BUILD', '')\n\
             \x20 rctx.file('result.txt', repr(r))",
            "",
            "",
        );
        let ev = s.evaluator(&["--repository_disable_download"]);
        let dir = s
            .fetch(&ev, FOO)
            .unwrap_or_else(|e| panic!("{e}\n{:?}", s.events()));
        assert_eq!(s.read(&dir.join("result.txt")), "struct(success = False)");
    })
}

/// `getUrls`: `new RepositoryFunctionException(new IOException("Bad URL: " + urlString, e),
/// PERSISTENT)` for a string `new URI(...)` rejects (a space in the path).
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:477 StarlarkBaseExternalContext.getUrls
#[test]
fn bad_url_keeps_io_exception_class_name() {
    run(|| {
        let s = scratch_with_rule(
            "  rctx.download(url = 'https://example.com/b c', output = 'x.txt')",
            "",
            "",
        );
        assert_eq!(
            fetch_error(&s, &[]),
            "java.io.IOException: Bad URL: https://example.com/b c"
        );
        assert_reported(
            &s,
            "Error in download: java.io.IOException: Bad URL: https://example.com/b c",
        );
    })
}

/// `getUrls`: `new IOException("urls not set")`.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:469 StarlarkBaseExternalContext.getUrls
#[test]
fn urls_not_set_keeps_io_exception_class_name() {
    run(|| {
        let s = scratch_with_rule("  rctx.download(url = [], output = 'x.txt')", "", "");
        assert_eq!(fetch_error(&s, &[]), "java.io.IOException: urls not set");
    })
}

/// `getUrls`: `new IOException("Unsupported protocol: " + url.getScheme())`.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:481 StarlarkBaseExternalContext.getUrls
#[test]
fn unsupported_protocol_keeps_io_exception_class_name() {
    run(|| {
        let s = scratch_with_rule(
            "  rctx.download_and_extract(url = 'ftp://example.com/x.tar.gz')",
            "",
            "",
        );
        assert_eq!(
            fetch_error(&s, &[]),
            "java.io.IOException: Unsupported protocol: ftp"
        );
    })
}

/// `getUrls`: plain http URLs are dropped when no checksum is given, and nothing is left.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:494 StarlarkBaseExternalContext.getUrls
#[test]
fn no_urls_left_keeps_io_exception_class_name() {
    run(|| {
        let s = scratch_with_rule(
            "  rctx.download(url = 'http://example.com/x.txt', output = 'x.txt')",
            "",
            "",
        );
        assert_eq!(
            fetch_error(&s, &[]),
            "java.io.IOException: No URLs left after removing plain http URLs due to missing \
             checksum. Please provide either a checksum or an https download location."
        );
    })
}

/// `createFile`: `makeDirectories` fails because the parent of `a/b` is the file `a`.
/// `UnixFileSystem.createDirectoryAndParents` throws `new IOException(path + " (File exists)")`,
/// and `createFile` rethrows it as `RepositoryFunctionException(e, TRANSIENT)`.
// Java: com/google/devtools/build/lib/unix/UnixFileSystem.java:288 UnixFileSystem.createDirectoryAndParents
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1522 StarlarkBaseExternalContext.createFile
#[test]
fn file_failure_keeps_io_exception_class_name() {
    run(|| {
        let s = scratch_with_rule("  rctx.file('a', 'x')\n  rctx.file('a/b', 'y')", "", "");
        let expected = format!(
            "java.io.IOException: {}/a (File exists)",
            repo_dir(&s).display()
        );
        assert_eq!(fetch_error(&s, &[]), expected);
        assert_reported(&s, &format!("Error in file: {expected}"));
    })
}

/// `checkInOutputDirectory` wraps an `EvalException` (`Starlark.errorf`), whose `toString()` is
/// its message: no class name.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:334 StarlarkBaseExternalContext.checkInOutputDirectory
#[test]
fn file_outside_of_the_repository_directory_has_no_class_name() {
    run(|| {
        let s = scratch_with_rule("  rctx.file('../outside.txt', 'x')", "", "");
        assert_eq!(
            fetch_error(&s, &[]),
            format!(
                "Cannot write outside of the repository directory for path {}",
                s.output_base.join("external/outside.txt").display()
            )
        );
    })
}

/// `readFile`: `FileSystemUtils.readContent` opens a `FileInputStream`, which throws
/// `FileNotFoundException("<path> (No such file or directory)")`; `readFile` rethrows it as
/// `RepositoryFunctionException(e, TRANSIENT)`, so the subclass name is printed.
// Java: com/google/devtools/build/lib/vfs/DiskBackedFileSystem.java:64 DiskBackedFileSystem.getInputStream
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1645 StarlarkBaseExternalContext.readFile
#[test]
fn read_failure_keeps_file_not_found_exception_class_name() {
    run(|| {
        let s = scratch_with_rule("  rctx.read('missing.txt')", "", "");
        let expected = format!(
            "java.io.FileNotFoundException: {}/missing.txt (No such file or directory)",
            repo_dir(&s).display()
        );
        assert_eq!(fetch_error(&s, &[]), expected);
        assert_reported(&s, &format!("Error in read: {expected}"));
    })
}

// ---------------------------------------------------------------------------------------------
// S1-6: --nofetch.

/// Java: src/test/shell/bazel/starlark_git_repository_test.sh:548 test_git_repository_and_nofetch
/// (adapted: a local rule instead of `git_repository`, and the whole message is checked).
///
/// With fetching disabled and no repo on disk, `computeInternal` throws `new IOException("to
/// fix, run\n\tbazel fetch //...\nExternal repository " + repositoryName + " not found and
/// fetching repositories is disabled.")`; REPOSITORY_DIRECTORY fails with that `IOException`.
// Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:399 RepositoryFetchFunction.computeInternal
#[test]
fn test_repository_and_nofetch() {
    run(|| {
        let s = scratch_with_rule("  rctx.file('BUILD', '')", "", "");
        let ev = s.evaluator_with_precomputed(
            &[],
            RepoPrecomputedValues {
                fetch_disabled: true,
                ..RepoPrecomputedValues::default()
            },
        );
        assert_eq!(
            s.fetch(&ev, FOO),
            Err(
                "to fix, run\n\tbazel fetch //...\nExternal repository @@+my_rule+foo not found \
                 and fetching repositories is disabled."
                    .to_owned()
            )
        );
        assert!(!repo_dir(&s).exists(), "nothing is fetched with --nofetch");
    })
}

// ---------------------------------------------------------------------------------------------
// S2-9: print() is reported when it runs.

/// What the handler saw of `print('<prefix>before')`, `report_progress('<prefix>mid')` and
/// `print('<prefix>after')`, in order.
fn print_and_progress_order(s: &Scratch, prefix: &str) -> Vec<&'static str> {
    s.events()
        .iter()
        .filter_map(|e| {
            if e.starts_with("POST: ") {
                e.contains(&format!("\"{prefix}mid\""))
                    .then_some("progress")
            } else if e.ends_with(&format!(": {prefix}before")) {
                Some("print before")
            } else if e.ends_with(&format!(": {prefix}after")) {
                Some("print after")
            } else {
                None
            }
        })
        .collect()
}

/// The thread's print handler is `Event.makeDebugPrintHandler(env.getListener())`: each `print`
/// is a DEBUG event handed to the environment's listener at once, and DEBUG events are not
/// stored for replay (`Event.storeForReplay`), so the reporter sees them before whatever the
/// implementation reports next. `report_progress` posts its `FetchProgress` to the same listener.
// Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:656 RepositoryFetchFunction.fetch
// Java: com/google/devtools/build/lib/events/Event.java:425 Event.makeDebugPrintHandler
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1807 StarlarkBaseExternalContext.reportProgress
#[test]
fn repository_rule_print_is_reported_when_it_runs() {
    run(|| {
        let s = scratch_with_rule(
            "  print('rule before')\n\
             \x20 rctx.report_progress('rule mid')\n\
             \x20 print('rule after')\n\
             \x20 rctx.file('BUILD', '')",
            "",
            "",
        );
        let ev = s.evaluator(&[]);
        s.fetch(&ev, FOO)
            .unwrap_or_else(|e| panic!("{e}\n{:?}", s.events()));
        assert_eq!(
            print_and_progress_order(&s, "rule "),
            ["print before", "progress", "print after"],
            "{:#?}",
            s.events()
        );
    })
}

/// The same for a module extension's thread.
// Java: com/google/devtools/build/lib/bazel/bzlmod/RegularRunnableExtension.java:290 RegularRunnableExtension.runInternal
#[test]
fn module_extension_print_is_reported_when_it_runs() {
    run(|| {
        let s = Scratch::new();
        s.file(
            "MODULE.bazel",
            "ext = use_extension('//:defs.bzl', 'ext')\nuse_repo(ext, 'foo')\n",
        );
        s.file("BUILD", "");
        s.file(
            "defs.bzl",
            "def _repo_impl(rctx):\n\
             \x20 rctx.file('BUILD', '')\n\
             repo = repository_rule(implementation = _repo_impl)\n\
             def _ext_impl(mctx):\n\
             \x20 print('ext before')\n\
             \x20 mctx.report_progress('ext mid')\n\
             \x20 print('ext after')\n\
             \x20 repo(name = 'foo')\n\
             ext = module_extension(implementation = _ext_impl)\n",
        );
        let ev = s.evaluator(&[]);
        s.fetch(&ev, "+ext+foo")
            .unwrap_or_else(|e| panic!("{e}\n{:?}", s.events()));
        assert_eq!(
            print_and_progress_order(&s, "ext "),
            ["print before", "progress", "print after"],
            "{:#?}",
            s.events()
        );
    })
}

// ---------------------------------------------------------------------------------------------
// S2-11: repository_ctx.attr.

/// `repository_ctx.attr` is the `RepoDefinition` record, a `Structure` that is not a struct:
///
/// - `repr` / `str`: `StarlarkValue.repr`'s default, `<unknown object <class name>>`;
/// - `type`: `Starlark.classType`, which ends at `Class.getSimpleName()`;
/// - `dir`: the sorted field names (`attributeIndices` keys and `name`);
/// - `hasattr`: `getValue(name) != null`; there are no `to_json` / `to_proto` methods;
/// - `getattr` with a default for an unknown field gives the default;
/// - the record's `equals` makes two reads of `rctx.attr` equal;
/// - `json.encode` treats a `Structure` as an object of its `dir()` fields.
// Java: com/google/devtools/build/lib/bazel/repository/RepoDefinition.java:31 RepoDefinition
// Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr
// Java: net/starlark/java/eval/Starlark.java:337 Starlark.classType
// Java: net/starlark/java/eval/Starlark.java:967 Starlark.hasattr
// Java: net/starlark/java/eval/Starlark.java:1024 Starlark.dir
// Java: net/starlark/java/lib/json/Json.java:221 Json.Encoder.encode
#[test]
fn repository_ctx_attr_is_the_repo_definition_structure() {
    run(|| {
        let s = scratch_with_rule(
            "  a = rctx.attr\n\
             \x20 print(a)\n\
             \x20 rctx.file('BUILD', '')\n\
             \x20 rctx.file('attr.txt', '\\n'.join([\n\
             \x20   repr(a),\n\
             \x20   str(a),\n\
             \x20   type(a),\n\
             \x20   repr(dir(a)),\n\
             \x20   repr([hasattr(a, n) for n in ['greeting', 'salutation', 'name', 'nope', 'to_json', 'to_proto']]),\n\
             \x20   repr(getattr(a, 'nope', 'dflt')),\n\
             \x20   repr([a.name, a.greeting, a.salutation]),\n\
             \x20   repr(a == rctx.attr),\n\
             \x20   json.encode(a),\n\
             \x20 ]))",
            ", attrs = {'greeting': attr.string(), 'salutation': attr.string(default = 'dear')}",
            ", greeting = 'hello'",
        );
        let ev = s.evaluator(&[]);
        let dir = s
            .fetch(&ev, FOO)
            .unwrap_or_else(|e| panic!("{e}\n{:?}", s.events()));
        let unknown_object =
            "<unknown object com.google.devtools.build.lib.bazel.repository.RepoDefinition>";
        assert_eq!(
            s.read(&dir.join("attr.txt")).lines().collect::<Vec<_>>(),
            [
                unknown_object,
                unknown_object,
                "RepoDefinition",
                r#"["greeting", "name", "salutation"]"#,
                "[True, True, True, False, False, False]",
                r#""dflt""#,
                r#"["+my_rule+foo", "hello", "dear"]"#,
                "True",
                r#"{"greeting":"hello","name":"+my_rule+foo","salutation":"dear"}"#,
            ]
        );
        // print(): StarlarkValue.debugPrint defaults to str.
        let events = s.events();
        assert!(
            events
                .iter()
                .any(|e| e.ends_with(&format!(": {unknown_object}"))),
            "{events:#?}"
        );
    })
}

/// A missing field fails with `RepoDefinition.getErrorMessageForUnknownField`: `"unknown
/// attribute " + field + SpellChecker.didYouMean(field, getFieldNames())`.
// Java: com/google/devtools/build/lib/bazel/repository/RepoDefinition.java:65 RepoDefinition.getErrorMessageForUnknownField
// Java: net/starlark/java/spelling/SpellChecker.java:101 SpellChecker.didYouMean
#[test]
fn repository_ctx_attr_unknown_attribute_message() {
    run(|| {
        let s = scratch_with_rule(
            "  x = rctx.attr.greting",
            ", attrs = {'greeting': attr.string()}",
            ", greeting = 'hello'",
        );
        assert_eq!(
            fetch_error(&s, &[]),
            "unknown attribute greting (did you mean 'greeting'?)"
        );
    })
}

/// `to_json` is a method of `StructImpl`, which `RepoDefinition` is not: it is one more unknown
/// field (no spelling suggestion: no field name is within the edit distance).
// Java: com/google/devtools/build/lib/bazel/repository/RepoDefinition.java:65 RepoDefinition.getErrorMessageForUnknownField
#[test]
fn repository_ctx_attr_has_no_to_json() {
    run(|| {
        let s = scratch_with_rule(
            "  x = rctx.attr.to_json()",
            ", attrs = {'greeting': attr.string()}",
            ", greeting = 'hello'",
        );
        assert_eq!(fetch_error(&s, &[]), "unknown attribute to_json");
    })
}
