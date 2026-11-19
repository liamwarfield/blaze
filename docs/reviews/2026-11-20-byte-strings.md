# Review: byte-strings migration (2026-11-20)

Reviewer role: independent reviewer (Fable 5.2), read-only. Four sub-reviewers (Fable 5.2), one
per slice; every finding below was re-read on both sides by the lead reviewer unless it says
otherwise.

**Commit read: `2574fef`** (diff `a602a17..2574fef`, 392 files). Read from a `git archive`
extract of that commit, not from the working tree. HEAD `5a4e4a0` adds one document only.
No Cargo was run. Programs run: `scripts/bytes/compare` on the saved recordings, the prebuilt
`target/debug/starlark_conformance` and the oracle's own interpreter (`StarlarkProbe.java`) for
differential string tests, small JDK/Gson probes for Java-side behaviour.

**Verdict: STOP.** Blocking: S1-1 to S1-6. The design, the conversion core
(`blaze_util::string_encoding`), the interpreter patches 0032-0036 and the conversion direction
at every `StringEncoding` site that was read are right; the STOP is for boundary sites the
migration missed or got wrong, each with a concrete non-ASCII input.

Out of scope, as instructed: `crates/blaze_remote` and the RESERVED files of blaze_exec /
blaze_sky_exec / blaze_buildtool (the remote lead is still converting them). Only the
completeness of `07-byte-strings-exec-sites.md` was checked for them (section 4).

## 1. Coverage

| area | how |
|---|---|
| `blaze_util::string_encoding`, `Fingerprint`, seam guard, seam leftovers | full read |
| starlark-rust patches 0032-0036, lexer, string methods, repr, blaze_starlark json | full read + differential run against Bazel's interpreter: 256 chars x 38 operations, 699 random Latin-1 strings x 53 operations, 239 json files, about 60 lexer files |
| blaze_vfs `unix_file_system.rs`, `os_path_policy.rs` | full; `path.rs`, `path_fragment.rs` sampled |
| blaze_shell (process launch, escapers) | full for `java_process.rs`, `java_file.rs`, escapers; rest sampled |
| blaze_client, blaze_runtime, blaze_events, blaze_options, blaze_optdefs | grep-complete for the boundary patterns of rules section 2, logic sampled; rc files full |
| blaze_cmdline `label_validator.rs` | full; rest by grep |
| blaze_bzlmod, blaze_bzlmod_model (lockfile adapters, Gson reader/writer, registry, URI) | full, per-adapter table against `GsonTypeAdapterUtil` |
| blaze_repo_rules, blaze_repo_fetch (context methods, archives, patch, netrc, proxy, rewriter) | full |
| blaze_query, blaze_query_engine, blaze_cquery (every ported output format) | full for the formatters; per-format charset table agrees with Java |
| blaze_actions (diff and its Java), blaze_packages / blaze_sky_loading source decoding, glob, `.bazelignore` | full |
| blaze_bep, blaze_build_config, blaze_analysis, blaze_rules | sampled |
| free files of blaze_exec, blaze_sky_exec, blaze_buildtool, blaze | grep-complete |
| workspace-wide greps (all crates except blaze_remote): direct `std::fs`/`Path::new`/`OpenOptions`; `from_utf8*`/`to_string_lossy`/`env::var`/`read_to_string` (35 hits left, all read); `{:?}` in `Display` impls | full |
| evidence tools `scripts/bytes/{run,compare,telemetry-compare}`, `scripts/w4/scenarios/non-ascii` | full read; `compare` re-run |
| not reviewed | `blaze_client/startup_options.rs` line by line; BEP fields beyond `TargetCompleteEvent`, URI quoting and the JSON transport; `ctx.expand_location` / `resolve_command` internals; `.scl` literal checks |

Checked and found equal (the parts the brief asked about by name):

- Direction at Java's `StringEncoding` sites: query proto 24 + `AttributeFormatter` 18,
  `GsonTypeAdapterUtil` (every write side and every read side Blaze has; three read sides,
  Java :83/:131/:247, have no Rust counterpart because Blaze never deserializes them),
  `AttributeValuesAdapter` 4, `FactsAdapter` (none, as Java), `TargetCompleteEvent`,
  `RegexUtil`/`Converters`, `SimpleLogHandler`, `ExecutionInfoModifier`, `ProxyHelper` URL
  decoding. No extra conversion found.
- Oracle quirks: cquery jsonproto (Unicode then Latin-1 writer, `?`), query streamed_jsonproto
  (UTF-8), aquery jsonproto (raw internal through the Latin-1 stream), aquery text (escaped
  Unicode + base64 of `getBytes(UTF_8)`), action keys (`add_string` = `as_bytes()`),
  FileWrite / param files / manifests / templates (`latin1_encode`): each where Java has it and
  nowhere else.
