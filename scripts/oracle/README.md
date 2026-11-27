# scripts/oracle: recorder, comparer, scoreboard

The referee for Blaze. `record` runs a Bazel-compatible binary (the pinned oracle, Bazel 9.3.2,
or a Blaze build) on the corpus projects (`corpus/projects.json`) and stores normalised,
structured outputs. `compare` diffs two recordings item by item. `scoreboard` scores a recording
against the oracle reference `oracle-9.3.2`. Python 3 standard library only. Every tool prints
usage with `help`.

```
scripts/oracle/record --binary ~/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64 \
    --label oracle-9.3.2 --project all --force      # the reference exists; replacing it needs --force
scripts/oracle/record --binary <blaze> --label blaze-<commit> --project all
scripts/oracle/compare oracle-9.3.2 blaze-<commit> [--project re2] [--surface aquery_actions]
scripts/oracle/scoreboard blaze-<commit> [--markdown | --json] [--write [--allow-dirty "REASON"]]
python3 -I -m unittest discover -s scripts/oracle      # normaliser, comparer and recorder tests
```

Modules: `normalise.py` and `surfaces.py` (raw output to items), `oracle_compare.py` (statuses,
allow list, denominators), `oracle_report.py` (what the tools print, the score document),
`oracle_common.py` (paths, provenance); `fixture_records.py` is test support.

This directory, `corpus/` and `docs/state/` are protected paths
(`scripts/gate/protected-paths.txt`): they change only in a batch that names them.

## What counts as a pass

Only `MATCH` is a pass. `compare` exits 0 only when every project/surface in scope is `MATCH`
and at least one was compared.

- The scope is every project recorded under either label and every surface. A project or
  surface that the candidate did not record is `MISSING`, and each of its reference items counts
  as failed; one that only the candidate has is `NO_REFERENCE`. Both fail. `--project` and
  `--surface` restrict the scope explicitly; the result line then says `[restricted to ...]`, what
  was left out is listed with its item count, and the report file gets its own name.
- A surface recorded by neither side is `NOT_RECORDED`: excluded, and listed as excluded.
- A surface that a side could not produce is `FAILED`. So is one whose `items/` file does not
  match the sha256 in its `manifest.json`; for the reference, the file must also match the sha256
  committed in `corpus/oracle/<project>.json`.
- Zero items on both sides is `MATCH (empty)` only if the reference is genuinely empty and both
  ran: the commands the surface comes from were run on both sides, succeeded on the reference and
  exited with the same code on the candidate. Otherwise `FAILED`.
- A surface named by a declared exception of a recording (below) is compared and marked `*`, and
  never passes.
- An allow-list entry that excuses more items than its `max_items` for the project excuses none.
- A DIFF item is excused only when every one of its differing JSON paths matches the entry's
  `only_fields`: `diff_paths` collects all of them, without a limit (audit R2A-1: a limit of 200
  let 200 allowed paths hide one that was not allowed). Only the printed and stored list is cut
  (50 paths kept per item, `paths_total` gives the count).

`compare` and `scoreboard` print, for both sides and each project, the binary's sha256 and path,
the recording date and the Blaze repository commit with its dirty flag; then the denominators in
reference items: `total = matching + allowed + failed`, `compared`, `extra`, the count of
surfaces per status, and what was excluded and why. `match%` is `matching / (total - allowed)`,
rounded down, so a missing surface lowers it and `100.0` means every item.

`scoreboard <label> --write` writes `corpus/scores/<label>.json` and `.md`. They depend only on
the two recordings, the referee (`referee_sha256`: `normalise.py`, `surfaces.py`,
`oracle_common.py`, `oracle_compare.py`, `allow.json`; `oracle_common.py` since batch 4, because
it holds the daemon-mode trigger and `server_states`) and `--allow-dirty`, so they can be regenerated and diffed. A
score is committed with the commit that accepts it; `scripts/gate/compare-scores` holds a later
score against it.

