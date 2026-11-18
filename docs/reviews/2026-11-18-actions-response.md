# Response to the actions review

Date 2026-11-18. Answers `2026-11-18-actions.md` (1 S1, 2 S2, 8 S3 groups, 4 S4, and the probes
outside the area). Written by the implementer who owns `blaze_actions`, `blaze_collect`,
`blaze_cc_features` and `blaze_util`.

Tree: HEAD `0015449` plus uncommitted changes in those four crates, `scripts/actions-probe/` (new),
`scripts/string-encoding/` (new) and this file. Nothing is committed. No file of another owner was
edited.

Verification, all on that tree:

- `scripts/cargo-capped test -p blaze_util -p blaze_collect -p blaze_actions -p blaze_cc_features`
  (the full suites): 445 passed, 0 failed, 68 ignored (blaze_actions 203/54, blaze_cc_features
  123/1, blaze_collect 100/13, blaze_util 19/0). 33 `unported!` call sites remain in
  `blaze_actions/src` and `blaze_cc_features/src`.
- `oracle_action_keys.rs`: 238 of 238 reconstructed oracle actions match (brotli 14, re2 224),
  0 mismatched. `golden_brotli_compile_test.rs`: 2 of 2, argv exact.
- `clippy --all-targets -D warnings` on the four crates: clean. `rustfmt --edition 2024`: clean.
- Dependents, because `Fingerprint::add_string` changed for everyone: full suites of
  `blaze_exec`, `blaze_repo_rules`, `blaze_build_config`, `blaze_packages`, `blaze_analysis`: 0
  failed. `blaze_rules` and the `blaze_sky_loading` tests did not compile at the time, from other
  owners' in-progress edits (`StarlarkOptionList`, `assert_contains_event`), so nothing above them
  was run, and **nothing was verified through the `blaze` binary**.

## Summary

| finding | state | where | test |
|---|---|---|---|
| S1-1 non-ASCII action keys | fixed for every string that is UTF-8 text, now; for every string after the decoding switch | `blaze_util::string_encoding`, `Fingerprint::add_string` | `non_ascii_action_keys.rs` (78 oracle actions) |
| S2-1 lossy `.d` paths | fixed | `blaze_cc_features/src/dependency_set.rs` | `dependency_set_test.rs` (3 new) |
| S2-2 non-global `map_each` accepted | fixed | `args.rs` `is_global_function` | `map_each_global_test.rs` (13 oracle cases) |
| S3-1 non-global `map_each` panics | **open**: needs a starlark-rust accessor (request 1) | `args.rs` `starlark_function_location` | `map_each_not_global_error_text`, ignored |
| S3-2 `write(content = args)` with a directory | this crate's half ported; the call is `blaze_exec`'s (request 2) | `parameter_file_write_action.rs` | `test_write_command_line_with_tree_artifact_expansion_default` |
| S3-6 template that is not UTF-8 | right after the switch; loud `unported!` before | `template.rs`, seam | `string_encoding_test.rs` |
| S3-3, 4, 5, 7 | deferred | below | |
| S3-8 | not this owner's (`blaze_rules`) | | |
| stale `#[ignore]` reasons | 22 reworded, 3 tests un-ignored | below | |
| the two unchecked oracle digests | re-derived, both right | `non_ascii_probe` | `non_ascii_action_keys.rs` |
| S4-2 probes only in comments | fixed for all eight digests | `tests/data/non_ascii_probe`, `scripts/actions-probe/run` | |
| S4-3 stand-ins in `blaze_cc_features` | `ParameterFileType` migrated; the rest needs one change with `blaze_rules` (request 3) | `blaze_cc_features/src/actions.rs` | |
| S4-1, S4-4 | deferred | below | |

## S1-1. One string convention for fingerprints

### What was decided and built

