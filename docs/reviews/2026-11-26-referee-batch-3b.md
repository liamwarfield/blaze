# Referee batch 3b (2026-11-26): implementer's report for the auditor

Follow-up to the audit of batch 3 (`docs/reviews/2026-11-26-audit-referee-3.md`). Tree: working
tree on HEAD `a673e29b` (batch 3 committed as b94556a; HEAD moved while I worked; no commit
after b94556a touches `scripts/oracle`). Nothing is committed.

- referee sha256: `ab46a162fe33ea616779e75a08f6405af18323c9b87bd49c306d96b7f802fcf0` (was
  `4afe2977…`; only `oracle_compare.py` of the referee files changed)
- extractor sha256: `c215d2d7087c8ff0b2094a42201fd889a6a2e33a5709b8fba5d8413daa993f97`, the
  same as batch 3. `normalise.py`, `surfaces.py` and `allow.json` are unchanged, so no
  recording was renormalised.
- Unit tests (`python3 -I -m unittest discover -s scripts/oracle`): 159 run, 159 passed, 0
  failed, 0 skipped (148 before; 11 new; 2 existing tests adjusted, see A1/A2 and A3).
- Verification binary: `~/.cache/blaze/work/root-runs/blaze-ce81d5a`, sha256 `d6fb00d5a0b4…`.

`root-suite-5f68157` was inactive for the whole batch. The four recordings ran one at a time,
each in `systemd-run --user --collect --wait -p MemoryMax=12G -p MemorySwapMax=0
--slice=app-t3code-agents-jobs.slice` with `record --no-scope`. No git index command was used.

## Required items

### A1. In daemon mode a state mismatch or a failed shutdown fails the comparison

`oracle_compare.py`: new `server_checked_exit_codes()` and `SHUTDOWN_ITEM`, called from
`compare_labels` for the `exit_codes` surface of a candidate under the daemon-mode rules
(A2). It runs at compare time on copies of the loaded surfaces, so no recording's items
change.

- A command whose state differs between the two sides gets the value
  `{"exit_code": n, "server_state": s}` on both sides. That makes it a DIFF with path
  `server_state`, or `exit_code` as well when the codes differ too. A state that is unknown on
  one side counts as different.
- A final `shutdown` with a non-zero or missing exit code, or with a server left alive, adds the
  candidate-only item `__shutdown__` (EXTRA). The surface becomes DIFF and `compare-scores`
  reports its status falling from MATCH.
- One-process candidates are unaffected and keep the warning. Under strict rules the warning
  text says that the commands fail (`oracle_report.server_state_lines`).

Tests (`test_compare.DaemonRulesTest`):
- Fail on the committed code: `test_a_cold_command_fails_its_exit_code_item`,
  `test_a_failed_shutdown_fails_exit_codes` (three shapes: exit 1, survivor, not recorded).
- Guards, pass before and after: `test_equal_states_and_a_clean_shutdown_match`,
  `test_one_process_candidates_keep_the_warning`.

### A2. The strict rules follow the observed server

`oracle_common.py`: `observed_warm_server(manifest)` is true when `record` recorded a command
as warm (from the pid file). `strict_server_rules(manifest)` is mode `daemon` or an observed
warm server.

- `compare_labels` uses it for A1 and for the allow list: it passes `mode "daemon"` to
  `AllowList.match`, so `only_fields_in_daemon_mode` applies.
- `provenance` adds `strict_server_rules`, and the server line says
  `(daemon-mode rules)`.
- Recordings made before batch 3 have no recorded states, so they are never strict: the oracle
  self-checks and the base score as before.

Not done, and why: the extractor's start-line decision in `surfaces.s_build_stderr` still
follows the requested mode. Changing it changes the extractor hash. It is covered meanwhile: a
build that started a server is recorded `cold`, and under the strict rules that is a
`server_state` DIFF (A1). It is listed with the next extractor change below.

Tests:
- Fail on the committed code:
  `test_an_observed_warm_server_gets_the_strict_rules_without_the_mode` (mode unset, warm
  states: a cold build fails; plus unit cases of `strict_server_rules`) and
  `test_an_observed_warm_server_gets_the_daemon_allow_fields` (`builtValues` excused for a
  cold candidate, not for an observed-warm one).
- Adjusted: `ServerStateTest.test_daemon_mode_against_a_server_has_no_warning` now expects the
  `(daemon-mode rules)` marker.

### A3. An empty field list excuses nothing