- `--write` refuses (exit 2, nothing written) a candidate recording that cannot back an accepted
  score: no recorded commit, a dirty tree, or a binary whose origin is not verified
  (`binary_origin.commit_verified`, below; recordings made before that field existed count as
  not checked). `--allow-dirty "REASON"` writes anyway and stores the reason (`allow_dirty`);
  the problems are always in the score (`recording_problems`) and printed.
- Every row holds `matching_keys`: the first 10 hex digits of sha256 of each matching reference
  item key, sorted, space-separated. `compare-scores` uses them to require that every item that
  matched still matches (audit R2A-4), not only that the count did not drop.

## Recording

For each project, `record`:

1. copies the read-only checkout (`~/.cache/blaze/corpus/<project>`) to
   `~/.cache/blaze/work/<label>/<project>` with `rsync -a --delete`, minus `bazel-*` and `.git`,
   then restores every modified, untracked or ignored path to the checkout's git `HEAD` (the
   corpus checkouts carry a `MODULE.bazel.lock` written by earlier runs; the copy does not);
2. stops any server and deletes the per-label output user root
   `~/.cache/blaze/oracle-out/rec-<label>` (every recording starts from a fresh output base and
   install base; the repository cache `~/.cache/blaze/repo-cache` is shared);
   Before any of this, `record` refuses a label that is already in use (see "Labels and commit
   binding");
3. runs the steps below with startup options `--output_user_root=<that root> --nohome_rc
   --nosystem_rc` (and `--max_idle_secs=N`, see "Server mode"), command options
   `--repository_cache=... --color=no --curses=no`, environment `HOME` and `PATH=/usr/bin:/bin`
   only (and `BLAZE_SERVER_MODE`, see "Server mode"), inside one `systemd-run --user --scope -p
   MemoryMax=12G -p MemorySwapMax=0` (the Bazel server lives in the scope; `--no-scope` disables
   it).

| step | commands | surfaces |
|---|---|---|
| mod | `mod graph --output=json`, then a copy of `MODULE.bazel.lock` | `mod_graph`, `lockfile` |
| build | the project command (`build //...` / `test //:all //python:all`) with `--execution_log_json_file`, `--execution_log_sort`, `--build_event_json_file`; then a hash walk of `bazel-bin` and `bazel-testlogs` and a copy of every `test.xml` | `build_stderr`, `execlog`, `bep`, `outputs`, `tests` |
| query | `query P` and `query deps(P)`, each `--output=label_kind` and `--output=streamed_jsonproto` | `query_label_kind`, `query_jsonproto` |
| cquery | `cquery deps(P) --output=label_kind`, `cquery P --output=jsonproto` | `cquery_label_kind`, `cquery_jsonproto` |
| aquery | `aquery deps(P) --output=jsonproto --include_param_files --include_file_write_contents` | `aquery_actions` |
| (always) | `info output_base execution_root install_base` (paths for the normaliser), `shutdown` | `exit_codes` |

`P` is the project's target patterns joined with ` + `. The build runs right after `mod graph`,
on the server that `mod graph` started (a warm server, not a fresh one: audit 3, B1), so its
counters do not depend on the query steps. `--surfaces` selects steps;
skipped steps make their surfaces `SKIPPED`, which `compare` reports as `MISSING` (a failure)
when the reference has them.

A command that fails does not abort the run. Each surface whose output is missing or
unparseable is stored as `FAILED` with the error and the stderr tail of its commands; every exit
code is itself an item of `exit_codes`. This is how a Blaze that lacks a flag still gets scored
on everything else.

Layout of `~/.cache/blaze/oracle-records/<label>/<project>/`:

- `manifest.json`: binary path and sha256, the date, `blaze_repo` (commit and dirty state of the
  Blaze checkout the recorder ran from), `binary_origin` (whether the binary can come from that
  commit, below), `replaced`/`forced` (what `--force` overrode), checkout commit of the project, workspace fix-ups, the
  normaliser context (paths, user, host), every command line with exit code and duration, item
  counts per surface, `extractor_sha256` (the `normalise.py`, `surfaces.py` and, since batch 4,
  `oracle_common.py` that built `norm/` and `items/`), and the sha256 of every file below.
- `raw/`: outputs exactly as the binary wrote them (stdout/stderr per command, exec log, BEP,
  lockfile, output-tree walk, `testxml/`).
- `norm/`: normalised outputs for people to read and diff: `mod_graph.json`, `MODULE.bazel.lock`,
  `query_*.txt|jsonl`, `cquery_*`, `aquery_deps_jsonproto.json`, `aquery_actions.json` (the
  per-action summary), `execlog.json`, `bep.json`, `outputs.json`, `tests.json`,
  `build.stderr.txt`.
- `items/<surface>.json`: `{status, items: {key: value}}`, what `compare` reads.

`record --renormalise --label L --project P` rebuilds `norm/` and `items/` from `raw/` without
running anything; use it after changing the normaliser or the extractors. The new directories are
built in `<record>/.stage` and renamed into place when complete (other tools and tests read the
reference while it is rebuilt), and the manifest gets a `renormalised` entry. `compare` warns when
a recording's `extractor_sha256` is not the current one.

For label `oracle-9.3.2`, `record` also writes `corpus/oracle/<project>.json` (committed): item
counts per surface, exit codes, timings, declared exceptions and the sha256 of every `norm/` and
`items/` file. `compare` checks the reference against these hashes on every run, so a change to
the reference is a change to a committed, protected file.

## Server mode

Bazel runs every command of a recording on one server: `mod graph` starts it (cold), every later
command finds it (warm), and the recorder's final `shutdown` stops it. Blaze by default runs one
process per command, so every Blaze command is cold (decision D12, audit 3 B1).
`record --blaze-server daemon` runs Blaze with `BLAZE_SERVER_MODE=daemon`, its persistent
server: the same command sequence on one server, as Bazel. That mode also passes the startup
option `--max_idle_secs=60` (`--max-idle-secs N` changes it; it can be given for any binary, so
an oracle recording can use the same startup options) and ends with `shutdown`.

What the manifest records (every recording made since batch 3):

- per command, `server`: `pid_before` and `pid_after` (the live pid in
  `<output base>/server/server.pid.txt` whose command line has an argument naming the output
  user root or a path below it, the root ending at `/` or at the end of the argument),
  `start_line` (the client printed "Starting local ... server"), and `state`: `warm` when the
  same server was alive before and after and the client started none, `cold` otherwise;
- per recording, `server`: `blaze_server_mode` (null: unset), `max_idle_secs`, `stopped_before`
  (servers of an earlier run in the output user root, stopped before it is wiped; a pid file
  whose process does not name the root is ignored), `shutdown_exit_code`, and
  `alive_after_shutdown` (servers still alive 10 s after `shutdown`; they are stopped then).

`compare` and `scoreboard` print, per side and project, the mode and which commands ran cold or
warm (for recordings made before batch 3, read from `raw/<command>.stderr`: cold when the client
printed its start line; marked `from stderr`), and warn when a command ran warm on one side and
cold on the other, or when the final shutdown failed or left a server alive. The score document
carries them in `provenance` (`server_mode`, `server_states`, `shutdown`,
`strict_server_rules`).

The daemon-mode rules (batch 3b; trigger widened in batch 4, audit of batch 3b C1) apply to a
candidate whose manifest has `blaze_server_mode` `daemon`, or one of whose commands did not start
a server by any source, whatever mode was asked for (a binary whose default is the server gets
them too; `oracle_common.strict_server_rules`): `server.start_line` false or `server.state`
`warm` in the manifest, or a `raw/<command>.stderr` without the client's start line (read even
when the manifest has states, so the pid file is not the only witness). One-process Blaze prints
the start line on every command and is not judged by them; Bazel is, after its first command,
so the oracle against itself is too (its three recordings agree in states and in the narrowed
allow fields). Under them:

- a command that ran cold on one side and warm on the other makes its `exit_codes` item a DIFF
  (both values become `{"exit_code", "server_state"}`; path `server_state`);
- a final `shutdown` that exited non-zero, left a server alive or was not recorded adds the
  candidate-only item `__shutdown__` to `exit_codes` (EXTRA). A recording without any `server`
  record (made before batch 3: the recorder did not record the shutdown then; `oracle-9.3.2-rerun`
  and `-check`) is not given the item; `compare` prints a note that its shutdown is not checked
  (`oracle_common.shutdown_recorded`). A `server` record without the shutdown fields still fails;
- allow-list entries use `only_fields_in_daemon_mode` (below).

Both are decided at compare time (`oracle_compare.server_checked_exit_codes`), so no
recording's items change. Other candidates (one process per command, and recordings made before
batch 3) get the warning only, and score as before.

Masks and allow-list entries that exist only because of the cold/warm difference, and what
daemon mode does with them:

- the client's server-start line in `build_stderr` (it has no prefix, so the stderr summary
  drops it; Blaze in one-process mode prints it on every command): kept and compared for a
  recording judged by the daemon-mode rules (`normalise.SERVER_START_RE`; batch 4, audit of
  batch 3b A2: the same `strict_server_rules` as at compare time, applied by the extractor to the
  recording's own manifest and `raw/`); dropped otherwise (one-process recordings), as before.
- Four of the nine `only_fields` of `build-metrics-volatile-counters`: `builtValues`,
  `changedValues`, `cleanedValues` and `postInvocationSkyframeNodeCount` (and, since batch 4,
  `dirtiedValues`, which the extractor now sorts; see "Normalisation"). In `oracle-9.3.2`,
  `-rerun` and `-check` they are equal (per SkyFunction and in order; node count brotli 7305, re2
  23543); only the `evaluatedValues` counts, the order of `dirtiedValues` and the re2 action
  cache `sizeInBytes` vary. So they excuse nothing but a one-process Blaze, whose build runs cold
  while the oracle's runs warm (audit 3, B1: they "hide other cold-against-warm differences").
  The entry's `only_fields_in_daemon_mode` leaves them out: for a candidate recorded in daemon
  mode only the fields the oracle varies in are excused. One-process candidates and the oracle
  against itself use `only_fields`, as before.
- `runfiles-manifest-action-key` (absolute paths) is not about the server and stays active.
- Added for daemon mode, not cold/warm masks: the recorder's `--max_idle_secs` (startup option
  lists and its `--option_sources` pair) and `--client_env=BLAZE_SERVER_MODE=daemon` are
  removed from the BEP command lines on both sides (see "Normalisation"), because the reference
  was recorded without them. Any other startup option, source or client variable is compared.