The convention is the string model of decisions.md 2026-11-18: **strings reaching a fingerprint
are internal-form** (one char `<= U+00FF` per byte), and `Fingerprint::add_string` writes the
string's UTF-8, which is then exactly Java's `writeStringNoTag` of the Latin-1 chars. There is one
string method. No call site chooses an encoding or converts.

Source decoding is not switched yet, so today those strings are still Unicode. To be right on both
sides without a second edit of every site, `blaze_util::string_encoding` has a **migration seam**:

- `STRINGS_ARE_INTERNAL: bool` (now `false`): whether Blaze's strings are internal strings yet.
- `to_internal(s)`: the internal string `s` stands for. `Fingerprint::add_string` and
  `Fingerprint::get_hex_digest` call it, and nothing else in the four crates does (one more use:
  Java's `PathFragment.hashCode` in `runfiles.rs`).
- `internal_string_bytes(s)` (`StringUnsafe.getInternalStringBytes`),
  `internal_string_from_bytes(b)` (`new String(b, ISO_8859_1)`), `internal_length(s)`
  (`String.length()`): for the places where Java takes the bytes or the length of an internal
  string.

Before the switch the seam converts (Unicode to internal for hashing, UTF-8 bytes for content);
after it each seam function is the plain function its documentation names. **The switch is one
edit: set `STRINGS_ARE_INTERNAL` to `true` in the same change that turns source decoding.** A test
can force either side per thread (`with_strings_are_internal_for_testing`), and the tests below
run both.

Step 1 of the migration plan is done: `blaze_util::string_encoding` ports `StringEncoding` and
the needed parts of `StringUnsafe` (`internal_to_unicode`, `unicode_to_internal`,
`internal_to_platform`, `platform_to_internal`, `latin1_decode`, `latin1_encode`, `is_internal`).
`StringEncodingTest` is ported (7 Java methods; the two platform tests keep only the Linux
Latin-1 branch, which is the one Blaze has, see the module comment). `internal_to_unicode` uses a
port of the JDK's replacing UTF-8 decoder, not `from_utf8_lossy`, because the two differ (an
encoded surrogate is one U+FFFD in Java, three in Rust); it is checked against the oracle's
embedded JDK 25 on 1,586,364 byte sequences (`scripts/string-encoding/run`).

### Sites

Every site the review lists hashes with `add_string` or a helper built on it. Audited against the
Java, each carries the invariant in a comment, and `actions/fingerprint_util.rs` states it for the
crate:

- `NestedSetFingerprintCache.addToFingerprint` (`nested_set_fingerprint_cache.rs`).
- `StarlarkCustomCommandLine`: the 13 sites (arg names, values, `format`, `format_each`,
  `before_each`, `join_with`, `format_joined`, `terminate_with`, labels, objects).
- `ActionEnvironment.addTo`, `SpawnAction` execution info, `ActionKeyComputer` exec properties.
- Not probed by the review, same rule: `platform_info.rs`, `runfiles.rs` prefix,
  `repo_mapping_manifest_action.rs`.
- The 22 former `add_internal_string` calls in `blaze_actions` (`CustomCommandLine`,
  `AbstractCommandLine`, `FileWriteAction`, `TemplateExpansionAction`, `UnresolvedSymlinkAction`,
  param file flag format, paths, `LazyWritePathsFileAction`) are now `add_string`. The hand-made
  encoder is gone; `Fingerprint::add_internal_string` remains only as an alias of `add_string`
  for the other crates' 23 callers.

Byte and length sites, now through the seam (so they follow the switch too): `FileWriteAction`
(compression threshold, gzip input, decompression, writer), `ParameterFile.writeContent`,
`ParameterFileWriteAction.getStringContents`, `LazyWritePathsFileAction`,
`LazyWriteNestedSetOfTupleAction`, `SourceManifestAction`, `RepoMappingManifestAction`,
`Template` (artifact content), `WorkspaceStatusAction.parseValues`, `DependencySet`,
`SolibSymlinkAction.maybeHashPreserveExtension` (name length and hash), and the argument lengths
behind the param-file threshold (`SimpleArgChunk.totalArgLength`, `CommandLines`,
`SingleStringArgFormatter.formattedLength`, `StarlarkCustomCommandLine`).

