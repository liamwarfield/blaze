# Audit of referee batch 3b (2026-11-26), area `referee-batch-3b`

Regression auditor, read-only. Subject: the uncommitted diff `git diff HEAD -- scripts/oracle`
(HEAD `a673e29b`; batch 3 is `b94556aa`), the fixer's report
`docs/reviews/2026-11-26-referee-batch-3b.md` (sha256 `d1c1438f…`) and the rescored base.

**Verdict: PASS**, bound to the file hashes in "Files root may commit". One condition, C1
below: it must be in before a recording of a Blaze whose *default* is the server is accepted.
Recordings made with `--blaze-server daemon` are not affected by C1.

No bazel or blaze process was started (`root-suite-5f68157` was active throughout). Python only,
on existing recordings; nothing written outside this report and my scratchpad.

## What I checked

- All 10 hashes of the fixer's table match the working tree. `normalise.py`, `surfaces.py`,
  `allow.json` are unchanged from HEAD; extractor `c215d2d7…`, referee `ab46a162…`.
- Unit tests on the working tree: 159 run, 159 passed.
- Old code: `oldcode/` equals HEAD for the ten non-test files, and its three test files equal
  the working tree's. The fixer's log there shows 12 failures or errors: 9 new tests, the one
  reworded test, and the two that error only outside the repository. I read the log; I did not
  rerun it.
- Base recomputed by me (`scoreboard blaze-ef66f55e904e --json --out <scratchpad>`):
  - against the fixer's `referee3b.json`: every key equal except the `allow_dirty` text;
  - against the committed `referee3.json`: `rows`, `totals`, `verdict` equal (13,850 matching,
    56 allowed, 145,082 failed); only `provenance` (new `strict_server_rules: false`),
    `referee_sha256`, `warnings` (the three "summary differs from HEAD" lines are gone) and
    `allow_dirty` differ. `staged` is `null`.
- `scripts/gate/compare-scores` against `referee3b.json` (and against my recomputed base):
  `r3b-ce81d5a` PASS, `r3bd-ce81d5a` PASS, each rows 39, gained 2, regressed 0.
- Evasion cases: the manifest of `r3bd-ce81d5a` altered in memory, scored, and put through
  `compare-scores.compare` against the base.

  | case | strict | `exit_codes` | gate |
  |---|---|---|---|
  | as recorded | yes | MATCH 10/10 | pass |
  | daemon, `build` cold | yes | DIFF 9/10 | regression |
  | daemon, shutdown exit 1 | yes | DIFF (1 extra) | regression |
  | daemon, a server alive after shutdown | yes | DIFF (1 extra) | regression |
  | daemon, shutdown not recorded | yes | DIFF (1 extra) | regression |
  | daemon, no per-command states in the manifest | yes | MATCH (states read from stderr) | pass, correctly |
  | mode unset, warm observed | yes | MATCH; `bep` stays DIFF | pass, correctly |
  | no `server` key and no states (shape of a recorder before batch 3) | **no** | MATCH; `bep` becomes MATCH | **passes leniently** |
  | mode unset, no pid seen, no start line | **no** | MATCH; `bep` becomes MATCH | **passes leniently** |

## Answers

- **A1: done.** A recording with mode `daemon` cannot avoid the check.
  - Without per-command states, the states come from raw stderr; without stderr they are
    unknown, and unknown against the reference's known state is a difference.
  - Without shutdown fields, `shutdown_exit_code` is `None`, which is not 0, so `__shutdown__`
    is added.
  - A failed shutdown adds an EXTRA item only, so `matching` does not fall. The gate catches it
    through the status (MATCH → DIFF). That holds because the base's `exit_codes` rows are MATCH
    for brotli and re2. It would not hold for a project whose base row is already DIFF.
  - No allow-list entry names `exit_codes` (the two entries are `aquery_actions` and `bep`).
- **A2: sound for the interim, with one hole (C1).**
  - The start-line part can wait. A recording with the mode unset drops the line at extraction,
    but a build that printed it is recorded `cold`, and that is a `server_state` DIFF once the
    recording is strict.
  - Today Blaze does not default to the server: `r3b-ce81d5a` (mode unset) has all 10 commands
    cold with the start line on each.
  - A Blaze that defaults to the server is strict only if the recorder saw its pid file. The
    last two rows of the table are lenient: the cold-server fields are excused and `bep` scores
    MATCH. Neither shape comes out of today's recorder with today's Blaze server (`r3bd` shows
    the pid file is found), so this is masked, not live.
- **A3: done.** `prepare` refuses a DIFF entry with a missing or empty `only_fields`, or a
  present and empty `only_fields_in_daemon_mode`; `match` excuses nothing for a DIFF without
  fields. The committed `allow.json` loads.
- **B1: done** in the score (`staged`: `null`, or `{stage, records, summaries}`). Gate text
  below.
- **B2: done.** `names_path` matches the root followed by `/` or the end of one argument.

## Findings

S1: none.

S2:

- **C1. Strictness depends on the recorder's pid-file view** (`oracle_common.py`
  `observed_warm_server`). Expected: a candidate is strict when its mode is `daemon`, or when
  any of its commands did not start a server by either source: `server.start_line` is false in
  the manifest, or `server_states()` gives `warm` (which covers the stderr source). One-process
  Blaze prints the start line on every command, so the base and `r3b` stay lenient. This is
  compare-time only: no renormalisation. Not blocking for this commit; required before the
  default-server switch, at the latest with the next extractor change.

