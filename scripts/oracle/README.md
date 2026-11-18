# scripts/oracle: recorder, comparer, scoreboard

The referee for Blaze. `record` runs a Bazel-compatible binary (the pinned oracle, Bazel 9.3.2,
or a Blaze build) on the corpus projects (`corpus/projects.json`) and stores normalised,
structured outputs. `compare` diffs two recordings item by item. `scoreboard` scores a recording
against the oracle reference `oracle-9.3.2`. Python 3 standard library only. Every tool prints
usage with `help`.

```
scripts/oracle/record --binary ~/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64 \
    --label oracle-9.3.2 --project all
scripts/oracle/record --binary <blaze> --label blaze-<rev> --project all
scripts/oracle/compare oracle-9.3.2 blaze-<rev> [--project re2] [--surface aquery_actions]
scripts/oracle/scoreboard blaze-<rev>
python3 -I -m unittest discover -s scripts/oracle      # normaliser and comparer tests
```

## Recording

For each project, `record`:

1. copies the read-only checkout (`~/.cache/blaze/corpus/<project>`) to
   `~/.cache/blaze/work/<label>/<project>` with `rsync -a --delete`, minus `bazel-*` and `.git`,
   then restores every modified, untracked or ignored path to the checkout's git `HEAD` (the
   corpus checkouts carry a `MODULE.bazel.lock` written by earlier runs; the copy does not);
2. stops any server and deletes the per-label output user root
   `~/.cache/blaze/oracle-out/rec-<label>` (every recording starts from a fresh output base and
   install base; the repository cache `~/.cache/blaze/repo-cache` is shared);
3. runs the steps below with startup options `--output_user_root=<that root> --nohome_rc
   --nosystem_rc`, command options `--repository_cache=... --color=no --curses=no`, environment
   `HOME` and `PATH=/usr/bin:/bin` only, inside one `systemd-run --user --scope -p MemoryMax=12G
   -p MemorySwapMax=0` (the Bazel server lives in the scope; `--no-scope` disables it).

| step | commands | surfaces |
|---|---|---|
| mod | `mod graph --output=json`, then a copy of `MODULE.bazel.lock` | `mod_graph`, `lockfile` |
| build | the project command (`build //...` / `test //:all //python:all`) with `--execution_log_json_file`, `--execution_log_sort`, `--build_event_json_file`; then a hash walk of `bazel-bin` and `bazel-testlogs` and a copy of every `test.xml` | `build_stderr`, `execlog`, `bep`, `outputs`, `tests` |
| query | `query P` and `query deps(P)`, each `--output=label_kind` and `--output=streamed_jsonproto` | `query_label_kind`, `query_jsonproto` |
| cquery | `cquery deps(P) --output=label_kind`, `cquery P --output=jsonproto` | `cquery_label_kind`, `cquery_jsonproto` |
| aquery | `aquery deps(P) --output=jsonproto --include_param_files --include_file_write_contents` | `aquery_actions` |
| (always) | `info output_base execution_root install_base` (paths for the normaliser), `shutdown` | `exit_codes` |

`P` is the project's target patterns joined with ` + `. The build runs right after `mod graph`,
on a fresh server, so its counters do not depend on the query steps. `--surfaces` selects steps;
skipped steps make their surfaces `SKIPPED` (not compared).

A command that fails does not abort the run. Each surface whose output is missing or
unparseable is stored as `FAILED` with the error and the stderr tail of its commands; every exit
code is itself an item of `exit_codes`. This is how a Blaze that lacks a flag still gets scored
on everything else.

Layout of `~/.cache/blaze/oracle-records/<label>/<project>/`:

- `manifest.json`: binary path and sha256, checkout commit, workspace fix-ups, the normaliser
  context (paths, user, host), every command line with exit code and duration, item counts per
  surface, and the sha256 of every file below.
- `raw/`: outputs exactly as the binary wrote them (stdout/stderr per command, exec log, BEP,
  lockfile, output-tree walk, `testxml/`).
- `norm/`: normalised outputs for people to read and diff: `mod_graph.json`, `MODULE.bazel.lock`,
  `query_*.txt|jsonl`, `cquery_*`, `aquery_deps_jsonproto.json`, `aquery_actions.json` (the
  per-action summary), `execlog.json`, `bep.json`, `outputs.json`, `tests.json`,
  `build.stderr.txt`.