### Tests and fixtures

`crates/blaze_actions/tests/data/non_ascii_probe/workspace` is a Bazel workspace with one target
per placement: all `Args` parameters (26 cases in `cases.bzl`, including `map_each` on lists and
depsets, param files, files with non-ASCII paths, an emoji, a raw byte), environment key and
value, default shell env, execution info, exec properties, plain arguments, `run_shell`, file
writes (plain, executable, compressed, from `Args`), `expand_template`, `template_dict`,
unresolved symlinks. `aquery.jsonproto` next to it is the oracle's output, byte for byte,
recorded by `scripts/actions-probe/run` (oracle binary, `--output_user_root=
~/.cache/blaze/oracle-out/ur-actions-fix`, scratch copy in `~/.cache/blaze/work/actions-fix`).

`tests/non_ascii_action_keys.rs` rebuilds every recorded action from the same `cases.bzl` and
requires the oracle's `actionKey`:

| side | sources and recorded strings read as | result |
|---|---|---|
| after the switch | Latin-1 (internal strings) | 78 of 78 |
| before the switch (today) | UTF-8 (Unicode strings) | 74 of 74; 4 not buildable (below) |
| seam on the wrong side | Unicode hashed as if internal (the reviewed behaviour) | the non-ASCII probes differ |

Unit tests: `fingerprint.rs` (both sides, all five string methods), `string_encoding_test.rs`.

### End to end: what is right now, and what only after the decoding switch

Shown at the level of `blaze_actions` and its Starlark `Args` natives (the tests above), not
through the binary.

Right now, with no further change anywhere:

- All 15 placements the review found wrong (`args.add("é")` and the other `Args` parameters,
  `map_each` results, `env` keys and values, `write(content = args)`), and the 5 that were
  already right stay right. In general: every string that is valid UTF-8 text, at every
  `Fingerprint` string site in the workspace.
- That includes the `CppCompileAction` sites, with no edit in `blaze_rules`:
  `cpp_compile_action.rs:556` (compiler options), `:623` (feature environment), `:628` (execution
  info) call `add_strings` / `add_string_map`.
- File contents, param files and manifests are written with the same bytes as before.

Only after the switch (`STRINGS_ARE_INTERNAL = true` together with Latin-1 source decoding and the
platform boundaries of migration step 2):

- Strings that are not UTF-8 text: `"\377"` and other octal escapes above `\177`, slices that cut
  a character. Probes `raw_byte`, `plain_args_raw_byte`, `write_raw_byte`,
  `write_args_raw_byte`. Today Blaze has no string for them and their keys are wrong.
- File names and file contents that are not UTF-8: `.d` paths, artifact templates, workspace
  status files. Today these are a loud `unported!`, never a silently different string.
- Anything that depends on Starlark's own byte semantics (`len`, indexing), which is the
  interpreter's part of the migration.

### Risks and what other owners need to know

- `Fingerprint::add_string` now converts for every caller in the workspace (17 files outside
  these crates call it). That is right wherever Java holds an internal string, which is the
  normal case, and it fixes non-ASCII there too (configuration checksums, for example). It is
  wrong for a caller that holds real Unicode where Java does as well, or one that already holds
  an internal string today (`blaze_repo_rules` `latin1_internal_string` results). I found no test
  that depends on either; ASCII is unaffected.
- If the decoding is switched and `STRINGS_ARE_INTERNAL` is not, every non-ASCII key is encoded
  once too often. `non_ascii_action_keys.rs` does not catch that (it forces each side); an end to
  end run does: `scripts/actions-probe/run --binary <blaze> --out <file>` and compare the
  `actionKey`s with the recording.

