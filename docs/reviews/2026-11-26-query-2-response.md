# Response to the query-2 review

Date 2026-11-26. Answers `2026-11-26-query-2.md` (STOP on S1-1, S1-2; S2-1 to S2-4; S4). Written by
the implementer of query-2 (task query-3).

Tree: every code change of this response is committed in checkpoint 40 (`ee4c27ea`); what is
left uncommitted is `third_party/starlark-rust/PATCHES.md` (patches 0063, 0064, 0062 round 2)
and this file. The numbers below were measured on `1958a452` plus those changes, uncommitted at
the time, with the same content as `ee4c27ea` for these files. Binary for the probes: `blaze` built
from that tree (debug), copied to `~/.cache/blaze/work/query-3/blaze`; oracle
`bazel-9.3.2-linux-x86_64`. Probes ran one workspace and one output root per side, startup
`--max_idle_secs=60`, `BLAZE_SERVER_MODE=daemon` for Blaze, `shutdown` after each series.

## Summary

| finding | state | where | evidence |
|---|---|---|---|
| S1-1 genquery gzip bytes | fixed (two causes) | workspace `Cargo.toml` (flate2 `zlib`), `gen_query_output_stream.rs` `flush` | oracle bytes at 36 B, 42.6 KB, 2.04 MB; unit test `flush_does_not_change_compressed_bytes` |
| S1-2 `print()` bypasses `System.err` | fixed, and a second bug found by the probe (file decoded as Latin-1) fixed | `starlark_output_formatter_callback.rs` | oracle probe with `é` and `€` (see below) |
| S2-1 aquery dump order | fixed: sorted by the key's `toString` | `aquery_processor.rs` | two servers give identical output |
| S2-2 CQUERY_TRANSITION_RESOLVER | items 1, 2 (graph), 3, 4 fixed; the evaluation statistics still count the node | `transitions_output_formatter_callback.rs` | oracle probe, rerun and configuration change |
| S2-3 step limit | ruling requested | below | |
| S2-4 filtered targets | shown wrong with Java evidence | `gen_query.rs` comment | `TargetPatternFunction.java:132` |
| S4 decisions entries | for root | below | |

## S1-1 genquery `compressed_output`: fixed

Two causes, both fixed:

1. **The deflate implementation.** flate2 now uses its `zlib` feature (libz-sys, linked against
   the system `libz.so.1`; workspace `Cargo.toml`, Cargo.lock gains `libz-sys 1.1.23`).
2. **`flush()`.** flate2's `GzEncoder::flush` writes a zlib `Z_SYNC_FLUSH` block. Java's
   `GZIPOutputStream(out, size)` and `GZIPOutputStream(out)` have `syncFlush == false`, so
   `DeflaterOutputStream.flush` only flushes the inner stream and adds no deflate bytes (JDK
   source, `DeflaterOutputStream.flush`). `GenQueryOutputStream.flush` now flushes the inner
   stream (`g.get_mut().flush()`). The same fix is in the profiler's `ProfileOutputStream`
   (`blaze_util/src/profiler/json_trace_file_writer.rs`, outside my crates; listed below).

Which zlib Bazel uses: the oracle's server runs the embedded JDK 25.0.2, whose `libzip.so` links
the system `/lib64/libz.so.1` dynamically. On this machine (Fedora 44) that is
**zlib-ng-compat 2.3.3** (reports `1.3.1.zlib-ng`), not stock zlib. The Blaze binary links the
same file (`ldd`). Bazel's genquery bytes therefore depend on the host's zlib, and Blaze matches
them by linking the same library. The review's suggestion "stock zlib, not zlib-ng" would have
produced different bytes on this host. A Blaze built with libz-sys's bundled zlib (no system
zlib found by pkg-config, or a static build) would no longer match.

Probe (`gq/BUILD`: genquery over 3, 60 and 1500 genrules; `--output=build` for the two large
ones; `compressed_output = True`):

| target | compressed size | oracle = Blaze | Blaze = JDK re-gzip of its text | stock zlib 1.3.1 = Blaze |
|---|---|---|---|---|
| small (label output, 27 B text) | 36 B | equal sha256 | equal | equal |
| medium (`--output=build`, 60 rules) | 42,588 B | same size, see note | equal | **different** (43,979 B) |
| large (`--output=build`, 1500 rules, 4.8 MB text) | 2,043,627 B | same size, see note | equal | **different** (2,072,262 B) |

