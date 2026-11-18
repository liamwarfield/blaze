# Score: blaze-12ca29ee95e0 against oracle-9.3.2

Verdict: **FAIL (3 surfaces DIFF; 8 surfaces MISSING; 2 surfaces under a declared recording exception) [5 surfaces recorded by neither side]**

Written by `scripts/oracle/scoreboard blaze-12ca29ee95e0 --write`; referee sha256 `26e4400ec7d2`. Counts are reference items: total = matching + allowed + failed; match% is matching over total minus allowed.

## Recordings

| side | project | recording |
|---|---|---|
| reference `oracle-9.3.2` | brotli | binary sha256 `2e14c55c2987` `~/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64`; recorded 2026-11-18T02:00:34+00:00; repo commit not recorded |
| reference `oracle-9.3.2` | re2 | binary sha256 `2e14c55c2987` `~/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64`; recorded 2026-11-18T02:06:36+00:00; repo commit not recorded |
| reference `oracle-9.3.2` | workerd | binary sha256 `2e14c55c2987` `~/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64`; recorded 2026-11-18T15:12:21+00:00; repo commit not recorded |
| candidate `blaze-12ca29ee95e0` | brotli | binary sha256 `fe43027c8a5a` `~/.cache/blaze/work/accept-12ca29ee95e0/target/debug/blaze`; recorded 2026-11-18T20:41:58+00:00; repo commit 12ca29ee95e0 clean |
| candidate `blaze-12ca29ee95e0` | re2 | binary sha256 `fe43027c8a5a` `~/.cache/blaze/work/accept-12ca29ee95e0/target/debug/blaze`; recorded 2026-11-18T20:48:11+00:00; repo commit 12ca29ee95e0 clean |
| candidate `blaze-12ca29ee95e0` | workerd | NOT RECORDED |

## Declared recording exceptions

- `oracle-9.3.2/workerd` **workerd-aquery-spliced** (kind spliced), surfaces aquery_actions, exit_codes: Not a recording made by scripts/oracle/record's protocol. On a fresh output base the oracle's `aquery deps(//src/workerd/server:workerd) --output=jsonproto` exits 8 (build interrupted) and leaves no usable output; label oracle-9.3.2-workerd holds that recording unchanged (aquery_actions FAILED, exit code 8). In this record the exit code of aquery_deps_jsonproto was changed by hand from 8 to 0 and raw/aquery_deps_jsonproto.stdout was replaced by the output of the same command run on ~/.cache/blaze/oracle-out/ur-workerd after a full oracle build (scripts/workerd/run oracle aquery ...; docs/workerd.md). Absolute paths in it name that output base and are not normalised. A candidate that exits 0 and prints these actions matches something the oracle does not do under the protocol, so the two surfaces are compared for information and never count as a pass.

## Surfaces