- Not masks: `--restart_reason` (one-process Blaze sends `no_daemon` with every command; the
  oracle's build on a warm server sends none) and the re2 `compatibility_level` warning that a
  cold Blaze build prints again are compared in both modes; in daemon mode they must match.

## Staging a referee change

A change to `normalise.py` or `surfaces.py` changes the items of every recording, the
reference's included, and the reference's items are checked against the committed
`corpus/oracle/<project>.json`. To renormalise and score without touching the shared recordings
or the committed files, set `BLAZE_ORACLE_STAGE=<dir>`: the tools then read and renormalise
recordings in `<dir>/oracle-records`, check the reference against `<dir>/corpus-oracle`, and
`scoreboard --write` writes to `<dir>/scores`. Copy what you need there first (`raw/` may be
hard-linked: renormalising only reads it). Every report made that way says so in a `staged:`
warning, which a score document keeps, and in the top-level field `staged` of the score
document (`{"stage", "records", "summaries"}`; `null` for an unstaged score; scores written
before batch 3b lack the field). After the referee change is accepted, root
renormalises the shared recordings (`record --renormalise`), which writes the summaries, and
commits them with the referee.

## Labels and commit binding

A score names a label, and a label names one binary from one tree:

- `manifest.json` records `blaze_repo`: `commit` (`git rev-parse HEAD`), `dirty` (anything in
  `git status --porcelain`), `dirty_paths`, and `dirty_diff_sha256` (sha256 over `git diff HEAD
  --binary` plus path and content hash of every untracked file; null for a clean tree). It is
  the checkout `record` ran from: build the binary from that checkout. With `binary_sha256`,
  `binary` and `created` this is what `compare` and `scoreboard` print for both sides.
- `binary_origin` (audit R2A-2): `commit_verified` is true only when the binary's real path lies
  inside `<recorder's checkout>/target` (AGENTS.md forbids `CARGO_TARGET_DIR`) and the binary is
  not older than that checkout's HEAD commit; `reasons` says why not. `record` warns when it is
  false (always for the oracle binary, which is not built here). The commit is not stamped into
  the binary (that needs a Rust change), so this is a file-system check: it catches a binary from
  another checkout or an older build, not a binary rebuilt from a dirty tree and then cleaned.
  `compare` and `scoreboard` print it with the commit (`binary from that commit verified`,
  `binary origin NOT VERIFIED`, `binary origin not checked`).
- `record` refuses to write to a label/project that exists (even an unfinished directory), and
  refuses to add a project to a label whose other projects were recorded with another binary.
  `--force` overrides both; the new manifest then has `replaced` (date, binary and tree of what
  it replaced) and `forced` (the conflicts it overrode), and `compare`/`scoreboard` print both
  as warnings (audit R2A-5).
- Label convention for Blaze: `blaze-<commit, 12 hex digits>`, with `-dirty-<first 8 of
  dirty_diff_sha256>` for an uncommitted tree. An accepted score comes from a clean tree.
- `compare` warns when a label mixes binaries or commits across projects, when the candidate
  has no recorded commit (recordings older than this rule), and when it was recorded from a dirty
  tree.

## Declared exceptions

A recording that was not produced by the protocol above says so in its manifest:

```
"exceptions": [{"id": "...", "kind": "spliced", "surfaces": ["aquery_actions", "exit_codes"],
                "explanation": "...", ...}]
```

`id`, `kind`, `explanation` and a non-empty list of `surfaces` are required (`record` refuses a
malformed entry; `compare` treats one as covering every surface). `compare` and `scoreboard`
print every exception of both sides; the named surfaces are still compared, are marked `*`, and
never count as a pass. For the reference label the exceptions are those of the committed
summary `corpus/oracle/<project>.json` together with the manifest's (audit R2A-5: the manifest in
`~/.cache` is not hash-anchored, so an exception deleted there still applies); a difference
between the two is printed as a warning, and so is a summary that differs from its version at
git `HEAD`. `record --renormalise` keeps the field and copies it to the committed
summary. To declare one, add it to the record's `manifest.json` and renormalise.