Note: `--output=build` writes the BUILD file's absolute path, which differs between the two
workspaces (`ws-o`, `ws-b`), so the sha256s differ; the decompressed texts are equal after
replacing that path. The byte check is therefore: decompress each side's file and gzip the
text again with `GZIPOutputStream(System.out, 8192)` on the oracle's own JDK
(`~/.cache/blaze/work/query-3/gz/Gz.java`). That reproduces the oracle's file exactly at all
three sizes (validating the reference), and reproduces Blaze's file exactly at all three sizes.
The stock-zlib column used libz-sys's bundled zlib 1.3.1 sources with Java's parameters
(`~/.cache/blaze/work/query-3/stockz/gz.c`).

`command.profile.gz`: the content differs per run (timestamps), so the check is the same
re-gzip. Blaze's file (13,569 B, 134,430 B of JSON) equals the JDK's gzip of its decompressed
JSON; so does the oracle's (14,561 B). Header `1f 8b 08 00 00000000 00 ff` on both.

Other places Blaze gzips: only these two (`GzEncoder`/`GzBuilder` in `crates/*/src`).
`FileWriteAction`'s compressed in-memory content (Java `FileWriteAction.java:308`) and
`AllocationTracker` are not gzip paths in Blaze.

## S1-2 `print()` in the cquery formatter: fixed, plus a decoding bug

`StderrPrint` now calls `blaze_events::io::system_err_println("<location>: <message>")`. Grep
of my crates for `eprintln!`/`println!`: only `crates/blaze_query/src/bin/blaze-query-dev.rs`
(a dev binary).

The oracle probe found a second difference. Java reads `--starlark:file` with
`ParserInput.readFile`, which is `fromUTF8` (`ParserInput.java:61`): the file's text is a
Unicode string, not an internal one, unlike BUILD and .bzl sources. The format file
`print("été €", target.label); return str(target.label) + " é€"` gives, on the oracle,
`é` as the byte 0xE9 and `€` as `?` on both stderr and stdout (the ISO-8859-1 `System.err` and
`OutputStreamWriter`). Blaze decoded the file as Latin-1 and so wrote the UTF-8 bytes back
unchanged. The file is now decoded with `string_encoding::java_new_string_utf8` (the JDK's
replacing decoder). The output streams already encode with `latin1_encode_lossy`.
With the fix, the oracle probe is identical on stdout (24 bytes: `@@//sl:a \xe9?`) and on
stderr (`sl/print.star:2:10: \xe9t\xe9 ? //sl:a`).

## S2-1 aquery `--skyframe_state` order: fixed by sorting

`dump_skyframe_state` collects the ConfiguredTargetKey and AspectKey keys and sorts them by
their `toString` (Debug form). Java's order is `InMemoryGraphImpl`'s `ConcurrentHashMap`
iteration order over every node in the graph. It depends on the table size (all nodes of all
kinds) and on the Java `hashCode` of every key type, so it is not portable; the PORT note says
so. Probe (`build //aq:all //tr:all` then `aquery --skyframe_state --output=textproto`, twice,
with a server restart in between): Blaze's two outputs are identical (5,802 B, the same size as
the oracle's). The oracle's order (g4, g5, g1, g7, …) is its hash order.
**Ruling wanted (decisions.md):** Java's `ConcurrentHashMap` order of the dump is not reproduced;
Blaze sorts by key string.

## S2-2 CQUERY_TRANSITION_RESOLVER

1. **"did not complete".** Before the resolver's evaluation the formatter reads the evaluator's
   `computed_count()`. If the evaluation computed any node other than the resolver's own (the
   delta is above 1), a value Java's lookup environment reads was not done, and the formatter
   reports `Failed to evaluate <label>: ...: DependencyResolver.evaluate did not complete` and
   stops, where Java does (`CqueryTransitionResolver.java:147`).