`oracle_compare.py`:
- `AllowList.prepare` refuses an entry that excuses DIFF items when `only_fields` is empty or
  missing, or when `only_fields_in_daemon_mode` is present and empty.
- `AllowList.match` excuses nothing for a DIFF when the applicable list is empty (defence in
  depth).

Tests (`test_compare.EmptyFieldListTest`): `test_prepare_refuses_empty_lists` and
`test_match_excuses_nothing_without_fields`, both failing on the committed code.

Adjusted: `test_surfaces.DiffAndAllowTest.test_project_filter` used a DIFF entry without
`only_fields`, which is exactly the hole. It now names `only_fields: ["a"]` and tests the same
thing (the project filter).

### B1. The staged state is a field of the score

`oracle_report.py`: `staged_field()`, and the score document gains the top-level key `staged`:

- `null` for an unstaged score;
- `{"stage", "records", "summaries"}` (home-relative paths) under `BLAZE_ORACLE_STAGE`.

The `staged:` warning line stays. Tests (`test_compare.StagedFieldTest`): `test_unstaged` and
`test_staged`, both erroring on the committed code (no field).

What `scripts/gate/compare-scores` must check (for the gate's owner):

1. Refuse with exit 2 (`NOT COMPARABLE`) when the base or the new score has a non-null
   `staged`, and print its `stage` path.
2. For a score without the key (written before batch 3b, such as `referee3.json`): treat it as
   staged when any entry of `warnings` starts with `staged:`, and print a NOTE that the score
   predates the field. The committed `referee3.json` has no such line.
3. Do not take a missing key as proof of an unstaged score in any other way.

### B2. The root check matches the path component

`record`: new `names_path(cmdline, root)`, used by `live_servers`. One NUL-separated argument
must contain the root followed by `/` or by the end of the argument. `rec-blaze-x` no longer
matches `--output_base=…/rec-blaze-x-rerun/ob`.

Test: `test_record.ServerModeTest.test_a_server_of_another_label_with_a_longer_name_is_left_alone`
(a live process naming `<root>-rerun` behind a stale pid file in `<root>` is neither counted
nor stopped). It fails on the committed code, which would have stopped it.

### README

`scripts/oracle/README.md` covers the daemon-mode rules and their trigger, the
`server_state` and `__shutdown__` items, the `staged` field, the refusal of empty field lists,
and the exact root match.

## Base under this referee

`corpus/scores/blaze-ef66f55e904e.referee3b.json` and `.md` (new files; `referee3.json` is not
touched). Scored unstaged from the shared, already renormalised records with `scoreboard
--allow-dirty "<ruling 2026-11-19, rescored under referee batch 3b …>" --json/--markdown
--out`.

Against the committed `referee3.json`:
- `rows`, `totals` and `verdict` are identical (matching 13,850, allowed 56, failed 145,082).
- New: `staged: null`, and `provenance.*.strict_server_rules: false` on every side.
- Changed: `referee_sha256` and the `allow_dirty` text.
- Gone: three warnings that root's scoring of `referee3.json` carried ("corpus/oracle/*.json
  differs from its committed version at HEAD": it was scored before the summaries were
  committed).

`compare-scores` needs the base and the candidate scored by the same referee, so this is the
base for the verification below. Root may instead regenerate `referee3.json` itself. Oracle
against itself under this referee, on the shared records: three pairs, each exit 0 and
`100% MATCH`, 13,916 = 13,858 + 58.

## Verification (blaze-ce81d5a, against `referee3b.json`)

| label | mode | matching of 13,860 judged (brotli+re2) | `compare-scores` |
|---|---|---|---|
| `r3b-ce81d5a` | one process per command | 13,852; allowed 58 | PASS: rows 39, gained 2, regressed 0 |
| `r3bd-ce81d5a` | daemon | 13,858; allowed 56 | PASS: rows 39, gained 2, regressed 0 |

Scores: `~/.cache/blaze/work/referee-3b/scores/{r3b,r3bd}-ce81d5a.json`. Both have
`staged: null`.

- `r3b`, rows that change against the base:
  - brotli/bep: matching 37 → 38, allowed 0 → 1.
  - re2/bep: matching 109 → 110, allowed 0 → 1.
  - `buildMetrics` is now allowed (garbageMetrics ruling plus the cold-server fields of
    one-process mode).
  - Still failing: the three command-line items (`--restart_reason=no_daemon`) and the re2
    build_stderr EXTRA `compatibility_level` warning. All 10 commands ran cold (warning only).
