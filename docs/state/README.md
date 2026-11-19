# docs/state: the accepted state

What a later revision is held against. Everything here is written by a tool from a named commit
and changes only through that tool. This directory, `corpus/` and the judge scripts are protected
paths (`scripts/gate/protected-paths.txt`).

| file | written by | holds |
|---|---|---|
| `tests-baseline.json.gz` | `scripts/gate/baseline` | commit, counts, test executables and the result (`pass`, `fail`, `ignored`) of every Rust test name of the workspace at the accepted commit |
| `tests-baseline.json.gz.sha256` | `scripts/gate/baseline` | its sha256 (`sha256sum -c` format) |
| `../../corpus/scores/<label>.json`, `.md` | `scripts/oracle/scoreboard <label> --write` | the oracle score of one recorded label: both recordings' binary sha256, commit, dirty flag and binary-origin check, and per project and surface total, compared, matching, allowed, failed, extra and `matching_keys` (digests of the matching item keys); `recording_problems` and the `--allow-dirty` reason, if any |
| `oracle-test-sites.md` | by hand, from `scripts/gate/check-oracle-tests --list` | the Rust tests that pass silently when their oracle evidence is absent (audit S2-9), with the paths they need |
| `../../corpus/oracle/<project>.json` | `scripts/oracle/record` (reference label) | the reference recording's file hashes, counts, exit codes and declared exceptions |

`scripts/gate/baseline show` prints the baseline's commit and counts and checks the sha256.

## The rules

1. **No name that passed may be lost.** Every test name that is `pass` in the baseline must be
   `pass` in the candidate. A name that now fails or is ignored is LOST; a name that is not there
   (deleted, renamed, its crate did not build, its executable crashed) is ABSENT. Both are
   regressions. New passes never compensate. (`scripts/gate/compare-tests`)
2. **No surface may lose ground.** Against the score of the last accepted label: every
   project/surface row still exists, a `MATCH` stays `MATCH`, the status does not get worse,
   `matching` does not decrease, and every item that matched still matches (by item-key digest;
   a score written before `matching_keys` existed is compared by count, with a warning).
   (`scripts/gate/compare-scores`; it refuses to compare scores made by different referees.)
3. **Missing evidence fails.** A project or surface the candidate did not record is `MISSING`
   and counts as failed; a test executable that crashed is a `problem` and the result cannot
   become a baseline.
4. **Every number names its tree.** A baseline comes from a clean checkout of a commit. A label
   is recorded once, from one binary; its manifest holds the commit, the dirty flag, the
   binary's sha256 and whether the binary lies in that checkout's `target/` and is not older than
   its HEAD commit (`binary_origin.commit_verified`), and `compare`/`scoreboard` print them.
   `scoreboard --write` refuses a label with no commit, a dirty tree or an unverified binary
   unless `--allow-dirty "REASON"` (stored in the score). A number in README or STATUS cites the
   label (`corpus/scores/<label>.md`) or the baseline commit.
5. **The judge changes only by name.** A commit that touches a protected path is rejected by
   `scripts/git-hooks/pre-commit` unless `BLAZE_PROTECTED_OK="<batch>: <why>"` names the approved
   batch; the text lands in the commit message (`Protected-Paths-OK:`). After a referee change,
   the three oracle-vs-oracle comparisons in `scripts/oracle/README.md` must print `100% MATCH`,
   and the last accepted label is scored again before the next comparison.

## Accepting a revision

`C` is the commit to accept, `W` a clean checkout of it (`git worktree add --detach W C`). Tools
run from `W` where noted, so that what they record is `C` with a clean tree.

```
# 0. Oracle-driven tests must have their evidence on this machine (else they pass vacuously)
scripts/gate/check-oracle-tests --checkout W                                         # must exit 0

# 1. Rust tests, per name, on the clean checkout (builds W/target; cargo runs through W/scripts/cargo-capped)
scripts/gate/run-tests /path/tests-C.json --checkout W
scripts/gate/compare-tests docs/state/tests-baseline.json.gz /path/tests-C.json     # must exit 0

# 2. Oracle surfaces: build blaze in W, record with W's recorder under a new label
(cd W && scripts/cargo-capped build -p blaze)
W/scripts/oracle/record --binary W/target/debug/blaze --label blaze-<C, 12 hex> --project all
scripts/oracle/compare oracle-9.3.2 blaze-<C>                 # prints the differences; exit 0 only at 100%
scripts/oracle/scoreboard blaze-<C> --write                    # corpus/scores/blaze-<C>.json and .md
scripts/gate/compare-scores corpus/scores/<last accepted label>.json corpus/scores/blaze-<C>.json   # must exit 0

# 3. Accept: new baseline and score, committed as a protected-path batch
scripts/gate/baseline /path/tests-C.json
BLAZE_PROTECTED_OK="<batch>: accept C" git commit docs/state corpus/scores -m "Accept C"
```

`scripts/gate/baseline` refuses a result from a dirty tree, a result with problems, and a result
that loses a name of the current baseline. `--allow-loss "REASON"` accepts named losses and
stores them with the reason in the new baseline (`accepted_losses`); `--name-map` maps renamed
tests (`old<TAB>new<TAB>reason`). The baseline bytes depend only on the result: JSON with sorted
keys, gzipped without name or timestamp.

Test names are `<package>::<target>::<test path>` with target `lib`, `bin/<name>`,
`test/<file under tests/>` or `doc`; the package comes from Cargo's build messages, so two crates
with the same test file name do not collide.

## Not covered here

- Upstream integration tests have their own per-name files and comparer
  (`corpus/upstream-tests/`, `scripts/upstream-tests/compare`).
- `compare-scores` ratchets item names through digests only for scores that carry
  `matching_keys` (written by the referee of batch 2 or later); for older scores it falls back to
  counts and says so. `scripts/oracle/compare <old label> <new label>` compares the recordings
  themselves while both are on disk.
- `binary_origin` is a file-system check, not a commit stamped into the binary; labels recorded
  before it (`blaze-12ca29ee95e0`, `blaze-ef66f55e904e`) say `binary origin not checked`.
- The hooks stop accidents and scope creep. They do not stop a committer who replaces the hooks;
  reviews read the diff of `scripts/git-hooks` and `scripts/gate`.