2. **Nodes in the graph.** `close()` deletes every CQUERY_TRANSITION_RESOLVER node
   (`evaluator.delete(...)` and an evaluation with no roots, which applies the deletion), so
   the nodes do not survive into later commands and are not counted by
   `post_invocation_skyframe_node_count`. Remaining deviation: the per-function evaluation
   statistics of this command (`evaluated_values`) still count them. Removing that needs
   `DependencyResolver` on a read-only environment in the engine; I did not do that.
   I could not observe the node count through Blaze: its BEP leaves `buildGraphMetrics` empty
   for cquery (outside my crates). The oracle's count is 718 before and after a transitions
   query.
3. **Replayed events.** Follows from 2: the nodes are recomputed by the next query, so the
   events are posted again.
4. **Interrupt.** An interrupted evaluation now ends the formatter with
   `OutputFormatterError::Interrupted` (Java's `InterruptedException`). A missing value with no
   error stays `unported!`.

Probe: `cquery 'deps(//tr:top)' --transitions=lite|full` equals the oracle apart from the
workspace path in the Starlark transition's name. A second identical query in the same server
gives identical output. After `--compilation_mode=dbg` (configuration change) it equals the
oracle. No "did not complete" in any of them.
**Ruling wanted (decisions.md):** the Blaze-only node and its deletion at `close()`.

## S2-3 the 500,000-step limit: ruling requested

Java counts one step for every statement executed (`Eval.exec`, `Eval.java:295`) and every
expression evaluated (`Eval.eval`, `Eval.java:547`) on the thread. starlark-rust compiles to
bytecode: constants are folded, some expressions vanish, others become several instructions.
Counting Java's steps would mean carrying a weight on every bytecode instruction (the number of
AST statements and expressions it stands for) through the compiler and its optimisations. That
is a larger patch than any in the list.

The same counter backs `--max_computation_steps` and the package's computation steps
(`TargetDefinitionContext.java:284`, `package_piece.rs` `computation_steps`). It should be
decided once for all three users.

I did not put `unported!` on the format call: it runs for every `--output=starlark` query, and a
panic there would turn every passing query into a crash, while the gap only shows above 500,000
steps.

Options for the ruling:
- (a) Accept the gap for now. The PORT comment stays, and an entry in decisions.md names the
  three users.
- (b) Write the counting patch as its own task.
- (c) Count bytecode instructions as an approximation. I do not recommend it: it would give
  different numbers from Java, which the porting rules forbid.

I recommend (a), with (b) as a planned task.

## S2-4 genquery filtered targets: shown wrong

Java's `TargetPatternFunction` never fills the filtered set: `TargetPatternValue` is built with
`ResolvedTargets.builder()` and only `addAll(targets)`, so `getFilteredTargets()` is always
empty (`TargetPatternFunction.java:132`). Adding it to the preloader's package keys would add
nothing. The preloader carries a comment that cites this.

## S4 (root)

`docs/decisions.md` entries wanted for:
- `blaze_analysis::skyframe_env` (the reviewer found it sound; the argument form is preferred
  if the factory trait is touched again);
- CQUERY_TRANSITION_RESOLVER with its deletion at `close()`;
- S2-1 sorting;
- S2-3 (the ruling above).

## Changes outside my crates

- workspace `Cargo.toml`: flate2 `default-features = false, features = ["zlib"]` (named by the
  task); `Cargo.lock`.
- `crates/blaze_util/src/profiler/json_trace_file_writer.rs`: `flush` of the gzip stream
  flushes the inner stream only (same cause as S1-1).

## Verification

On `1958a452` plus the changes (now in `ee4c27ea`):

- `scripts/cargo-capped check --workspace --all-targets`: clean.
- `test -p blaze_cquery -p blaze_rules`: blaze_cquery 27 passed, 25 ignored, plus
  java_order_test 4 passed; blaze_rules 71 passed, 22 ignored (one new test). 0 failed.
- `test -p blaze_util --lib profiler`: 7 passed.
- Earlier on the same tree (before the `flush` and decoding fixes, which do not touch them):
  `test -p blaze_sky_top` 945 passed, 0 failed, 168 ignored; `test -p blaze_starlark
  -p blaze_analysis`: 0 failed; `test -p starlark_conformance --test patches` 61 passed.
- Oracle probes: as above. Also identical: `--show_config_fragments=direct|transitive` on brotli
  `//...` (stdout; the transitive run's stderr differs only in the loaded-package count, 66 against
  65, a loading statistic), the label `str()` cases of `test_starlark_output_mode` with and
  without `--incompatible_unambiguous_label_stringification`, and the `build_options` /
  `providers` argument errors.