## S2-1. `.d` paths keep their bytes

`dependency_set.rs` no longer uses `from_utf8_lossy`. Paths and the output file name go through
`internal_string_from_bytes`: after the switch every byte is kept (Java's
`new String(content, 0, w, ISO_8859_1)`); before it a path that is not UTF-8 is `unported!` with
the path in the message, as the review suggested. `write` uses `internal_string_bytes`. Tests:
`dot_d_parser_utf8_filename` (both sides), `dot_d_parser_non_utf8_filename_after_the_decoding_switch`
(`caf\xe9.h`, the review's input), `..._before_the_decoding_switch` (the loud failure).

## S2-2. `map_each` must be a global of its defining module

`is_global_function` is now `blaze_starlark::defining_module::is_global_of_defining_module`
(patch 0017), and `is_starlark_function` is "a `def` or `lambda`" (`DefModule::of_function`). The
PORT note that admitted the misclassification is gone.

`tests/data/map_each_probe/workspace/cases.bzl` has 13 cases (the review's `nested_top` probe as
`nested_add_all`, plus `add_joined`, `TemplateDict.add_joined`, lambdas at top level and at the
call, aliases, a builtin, `allow_closure`). `expected.txt` is what the oracle did with each
(recorded by `scripts/actions-probe/run`). `map_each_is_accepted_exactly_when_the_oracle_accepts_it`
passes: 7 accepted, 6 rejected, as on the oracle.

## S3

1. **Non-global `map_each` panics instead of Bazel's error: open.** The rejection is right (S2-2)
   and the error text is written (`map_each_not_global_error`), but it names where the function
   is declared (`.../cases.bzl:9:9`), and starlark-rust has no accessor for that position; it is
   in a crate I do not own, with uncommitted edits by its owner. Until request 1 lands a rejected
   `map_each` is still `unported!`. `map_each_not_global_error_text` holds the six oracle
   messages and is ignored with that reason. This was one of the two mandatory items and it is
   not done.
2. **`write(content = args)` with a directory: this crate's half is ported.**
   `ParameterFileWriteAction::new_deterministic_writer_with_input_metadata_provider` is Java's
   `newDeterministicWriter(ctx)` with `ctx.getInputMetadataProvider()`; the JUnit test
   `testWriteCommandLineWithTreeArtifactExpansionDefault` is ported and passes. The panic the
   review hit is raised by `blaze_exec` before it gets here (request 2), so end to end it still
   panics.
3. Path mapping (`--experimental_output_paths=strip`): deferred. `StrippingPathMapper` is a class
   of its own with its own tests; not a cheap port, and off by default.
4. Fileset expansion: deferred (filesets do not occur in the corpus). Float stringification:
   deferred; `StarlarkFloat.toString` needs Java's `%g` rules, and I did not verify that
   starlark-rust's float `str` equals them.
5. `CustomCommandLine.addFormatted` beyond `%s`, `%S`, `%%`, `%n`: deferred; it is
   `String.format`, used with other specifiers only by native rules that are not ported.
6. Template artifacts that are not UTF-8: read exactly after the switch (seam); `unported!`
   before.
7. `addExecutableArguments` after `setShellCommand`: deferred; no ported caller.
8. C++ items are in `blaze_rules`.

`#[ignore]` reasons: the 18 that said "needs action execution (wave W6)" now say that execution is
in `blaze_exec` and name the test there, or say that there is none; four others were corrected
(`FileType` exists only above this crate; `mapped_artifact` needs `PathMapper.storeIn`). Un-ignored
and passing: `test_mangled_path` (`Actions.escapedPath` was ported long ago), `simple_test_sha1`
and `concurrency_test_sha1` (`DigestHashFunction::Sha1` added to `blaze_util`).

The two unresolved-symlink digests the review could not check (`symlink_action_test.rs`) are
targets `symlink_unresolved_ascii` and `symlink_unresolved_dir` of the probe: the oracle gives
`5a7cb3bc…` and `d1966e8d…`, the hard-coded values. One fact came out of it: the key holds
`target_path` as written (`../some/./target`), while aquery prints it normalized.

## S4

1. `to_list` cache as a `Weak`: deferred. Results are the same; I did not measure the cost.
2. Fixed: all eight hard-coded oracle digests are now targets of the probe workspace, recorded by
   the script and checked from the recording; the older tests point to it.
3. See "blaze_cc_features" below.
4. Deferred. The boxed map fn is zero-sized, so `Box::leak` allocates nothing; the identity
   question (all such boxes share an address) stands and needs the cache to key on the type.

## blaze_cc_features on blaze_actions types

No cycle: `blaze_actions` depends on `blaze_util`, `blaze_collect`, `blaze_vfs`, `blaze_cmdline`,
`blaze_protos`, `blaze_events`, `blaze_shell`, `blaze_starlark`, and none of them reaches
`blaze_cc_features`. The dependency is added and `ParameterFileType` is now the `blaze_actions`
enum (same variants, so `blaze_rules` compiles unchanged).

`Artifact`, `PathMapper`, `InputMetadataProvider` and `ParamFileInfo` are still stand-ins. Not a
cycle problem: replacing them changes signatures that `blaze_rules` implements and calls
(`cc_artifact.rs` and four other files), which I may not edit, and doing only my half breaks the
workspace build. It has to be one change across both crates (request 3).

## Added on request from another crate

`blaze_analysis` (in progress, analysis review S1-3) imports
`blaze_actions::analysis::starlark::args::snapshot_args` and
`template_dict::snapshot_template_dict`, which did not exist, so that crate did not compile. Both
are added: a copy of the `Args` / `TemplateDict` state at the call, which is what Java reads when
an action is created. `tests/args_snapshot_test.rs` has the review's scenario (first action
`[x]`, second `[x, y]`). Nobody asked me for them directly; the analysis lead should confirm the
signature `(Value, Heap) -> Value` is what was meant.

