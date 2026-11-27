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