S3:

- **C2.** `referee_sha256` covers `normalise.py`, `surfaces.py`, `oracle_compare.py` and
  `allow.json`. `strict_server_rules` now lives in `oracle_common.py`, outside the hash, so two
  scores with one referee hash could be judged by different strictness rules. Expected: add
  `oracle_common.py` to the hash, or move the rule into `oracle_compare.py`. Do it with C1; it
  changes the hash and costs one rescoring of the base.
- **C3.** Once a daemon-mode score is the base, a one-process candidate regresses on `bep`
  (41 → 38 matching in brotli). That is correct, but root should record that the ratchet does
  not go back to one-process recordings.

## B1: the check for `scripts/gate/compare-scores`

Add this function and call it first in `compare()` (`blockers.extend(staged_blockers(base, new))`),
so the tool prints `NOT COMPARABLE` and exits 2:

```python
def staged_blockers(base, new):
    """A score made from a staging area (BLAZE_ORACLE_STAGE), or one that cannot say, is refused."""
    out = []
    for name, doc in (("base", base), ("new", new)):
        if "staged" not in doc:
            out.append("%s score has no `staged` field (written before referee batch 3b): "
                       "score the label again" % name)
        elif doc["staged"] is not None:
            st = doc["staged"]
            out.append("%s score was made from a staging area (%s): score the label again "
                       "without BLAZE_ORACLE_STAGE" % (name, st.get("stage") if isinstance(st, dict) else repr(st)))
        if any(isinstance(w, str) and w.startswith("staged:") for w in doc.get("warnings") or []):
            out.append("%s score carries a `staged:` warning" % name)
    return out
```

- This is stricter than the fixer's rule 2 (a missing key is refused, not inferred from the
  warnings). It refuses nothing that is comparable today: every score without the key has
  another referee hash than `referee3b.json`.
- Add to the docstring: "A score with a non-null `staged` field, or without the field, is not
  comparable (exit 2)."
- Gate test fixtures that build score documents need `"staged": None`.

## The prepared A5–A8 patch

`next-extractor-A5-A8.patch` (sha256 `811b2a54…`, 179 lines; `git apply --check` is clean on
this tree). Read only as a diff; not run by me.

- It does what A5–A8 asked: digits-only size mask, `presence()` keeps empty or zero values,
  `install_md5` masked only for 32 hex digits inside the existing `combinedForm`,
  `dirtiedValues` sorted and taken off the daemon list.
- Root should schedule it as the next extractor change, together with C1, C2 and the A2
  start-line decision (`s_build_stderr` keeps the line under the strict rules).
- Renormalisation: in place, one at a time, `oracle-9.3.2` (brotli, re2, workerd; the brotli
  and re2 summaries change, workerd's only in `extractor_sha256`), `-rerun`, `-check`,
  `blaze-ef66f55e904e`, and every candidate label still to be compared. Then the three oracle
  self-checks, the base scored once more, and this role re-checks. The fixer measured that only
  the oracle's `buildMetrics` item changes; that is the fixer's measurement.

## What `r3bd-ce81d5a` shows about Blaze (not a referee finding)

In daemon mode at `ce81d5a` the only failing items besides the 56 allowed runfiles keys are the
two `buildMetrics` items. `builtValues` lists 52 SkyFunctions against the oracle's 43 in brotli
(54 against 45 in re2); by the fixer's counts 7429 values against 4954 (re2 23638 against
21190), and `postInvocationSkyframeNodeCount` is 7300 against 7305 (23512 against 23543). The
surplus is module resolution and .bzl loading (BAZEL_DEP_GRAPH, REPO_SPEC, SINGLE_EXTENSION*,
about twice the BZL_LOAD and MODULE_FILE), so the build evaluates again what `mod graph` left in
the server, although `dirtiedValues`, `changedValues` and `cleanedValues` equal the oracle's and
so a graph was there. Timing from the manifests, which I checked: the first `cquery` after the
build takes 0.73 s (brotli) and 2.12 s (re2), the next one 0.26 s and 0.72 s, against 0.11 s for
the oracle's. That fits the build's analysis state not being reused by the next analysis
command. It is a timing sign only; no compared surface shows it. Both points are for the
server and skyframe owners: which keys `mod graph` and the build share, and what is dropped at
the end of a build.

## Files root may commit

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
| docs/reviews/2026-11-26-referee-batch-3b.md | d1c1438fd98c5692da106c208397238384d58971dc64e7a0f76d350d549100a7 |

The fixer's base files can be committed as they are (unstaged, rows confirmed). The gate change
for B1 is root's own edit and is not covered by these hashes.

## After the commit

- A recording made with `--blaze-server daemon` may be accepted as a score, under the usual
  label rules (clean checkout, `blaze-<commit>`). `r3b-ce81d5a` and `r3bd-ce81d5a` are
  verification labels from a dirty tree, never a score or a base.
- A recording with the mode unset of a Blaze whose default is the server: not before C1.
- The ratchet base is `corpus/scores/blaze-ef66f55e904e.referee3b.json`. `referee3.json` and
  `referee2.json` stay for history only.