- `items/<surface>.json`: `{status, items: {key: value}}`, what `compare` reads.

`record --renormalise --label L --project P` rebuilds `norm/` and `items/` from `raw/` without
running anything; use it after changing the normaliser or the extractors.

For label `oracle-9.3.2`, `record` also writes `corpus/oracle/<project>.json` (committed): item
counts per surface, exit codes, timings and the sha256 of every `norm/` and `items/` file.

## Surfaces and item keys

| surface | item key | value |
|---|---|---|
| `exit_codes` | command name | exit code |
| `mod_graph` | module key (`<root>`, `name@version`) | the node, with dependency lists reduced to keys (an `unexpanded` repeat never overrides the expanded node) |
| `lockfile` | `__bytes_sha256__`; `topkey[subkey]` for object-valued top-level keys (`registryFileHashes[url]`, `moduleExtensions[ext]`, ...); `topkey` otherwise | the value |
| `query_label_kind` | `pat <label>` / `deps <label>`; `__order__ pat|deps` | kind; the output order |
| `query_jsonproto` | `pat <label>` / `deps <label>`; `__order__ ...` | the target message |
| `cquery_label_kind` | `deps <label> (<config>)` | kind |
| `cquery_jsonproto` | `target <label> (<checksum>)`, `configuration <checksum>` | result (with `configurationId` replaced by the checksum), configuration (without its ordinal id) |
| `aquery_actions` | primary output exec path (` #2` on a collision) | mnemonic, target, rule class, configuration, aspects, actionKey, argv, env, executionInfo, sorted input and output path sets (depsets and path fragments resolved), param files, file-write and template contents |
| `build_stderr` | normalised summary line | count |
| `execlog` | first listed output (spawns without outputs: mnemonic + argv); `__order__` | the SpawnExec without `metrics`/`walltime` |
| `bep` | the canonical event id | the normalised event |
| `outputs` | `bin/<path>` / `testlogs/<path>` | file: sha256 of path-normalised bytes, executable bit, whether normalisation changed it; symlink: normalised target; empty dir; test logs: existence only |
| `tests` | test label | BEP overall status, run/shard/attempt statuses, and `[suite, case, status]` from `test.xml` |

Proto ids (aquery artifacts, depsets, path fragments, configurations; cquery configuration ids;
BEP named sets) never appear in keys or values: they are resolved to the paths, files and
checksums they stand for, so a binary that numbers them differently still matches.

## Normalisation

Generic (every string, `normalise.Context.text`; file contents via `Context.bytes`, paths only):

- absolute paths become placeholders, the most specific first: `<EXECROOT>`, `<OUTPUT_BASE>`,
  `<INSTALL_BASE>`, `<OUTPUT_USER_ROOT>`, `<WORKSPACE>`, `<RECORD_DIR>`, `<BINARY>`,
  `<BINARY_DIR>`, `<HOME>`;
- `<OUTPUT_BASE>/sandbox/<strategy>/<N>/` sandbox numbers become `<N>`;
- UUIDs become `<UUID>`, ISO-8601 timestamps `<TIMESTAMP>`;
- the user name and short host name, as whole words, become `<USER>` and `<HOST>`.

Per format (`surfaces.py`):

- stderr summary: only `INFO`/`WARNING`/`ERROR`/`DEBUG` lines and the test summary; drop
  progress, `Invocation ID`, `Elapsed time`, `INFO: From ...` banners; durations become `<DUR>`,
  PIDs `<PID>`; lines are counted, not ordered (their order depends on scheduling).
- exec log: drop `metrics` and `walltime`; digests of spawn outputs under `testlogs/` become
  `<timing-dependent>`.
