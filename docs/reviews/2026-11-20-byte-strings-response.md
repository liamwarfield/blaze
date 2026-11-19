# Response: byte-strings review (2026-11-20)

Answers `docs/reviews/2026-11-20-byte-strings.md` (verdict STOP at `2574fef`).

## Round 1 response

Fixer: Opus 5.5. The tree is root's checkpoint `04519bf`, which contains every change below
(the fixer's files are unchanged since that commit). Numbers come from that working tree:
`scripts/cargo-capped test` of the 15 changed crates; `scripts/bytes` with
`target/debug/blaze` built from it (12:32, after the last source edit).

Every Java line was read at the pin. Where the review's reading and the oracle disagree, the
oracle wins (section 3, tar).

### S1

**S1-1, fixed.** `crates/blaze_options/src/rc/rc_file.rs:39`: `Path::new(&*internal_to_platform(..)).exists()`
(C++ `workspace_layout.cc:80` `PathExists` = `access(F_OK)` on the raw bytes).
Tests: `rc_file_test::workspace_relative_import_resolves_in_non_ascii_workspace`; end to end,
`scripts/bytes` `rc_import.txt` (a workspace at `~/.cache/blaze/work/bytes_rc_é` whose
`.bazelrc` does `import %workspace%/a.bazelrc` then `try-import %workspace%/b.bazelrc`; the
oracle prints the `--output=build` form, so both resolved).

**S1-2, fixed.** `crates/blaze_shell/src/util/command_failure_utils.rs:67`: `chars().count()` on
both operands (`CommandFailureUtils.java:105-106`). The stale PORT comment is gone.
Tests: `command_failure_utils_test::describe_command_failure_counts_chars_of_internal_strings`;
end to end, `scripts/bytes` `failmsg:fail.stderr:1` (genrule `//fail:long`: 133-char `bash -c`
argument with 30 `é`, 193 UTF-8 bytes; the oracle prints the whole command).

**S1-3, fixed.** The repr of all four providers now uses `blaze_starlark::printer::quote`
(`Printer.java:225-260`):
`crates/blaze_analysis/src/platform/template_variable_info.rs:87`,
`crates/blaze_analysis/src/test/execution_info.rs:75`,
`crates/blaze_analysis/src/providers/run_environment_info.rs:80`,
`crates/blaze_rules/src/config/config_feature_flag_provider.rs:113`. The prefix question from
section 3 was settled on the oracle: all four print `struct(` with the fields sorted
(`NativeInfo.java:125`), so `ExecutionInfo(` and `RunEnvironmentInfo(` were wrong and now print
`struct(`. Oracle output, bytes as they are:
`struct(exec_group = "test", requirements = {"K": "<C3 A0 F0 9F 90 B7 FF>"})`.
Tests: one `byte_strings_tests` module in each of the four files. End to end: `scripts/bytes`
package `//providers`, the 6 `CASE prov_*` lines of `providers.stderr`, including `str()` and a
real `config_feature_flag`.

**S1-4, fixed and confirmed by a run.** `crates/blaze_repo_fetch/src/downloader/socket_transport.rs:521`
`proxy_authorization` (used only for the CONNECT tunnel) now encodes
`latin1_encode_lossy(internal_to_unicode(user) + ":" + internal_to_unicode(password))`
(`ProxyHelper.java:330-333`). I ran the oracle's own JDK (Zulu 25.0.2,
`-Dfile.encoding=ISO-8859-1`) against a fake proxy: after a 407 it sends
`Proxy-authorization: Basic 6Tpwdw==` for user `é`, password `pw`. That is the review's value.
Test: `socket_transport::tests::connect_tunnel_credentials_are_latin1_of_the_unicode_string`
(`é` gives `6Tpwdw==`; `中` gives `Pzpwdw==`; ASCII is unchanged). Not end to end: no proxy in
scripts/bytes.
*Seen in passing, left as is:* the JDK sends the CONNECT without credentials first and adds them
only after a 407 with a `Basic` challenge (and would use UTF-8 for `charset="UTF-8"`). It also
writes `User-Agent`, `Host` without the port, `Accept` and `Proxy-Connection`. Blaze sends the
credentials pre-emptively in a shorter request. This is an older emulation choice in this
module, not a string defect.