- Security: every system call takes the path through one function (`unix_file_system.rs:35 os`),
  which is the JNI's mapping (`latin1_jni_path.cc:121-124`, `?` per UTF-16 unit); the `?`
  collisions are the ones Bazel has. Archive traversal checks (tar, zip, ar, 7z, patch) run on the
  same one-char-per-byte string that reaches the OS; nothing is converted after a check; Rust is
  not laxer than Java. `LabelValidator`: package names ASCII only, target names accept every
  char >= U+0080, repository and module names ASCII classes, as Java. FileWrite digests are over
  the bytes written. No `url` crate, no serde_json on internal strings, no IDN or
  percent-encoding step that Java lacks.
- Sorting: no place sorts an internal string by anything but `str` order (one exception on
  Unicode strings, S2-5).
- Tests changed in the commit: every renamed or deleted test is in the name map and its
  surviving half checks the same oracle data (78 action keys, the aquery file, 9 mnemonic cases,
  69 Starlark cases); `unicodeToInternal` wrapping matches the Java tests read
  (`QueryParserTest:301-305`, `TemplateExpansionActionTest:253`); no expectation was changed to
  fit Blaze; the 26 conformance recordings reproduce from Bazel's interpreter.

## 2. Findings

### S1 (blocking)

**S1-1. `import %workspace%/...` in an rc file fails when the workspace path is not ASCII.**
Rust `crates/blaze_options/src/rc/rc_file.rs:37`
(`std::path::Path::new(&resolved_path).exists()` on an internal string; called at :271).
C++ `src/main/cpp/workspace_layout.cc:80` (`PathExists` on the raw bytes).
Input: workspace `/tmp/wsé`, `.bazelrc` with `import %workspace%/extra.bazelrc`.
Bazel reads the file. Blaze stats `/tmp/ws` + C3 83 C2 A9 and exits with "Nonexistent path in
import declaration"; with `try-import` the file and its options are silently dropped.
The file was edited by the migration; `can_read_file` (`option_processor.rs:389`) is right.
Fix: `internal_to_platform` (or use the unused correct copy in
`blaze_client/src/workspace_layout.rs:72`). Section 7.3 claims a `wsé` workspace with a
non-ASCII `.bazelrc` as evidence; it had no import.

**S1-2. Abbreviated spawn failure message counts UTF-8 bytes.**
Rust `crates/blaze_shell/src/util/command_failure_utils.rs:65-68`
(`message.len() + command_element.len() > 200`; the comment says `str::len()` is Java's
`length()`, true only before the switch; file not touched by the migration).
Java `util/CommandFailureUtils.java:105-106` (`length()`).
Input: a failing genrule whose `bash -c` argument is 70 ASCII chars and 40 `é` (150 Java chars,
230 Rust bytes), without `--verbose_failures`.
Bazel prints the command in full; Blaze prints `... (remaining 1 argument skipped)`.
Fix: `chars().count()` on both operands.