The one exception today is `workerd-aquery-spliced` in `oracle-9.3.2/workerd`: on a fresh output
base the oracle's `aquery deps(//src/workerd/server:workerd)` exits 8 (build interrupted), which
is what label `oracle-9.3.2-workerd` holds. In `oracle-9.3.2/workerd` the exit code was changed
by hand to 0 and the output replaced by one from a built output base (`protocol_exit_code: 8`,
`stored_exit_code: 0`, `protocol_recording`). It is useful for comparing actions; it is not
evidence that exit 0 is right.

## Surfaces and item keys

| surface | item key | value |
|---|---|---|
| `exit_codes` | command name | exit code |
| `mod_graph` | module key (`<root>`, `name@version`) | the node, with dependency lists reduced to keys (an `unexpanded` repeat never overrides the expanded node) |
| `lockfile` | `__bytes_sha256__`; `topkey[subkey]` for object-valued top-level keys (`registryFileHashes[url]`, `moduleExtensions[ext]`, ...); `topkey` otherwise | the value |
| `query_label_kind` | `pat <label>` / `deps <label>`; `__order__ pat|deps` | kind; the output order |
| `query_jsonproto` | `pat <label>` / `deps <label>`; `__order__ ...` | the target message |
| `cquery_label_kind` | `deps <label> (<config>)`; `__order__ deps` | kind; the item keys in output order |
| `cquery_jsonproto` | `target <label> (<checksum>)`, `configuration <checksum>`; `__order__ results`, `__order__ configurations` | result (with `configurationId` replaced by the checksum), configuration (without its ordinal id); the result keys in output order, `[id, checksum]` of the configurations in output order |
| `aquery_actions` | primary output exec path (` #2` on a collision); `__order__ actions`, `__order__ action ids`, `__order__ <list>` | mnemonic, target, rule class, configuration, aspects, actionKey, argv, env, executionInfo, sorted input and output path sets (depsets and path fragments resolved), param files, file-write and template contents; order items below |
| `build_stderr` | normalised summary line | count |
| `execlog` | first listed output (spawns without outputs: mnemonic + argv); `__order__` | the SpawnExec without `metrics`/`walltime` |
| `bep` | the canonical event id | the normalised event |
| `outputs` | `bin/<path>` / `testlogs/<path>` | file: sha256 of path-normalised bytes, executable bit, whether normalisation changed it; symlink: normalised target; empty dir; test logs: existence only |
| `tests` | test label | BEP overall status, run/shard/attempt statuses, and `[suite, case, status]` from `test.xml` |