**S1-5, fixed.** `crates/blaze_repo_fetch/src/downloader/url_rewriter.rs:431`:
`latin1_encode_lossy(&user_info)` (`UrlRewriter.java:178`; the config is read as UTF-8 at :125,
so the userinfo is Unicode). Test: `url_rewriter_test::rewritten_url_user_info_is_encoded_as_latin1`
(`中:pw` gives `Basic Pzpwdw==`; `é:pw` gives `Basic 6Tpwdw==`).

**S1-6, fixed.** `crates/blaze_bzlmod_model/src/gson.rs`. `\u` now takes four UTF-16 units and
each must be an ASCII hex digit (`hex4`, :93). A short input is "Unterminated escape sequence",
anything else is "Malformed Unicode escape \u<4 units>". The surrogate look-ahead uses the same
check. A leading U+FEFF is skipped before the reader starts (:72), so columns count from after
it, as Gson's `lineStart++` does.
Tests: `crates/blaze_bzlmod_model/tests/gson_reader_test.rs` (7 tests). Expected messages come
from Gson 2.11.0 inside the oracle's `A-server.jar`, run on its JDK: `\uabc€` at column 4,
`\u+123`, `\uab"}` at column 10 with path `$.a`, unterminated at column 5 with path `$[0]`, a
valid escape, one BOM accepted, two BOMs an error.
*Left as is:* Gson 2.11 appends `\nSee https://github.com/google/gson/blob/main/Troubleshooting.md#malformed-json`
to MalformedJsonException messages. The reader never did; this predates the migration (the
`PORT:` note on `parse_lenient` covers message differences).

### S2

**S2-1, fixed, with one part left for the interpreter owner.**
`eval.set_utf8_byte_strings(thread.semantics.utf8_byte_strings())` is set at
`starlark_rule_configured_target_util.rs:196`, `starlark_aspect_factory.rs:107` and
`compiled_module_file.rs:199`.
The flag could not be turned off at all: `crates/blaze_build_config/src/top_level.rs` had an
`unported!` guard from before the migration that made `--nointernal_starlark_utf_8_byte_strings`
fail. The review did not mention it. Java reads the key only in `StringModule.java:332` and
`Json.java:353`, and `FactsAdapter.java:41` fixes it to true. Both readers now take the
evaluator flag at every evaluator, so I removed the guard and left a PORT note (:76). The
Blaze-only test `unread_key_utf_8_byte_strings_is_loud` became `key_utf_8_byte_strings_is_read`
(`build_language_options_semantics_test.rs`).
End to end: `scripts/bytes` workspace `side/flagoff`, run with
`--nointernal_starlark_utf_8_byte_strings --aspects=//:defs.bzl%a`. It compares `strip()` in
MODULE.bazel, a .bzl, a BUILD file, a rule implementation and an aspect implementation with the
oracle (`"a"`). All 5 match.
**Open:** `json.decode('"\\ud83d\\ude00"')` with the flag off has length 2 in Bazel (a UTF-16
pair) and 1 in Blaze (`crates/blaze_starlark/src/json.rs:474`, outside my files). This is the
case `flagoff_rule_json` and the one remaining scripts/bytes difference. It is also noted in
`top_level.rs`.

**S2-2, fixed.** `crates/blaze_vfs/src/path_fragment.rs:382`: both paths go through
`internal_to_unicode`. Lengths and the separator index are in UTF-16 units; the comparison is
over the chars of the same prefix (`PathFragment.java:442-453`).
`tests/path_fragment_test.rs::test_starts_with_ignoring_case` now wraps the Java test's literals
in `unicode_to_internal` as `PathFragmentTest.java:475-480` does; the `starts_with` lines at
:435-437 got the same wrap. It adds the review's two cases: `/inc/é` against `/inc/É` is true,
and E3 A9 against C3 A9 is false.

**S2-3, left open.** `crates/blaze_protos/src/text_format.rs` is being rewritten by another
implementer, and the task excludes it.

**S2-4, fixed.** New `blaze_util::string_encoding::jni_latin1_chars` (:129): one byte per
UTF-16 unit, `?` for each unit above U+00FF (`latin1_jni_path.cc:121-124`, the JDK's
`getString8859_1Chars`). It is used at `fs_utils.rs:233`, `http_connector.rs:276` and
`blaze_bzlmod_model/src/registry.rs:153`. `download_manager.rs:506 replace_fs_unsafe_chars`
now writes one `_` per UTF-16 unit (Guava `CharMatcher`, `DownloadManager.java:515-535`).
Tests: `string_encoding_test::jni_latin1_chars_is_per_utf16_unit`,
`fs_utils::byte_strings_tests::unicode_path_as_seen_by_file_system_is_per_utf16_unit` (`🐷.txt`
becomes `??.txt`), `download_manager::byte_strings_tests::replace_fs_unsafe_chars_is_per_utf16_unit`.
blaze_vfs `os()` was already right; I did not change it.