**S1-3. `repr`/`str`/`print` of four native providers escapes bytes 0x7F-0xA0 and 0xAD.**
Rust `crates/blaze_analysis/src/platform/template_variable_info.rs:83`,
`crates/blaze_analysis/src/test/execution_info.rs:78,82`,
`crates/blaze_analysis/src/providers/run_environment_info.rs:83,88`,
`crates/blaze_rules/src/config/config_feature_flag_provider.rs:112,116` (Rust `{:?}`).
Java `packages/NativeInfo.java:125-138` (`printer.repr`), `Printer.java:239` (escapes only
`"`, `\`, `\n`, `\r`, `\t` and chars below 0x20).
Input: `print(platform_common.TemplateVariableInfo({"K": "à"}))` (à = C3 A0).
Bazel: `struct(variables = {"K": "à"})`. Blaze: `{"K": "Ã\u{a0}"}`.
Every emoji (byte 9F) and most CJK text hit it. Correct before the switch, so this is a
regression of the switch. Fix: `blaze_starlark::printer::quote`.

**S1-4. Proxy credentials on the CONNECT tunnel are sent in the wrong bytes.**
Rust `crates/blaze_repo_fetch/src/downloader/socket_transport.rs:511-528`
(`basic_http_authentication_encode` = bytes of the internal string).
Java `bazel/repository/downloader/ProxyHelper.java:330-333`: for HTTPS through a proxy the
credentials go to the JDK `Authenticator` as `internalToUnicode(user/password)`, and the JDK's
`BasicAuthentication` encodes them as ISO-8859-1.
Input: `HTTPS_PROXY=http://é:pw@proxy:3128`, an https download.
Bazel sends `Basic 6Tpwdw==` (E9 3A 70 77); Blaze sends `Basic w6k6cHc=` (C3 A9 3A 70 77).
Plain-HTTP proxying is equal. Section 7.2 lists "`ProxyHelper` credentials (2 calls)" as
converted. The JDK side is from reading and `javap` on the oracle's JDK, not from a run.
Fix: for the tunnel, `latin1_encode_lossy(internal_to_unicode(user + ":" + password))`.

**S1-5. Userinfo of a rewritten URL is truncated to the low byte instead of `?`.**
Rust `crates/blaze_repo_fetch/src/downloader/url_rewriter.rs:428` (`c as u32 as u8`; the PORT
comment says "identical for ASCII"; line added by this commit).
Java `bazel/repository/downloader/UrlRewriter.java:178` (`getBytes(ISO_8859_1)`).
Input: downloader config `rewrite (.*) 中:pw@mirror.example.com/$1`.
Bazel sends `Basic Pzpwdw==` ("?:pw"); Blaze sends `Basic LTpwdw==` ("-:pw", U+4E2D & 0xFF).
Fix: `latin1_encode_lossy`.

**S1-6. The Gson reader port panics on a `\u` escape followed by a multi-byte char, and rejects
a leading BOM.**
Rust `crates/blaze_bzlmod_model/src/gson.rs:321` (`&rest[..4]` after a length check in bytes),
:322 (`from_str_radix` accepts `\u+123`), :69-87 (no U+FEFF skip).
Java: Gson `JsonReader.readEscapeCharacter` (`MalformedJsonException`), `fillBuffer` (skips a
leading U+FEFF).
Inputs: a registry `source.json` or a `MODULE.bazel.lock` containing `"\uabc€"`: Bazel reports
a malformed-JSON error, Blaze panics (registry JSON comes from the network); a lockfile that
starts with EF BB BF parses in Bazel and fails in Blaze with "JSON document was not fully
consumed."
Fix: take four chars and require hex digits; skip the BOM.

### S2

**S2-1. `--nointernal_starlark_utf_8_byte_strings` is not wired at three evaluators.**
`crates/blaze_analysis/src/starlark/starlark_rule_configured_target_util.rs:191` (rule
implementation functions), `crates/blaze_analysis/src/starlark/starlark_aspect_factory.rs:102`,
`crates/blaze_bzlmod/src/module_file/compiled_module_file.rs:199`: `Evaluator::new` without
`set_utf8_byte_strings`, so the patch default (true) applies. Java reads the thread's semantics
(`StringModule.java:326-334`, `Json.java:346-353`). With the flag off, `"\240a".strip()` in a
rule implementation is `"a"` in Bazel and `"\240a"` in Blaze. Masked at the default.
Also with the flag off: `json.decode('"\\ud83d\\ude00"')` is 2 chars in Java, 1 in Blaze
(`crates/blaze_starlark/src/json.rs:474`).

**S2-2. `PathFragment::starts_with_ignoring_case` folds internal chars without converting.**
`crates/blaze_vfs/src/path_fragment.rs:377-398`; Java `vfs/PathFragment.java:442-443`
(`internalToUnicode` on both). `/inc/é` against prefix `/inc/É`: Java true, Blaze false; bytes
E3 A9 against C3 A9: Java false, Blaze true. Masked: the only caller is the Windows branch of
header discovery.

**S2-3. Text-proto strings use `String::from_utf8_lossy`, not the JDK decoder.**
`crates/blaze_protos/src/text_format.rs:673`, `:1423` (new file; its PORT comment says the two
are equal, `string_encoding.rs:293-296` records that they are not). Java `TextFormat.java:1615`.
`remote_execution_properties` with `value:"\355\240\200"`: one U+FFFD in Bazel, three in Blaze.

**S2-4. A char above U+FFFF in a Unicode path becomes one `?`; the JNI writes two.**
`crates/blaze_repo_fetch/src/fs_utils.rs:231 unicode_path_as_seen_by_file_system`
(`latin1_encode_lossy`); also `http_connector.rs:275`, `blaze_bzlmod_model/src/registry.rs:152`,
`download_manager.rs:504`. `latin1_jni_path.cc:121-124` is per `jchar`; blaze_vfs `os()` does
it right. Patch header `+++ b/🐷.txt`: Bazel creates `??.txt`, Blaze `?.txt`.

**S2-5. Workspace-status keys are ordered by code point; Java's `TreeMap` by UTF-16 unit.**
`crates/blaze_actions/src/bazel/bazel_workspace_status_module.rs:202`; Java
`bazel/BazelWorkspaceStatusModule.java:139-140`. Keys `STABLE_Ａ` (U+FF21) and `STABLE_🐷`
come out in the opposite order. PORT-marked in the code, not in section 7.5.

**S2-6. Smaller deviations, each verified by reading.**
- `URISyntaxException` index in UTF-8 bytes: `crates/blaze_repo_fetch/src/uri.rs:47-98`
  (`auth = {"https://h/é x": ...}`: Bazel index 12, Blaze 14; `blaze_bzlmod_model/src/java_uri.rs`
  is right).
- `compute_exec_path_length_diff` uses `.len()`:
  `crates/blaze_actions/src/actions/path_mapper.rs:110-113`; Java `actions/PathMapper.java:116-118`.
  Masked: only the no-op mapper reaches it.
- Attribute-name keys unconverted in the predeclared-input hash JSON:
  `crates/blaze_repo_rules/src/repository/digest_writer.rs:347`; Java applies `STRING_ADAPTER` to dict keys
  (`GsonTypeAdapterUtil.java:64`). Masked: attribute names are ASCII.
- NUL is not a token separator in the rc tokenizer: `crates/blaze_options/src/rc/tokenize.rs:4,35,85`;
  C++ `src/main/cpp/util/strings.cc:186,235` (`strchr` matches the terminator).
- A path with an embedded NUL: the JNI truncates at the NUL, Blaze returns `InvalidInput`
  (safer, but a deviation; labels cannot carry NUL).
- `socket_transport.rs:222,228` trims status-line parts with Rust `trim()` (also U+0085, U+00A0).

**S2-7. Record and guard.**
- Section 7.4 says the seam functions and `Fingerprint::add_internal_string` stay "until those
  callers are gone". At `2574fef` no crate calls any of them (grep over all crates, blaze_remote
  included); only blaze_util's own tests do. They are dead code, and the `Result` of
  `parser_input_from_latin1` keeps an unreachable `unported!` at
  `crates/blaze_bzlmod/src/module_file/compiled_module_file.rs:114`.