| project | surface | status | total | compared | matching | allowed | failed | extra | match% |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|
| brotli | exit_codes | MATCH | 10 | 10 | 10 | 0 | 0 | 0 | 100.0 |
| brotli | mod_graph | MATCH | 27 | 27 | 27 | 0 | 0 | 0 | 100.0 |
| brotli | lockfile | MATCH | 193 | 193 | 193 | 0 | 0 | 0 | 100.0 |
| brotli | query_label_kind | MATCH | 162 | 162 | 162 | 0 | 0 | 0 | 100.0 |
| brotli | query_jsonproto | MATCH | 162 | 162 | 162 | 0 | 0 | 0 | 100.0 |
| brotli | cquery_label_kind | MATCH | 177 | 177 | 177 | 0 | 0 | 0 | 100.0 |
| brotli | cquery_jsonproto | MATCH | 18 | 18 | 18 | 0 | 0 | 0 | 100.0 |
| brotli | aquery_actions | MATCH | 68 | 66 | 66 | 2 | 0 | 0 | 100.0 |
| brotli | build_stderr | MATCH | 5 | 5 | 5 | 0 | 0 | 0 | 100.0 |
| brotli | execlog | MATCH | 44 | 44 | 44 | 0 | 0 | 0 | 100.0 |
| brotli | bep | DIFF | 42 | 41 | 37 | 1 | 4 | 0 | 90.2 |
| brotli | outputs | MATCH | 96 | 96 | 96 | 0 | 0 | 0 | 100.0 |
| brotli | tests | MATCH (empty) | 0 | 0 | 0 | 0 | 0 | 0 | - |
| brotli | **total** | | 1004 | 1001 | 997 | 3 | 4 | 0 | 99.6 |
| re2 | exit_codes | MATCH | 10 | 10 | 10 | 0 | 0 | 0 | 100.0 |
| re2 | mod_graph | MATCH | 29 | 29 | 29 | 0 | 0 | 0 | 100.0 |
| re2 | lockfile | MATCH | 191 | 191 | 191 | 0 | 0 | 0 | 100.0 |
| re2 | query_label_kind | MATCH | 1091 | 1091 | 1091 | 0 | 0 | 0 | 100.0 |
| re2 | query_jsonproto | MATCH | 1091 | 1091 | 1091 | 0 | 0 | 0 | 100.0 |
| re2 | cquery_label_kind | MATCH | 3296 | 3296 | 3296 | 0 | 0 | 0 | 100.0 |
| re2 | cquery_jsonproto | MATCH | 39 | 39 | 39 | 0 | 0 | 0 | 100.0 |
| re2 | aquery_actions | MATCH | 1428 | 1374 | 1374 | 54 | 0 | 0 | 100.0 |
| re2 | build_stderr | DIFF | 7 | 7 | 7 | 0 | 0 | 1 | 100.0 |
| re2 | execlog | MATCH | 552 | 552 | 552 | 0 | 0 | 0 | 100.0 |
| re2 | bep | DIFF | 114 | 113 | 109 | 1 | 4 | 0 | 96.4 |
| re2 | outputs | MATCH | 5043 | 5043 | 5043 | 0 | 0 | 0 | 100.0 |
| re2 | tests | MATCH | 21 | 21 | 21 | 0 | 0 | 0 | 100.0 |
| re2 | **total** | | 12912 | 12857 | 12853 | 55 | 4 | 1 | 99.9 |
| workerd | exit_codes | MISSING* | 9 | 0 | 0 | 0 | 9 | 0 | 0.0 |
| workerd | mod_graph | MISSING | 0 | 0 | 0 | 0 | 0 | 0 | - |
| workerd | lockfile | MISSING | 0 | 0 | 0 | 0 | 0 | 0 | - |
| workerd | query_label_kind | MISSING | 46371 | 0 | 0 | 0 | 46371 | 0 | 0.0 |
| workerd | query_jsonproto | MISSING | 46371 | 0 | 0 | 0 | 46371 | 0 | 0.0 |
| workerd | cquery_label_kind | MISSING | 38892 | 0 | 0 | 0 | 38892 | 0 | 0.0 |
| workerd | cquery_jsonproto | MISSING | 4 | 0 | 0 | 0 | 4 | 0 | 0.0 |
| workerd | aquery_actions | MISSING* | 13425 | 0 | 0 | 0 | 13425 | 0 | 0.0 |
| workerd | build_stderr | NOT_RECORDED | 0 | 0 | 0 | 0 | 0 | 0 | - |
| workerd | execlog | NOT_RECORDED | 0 | 0 | 0 | 0 | 0 | 0 | - |
| workerd | bep | NOT_RECORDED | 0 | 0 | 0 | 0 | 0 | 0 | - |
| workerd | outputs | NOT_RECORDED | 0 | 0 | 0 | 0 | 0 | 0 | - |
| workerd | tests | NOT_RECORDED | 0 | 0 | 0 | 0 | 0 | 0 | - |
| workerd | **total** | | 145072 | 0 | 0 | 0 | 145072 | 0 | 0.0 |
| all | **total** | | 158988 | 13858 | 13850 | 58 | 145080 | 1 | 8.7 |

Surfaces in scope: 39 = MATCH 23 + DIFF 3 + FAILED 0 + MISSING 8 + NO_REFERENCE 0 + NOT_RECORDED 5.

## Allowed and excluded

- allowed by `build-metrics-volatile-counters`: 2 items. Skyframe evaluates in parallel, so how many nodes of each SkyFunction are evaluated or dirtied in one invocation varies between identical runs (re2: a few per cent). The serialized action cache size also varies by a few hundred bytes on re2 (161890 vs 161688) with identical actions and outputs. Action, artifact, target, package and per-mnemonic action counts in the same event are still compared.
- allowed by `runfiles-manifest-action-key`: 56 items. SourceManifestAction's action key fingerprints the manifest content, which names absolute execroot paths. Every recording has its own workspace copy and output base, so the hash differs; the normalised manifest bytes (outputs surface), argv, inputs and outputs still match.
- excluded, workerd, recorded by neither side: 5 surfaces (0 reference items): build_stderr, execlog, bep, outputs, tests