Proto ids (aquery artifacts, depsets, path fragments, configurations; cquery configuration ids;
BEP named sets) never appear in the keys or values of the per-target, per-action and per-event
items: they are resolved to the paths, files and checksums they stand for, so that each action
is judged on its own whatever the numbering. Order and numbering are judged separately, by the
`__order__` items.

### Order items

Where Bazel's output order is deterministic it is compared, as one item per list, so that a
different order costs one item and does not hide which targets and actions are right:

| item | value |
|---|---|
| `query_*`: `__order__ pat`, `__order__ deps` | labels in output order |
| `cquery_label_kind`: `__order__ deps` | item keys in output order |
| `cquery_jsonproto`: `__order__ results` | result keys in output order |
| `cquery_jsonproto`: `__order__ configurations` | `[id, checksum]` per configuration, in output order |
| `aquery_actions`: `__order__ actions` | `{"order": [action keys in output order]}` |
| `aquery_actions`: `__order__ action ids` | `{"order": [[key, targetId, configurationId, primaryOutputId, inputDepSetIds, outputIds, aspectDescriptorIds], ...]}` |
| `aquery_actions`: `__order__ targets`, `artifacts`, `depSetOfFiles`, `pathFragments`, `configuration`, `ruleClasses`, `aspectDescriptors` | `{"order": <the list as printed, ids included>}` (only lists the output has) |
| `execlog`: `__order__` | spawn keys in (sorted) log order |

aquery order values are objects, so that every item of that surface is an object. Evidence that
the oracle is deterministic here, from `oracle-9.3.2`, `-rerun` and `-check`, both projects:
`raw/cquery_deps_label_kind.stdout` is byte-identical in all three; `norm/cquery_jsonproto.json`
is byte-identical (results, configurations and their ids); in `norm/aquery_deps_jsonproto.json`
the lists `targets`, `artifacts`, `depSetOfFiles`, `pathFragments`, `configuration` and
`ruleClasses` are identical with their ids, and `actions` is identical position by position
except for the `actionKey` of the runfiles manifest actions (2 in brotli, 54 in re2: the
allow-list entry). So the per-action items plus the order items decide the whole normalised
aquery container. Not compared as order: the BEP event sequence and the build stderr line order
(scheduling-dependent; counted or keyed instead).

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
  PIDs `<PID>`; lines are counted, not ordered (their order depends on scheduling). The client's
  server-start line is kept only under the daemon-mode rules (see "Server mode").
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
  a group is kept); the value of `--install_md5` (the installation's identity: Bazel's md5 of
  its embedded files, Blaze's own install key) becomes `<INSTALL_MD5>` when it is 32 lowercase
  hex digits (batch 4, A7; any other value stays and differs), in the string form, in
  `optionValue` and inside the printed `combinedForm`, which is otherwise kept; the option itself
  is compared; the startup option `--max_idle_secs` (set by the recorder in daemon mode, see
  "Server mode"; the reference was recorded without it) is removed from every startup-option
  list on both sides, the reference's default `--max_idle_secs=10800` included, together with
  the empty entry Bazel's client sends after each original startup option (`blaze.cc`
  `BlazeServer::Communicate` adds two `startup_options` per option; the BEP shows `{}`), and
  its `max_Uidle_Usecs:<source>` pair from the client's `--option_sources` value; likewise the
  recorder's `--client_env=BLAZE_SERVER_MODE=...` (the client forwards its environment; Bazel's
  would forward the variable too);
  `testResult` drops `timingBreakdown` and the digests and lengths of test outputs; children
  lists are sorted.