# Round 2

Response to `docs/reviews/2026-11-26-query-2-round2.md` (STOP at 38cd662). The fixes below are
in `ef8c8ed1` (checkpoint 42).

## P63-1 (S1): a key-tracking `ctx.var` passed whole to a native

Each Blaze reader of a caller-supplied dict now calls
`starlark::values::dict::dict_record_all_keys_accessed(x)` before `DictRef::from_value`, which
records every key of a key-tracking dict as the Java reader does. The sites and the Java read
each mirrors:

| Rust site | Java reader | Java read |
|---|---|---|
| `blaze_analysis/src/starlark/util.rs:215` `dict_cast_string_string` (`env`, `substitutions`, ...) | `Dict.cast` (`Dict.java:666`) | `entrySet()` |
| `blaze_analysis/src/starlark/util.rs:263` `dict_cast_string_object` | `Dict.cast` | `entrySet()` |
| `blaze_analysis/src/starlark/starlark_rule_context.rs:1282` `dict_cast_string_artifact` | `Dict.cast` | `entrySet()` |
| `blaze_analysis/src/providers/run_environment_info.rs:145` | `Dict.cast` | `entrySet()` |
| `blaze_analysis/src/platform/template_variable_info.rs:172` | `Dict.noneableCast` | `entrySet()` |
| `blaze_rules/src/cpp/util.rs:173`, `:209` | `Dict.cast` | `entrySet()` |
| `blaze_rules/src/cpp/variables.rs:62` | a `Dict` as an `Iterable` | `keySet().iterator()` |
| `blaze_starlark/src/json.rs:109` `json.encode` | `Json.Encoder.encode` (`Json.java:173`) | `keySet()` |
| `blaze_packages/src/proto.rs:310` `proto.encode_text` | `Proto.TextEncoder.field` | `entrySet()` |

`DictRef::from_value` readers that remain read dicts Blaze builds itself (never `ctx.var`).
The PATCHES.md limitation of 0063 is removed.

- Blaze test: `blaze_sky_top` `required_config_fragments_test::starlark_ctx_var_read_whole_by_natives`
  (env and `json.encode(ctx.var)` record `["a", "b"]`; `ctx.var == x` records `[]`): passes.
- Oracle (`cquery //cv2:<t>` with `--define` flags and `--show_config_fragments=direct`, one
  reused workspace per side): `env = ctx.var`, `substitutions = ctx.var`,
  `json.encode(ctx.var)`, and `{"a": "b"} == ctx.var` (below) give identical stdout. stderr is
  identical except the `substitutions` case's loaded-package count (a loading statistic).

## P63-2 (S2): `x == ctx.var`

Patch 0063 round 2 (`types/dict/value.rs`): the `equals` hook calls `record_equals_accesses`
with the left side's kind. A left side whose Java contents are an `ImmutableMap`
(`java_contents_immutable`) records every key of the tracking dict (`ImmutableMap.equals` ->
`entrySet()`); a mutable left side of the same size records `y.get(k)` for its own keys in its
order, stopping at the first mismatch (`AbstractMap.equals`); different sizes record nothing;
`ctx.var == x` records nothing. Cases in `starlark_conformance/tests/patches.rs`
`patch_0063_key_tracking_dict` (equal, a missing key, a mismatching value, different sizes,
tracking dict on the left, immutable left side). The oracle case `{"a": "b"} == ctx.var`
(sizes differ from `ctx.var`, so nothing is recorded) is identical (above); the same-size and
immutable cases are covered by the conformance test only.

## Z-1 (S2): gzip bytes depend on the zlib linked