- `r3bd`, rows that change against the base:
  - brotli/bep: 37 → 41.
  - re2/bep: 109 → 113.
  - re2/build_stderr: DIFF → MATCH.
- `r3bd` is judged by the strict rules:
  - One server per project (brotli pid 819805); `mod_graph` cold, 9 commands warm, the same as
    the oracle.
  - `shutdown` exit 0, nothing alive after it, `stopped_before` empty.
  - `exit_codes` MATCH in both projects under the A1 check.

Every DIFF item of the daemon recording `r3bd-ce81d5a` (the 56 runfiles action keys are
allowed):

| project | item | cause |
|---|---|---|
| brotli | `{"buildMetrics":{}}` | `builtValues` 7429 against the oracle's 4954, and `postInvocationSkyframeNodeCount` 7300 against 7305. These are the only paths outside the daemon list. |
| re2 | `{"buildMetrics":{}}` | `builtValues` 23638 against 21190, and node count 23512 against 23543. These are the only paths outside the daemon list. |

What the recording shows about the kept graph:

- **`builtValues` and the node count still differ**, practically unchanged from 374c631 (brotli
  7432 / 7303, re2 23642 / 23516 then).
  - Built only by Blaze during the build: BAZEL_DEP_GRAPH, BAZEL_MODULE_INSPECTION,
    CLIENT_ENVIRONMENT_VARIABLE, STARLARK_BUILTINS (1 each), REPO_SPEC and YANKED_VERSIONS
    (27 brotli / 29 re2), SINGLE_EXTENSION, SINGLE_EXTENSION_EVAL and SINGLE_EXTENSION_USAGES
    (26 each).
  - Built more often: BZL_LOAD 787 vs 383, MODULE_FILE 314 vs 156 (brotli).
  - So the build evaluates again the module-resolution and .bzl nodes that Bazel's server
    keeps from `mod graph`.
- **The build did find a graph from `mod graph`.** Its `dirtiedValues` equals the oracle's as a
  multiset (1649), and so do `changedValues` (4) and `cleanedValues` (43/46). A graph discarded
  before the build gives 0 for all three, as the one-process recording `r3b` shows. So the
  recording does not show "discarded after every command" between `mod graph` and the build.
- **Timing points to a discard after the build** (evidence weaker than the counters):
  - The first `cquery` after the build takes 0.73 s (brotli) and 2.12 s (re2). That is close to
    one-process (1.07 / 2.28 s), against 0.26 / 1.22 s at 374c631.
  - The following `cquery` and `aquery` are fast again (0.26 / 0.31 s brotli).
  - That fits the analysis state of the build not being reused by the next analysis command.
    BEP is recorded for the build only, so no surface sees it directly.
- `memoryMetrics.garbageMetrics` is absent, as ruled (allowed). The action-cache
  `sizeInBytes` is 159015 against 161890 in re2 (excused by name; A8 note).

Both labels were recorded from a dirty tree with a binary outside this checkout. They are
verification labels, not acceptable as a score.

## For the next extractor change (A5–A8): prepared, not applied

Each of these changes `surfaces.py`, and so the extractor hash. Applying them in the tree
would have made every recording "extracted by another version". They are in
`~/.cache/blaze/work/referee-3b/next-extractor/next-extractor-A5-A8.patch` (sha256
`811b2a54224d4c00c7879e2521beef45b37e1bc49d1d1ac2eaaad86348807f8d`, against this tree;
`git apply --check` clean), with the patched referee copy beside it in `repo/scripts/oracle`.

The changes:

- **A5.** A size in `ARTIFACT_SIZE_MASKED` is masked only when it is a string of decimal digits.
  Anything else stays and differs.
- **A6.** `presence()` keeps an empty list, `null`, `0`, `"0"` and `false` as they are (proto3
  JSON never prints them for a set field), so they differ from `<measured>`/`<list>`.
- **A7.** `install_md5` is masked only when its value is 32 lowercase hex digits, and the value
  is replaced inside the existing `combinedForm`, which is otherwise kept. The string form
  requires the same.
- **A8.** `buildGraphMetrics.dirtiedValues` is sorted by SkyFunction in the extractor, and
  `only_fields_in_daemon_mode` drops `dirtiedValues` (it stays in `only_fields`: a
  one-process build dirties nothing).
- Tests: `test_surfaces.NextExtractorTest` (5 tests). They fail on this tree's extractor and
  pass on the patched copy: 164 pass there, apart from the two tests that error only outside
  the repository.