- BEP `buildMetrics` (referee batch 3; review 2026-11-25-reporting S1-1: dropping whole
  sub-messages let an empty `artifactMetrics` score as identical): `actionSummary`,
  `targetMetrics`, `packageMetrics`, `buildGraphMetrics`, `cumulativeMetrics` and
  `remoteAnalysisCacheStatistics` are compared without their wall-clock keys (as above);
  `artifactMetrics` counts are compared, and so is `sourceArtifactsRead.sizeInBytes`; the
  `sizeInBytes` of `outputArtifactsSeen`, `outputArtifactsFromActionCache` and
  `topLevelArtifacts` become `<path-dependent>` when they are a string of decimal digits (batch 4,
  A5; anything else stays and differs) (runfiles manifests name absolute paths and test
  logs carry times: the three oracle recordings differ there, brotli 7671064/7671088/7671088
  for `outputArtifactsSeen`); `memoryMetrics`, `timingMetrics`, `networkMetrics`,
  `workerMetrics`, `workerPoolMetrics` and `dynamicExecutionMetrics` only measure the machine and
  are compared by presence: which fields are set (values `<measured>`, a non-empty list
  `<list>`; an empty list, `null`, `0`, `"0"` and `false`, which proto3 JSON never prints for a
  set field, stay as they are: batch 4, A6). `buildGraphMetrics.dirtiedValues` is sorted by
  SkyFunction (batch 4, A8: Bazel prints it in an order that varies between identical runs).
  The `buildGraphMetrics` counters that vary between identical oracle runs, `evaluatedValues`
  among them by name (a declared deviation, decisions 2026-11-26), stay with the allow-list entry
  `build-metrics-volatile-counters`.
- tests: `0x...` memory addresses in test case names (Python parameterized tests print
  function reprs) become `0x<ADDR>`.
- outputs: every regular file under `bazel-testlogs` (`test.log`, `test.xml`,
  `test.cache_status`, `test.outputs/`) is recorded by existence only: these carry run times and
  test logs. Their meaning is compared by the `tests` surface (statuses from BEP; suite, case and
  result from `test.xml`, ignoring times and `system-out`).
- order: items are compared as sets. Output order is compared only where it is deterministic
  (`__order__` items for query, cquery, aquery and the sorted exec log; see "Order items").

## Allow list

`scripts/oracle/allow.json` lists residual differences between two recordings of real Bazel
that normalisation cannot remove. An entry:

```
{"id": "...", "projects": "*" | ["re2"], "surface": "aquery_actions",
 "item": "<regex searched in the item key>", "kinds": ["DIFF" | "MISSING" | "EXTRA"],
 "only_fields": ["<regex fully matching a differing JSON path>", ...],
 "max_items": {"brotli": 2, "re2": 54},
 "only_fields_in_daemon_mode": ["<regex>", ...],     (optional)
 "condition": "...", "reason": "...", "daemon_mode_reason": "..."}
```