## Requests for other owners

1. **starlark-rust (loading owner): a function's declaration position.** `DefInfo::signature_span`
   exists; expose it, for example `DefModule::def_location(function) -> Option<FileSpan>`, with
   Java's `StarlarkFunction.getLocation` meaning: the name of a `def`, the `lambda` keyword of a
   lambda. Expected values are in `tests/data/map_each_probe/expected.txt` (`9:9` for a nested
   def, `21:11` and `36:33` for lambdas). Then `args.rs` `starlark_function_location` is one line
   and the ignored test is enabled. `blaze_analysis` `validate_is_top_level_starlark_function`
   needs the same.
2. **blaze_exec: call the new writer.** In
   `analysis/actions/abstract_file_write_action.rs` `parameter_file_writer`, replace the
   `unported!` for actions with inputs by
   `action.new_deterministic_writer_with_input_metadata_provider(<ctx's provider>)`.
3. **blaze_rules (analysis lead): finish the stand-in removal with me.** `cc_artifact.rs`
   disappears once `blaze_cc_features` takes `blaze_actions::Artifact`; that also removes its
   `unported!` for tree artifacts in link variables (S3-8).
4. **blaze_rules: `cpp_compile_action.rs:1116`** `read_lines_as_latin1` uses
   `String::from_utf8_lossy`; use `string_encoding::internal_string_from_bytes`. Its key sites
   need nothing (above).
5. **Everyone with `add_internal_string`** (`blaze_exec` 5 calls, `blaze_repo_rules` 18): it is
   now the same as `add_string`; rename when convenient, then I delete the alias.
   `blaze_protos::java::{unicode_to_internal, decode_latin1}` duplicate `blaze_util::string_encoding`.