- Adjusted with the patch: `InstallMd5Test.test_string_form_is_masked` (its 8-digit key is no
  longer masked, so it uses 32 digits), and `DaemonModeAllowTest.…four_cold_server_fields…`
  (a `dirtiedValues` path is excused only for one-process candidates).

Measured renormalisation cost, on staged copies of `oracle-9.3.2`, `-rerun`, `-check` and
`blaze-ef66f55e904e` (brotli, re2), renormalised with the patched extractor:

| recording | items that change | why |
|---|---|---|
| `oracle-9.3.2`, `-rerun`, `-check` (brotli, re2) | `bep.json` only: the `buildMetrics` item | equal to the old item once `dirtiedValues` is sorted (checked for all 6), so A5–A7 change no item |
| `blaze-ef66f55e904e` (brotli, re2) | none | no `dirtiedValues`; values well formed |
| `oracle-9.3.2/workerd` | none expected | no BEP; only `normalise_bep` paths change. Not re-extracted (3 GB) |

- The oracle self-checks with the patch stay `100% MATCH` (13,858 + 58).
- All six oracle self-comparison `buildMetrics` diffs fit the narrowed daemon list.
- Cost for root: renormalise in place `oracle-9.3.2` (brotli, re2, workerd with `--summary`;
  the brotli and re2 summaries change in `items/bep.json` and `norm/bep.json`, workerd's only
  in `extractor_sha256`), then `-rerun`, `-check`, `blaze-ef66f55e904e`, and any candidate label
  to be compared. These take seconds each; workerd took about a minute in batch 3.
- Then rescore the base once.
- Also include then: the start-line decision of A2 (`s_build_stderr` keeps the line when
  `oc.strict_server_rules(manifest)`). Identical items for every existing recording, since none
  has an observed warm server without mode `daemon`.

## Old code against the new tests

The committed (b94556a) versions of the ten non-test files, run with the new test files
(`~/.cache/blaze/work/referee-3b/oldcode/`):

- Fail or error: 9 of the 11 new tests, plus the adjusted
  `test_daemon_mode_against_a_server_has_no_warning` (wording).
- Pass (guards): 2.
- Error only outside the repository: `test_this_checkout` and
  `test_the_committed_workerd_summary_declares_its_splice`.
- Every other old test passes.

## Left on disk

- `~/.cache/blaze/work/referee-3b/`: `logs/` (run script and record logs), `scores/` (the two
  verification scores, comparison reports, self-check reports), `oldcode/`,
  `next-extractor/` (the patch and the patched copy).
- Recordings `r3b-ce81d5a` and `r3bd-ce81d5a`.
- Deleted: their output roots and workspace copies, and the patch's staged copies.
- Disk: 93 GB free on /home.

## sha256 of the changed files

| file | sha256 |
|---|---|
| scripts/oracle/oracle_common.py | 9afcc42e9916351583c7acb1d69f73a749267a85bfe0bb3067bac9634bd8739c |
| scripts/oracle/oracle_compare.py | 665811f45ed952c9fbc9794187344ed7957a63434f62ebaa9d5dd4c19f627404 |
| scripts/oracle/oracle_report.py | 58b26ce072989b0991ae8927132ba11b54a43c6c6ca7197201239295df2b2784 |
| scripts/oracle/record | 1adae81d90d419f711866ae008f0e41003d41d01f581a094e5938ecd8d1fdfcd |
| scripts/oracle/README.md | b45f90b679df9000e3605cdb561cadf22b5981239e8a7daf7deb6d3616f63b69 |
| scripts/oracle/test_compare.py | 48e89be0964a2d7c920d965c0ad346495eb9a8f069df47a4571c31f55dadfba3 |
| scripts/oracle/test_record.py | 8574f1dde4a1dea06a76c06ef9cfe352acb27ddd335adc1471eeff33196e61ed |
| scripts/oracle/test_surfaces.py | 564bd89c16a0bd5c85199a1764cbba3b26abf8c1baf3e5dfe645ab145461f5e1 |
| corpus/scores/blaze-ef66f55e904e.referee3b.json | 35264b5b83c09a72b5ef07d4b21bb4d37fc977e7fb819bc56e958dd0cef53c2c |
| corpus/scores/blaze-ef66f55e904e.referee3b.md | 6ed8c220f696f4b031d93b803600d028e411b30a0416937baacc43293dc8c39c |

Unchanged: `normalise.py`, `surfaces.py`, `allow.json`, `compare`, `scoreboard`,
`fixture_records.py`, `test_normalise.py`. This report cannot hold its own hash; it is given in
the final answer.