**S2-5, fixed.** `crates/blaze_actions/src/bazel/bazel_workspace_status_module.rs:207`: new key
type `JavaStringKey` ordered by `encode_utf16` (Java `String.compareTo`). The stable and
volatile maps and `parse_workspace_status` use it. `overall_map` keeps `BTreeMap<String, _>`
because its consumer, `BuildInfoEvent` in blaze_exec, is not my file; a PORT note there says so.
The file contents, which are what reaches output, are now in Java order.
Test: `parse_workspace_status_orders_by_utf16_units` (`STABLE_🐷` sorts before `STABLE_Ａ`).

**S2-6, item by item:**
- URI index: fixed. `crates/blaze_repo_fetch/src/uri.rs:48 fail` converts the byte offset to
  UTF-16 units. Test `uri::tests::syntax_error_index_counts_utf16_units` (expected values from
  `new java.net.URI(...)` on JDK 25: 12, 11, 1).
- `compute_exec_path_length_diff`: fixed (`path_mapper.rs:112`, `chars().count()`). No test:
  only the no-op mapper exists, and it overrides the method.
- Attribute-name keys in the predeclared-input JSON: fixed (`digest_writer.rs:352`). Test
  `digest_writer::tests::attribute_names_and_values_are_converted_to_unicode`. The adapter
  chain was re-read: `toJson(attrValues)` serializes `AutoValue_AttributeValues` reflectively,
  and with `enableComplexMapKeySerialization` (`GsonTypeAdapterUtil.java:413`) the `Dict` keys
  go through `STRING_ADAPTER`.
- NUL in the rc tokenizer: fixed (`crates/blaze_options/src/rc/tokenize.rs:6`, `'\0'` added to
  the separators). Test `tokenize::byte_strings_tests::nul_separates_tokens`, including that a
  NUL inside quotes stays.
- Embedded NUL in a path: **left as is.** Java is not uniform here. The JNI calls (stat, readdir,
  symlink: `latin1_jni_path.cc`) truncate at the NUL. The JDK stream calls used by
  `getInputStream`/`getOutputStream` (`new FileInputStream(String)`) reject the path as
  `FileNotFoundException("Invalid file path")`. Copying both per operation in blaze_vfs would be
  a design change for an input that labels cannot carry. Blaze's uniform `InvalidInput` stays,
  and the difference stays recorded.
- Status-line trim: fixed. `socket_transport.rs:223` uses Java `String.trim()` (chars `<= ' '`).
  Test `status_line_trim_keeps_latin1_spaces`.

**S2-7, record and guard:**
- Dead seam: removed. `to_internal`, `internal_string_bytes`, `internal_string_from_bytes`,
  `internal_length`, `strings_are_internal`, `with_strings_are_internal_for_testing`,
  `STRINGS_ARE_INTERNAL` and `Fingerprint::add_internal_string` are deleted. `Fingerprint`
  itself still called `to_internal`, an identity since the switch; the call is gone.
  `parser_input_from_latin1` (`string_encoding.rs:53`) now returns `Cow<str>`, so the
  unreachable `unported!` arms in `compiled_module_file.rs` and
  `blaze_sky_loading/src/starlark_util.rs` are gone. The module comment was rewritten. The
  tests that checked the pre-switch half are deleted (rules 07a section 3).
- Guard: `crates/blaze_util/tests/string_seam_guard_test.rs` was rewritten for the post-switch
  state. Each listed decoder must call `parser_input_from_latin1`, and no unlisted decoder may
  exist. New `bundled_starlark_sources_are_ascii` follows every `include_str!` of a
  `.bzl`/`.scl`/`.bazel` file in `crates/*/src` (bundled builtins, `starlark_transition.rs`) and
  fails if one is not ASCII.