`crates/blaze_rules/build.rs` (new; `blaze_rules` gains `libz-sys` with default features off so
the workspace's feature choice decides) panics when `DEP_Z_ROOT` is set. libz-sys reports
`cargo:root` only when it builds its bundled zlib, so the build fails exactly when the system
`libz` is not linked; the comment says why. The workspace `Cargo.toml` flate2 comment points to
it. Not exercised on the failing path (`LIBZ_SYS_STATIC=1` would rebuild the shared target for
every agent); the passing path builds on this host.

## S3: the two `unported!` paths around the transition resolver

Both are left, each with its reason in the message:

- `blaze_cquery/.../transitions_output_formatter_callback.rs:192`: a resolver node with
  neither value nor error. Java's resolver runs outside Skyframe; in Blaze this is a Skyframe
  cycle or catastrophe that analysis of the same key would already have reported, so there is no
  Java behaviour to port.
- `blaze_sky_analysis/src/cquery_transition_resolver.rs:136`: root causes after a successful
  resolution. Java throws `EvaluateException("expected empty: " + causes)`, whose text needs
  `Cause.toString` and `DetailedExitCode.toString` (not ported).

## Round-1 residuals (a)-(c) of S2-2

Not changed (the reviewer marked them masked, not blocking): (a) nodes computed when the
detection fires stay in the graph and statistics; (b) is the first S3 path above; (c) on an
interrupt earlier targets' resolver nodes are not deleted for that command.

## Also in this round

- Patch 0062 round 3: the interface names that `java_param_accepts` checks
  (`Iterable`/`iterable` no longer accept a depset, `StarlarkValue`), and a
  `JavaInterfaceMatcher` hook that Blaze registers (`blaze_analysis/src/starlark/mod.rs`) so
  Blaze types answer for the Java interfaces they port. The table is only consulted to order
  the errors of a call that already failed. Test `patch_0062_round_3_java_interface_names`;
  PATCHES.md updated; `patches/blaze.diff` regenerated.
- `starlark_conformance --test patches`: 62 passed, 0 failed.

# Round 3

Response to `docs/reviews/2026-11-26-query-2-round3.md` (STOP at b216603). Working tree on
98e71603 plus the changes below (not committed).

## P63-1 (S1): the remaining whole-dict readers

The recording now sits in the blaze_packages helpers, so a new caller cannot miss it:

- `starlark_defs/util.rs` `dict_cast` (Java `Dict.cast`, `entrySet()`): covers the provider
  `init` result (`provider.rs`), and every other `dict_cast` caller in `starlark_defs`
  (`rule_class_functions.rs`, `config.rs`, `implicit_outputs_function.rs`). Root's `"unknown"`
  class names at those call sites are kept.
- `types/type_.rs` `convert_dict` (Java `Type.DictType.convert`, `Type.java:574`
  `o.entrySet()`): covers `ctx.resolve_command(make_variables = ...)` and every
  `Type.*_DICT.convert` port; the `copy_and_lift_starlark_value` dict branch (Java
  `DictType.copyAndLiftStarlarkValue`, `entrySet()`); and the list conversion of a dict
  (`iterable_elements`, Java `Iterable` of a dict = `keySet().iterator()`).
- `select()` (Java `SelectorList.select`, `dict.entrySet()`): `build/select.rs` takes its argument
  as `ValueOf<DictRef>` (same type error) and records; `types/select.rs` `select_impl` records.
- `builtins/cc_internal.rs` `freeze` now goes through `Dict.immutableCopyOf`'s port
  (`dict_immutable_copy_of`), which returns an immutable dict (so `ctx.var` itself, keeping its
  tracking) and copies only mutable ones, as Java (the reviewer's non-blocking note).

Per-site recording removed: none. None of the ten round-2 sites calls these helpers: they are
themselves the `Dict.cast` helpers of blaze_analysis and blaze_rules, or ports of other Java
readers (`json.encode`, `proto.encode_text`), so their recording stays.

The tree check: blaze_sky_top `whole_dict_readers_test::every_starlark_dict_reader_records_or_is_listed`
scans `crates/*/src` (not test files) for `DictRef::from_value(`, `DictMut::from_value(`, a
`DictRef<'v>` parameter, `UnpackDictEntries<` and `DictType<`. Each reader must either call
`dict_record_all_keys_accessed` earlier in its function or be listed in `NOT_RECORDING` with a
reason: a phase without `ctx.var` (repository rules, module extensions, MODULE.bazel,
REPO.bazel, PROJECT.scl, transitions, loading-time computed defaults, documentation extraction,
the execution-time build-info callback), a type check (`instanceof Dict`), or a Java read that
does not record (`isEmpty`, `hashCode`, `Dict.immutableCopyOf` of an immutable dict,
`checkLabelDict`, which fails on string keys). A listed entry that no longer names a reader also
fails the test.