- BEP: drop `progress` events and progress children; inline named sets of files into
  `targetCompleted` output groups (sorted); drop every key that holds wall-clock values (names
  ending in `Millis`, `Ms`, `Nanos`, `Duration`, `Time`, `Epoch`, `testAttemptStart...`); the
  client's `--startup_time`, `--command_wait_time`, `--extract_data_time`, `--connect_time` values
  become `<DUR>`; `started` loses `uuid`, `serverPid`, `host`, `user`; `workspaceStatus` values of
  `BUILD_HOST`, `BUILD_USER`, `BUILD_TIMESTAMP`, `FORMATTED_DATE` become `<KEY>`;
  `buildToolLogs` keep only log names; `pattern.expanded.testSuiteExpansions` are sorted (suite and
  test labels; Bazel emits them from an unordered set); `--default_override` arguments and
  options (the client's `.bazelrc` lines) are stable-sorted by `<rc index>:<command>[:config]`
  group within each contiguous run (the client emits the groups in hash-map order; order within
  a group is kept); `buildMetrics` drop memory, timing, cumulative, network, worker,
  dynamic-execution and artifact metrics; `testResult` drops `timingBreakdown` and the
  digests and lengths of test outputs; children lists are sorted.
- tests: `0x...` memory addresses in test case names (Python parameterized tests print
  function reprs) become `0x<ADDR>`.
- outputs: every regular file under `bazel-testlogs` (`test.log`, `test.xml`,
  `test.cache_status`, `test.outputs/`) is recorded by existence only: these carry run times and
  test logs. Their meaning is compared by the `tests` surface (statuses from BEP; suite, case and
  result from `test.xml`, ignoring times and `system-out`).
- order: items are compared as sets. Output order is compared only where it is deterministic
  (`__order__` items for query and the sorted exec log).

## Allow list

`scripts/oracle/allow.json` lists residual differences between two recordings of real Bazel
that normalisation cannot remove. An entry:

```
{"id": "...", "projects": "*" | ["re2"], "surface": "aquery_actions",
 "item": "<regex searched in the item key>", "kinds": ["DIFF" | "MISSING" | "EXTRA"],
 "only_fields": ["<regex fully matching a differing JSON path>", ...],
 "max_items": <optional cap per project and surface>,
 "condition": "...", "reason": "..."}
```

The condition is checked on every run: a DIFF is excused only if every differing leaf path
matches one of `only_fields` (a DIFF entry must name them), and an entry that would excuse more
than `max_items` items excuses none of them and fails the comparison. Excused items are
`ALLOWED` in `compare` and `excluded` in `scoreboard`. They apply equally to Blaze recordings, so
keep entries narrow. Current entries:

- `runfiles-manifest-action-key`: the action key of runfiles manifest actions fingerprints
  manifest content that names absolute execroot paths, which differ per recording. Only
  `actionKey` may differ.
- `build-metrics-volatile-counters`: in BEP `buildMetrics`, the per-SkyFunction
  evaluated/built/changed/cleaned/dirtied counts (and their order), the post-invocation Skyframe
  node count and the action cache `sizeInBytes` vary between identical runs. The other counters in
  the event are compared.

Normalisation and the allow list were derived from three oracle recordings (`oracle-9.3.2`,
`oracle-9.3.2-rerun`, `oracle-9.3.2-check`); the third found the test-suite expansion order.

The re2 `query deps(...)` commands exit 3 on the oracle: `deps()` follows every `select()`
branch, including google_benchmark's `@libpfm+`, which is not in the repository cache and whose
only source URL (netcologne.dl.sourceforge.net) no longer resolves. The query step therefore runs
with `--keep_going` and records the partial result; exit code 3 is compared like any other. Seeding
the repository cache with that archive would make the result complete (and change the recording).

## Oracle against itself

The oracle is recorded twice, as `oracle-9.3.2` (the reference) and `oracle-9.3.2-rerun`, each
with a fresh output user root (the second reuses the repository cache), and
`compare oracle-9.3.2 oracle-9.3.2-rerun` must print `100% MATCH`. Any new residual difference
is normalised away (if it is identity, time or ordering) or added to the allow list with a
checked condition.

## Things that make surfaces hard to compare

- Configuration checksums appear in cquery keys, BEP ids and aquery configurations. If Blaze's
  build options differ in any way, every configured item moves at once (MISSING + EXTRA).
- `actionKey` (aquery) and the exec log's input digests are hashes: one wrong byte anywhere in an
  action's key computation or an input file shows only as a different hash.
- Test logs and `test.xml` bodies are timing-dependent and compared only through `tests`.
- BEP `buildMetrics` and the build stderr `INFO: Analyzed N targets (P packages loaded, T
  targets configured)` line count Skyframe work; they match oracle against itself on a fresh
  server, but they are internal counters Blaze must reproduce exactly to match.