- The five exec-site (b) lines (section 4), in the free files of blaze_exec:
  `persistent_string_indexer.rs:200` reads keys with `java_new_string_utf8`.
  `persistent_string_indexer.rs`, `action_cache.rs` and `compact_persistent_action_cache.rs`
  `dump` print through a new `println_latin1!` (`actions/cache/mod.rs:12`, a default-charset
  `PrintStream`). `message_input_stream_wrapper.rs:199` uses `java_new_string_utf8`.
  Test: `persistent_string_indexer_test::dump_writes_internal_strings_as_latin1`.
  The "check" line `local_spawn_runner.rs:592` is now resolved: the host name goes to
  `getaddrinfo` as its raw `gethostname` bytes (`local_spawn_runner.rs:600`), as the JDK does
  through `JNU_GetStringPlatformChars`; it no longer goes through `to_socket_addrs` on a `&str`.
  These five files were unmodified in the tree and marked `free` in the inventory. The task's
  step 3 names them, although its general list excludes blaze_exec, so I edited only those
  files.
- Corrections to `07-byte-strings.md` sections 7.2/7.4/7.5: **not made.** That record belongs to
  root. Text to change: 7.4, the seam functions are deleted (see above). 7.5, add S2-5 (now
  fixed), the PAX non-UTF-8 `path` (`compressed_tar.rs`) and the lone-surrogate lockfile escape
  (`gson.rs`), both still PORT-marked, and drop the claim that only lockfile facts or a Java
  literal yield chars above U+00FF. 7.2, "14 of `GsonTypeAdapterUtil`'s 17 have a Rust site";
  `command_failure_utils.rs` is now converted.

### S3

- `BasicHttpAuthenticationEncoderTest`: ported, all 7 Java tests (the review counted 5), in
  `crates/blaze_repo_fetch/tests/basic_http_authentication_encoder_test.rs`, including
  `unicodeToInternal("123£")`.
- Unported output formats: unchanged, still `unported!` (honest S3).

### S4

- Stale comments: `command_failure_utils.rs:63-64` (removed with S1-2) and
  `blaze_packages/src/attribute/string_util.rs:55` (rewritten).
- `starlark_conformance/tests/byte_strings.rs` SKIP-as-pass: **left as is.** It is not a
  one-line fix: failing when `~/.cache/blaze/work/bytes` is missing would break every checkout
  that has not recorded it. Root should decide whether the gate records it.
- `scripts/w4/scenarios/non-ascii` claim, and `artifact_factory.rs:88` (Windows-only reader):
  **left as is**, outside the one-line scope.

### Section 3 (older findings), the two assigned

- **Tar `rename_files` directory, fixed, and the review's reading corrected by the oracle.**
  Java `entry.isDirectory()` (`CompressedTarFunction.java:111`) never looks at the rename
  target, so the old Rust test on the renamed name was wrong. But a trailing `/` in the entry's
  own header name does not count either. Bazel decodes tar names with `MarkedIso88591Charset`,
  which appends a marker (`CompressedTarFunction.java:208-252`), so `getName().endsWith("/")` is
  false. Only the directory type, or a PAX `path` name (decoded without the marker), makes a
  directory. Probe: Commons Compress from `A-server.jar` says `isDirectory=true` for a regular
  entry `z/` read without Bazel's charset. The oracle's `repository_ctx.extract` with
  `rename_files = {"x": "d/", "z/": "w"}` writes **both `d` and `w` as files**.
  `compressed_tar.rs:353`: `is_dir() || (raw_name ends with '/' && name is a PAX path)`.
  Test: `decompressor_test::tar_rename_files_directory::directory_is_decided_by_entry_type_not_rename_target`
  (expected values from that oracle run).
- **Provider prefix:** settled on the oracle; see S1-3.

### Evidence

- Tests of the 15 changed crates (blaze_util, blaze_vfs, blaze_options, blaze_shell,
  blaze_analysis, blaze_rules, blaze_repo_fetch, blaze_bzlmod_model, blaze_bzlmod,
  blaze_build_config, blaze_actions, blaze_repo_rules, blaze_exec, blaze_packages,
  blaze_sky_loading): **3622 passed, 0 failed, 1117 ignored**. The first run had one failure,
  the guard test above, which was replaced; that test binary was re-run and gives 8 passed.
- `scripts/cargo-capped check --workspace --all-targets`: clean. Clippy on these crates: no
  warning in a changed file.
- `scripts/bytes`: the oracle was re-recorded with the new cases (all exit codes as expected;
  `fail` is 1 by design). Blaze gives **114 / 115**. The one difference is
  `case:flagoff.stderr:CASE flagoff_rule_json` (open part of S2-1, blaze_starlark). The 97
  earlier items all still match.
