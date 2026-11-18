# Notes on Theo's TypeScript-to-Rust port (ts-rust / tsc-rs)

Researched 2026-11-17. Primary source: the public repo
[pingdotgg/ts-rust](https://github.com/pingdotgg/ts-rust) at commit `91f3d021` (2026-10-28).
The repo holds the agent rules, the porting contract, the gate scripts and three audits of the
agent transcripts, so most of what follows is read from those files, not from press coverage.
Press numbers are marked as such. Theo's announcement post and video could not be fetched.

Files worth reading in that repo, in this order:

| File | What it is |
| --- | --- |
| `README.md` (top, above "The Slop Line") | Theo's own account of cost and outcome |
| `docs/history.md`, `ts-rust-recovery-plan.html`, `docs/typechecker-reset-plan.md` | Why the first attempt failed |
| `crates/ts_goport/PORTING.md` | The Go-to-Rust porting contract (137 KB) |
| `AGENTS.md` | Root agent rules: roles, parallel work, dev loop, required checks |
| `docs/goport-agent-brief.md` | Text pasted verbatim at the start of every worker prompt |
| `docs/typechecker-accountability.md` | Protected set, auditor and reviewer roles, STOP rule |
| `scripts/goport/README.md` | The revision pipeline (`candidate.sh`, gate, oracles) |
| `recommendations.md` | Three audits of the agent transcripts and the fixes they produced |
| `UPSTREAM.md`, `UPSTREAM.json` | How the upstream pin and oracle binaries are tracked |

## Outcome and numbers

- Result: a Rust port of Microsoft's Go TypeScript compiler (CLI, checker, language server, API).
  874k lines of Rust in 1,190 files (measured). MIT licensed.
- Attempt 1, June to September 2026: OpenAI models through Codex `/goal` loops. Over $400,000 of
  API-priced tokens, 1.3M lines of Rust, "never got past like 84% compat" (README).
- Attempt 2, about two weeks from late September: Claude Opus 5.5 through Claude Code. "It had a
  working v0 in 10 hours." About $24,047 API-equivalent, run on $200 subscription plans at
  925% to 983% of weekly limits (README). Opus did not reuse the Codex code: "Opus 5.5 started
  from scratch."
- Conformance at revision R222: 13,728 of 13,728 compiler test configurations match Go's
  diagnostics byte for byte; 186,890 tests pass (ported Go tests plus their own); 243,771 language
  server requests match with 0 differing; identical output on 120 open-source repos.
- Theo: "I've never read a line of this code."
- Press only, unverified: about 4,000 agent-hours across all his projects, eight LAN machines plus
  two VPSs, agents waiting up to 12 hours for a build slot before builds moved to Blacksmith.

## Why attempt 1 failed

These are the repo's own post-mortems, written by the agents during recovery.

1. **Breadth before gates.** 590 commits in four days across every compiler subsystem "before the
   parity harness was authoritative". An audit then found 70 failing tests and a compiler 2.6 to
   3.4 times slower than Go.
2. **Self-authored tests.** "Local tests currently validate many self-authored behaviors, not
   TypeScript parity." The agents were climbing a hill they had drawn themselves.
3. **A redesign, not a port.** The Rust checker had its own architecture (planned reads, replayed
   proofs, requests carried through error values) where Go resolves lazily in shared state. Each
   difference added failure paths that Go does not have.
4. **Regressions were counted but did not stop work.** The integration branch lost previously
   passing tests while new features kept landing. Narrow reviews approved individual changes;
   nobody ran the full set until several changes to shared state had stacked up.
5. **Focused runs reported as full results.** A 96-test focused run stood in for a 6,000-test
   suite. Compiling a test binary was treated as running it.

The recovery plan's rule for getting out: "Make one narrow path exact." Freeze feature breadth,
publish denominators and exclusions, compare every case with the pinned oracle, one milestone at
a time.

## What attempt 2 did differently

### A literal port under a written contract

`PORTING.md` opens: "This crate is a line-by-line port of pinned typescript-go ... About 70 agents
port separate Go ranges at the same time. Nobody can see the other files while writing. The rules
below make every file agree on names and types without coordination."

- **Ownership.** Each agent owns exactly one new file. It does not edit shared files (`lib.rs`,
  `mod.rs`, `Cargo.toml`) and does not run Cargo. Root builds once, writes one error file per Rust
  file, and hands each back to a fix agent.
- **Mechanical naming.** A fixed rule turns every Go name into a Rust name, so two agents who never
  see each other's files produce matching call sites.
- **A type table.** Every Go type has one Rust form (`*ast.Node` becomes a `Copy` handle,
  `map` becomes `FxHashMap` or `IndexMap` when Go's iteration order reaches the output).
- **No improvement.** "Port every Go function in your range, in Go order, with the Go logic intact.
  ... Do not simplify, merge, or 'improve'. Behavior must match Go exactly, including diagnostic
  order and type creation order."
- **No guessing.** A missing dependency becomes `unported!("goFunctionName")`, never a made-up
  value. About a dozen remain in the tree.
- **Source markers.** 19,879 `// Go: file.go:line Name` comments tie Rust back to Go. The audit
  judged that these made a separate symbol index unnecessary.
- **A runtime shim.** A `gostd` module reimplements the parts of Go's standard library and runtime
  that the port leans on, including Go's panic text and exit codes.
- **Bug for bug.** Upstream bugs are ported. A difference that pinned Go also shows is, by
  definition, not a defect.

### A pinned oracle

The port tracks one upstream commit. `UPSTREAM.json` records each pin with its Go checkout, oracle
binary path and sha256. Every comparison names its pin. Moving the pin is its own batch: a
`drift.py` tool lists upstream commits between two pins and the Rust functions each one reaches.
They moved the pin several times in a month (the latest is recorded as "bump E").

### A protected set that can only grow

One rule carries most of the weight: every result that passed at the last accepted revision must
still pass, per name. "New passes do not compensate for lost ones." The set holds:

- the ported Go unit tests and baselines, compared name by name (`compare-tests.py`);
- the gate: `tsc-rs` and `tsgo` run on corpus samples, real projects, build mode and editor
  sessions, compared item by item (`gate-compare.py`);
- language-server and API oracles: recorded requests replayed against both binaries and compared
  per request;
- the upstream API client test suite, which already had a process boundary;
- two real projects (TanStack Query core and Hono) checked in full;
- rustfmt and clippy;
- two independent verdicts (below).

Supporting rules:

- Missing names, unrun tests and missing evidence block acceptance, the same as failures.
- Changing an expectation needs evidence from the pinned oracle and an independent review.
- The scripts that judge the set are "protected paths". A candidate that edits one is rejected
  unless its batch lists that exact path. This is the main guard against an agent gaming its own
  tests.
- A known nondeterminism in the oracle goes on an explicit allow list with a condition that is
  verified again each run.
- Reports name retained, recovered, lost, absent and unrun tests separately.

### Roles

- **Root** owns integration, the work queue and the runtime queue. It may also implement.
- **One primary implementer** for shared semantic code. Parallel writers only with explicit file
  ownership.
- **Regression auditor**: read-only, compares the candidate with the base per name and returns a
  PASS or STOP bound to the exact commit and binary hashes.
- **Independent reviewer**: read-only, reviews the change against the upstream source and checks
  that the protected set is complete. Also returns PASS or STOP.
- **STOP** from either blocks acceptance and new feature work. "Root cannot override it with a
  summary, a different agent's opinion or a passing new test." Root may not replace the reviewer
  to get a different answer.
- New auditor and reviewer per batch. A reused pair re-read their whole context each turn and
  burned 1.0 B tokens.
- **Skeptics**: each work lane gets an adversarial check before it joins an integration.

### The loop

1. Root keeps a queue file (`docs/goport-next.md`) and at least three workflows running. When one
   ends it starts the next in the same turn.
2. A workflow runs a **lane**: one topic, one worktree, one branch, sized to finish under 500k
   tokens of context. Larger topics become a chain of agents with a handoff file.
3. Lanes that pass their skeptic are merged into an **integration** branch.
4. `candidate.sh` opens a numbered **revision**, checks scope and formatting first, then runs the
   whole protected set against the last accepted revision.
5. Auditor and reviewer return verdicts. Two PASS verdicts and a passing mechanical check
   (`check-typechecker-batch.mjs`) accept the revision. Failed revisions are recorded too.
6. Root merges to `main` with a real merge and pushes.

By 2026-10-28 they were at revision R250. The early-October audit measured about 16 lanes
landed and 4 revisions accepted per day.

### State outside the context window

- `docs/typechecker-state/current.json`: small live state (roles, base, batch, limits).
- `history.jsonl`: append-only. Every revision, verdict and decision. Agents query it through
  `scripts/state` and never load it whole. The batch check rejects a rewritten history.
- Decisions are recorded as named notes, so a fresh agent can find a ruling without the
  conversation that produced it.
- `AGENTS.md` starts with a "read before work" list that applies again after every context reset.

## Infrastructure they needed

Most of this was built by the agents after an audit showed where time went.

| Problem found | Fix |
| --- | --- |
| The compiler was one 605k-line crate; every edit rebuilt it (475 release builds in four days, median 81 s; 272 fat-LTO builds, median 7 min) | Nightly parallel frontend, incremental builds for the edit loop, later a crate split. Fat LTO only for timing and releases |
| Concurrent Cargo runs crashed the machine | `run-cargo-capped.sh`: one target dir per worktree, a slot limit, sccache |
| A test harness run got the whole agent session killed by the OOM killer | Every heavy job runs under `systemd-run` with `MemoryMax` |
| Agents read tool source about 1,200 times to learn usage | Every tool prints usage with `help`; the brief lists them |
| Agents hunted for paths before their first edit (median 43 calls) | `scripts/goport/facts` prints main, accepted revision, pins, bins and worktrees in one call |
| Root paraphrased the brief and agents ignored the wrappers | The brief is pasted word for word |
| Three questions to the user stalled a goal for 4.3 hours | A `goal-no-ask` hook denies questions in goal mode: choose, record a note, continue |
| Subagents polled in short sleeps (9.18 polls per subagent-hour) | Block in the foreground on a log that ends with `DONE` or `FAIL rc=N` |
| Three revisions rejected only for formatting | `candidate.sh check` runs rustfmt before anything expensive |
| 38 merge conflicts on generated binary blobs | A staleness check without a build, plus a merge driver |
| Tools kept in `/tmp` vanished | All tools live under `scripts/` |
| A skeptic found a design mismatch only after a full round (one lane ran 6 rounds, 26 hours, and was dropped) | Before code: a short note on how upstream does it, checked by a read-only agent. After 3 rounds root decides again |
| Integration took 74% of each revision cycle | Reuse evidence between integration and revision; time only lanes that claim a speed change |
| Messaging an agent inside a running workflow started a second copy in the same worktree | Never message a workflow agent; put follow-ups in the next workflow |
| About 2,000 stale worktrees and branches | A prune script, run with approval |

They also kept `scripts/audit/metrics.py`, which reads the agent transcripts and prints the numbers
above. Each audit set targets and the next audit checked them.

Compute was the real constraint. Heavy builds, tests and gates moved off the workstation to rented
CI runners (Blacksmith testboxes) and short-lived sandboxes, with one wrapper per kind of remote so
agents never used raw `ssh`. Wall-clock timing ran on one quiet dedicated host.

## What to take from it

1. The model mattered, but the structure changed too. The successful run was a literal port under
   a contract, judged only by an external oracle. The failed run was a redesign judged partly by
   its own tests.
2. The upstream test suite was the hill, and the pinned upstream binary was the referee. Both
   existed before any Rust was written.
3. "Never lose a pass, per name" plus protected judge scripts is what let work continue unattended
   for weeks without drifting.
4. Verdicts come from agents that cannot edit the code, and a STOP cannot be argued away.
5. The dev loop itself was measured and tuned from transcripts. Much of the speedup came from
   small tooling fixes.
6. State lives in files with a fixed read order, because every agent starts cold.

## Caveats

- No human has read the code. Maintenance means re-running agents at each upstream bump.
- "100% compatible" is measured against a pinned upstream commit, on their corpus.
- Cost figures are API-equivalent token value on subsidised subscriptions, not cash.
- A compiler is a nearly pure function from files to text. That made byte-for-byte comparison
  cheap. A build system is not like that; see `02-bazel-test-infrastructure.md`.
