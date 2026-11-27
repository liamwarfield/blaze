# Score: blaze-ef66f55e904e against oracle-9.3.2

Verdict: **FAIL (3 surfaces DIFF; 8 surfaces MISSING; 2 surfaces under a declared recording exception) [5 surfaces recorded by neither side]**

Written by `scripts/oracle/scoreboard blaze-ef66f55e904e --write`; referee sha256 `e7130cdba227`. Counts are reference items: total = matching + allowed + failed; match% is matching over total minus allowed.

## Recordings

| side | project | recording |
|---|---|---|
| reference `oracle-9.3.2` | brotli | binary sha256 `2e14c55c2987` `~/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64`; recorded 2026-11-18T02:00:34+00:00; repo commit not recorded; server mode unset (daemon-mode rules); cold (from stderr): mod_graph; warm (from stderr): build, query_label_kind, query_deps_label_kind, query_jsonproto, query_deps_jsonproto, cquery_deps_label_kind, cquery_jsonproto, aquery_deps_jsonproto, info |
| reference `oracle-9.3.2` | re2 | binary sha256 `2e14c55c2987` `~/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64`; recorded 2026-11-18T02:06:36+00:00; repo commit not recorded; server mode unset (daemon-mode rules); cold (from stderr): mod_graph; warm (from stderr): build, query_label_kind, query_deps_label_kind, query_jsonproto, query_deps_jsonproto, cquery_deps_label_kind, cquery_jsonproto, aquery_deps_jsonproto, info |
| reference `oracle-9.3.2` | workerd | binary sha256 `2e14c55c2987` `~/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64`; recorded 2026-11-18T15:12:21+00:00; repo commit not recorded; server mode unset (daemon-mode rules); cold (from stderr): mod_graph, aquery_deps_jsonproto; warm (from stderr): query_label_kind, query_deps_label_kind, query_jsonproto, query_deps_jsonproto, cquery_deps_label_kind, cquery_jsonproto, info |
| candidate `blaze-ef66f55e904e` | brotli | binary sha256 `2e240cc098ae` `~/.cache/blaze/work/accept-ef66f55e904e/target/debug/blaze`; recorded 2026-11-18T23:34:21+00:00; repo commit ef66f55e904e clean, binary origin not checked; server mode unset; cold (from stderr): mod_graph, build, query_label_kind, query_deps_label_kind, query_jsonproto, query_deps_jsonproto, cquery_deps_label_kind, cquery_jsonproto, aquery_deps_jsonproto, info |
| candidate `blaze-ef66f55e904e` | re2 | binary sha256 `2e240cc098ae` `~/.cache/blaze/work/accept-ef66f55e904e/target/debug/blaze`; recorded 2026-11-18T23:41:13+00:00; repo commit ef66f55e904e clean, binary origin not checked; server mode unset; cold (from stderr): mod_graph, build, query_label_kind, query_deps_label_kind, query_jsonproto, query_deps_jsonproto, cquery_deps_label_kind, cquery_jsonproto, aquery_deps_jsonproto, info |
| candidate `blaze-ef66f55e904e` | workerd | NOT RECORDED |

## Warnings

- brotli: commands on a warm server in oracle-9.3.2 and a cold one in blaze-ef66f55e904e: build, query_label_kind, query_deps_label_kind, query_jsonproto, query_deps_jsonproto, cquery_deps_label_kind, cquery_jsonproto, aquery_deps_jsonproto, info
- re2: commands on a warm server in oracle-9.3.2 and a cold one in blaze-ef66f55e904e: build, query_label_kind, query_deps_label_kind, query_jsonproto, query_deps_jsonproto, cquery_deps_label_kind, cquery_jsonproto, aquery_deps_jsonproto, info
- corpus/oracle/brotli.json differs from its committed version at HEAD: the reference is checked against an uncommitted summary
- corpus/oracle/re2.json differs from its committed version at HEAD: the reference is checked against an uncommitted summary
- corpus/oracle/workerd.json differs from its committed version at HEAD: the reference is checked against an uncommitted summary

## Not acceptable without --allow-dirty

- blaze-ef66f55e904e/brotli: binary origin not checked (recorded before binary_origin existed)
- blaze-ef66f55e904e/re2: binary origin not checked (recorded before binary_origin existed)
- accepted with `--allow-dirty`: root ruling 2026-11-19: accepted label recorded before commit verification existed; rescored from its existing recordings under referee batch 4 (renormalised in place with the batch 4 extractor; not re-recorded)

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
| brotli | bep | DIFF | 42 | 42 | 37 | 0 | 5 | 0 | 88.0 |
| brotli | outputs | MATCH | 96 | 96 | 96 | 0 | 0 | 0 | 100.0 |
| brotli | tests | MATCH (empty) | 0 | 0 | 0 | 0 | 0 | 0 | - |
| brotli | **total** | | 1004 | 1002 | 997 | 2 | 5 | 0 | 99.5 |
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
| re2 | bep | DIFF | 114 | 114 | 109 | 0 | 5 | 0 | 95.6 |
| re2 | outputs | MATCH | 5043 | 5043 | 5043 | 0 | 0 | 0 | 100.0 |
| re2 | tests | MATCH | 21 | 21 | 21 | 0 | 0 | 0 | 100.0 |
| re2 | **total** | | 12912 | 12858 | 12853 | 54 | 5 | 1 | 99.9 |
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
| all | **total** | | 158988 | 13860 | 13850 | 56 | 145082 | 1 | 8.7 |

Surfaces in scope: 39 = MATCH 23 + DIFF 3 + FAILED 0 + MISSING 8 + NO_REFERENCE 0 + NOT_RECORDED 5.

## Allowed and excluded

- allowed by `runfiles-manifest-action-key`: 56 items. SourceManifestAction's action key fingerprints the manifest content, which names absolute execroot paths. Every recording has its own workspace copy and output base, so the hash differs; the normalised manifest bytes (outputs surface), argv, inputs and outputs still match.
- excluded, workerd, recorded by neither side: 5 surfaces (0 reference items): build_stderr, execlog, bep, outputs, tests