- The guard (`crates/blaze_util/tests/string_seam_guard_test.rs`) finds decoders by the name
  `fn create_parser_input`. Source text that reaches `eval::compile` another way is not seen:
  the bundled builtins (`include_str!`, `blaze_sky_loading/src/bundled_fs.rs`,
  `blaze_build_config/src/starlark_transition.rs:782`). They are ASCII today (checked), but
  nothing asserts it.
- Five (b) lines of `07-byte-strings-exec-sites.md` in free files are still open at `2574fef`
  and section 7.4 does not name them: `blaze_exec/src/actions/cache/persistent_string_indexer.rs:199`
  and `:122-128`, `action_cache.rs:267-303`, `compact_persistent_action_cache.rs:1210-1236`,
  `exec/io/message_input_stream_wrapper.rs:199`. `exec/local/local_spawn_runner.rs:592`
  (host name to the resolver) is still "check".
- Section 7.5 misses: S2-5; the PAX `path` record that is not UTF-8
  (`compressed_tar.rs:297-302`: Bazel names the file EF BF BD, Blaze FF) and the lone-surrogate
  lockfile escape (`gson.rs:340`: Bazel `?`, Blaze EF BF BD), both PORT-marked in the code. Its
  sentence that a char above U+00FF can only come from lockfile facts or a Java literal is wrong:
  `"◵".title()` yields U+039C in both Bazel and Blaze.
- Section 7.2 "each of `GsonTypeAdapterUtil`'s 17": 14 have a Rust site, 3 read sides have none.
  "blaze_shell ... converted" does not hold for `command_failure_utils.rs` (S1-2).

### S3

- `BasicHttpAuthenticationEncoderTest` (5 tests, one with `unicodeToInternal("123£")`) has no
  port; the function changed in this commit.
- Unported output formats and branches fail loudly (`unported!`): query
  `xml`/`graph`/`minrank`/`maxrank`, cquery `textproto`/`build`/`files`/`starlark`/`transitions`,
  aquery `proto`/`streamed_proto`/`textproto`, three `ProtoOutputFormatter` stack branches
  (`proto_output_formatter.rs:477-486`). As section 7.5 says.

### S4

- `crates/starlark_conformance/tests/byte_strings.rs` prints SKIP and passes when
  `~/.cache/blaze/work/bytes` is missing; the gate cannot tell a skipped run from a passed one.