6. **Whoever switches source decoding:** set `string_encoding::STRINGS_ARE_INTERNAL` to `true` in
   the same change, then run `scripts/actions-probe/run --binary <blaze> --out <file>`.
7. **Root:** PORTING.md "Strings" still says `StringEncoding` is the identity; it should name the
   internal form, the seam and `blaze_util::string_encoding`.

## Files

- `crates/blaze_util`: `src/string_encoding.rs` (new), `src/fingerprint.rs`, `src/lib.rs`,
  `Cargo.toml` (`sha1`), `tests/string_encoding_test.rs` (new),
  `tests/data/java_utf8_decode_vectors.txt` (new, from the oracle's JDK).
- `crates/blaze_collect`: `nested_set_fingerprint_cache.rs`, `digest_map.rs`.
- `crates/blaze_actions/src`: 24 files (list in `git status`); the substantive ones are
  `actions/fingerprint_util.rs`, `analysis/starlark/args.rs`, `template_dict.rs`,
  `analysis/actions/file_write_action.rs`, `parameter_file_write_action.rs`.
- `crates/blaze_actions/tests`: `non_ascii_action_keys.rs`, `map_each_global_test.rs`,
  `args_snapshot_test.rs` (new); `data/non_ascii_probe/`, `data/map_each_probe/` (new);
  `oracle_common/mod.rs`; reasons and comments in seven older test files.
- `crates/blaze_cc_features`: `Cargo.toml`, `src/actions.rs`, `src/dependency_set.rs`,
  `tests/dependency_set_test.rs`.
- `scripts/actions-probe/run`, `scripts/string-encoding/{run,Utf8DecodeVectors.java}` (new).
- `Cargo.lock`: `blaze_util` gains `sha1`, `blaze_cc_features` gains `blaze_actions`.

---

# Round 2 response

Date 2026-11-18. Answers "Round 2" of `2026-11-18-actions.md` (PASS, bound to `f938830`): N1, N2,
N3 and S3-1.

Tree: HEAD `03409d9` plus uncommitted changes in `blaze_util`, `blaze_actions`,
`blaze_cc_features`, `third_party/starlark-rust` (patch 0024, authorized by root for S3-1),
`crates/starlark_conformance/tests/patches.rs` (its test), `scripts/actions-probe/run` and this
file. Nothing is committed.

Verification on that tree:

- Full suites of `blaze_util`, `blaze_collect`, `blaze_actions`, `blaze_cc_features`: 453 passed,
  0 failed, 67 ignored. `oracle_action_keys`: 238 of 238. `non_ascii_action_keys`: 78 of 78 after
  the switch, 74 of 74 before. clippy `-D warnings` and rustfmt clean on the four crates.
- `starlark_conformance` and `blaze_starlark` full suites (starlark-rust changed): 141 passed, 0
  failed. `cargo check --workspace`: clean.
- Through the binary (`target/debug/blaze` built from this tree, the 22 cases of the map_each
  probe, `build --nobuild` per case, compared with the oracle's `expected.txt`): 19 identical,
  among them all 13 `map_each` cases with the six error messages. The 3 that differ are the
  mnemonic cases explained under N2.

| finding | state | where | test |
|---|---|---|---|
| N1 aquery JSON off the seam | fixed | `aquery_output_handler.rs` `close` | `oracle_aquery_dump.rs`, 2 new |
| N2 non-ASCII mnemonic accepted | fixed in `blaze_actions`; one line left for `blaze_analysis` | `spawn_action.rs` `is_valid_mnemonic` | `spawn_action_mnemonic_test.rs`, 9 oracle cases |
| N3 constant not tied to the decoding | shared decoder function and a guard test; the decoders' own edit is two lines for their owners | `string_encoding::parser_input_from_latin1`, `tests/string_seam_guard_test.rs` | mutation-checked |
| S3-1 rejected `map_each` panics | fixed | starlark-rust patch 0024, `args.rs` | `map_each_not_global_error_text`, un-ignored |
| byte offset in `StringValueParser`'s error | fixed | `cc_toolchain_variables.rs` `java_position` | none new (ASCII tests unchanged) |

## N1. aquery JSON

`MonolithicOutputHandler::close` writes `internal_string_bytes(&json)`: Java's `PrintStream` in
ISO-8859-1, one byte per char of the internal strings. Today that is `as_bytes()`; after the
switch it is the Latin-1 encoding, so nothing is encoded twice.

`oracle_aquery_dump.rs` now replays the non-ASCII probe through `ActionGraphDump` and the handler
and compares all bytes with the recording:

- after the switch (recording read as Latin-1, internal strings): the oracle's file, byte for
  byte, 47882 bytes, including the `ff` of the raw-byte probes;
- before the switch (recording read as UTF-8): the oracle's file, byte for byte, except the stray
  bytes of the four probes that are not UTF-8 text, which have no string today.

The brotli and re2 replays still match.

## N2. Mnemonic

`is_valid_mnemonic` ports `!mnemonic.isEmpty() &&
CharMatcher.javaLetterOrDigit().matchesAllOf(mnemonic)` on the chars of the internal string
(`to_internal`), with Java's `Character.isLetterOrDigit` for Latin-1 written out (the letters of
categories Lu, Ll, Lo and ASCII digits; `²` is not a digit for Java).
`SpawnActionBuilder::set_mnemonic` uses it.

The result depends on the bytes, and the oracle confirms it. Nine cases were added to the probe
(`MNEMONIC_CASES` in `tests/data/map_each_probe/workspace/cases.bzl`, recorded in `expected.txt`):
`Qé` (c3 a9) rejected, `Qê` (c3 aa: `Ã`, `ª`) accepted, `Qµ` accepted, `Q²` rejected, `Q🐷`
rejected, `Q-x` and the empty string rejected, `Q1` accepted, `"Q\351"` (the byte e9) accepted.
`spawn_action_mnemonic_test.rs` checks all nine after the switch and eight before it (the last is
not UTF-8 text), including the message of the `IllegalArgumentException`.

**For the analysis lead, `blaze_analysis/src/starlark/starlark_action_factory.rs:549`**, replace
the condition line with:

```rust
    if !blaze_actions::analysis::actions::spawn_action::is_valid_mnemonic(mnemonic) {
```

Until that is in, the binary is wrong for `Qé` and `Q²` in a new way: the analysis copy accepts
them (`char::is_alphanumeric`), and the builder's precondition then fails as a crash (exit 101)
instead of `Error in run: mnemonic must only contain ...`. Before this round they were accepted
silently. With the line above the three remaining differences of the binary run reduce to
`mnemonic_raw_byte`, which needs the switch.

## N3. One switch

`string_encoding::parser_input_from_latin1(bytes)` is Java's `ParserInput.fromLatin1` on the
seam: UTF-8 text while `STRINGS_ARE_INTERNAL` is `false`, one char per byte when it is `true`. A
decoder that calls it reads the same constant as `Fingerprint::add_string`, so the two cannot be
switched separately.

The two decoders are not mine, so until they call it `blaze_util/tests/string_seam_guard_test.rs`
holds the line. It reads their source and fails when:

- the constant is `true` and a decoder does not call `parser_input_from_latin1`
  (checked: flipping the constant alone makes the test fail; the constant is back to `false`);
- the constant is `false` and a decoder neither calls it nor is the UTF-8 decoder any more
  (someone switched the decoding by hand);
- a `fn create_parser_input` exists in a file the guard does not list.

It is a source scan, so it is a tripwire, not a proof: the end-to-end probe of step 4 below stays
the real check. Requests 8 and 9 make the tie structural.

### Checklist for the switch

Also in the module comment of `blaze_util/src/string_encoding.rs`.

1. Before the switch, with no change of behaviour (each step is the identity while the constant
   is `false`):
   - `blaze_sky_loading/src/starlark_util.rs` and
     `blaze_bzlmod/src/module_file/compiled_module_file.rs`: `create_parser_input` gets its text
     from `string_encoding::parser_input_from_latin1(&bytes)`; `Err` keeps today's handling.
   - Every place where a string leaves or enters as bytes calls `internal_string_bytes` /
     `internal_string_from_bytes` instead of `as_bytes()` / `String::from_utf8*`: process argv,
     environment, working directory; file system paths and directory entries; console and event
     output; repository rule `read`, `execute`, `file`, `template`; command-line and rc intake;
     query, cquery and aquery writers. In my four crates this is done.
   - Every Java `String.length()` or index on an internal string calls `internal_length`.
   - Text that is really Unicode (protos, JSON, lockfile, registry) goes through
     `unicode_to_internal` / `internal_to_unicode` where Java calls `StringEncoding`.
2. The interpreter's part: starlark-rust escapes, case mapping, `strip`, `repr`, Blaze's
   `json.decode` (docs/research/07-byte-strings.md, section 4, option A).
3. Set `STRINGS_ARE_INTERNAL` to `true` in `blaze_util/src/string_encoding.rs`. Nothing else is
   part of the switch itself.
4. Check, in this order:
   - `scripts/cargo-capped test -p blaze_util -p blaze_actions` (the guard; the key and aquery
     tests run both sides);
   - `scripts/actions-probe/run --binary <blaze> --out <file>`, then `cmp <file>
     crates/blaze_actions/tests/data/non_ascii_probe/aquery.jsonproto`: all 78 actions byte for
     byte, the four `"\377"` probes included;
   - `scripts/bytes/run --binary <blaze>` against `~/.cache/blaze/work/bytes-out`;
   - the gate: brotli and re2 byte-identical.
5. Later: replace each seam function by the plain one its documentation names, delete
   `Fingerprint::add_internal_string` and the before-the-switch halves of the tests.

## S3-1. Bazel's error for a rejected `map_each`

starlark-rust patch **0024** (`third_party/starlark-rust/PATCHES.md`; the other agent's
immutable-set patch took 0025 after it): `DefInfo::declaration_span` and
`DefModule::declaration_span(function) -> Option<FileSpan>`, Java
`StarlarkFunction.getLocation`: the name of a `def`, the expression of a `lambda` (so the
`lambda` keyword). `signature_span` could not serve: for a lambda it is the span of the
parameters. Lines carry `BLAZE PATCH 0024`; `patches/blaze.diff` is regenerated (it now also
holds 0025 as it stood when I ran it); test `patch_0024_def_declaration_span` uses the probe's
`cases.bzl` and the oracle's positions (`9:9`, `21:11`).

`args.rs` builds the message from it. `map_each_not_global_error_text` is un-ignored and passes:
the six oracle lines, with `Error in add_all:` / `Error in add_joined:` and the position, for
`Args` and `TemplateDict`. No `unported!` is left on this path.

`blaze_analysis` `validate_is_top_level_starlark_function` (`resource_set`) can now do the same
with `DefModule::declaration_span` (request 10).

## Not changed in this round

R1 (the `"\377"` probes) needs the switch. S3-2 needs `blaze_exec` to call the writer (request
2). S3-3, 4, 5, 7, S4-1, S4-4 and the stand-ins of S4-3 stand as in round 1.

## Requests added in round 2

8. **blaze_sky_loading (loading owner):** in `starlark_util.rs` `create_parser_input`, take the
   text from `blaze_util::string_encoding::parser_input_from_latin1(&bytes)` (today: the same
   result as `String::from_utf8`).
9. **blaze_bzlmod:** the same in `module_file/compiled_module_file.rs`.
10. **blaze_analysis (analysis lead):** the mnemonic line above; and `resource_set`'s error with
    `DefModule::declaration_span`.
