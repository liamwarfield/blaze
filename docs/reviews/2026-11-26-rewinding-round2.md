# Review rewinding, round 2, with a first review of include scanning and `testing.analysis_test` (2026-11-26)

Commit under review: b216603 (checkpoint 43), read with `git show b216603:<path>`, not from the
working tree (which has uncommitted changes in `cpp_compile_action.rs` and the scanning context).
Reviewer: read-only, no Cargo. No Bazel or Blaze process was started: `root-suite-5f68157` was
active for the whole review. Everything here is from reading the Rust against the pinned Java.

Verdict: **STOP** (blocking: AT-1, one word of an error text). RW-1 to RW-4 are fixed. Include
scanning: no S1 found by reading.

## Scope

Read:

- Rewinding: `git diff dad60f0 b216603` of `skyframe_action_executor.rs`,
  `action_execution_function.rs`, `completion_function.rs`,
  `rewinding/action_rewind_strategy.rs`; the four `may_insensitively_propagate_inputs`
  overrides (by grep against Java's four overriders); `crates/blaze/tests/
  rewind_lost_inputs_test.rs` (whole file). Java: `ActionExecutionState.java:87-143`,
  `CompletionFunction.java:479`, `ActionExecutionMetadata.java:115`.
- Include scanning: `blaze_rules/src/includescanning/include_parser.rs:196-746` (parser, whole)
  against `IncludeParser.java:570-700, 742-993`; `legacy_include_scanner.rs` (whole) against
  `LegacyIncludeScanner.java:160-470, 640-993`; `cpp_include_scanning_context_impl.rs:60-330`
  against `CppIncludeScanningContextImpl.java` (whole); `IncludeScannerHelper` in
  `inclusion_with_context.rs` against `IncludeScannerHelper.java`; `mod.rs`; in
  `cpp_compile_action.rs`: `get_cmdline_includes`, `get_quote_include_dirs`,
  `get_include_dirs`, `get_system_include_dirs`, `find_used_headers`,
  `get_valid_undeclared_header_predicate`, `get_include_scanner_sources`, `discover_inputs`,
  the tail of `execute`, the builder's mandatory and prunable inputs, against
  `CppCompileAction.java:575-602, 663-760, 769-960, 1500-1610`;
  `CcCompilationContext.createIncludeScanningHeaderData` and `handleTreeArtifacts`.
- `testing.analysis_test`: `blaze_analysis/src/test/starlark_testing_module.rs:95-160`,
  `blaze_packages/src/starlark_defs/testing.rs:120-190`,
  `rule_class_functions.rs:1392-1545` against `rules/test/StarlarkTestingModule.java:60-180`
  and `Starlark.classType` (`Starlark.java:337-400`).

Not read: `IncludeScanningHeaderData` (Rust and Java), `InclusionWithContext`'s
`equals`/`hashCode`, `PathExistenceCache.java`, `locateRelative`/`isFile` on the Java side (Rust
only), `SkyframeInputMetadataProvider` reading a done value from the graph (only the weak
reference change in the diff), `ensure the metadata` path of a generated header on a remote
action file system, `create_rule` and `export_rule_function` for the analysis-test rule class,
the remote diffs (HTTP cache, chunked up/download), `build_driver_function.rs`,
`blaze_packages` glob changes, the nested-set walks of the rewind strategy, all tests except the
one named.

## Round-1 findings

| finding | state at b216603 |
|---|---|
| RW-1 (S1) | fixed |
| RW-2 (S1) | fixed by reading; no test with two copies (S3 below) |
| RW-3 (S2) | fixed |
| RW-4 (S2) | fixed |

- **RW-1.** `completion_function_exception_for_top_level_output` builds the exception with
  `Transience::Transient` (`completion_function.rs:1255`), as `CompletionFunction.java:479`; the
  two other constructors stay persistent. Test `top_level_output_exception_is_transient`.
- **RW-2.** The loop in `execute_action` is gone. A copy that sees the owner's `LostInputs`
  result waits for the rewind signal and returns `ExecutionFailure::RestartNode`
  (`skyframe_action_executor.rs:1602-1605`); `ActionExecutionFunction` turns that into
  `Reset::self_only(<the copy's key>)` (`action_execution_function.rs:570-578`), before the
  lost-inputs arm, and never into an error (`to_compute_error` panics on it). The restarted
  `compute` requests its deps again and reaches `execute_action` only when they are done, where
  the obsoleted state is no longer in the map. That is the sequence of
  `ActionExecutionState.getResultOrDependOnFuture` returning null after `obsolete` set the
  completion future. The copy can restart before the owner's `Reset` has dirtied the producers;
  Java has the same window (`obsolete` runs inside the owner's `compute`, before its `Reset` is
  returned), so this is not a deviation. Difference that stays, declared: a copy whose owner's
  rewinding fails reports the owner's error where Java's node stays in flight.
- **RW-3.** `Action::may_insensitively_propagate_inputs` (default `false`, `action.rs:629`),
  overridden by exactly Java's four classes; the strategy calls the method.
- **RW-4.** `RewindSignal::wait` and `wait_for_shared_result` register an interrupt listener and
  re-check under the lock they wait on, so no wake-up is lost; they end with
  `ExecutionFailure::Interrupted`. The helper thread of `wait_for_shared_result` blocks for ever
  if the owner panics before it sets the cell (a leaked thread, the copy itself is released by
  the interrupt).

## New findings

### S1

- **AT-1 (blocking). `analysis_test`: the wanted value type in the `attr_values` error is
  guessed.** Rust `crates/blaze_packages/src/starlark_defs/rule_class_functions.rs:1423` passes
  `"any"` as the value type name of `dict_cast(attr_values_api, "string", ..., "any", ...,
  "attr_values")`. Java `StarlarkTestingModule.java:82`
  `Dict.cast(attrValuesApi, String.class, Object.class, "attr_values")` prints
  `Starlark.classType(Object.class)`, which is `"unknown"` (`Starlark.java:385-390`).
  Input (a BUILD file): `testing.analysis_test(name = "t", implementation = impl,
  attr_values = {1: "x"})`. Bazel: `got dict<int, string> for 'attr_values', want
  dict<string, unknown>`; Blaze: `... want dict<string, any>`. Fix: `"unknown"`. The same
  class name is written `"Object"` at `starlark_defs/config.rs:350`
  (`analysis_test_transition(settings = {1: 2})`: Java `ConfigGlobalLibrary.java:127`
  `Dict.cast(changedSettings, String.class, Object.class, "changed_settings dict")`, so Bazel
  prints `want dict<string, unknown>`) and at `rule_class_functions.rs:1164` (a rule
  initializer that returns `{1: 2}`: Java `StarlarkRuleClassFunctions.java:1837`, same cast);
  fix those two `dict_cast` sites with it. The `sequence_cast(.., "Object", |_| true, ..)`
  sites (`aspect.rs`, `attr_module.rs`) can never print the name. Only `provider.rs:471` has it
  right.

### S2

- **RW-6. The end-to-end test does not show that a rewind happened.**
  `crates/blaze/tests/rewind_lost_inputs_test.rs` asserts exit code 0, "Build completed
  successfully, 2 total actions" and that the execution log has `//a:foo` on `remote` and
  `//a:bar` on `linux-sandbox`. A build that did not rewind but retried after the cache error
  ("Found transient remote cache error, retrying the build...",
  `--experimental_remote_cache_eviction_retries`, default 5) runs the same two spawns and can
  print the same summary line. The expected values are the oracle's, but the set of assertions
  does not separate the two paths. Fix: also assert that stderr has no "retrying the build"
  line, and one positive sign of the rewind taken from the oracle (the rewinding statistics of
  the build metrics event, or `--experimental_remote_cache_eviction_retries=0` on that build).
- **IS-1. The interrupt message of a scan is dropped.** `legacy_include_scanner.rs:577-586`:
  Java throws `InterruptedException("Include scanning interrupted while <operation> <source>")`;
  the Rust error carries no text. Declared in a `PORT` note; masked as long as nothing prints an
  `InterruptedException`'s message.
- **AT-2. Export errors of `analysis_test` are cut to the first.**
  `rule_class_functions.rs:1392-1396, 1512-1517`: Java joins every ERROR event of the export
  (`"Errors in exporting %s: %s"`, each message after a newline); Blaze stops at the first.
  Declared; shows with a rule class that has two export errors.

### S3

- No test runs a shared action with two nodes through a rewind (RW-2); `RewindingTest`'s 37
  cases stay ignored.
- Include scanning, `unported!` and marked: a tree artifact among the scanned sources or the
  declared headers (`expandTreeArtifacts`, `handleTreeArtifacts`), header modules with scanning
  (`findUsedModules`), a toolchain with a grep-includes tool (`SpawnIncludeScanner`), include
  hints (`Hints` rules; Bazel reads none), swig, `/showIncludes`,
  `getInputFilesForExtraAction` without an execution context, the include pool.

### S4

- `starlark_testing_module.rs:108-122` (the module in use) checks the BUILD thread before the
  parameter types; Java checks the types first. The W3 copy in `starlark_defs/testing.rs` has
  Java's order. Whether patch 0062's table reorders the two errors was not checked.

## Include scanning (first review): what was checked

- **Grammar.** The parser is Java's, byte for byte in what I compared: a line is split at `\n`
  with `\`-newline continuations joined (a file that ends in a backslash throws
  `ArrayIndexOutOfBoundsException` in Java and panics with that text here); leading whitespace
  is `Character.isWhitespace` for a byte (`\t`..`\r`, 0x1C..0x1F, space), `\`-newline and
  `/* */` comments, with Java's handling of an unterminated comment; `#`, then `include`,
  `import` or the first `__has_include` anywhere on the line with at least five bytes after it;
  `_next` after `include` and `__has_include` but not after `import`; `(` required for
  `__has_include`; `"..."` or `<...>`, where the search for the closing character starts one
  past the first name byte (so `""` is no inclusion) and a name that starts with `/` is dropped;
  `\`-newline pairs removed from the name; the name decoded as ISO-8859-1. A keyword that ends
  exactly at the end of the line is not found (`expect`). Line comments and raw strings are not
  recognised by Java and are not here: `// #include "x"` is no inclusion (the `#` is not first),
  an `#include` line inside a block comment that started on an earlier line or inside a raw
  string is an inclusion (a superset, which is safe). A computed include (`#include NAME`) is
  dropped by both (`createOtherInclusion` returns null).
- **Search order.** Relative to the including file first, for quote includes only, with the
  directory-existence shortcut for source includers; then the quote path list, which is the
  quote directories followed by the include directories, or the include list alone for angle
  includes; framework directories are tried at the index where the quote list turns into the
  include list (or at the `include_next` start if later), `Headers` then `PrivateHeaders`;
  `include_next` starts at the position after the one where the includer was found, and without
  context the helper finds that position by matching the includer's exec path.
  The lists: quote = context quote dirs + `-iquote` from copts (joined or separate, dangling
  flag warned on stderr); include = context include dirs + `-I`/`/I` from copts (joined form
  only, `-Imsvc...` skipped) + `-isystem` and `-imsvc` forms parsed from the full compiler
  options + the toolchain's built-in directories, deduplicated in that order; `-include x` and
  `-FI`/`/FI` are scanned first as quote inclusions relative to the main source.
- **Caches.** One supplier per execution phase (`executable_action_contexts`, from
  `execution_phase.rs:126`), so the parse cache, the path-existence cache and the scanners (one
  per triple of path lists, each with its inclusion cache) are shared by the actions of one
  command and gone with it, as Java's lifecycle manager. The inclusion cache's reuse rules
  (source artifact, or the same declared output; a not-found result only when no undeclared
  output was looked at) are Java's, including its documented wrong case (3).
- **A header outside the declared inputs.** An output-directory path counts as existing only if
  it is a declared header; an inclusion that resolves to an undeclared output or to a header
  that is not legal (`isLegalHeader`) is left out of the result without a message, as Java; an
  absolute path outside the built-in directories fails the action with "illegal absolute path
  to include file: <path>" (`ILLEGAL_ABSOLUTE_PATH`); I/O errors fail with "Include scanning
  IOException".
- **A scan cannot silently cover a missed header.** With a `.d` file, `execute` replaces the
  action's inputs by the `.d` file's list after the compile and runs `validate_inclusions` on
  it, exactly as Java (`CppCompileAction.java:1583-1610`), so a header the scan missed and the
  compiler found (computed include, a macro-built path) becomes an input of the action or fails
  as an undeclared inclusion. Where the compile runs without the scanned files present (sandbox,
  remote execution) a missed header fails the compile. Without a `.d` file there is no check
  after the compile in Java either; `getValidUndeclaredHeaderPredicate` is ported for that case.
  The mandatory inputs with scanning (toolchain files without includes, no declared include
  sources) and the prunable set (plus the compiler files) are the builder's Java.
- The result is sorted by exec path before it becomes a nested set, as Java.

## Verdict

**STOP**, bound to b216603. Blocking: AT-1 (`analysis_test`: `want dict<string, any>` where
Bazel prints `want dict<string, unknown>`; the same class name is `"Object"` in the
`analysis_test_transition` cast and in the rule-initializer cast). It is a one-word fix per
site; nothing else found blocks.