The condition is checked on every run: a DIFF is excused only if every differing leaf path
matches one of `only_fields` (a DIFF entry must name them); for a candidate under the
daemon-mode rules, of `only_fields_in_daemon_mode` when the entry has one (see "Server mode").
An entry that excuses DIFF items with an empty or missing `only_fields`, or with an empty
`only_fields_in_daemon_mode`, is refused (an empty list would excuse every difference), and an
empty list excuses nothing. `max_items` is required: per
project, the number of items the entry excused when the oracle was compared with itself. A
project that is not named has the cap 0 (there is no oracle-vs-oracle evidence for it; workerd
today). An entry that would excuse more items than the cap excuses none of them and fails the
comparison; an allow list with an entry that lacks `max_items` is refused. Excused items are
`ALLOWED` in `compare` and `allowed` in `scoreboard`, never counted as matching. They apply
equally to Blaze recordings, so keep entries narrow. Current entries:

- `runfiles-manifest-action-key` (max brotli 2, re2 54): the action key of runfiles manifest
  actions fingerprints manifest content that names absolute execroot paths, which differ per
  recording. Only `actionKey` may differ.
- `build-metrics-volatile-counters` (max brotli 1, re2 1): in BEP `buildMetrics`, the
  per-SkyFunction evaluated/built/changed/cleaned/dirtied counts (and their order), the
  post-invocation Skyframe node count and the action cache `sizeInBytes` vary between identical
  runs. The other counters in the event are compared. In daemon mode only the evaluated counts,
  the action cache size and the JVM's `garbageMetrics` are excused (batch 3: the oracle's three
  runs vary in nothing else but the dirtied order, which the extractor sorts since batch 4).

Normalisation and the allow list were derived from three oracle recordings (`oracle-9.3.2`,
`oracle-9.3.2-rerun`, `oracle-9.3.2-check`); the third found the test-suite expansion order.

The re2 `query deps(...)` commands exit 3 on the oracle: `deps()` follows every `select()`
branch, including google_benchmark's `@libpfm+`, which is not in the repository cache and whose
only source URL (netcologne.dl.sourceforge.net) no longer resolves. The query step therefore runs
with `--keep_going` and records the partial result; exit code 3 is compared like any other. Seeding
the repository cache with that archive would make the result complete (and change the recording).

## Oracle against itself

The oracle is recorded three times, as `oracle-9.3.2` (the reference), `oracle-9.3.2-rerun` and
`oracle-9.3.2-check`, each with a fresh output user root (the later ones reuse the repository
cache), and

```
scripts/oracle/compare oracle-9.3.2 oracle-9.3.2-rerun --project brotli --project re2
scripts/oracle/compare oracle-9.3.2 oracle-9.3.2-check --project brotli --project re2
scripts/oracle/compare oracle-9.3.2-rerun oracle-9.3.2-check
```

must each print `100% MATCH` (26 surfaces, 13,916 reference items, 58 allowed). The first two
are restricted because the reference also holds workerd, which was recorded once; without the
restriction they fail with workerd `MISSING`, as they should. Any new residual difference is
normalised away (if it is identity, time or ordering) or added to the allow list with a checked
condition and a `max_items` from these runs. Run all three after every change to this directory.

## Things that make surfaces hard to compare

- Configuration checksums appear in cquery keys, BEP ids and aquery configurations. If Blaze's
  build options differ in any way, every configured item moves at once (MISSING + EXTRA).
- `actionKey` (aquery) and the exec log's input digests are hashes: one wrong byte anywhere in an
  action's key computation or an input file shows only as a different hash.
- Test logs and `test.xml` bodies are timing-dependent and compared only through `tests`.
- BEP `buildMetrics` and the build stderr `INFO: Analyzed N targets (P packages loaded, T
  targets configured)` line count Skyframe work; they match oracle against itself on a warm
  server (after `mod graph`), but they are internal counters Blaze must reproduce exactly to
  match, and a one-process Blaze counts them on a cold one.