- `scripts/w4/scenarios/non-ascii`: "every line that carries non-ASCII text equals the oracle's"
  is true for 39 of 40 distinct lines. Blaze's step 22 prints
  `GLOB ["a.txt", "out_é.txt", "é.txt"]`, which the oracle never prints (no persistent server:
  the package is loaded again after the output file exists). Not a string defect.
- `artifact_factory.rs:88` leaves chars above U+FFFF unfolded (reader is Windows-only).
- Stale comments: `blaze_packages/src/attribute/string_util.rs:55`,
  `command_failure_utils.rs:63-64`.

## 3. Found in passing, older than this commit (not part of the verdict; root should route them)

Observed on both interpreters by the sub-reviewer; the Java side of the first two re-read by me.

- **S1. A tab between tokens is a parse error.**
  `third_party/starlark-rust/starlark_syntax/src/lexer.rs:1333` and `:809-817`; Java
  `net/starlark/java/syntax/Lexer.java:824-828` ignores it (tabs are errors only in
  indentation, :217-220). `x =\t1` and `x = 1\t# c` fail in Blaze with "tabs are not allowed",
  also in BUILD and .bzl files. The reverse also differs: a blank line holding a tab is an
  error in Java and accepted by Blaze.
- **S1. Empty needle with `start` past the end.** `starlark_syntax/src/fast_string.rs:198,230,244,251`;
  Java `StringModule.java:111-126` clamps. `"abc".find("", 5)`: 3 / -1;
  `"abc".count("", 2, 1)`: 1 / 0; `"abc".startswith("", 5)`: True / False.
- **S1.** `"abc"[None:2]` is an error (`str_type.rs:331-335`; Java `Starlark.java:692,698`).
  `fail("\n  x")` loses its leading whitespace (`crates/blaze_starlark/src/error.rs:244`; Java
  `MethodLibrary.java:888`). Strings have a `codepoints` method (`methods.rs:315`).
  A comment containing a lone CR is "invalid input" (`lexer.rs:1330`; Java `Lexer.java:555-561`).
  `type(json)` is "namespace" (Java "json").
- **S2. Invalid programs accepted:** `"{!r}".format(1)` (`dot_format_parser.rs:109`),
  `int("1_000")` (`lexer.rs:1271`), `b"abc"` literals (`lexer.rs:1361-1369`).
- **S1, not byte-related.** `compressed_tar.rs:335` tests the renamed name for a trailing `/`;
  Java `CompressedTarFunction.java:111` uses `entry.isDirectory()`. Entry `x` with
  `rename_files = {"x": "d/"}`: Bazel writes file `d`, Blaze makes a directory.
- The four provider `Display` impls of S1-3 print `ExecutionInfo(`/`RunEnvironmentInfo(` where
  Java prints `struct(` (not checked on the oracle).

## 4. Exec-sites inventory (`07-byte-strings-exec-sites.md`)

Complete for the free files of blaze_exec, blaze_sky_exec, blaze_buildtool and blaze: the
boundary greps found no wrong site that it does not list. All file access there goes through
blaze_vfs and all process launches through blaze_shell. Open items: the five (b) lines and one
"check" line named in S2-7, and S1-2, which lies in blaze_shell (a crate the inventory calls
"already converted") but produces blaze_exec's failure text. blaze_remote was not reviewed.

## 5. Evidence tools and claims

- `scripts/bytes/compare` compares a candidate recording with the oracle's
  (`~/.cache/blaze/work/bytes-out`); `run` records with the oracle binary by default and refuses
  to write another binary's output there. Expected values are the oracle's. Re-run on the saved
  recordings: 97 / 97. The saved Blaze recording is from a working-tree binary of 2026-11-18
  22:14, not from a build of `2574fef`; I could not rebuild.
- `telemetry-compare` runs both binaries on the same day and checks `id_day` against `hashlib`.
  Sound.
- The 97 cases, workerd and the non-ascii scenario exercise: Starlark string semantics, file
  contents, one argv and env value, one label, query/cquery/aquery output, lockfile and one
  repository rule. They do not exercise: rc imports (S1-1), failure messages (S1-2), provider
  repr (S1-3), proxies and the URL rewriter (S1-4, S1-5), malformed JSON (S1-6), or the flag
  turned off (S2-1). That is where the findings are.
- The new workerd result (full build exit 0, Bazel's summary line and sha256) is consistent
  with this review: none of the findings is on that path.

## 6. Verdict

**STOP at `2574fef`.** Blocking: S1-1, S1-2, S1-3, S1-4, S1-5, S1-6. S1-4 rests on the JDK's
Basic encoder as read, not run; if root shows with a run that the JDK sends other bytes, it
drops out. The S2 items and the record corrections of S2-7 do not block. Section 3 is not part
of this verdict.
