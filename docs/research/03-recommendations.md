# Recommendations for the Blaze port

Written 2026-11-17. Based on `01-theo-ts-rust.md` and `02-bazel-test-infrastructure.md`. Where a
point is my judgement and not something either source shows, it says so.

## The plan in one paragraph

Pin one Bazel release. Before porting anything, build the referee: a harness that runs the pinned
real Bazel and Blaze on the same workspace and compares structured outputs, plus a shim that runs
Bazel's own shell and Python integration tests against Blaze. Then port the Java literally, file
by file, under a written Java-to-Rust contract, in dependency order taken from Bazel's own BUILD
graph, porting the unit tests of the lower layers with the code. Accept work only through a
revision gate where nothing that passed may stop passing, judged by scripts the implementers
cannot edit and by two agents who cannot write code.

## Decisions to make before the goal starts

These change what the agents do, so settle them first. My recommendation is first in each.

1. **Pin.** Bazel 9.3.2 (stable, binary downloadable, source tag available). Not master and not
   the 10.0 release candidate. Every count in the research notes was taken on master and should
   be taken again at the pin.
2. **Literal port or redesign.** Literal: same classes, same functions, same order, same
   messages, bug for bug. Theo's failed attempt was a redesign and his successful one was not.
   Bazel's shell tests grep output for specific text about 3,400 times, much of it error
   messages, which also argues for literal.
   The cost is Rust that reads like Java.
3. **Starlark interpreter.** Port Bazel's own (`net/starlark/java`, 35k lines) instead of adopting
   `starlark-rust`. Bazel's built-in API is welded to its interpreter through about 900
   `@StarlarkMethod` annotations, and error text and evaluation details need to match. Keep
   `starlark-rust` as a reference only. This is my judgement; adopting the crate would save
   early weeks and cost exactness later.
4. **First platform.** Linux x86-64 only. Publish macOS and Windows as excluded.
5. **Out of scope for the first target.** My suggested list: Android and Apple rules, coverage,
   mobile-install, remote execution (keep remote cache), build event upload, dynamic execution,
   the Starlark debugger, Windows code paths. Each stays `unported!` and each exclusion is
   printed with every score.
6. **Client and server.** Keep Bazel's model (thin client, long-lived server, same protocol and
   startup options). 60 client tests and every `shutdown` and `info` test depend on it. Whether
   the client is a separate binary is free.
7. **Shipped tools.** Use upstream's prebuilt JavaBuilder, turbine, singlejar, test runner and
   remote worker. Port the C++ helpers (`linux-sandbox`, `process-wrapper`) late; use upstream's
   builds until then.
8. **Compute and budget.** See "Machine" below. Theo's successful run was about $24k
   API-equivalent for a codebase of similar size with an easier language pair and a cheaper test
   loop. Expect more than that here. This is an estimate, not a measurement.

## Build the referee first

Theo's first attempt wrote 590 commits before its parity harness had authority, and that code
was thrown away. The first milestone here contains no port code.

### Pin and oracle

- `UPSTREAM.json`: the pinned commit, the path of a read-only source checkout, the oracle binary
  path and its sha256. A place for later pins.
- `scripts/oracle`: runs pinned Bazel on a case and records its outputs, keyed by pin and case
  hash. Recorded answers are replayed, so the JVM cost is paid once.
- A normaliser for output-base paths, configuration hashes, timings and ordering, with its own
  tests.
- An allow list for cases where real Bazel differs from itself between runs. Each entry has a
  condition that is checked every run.

### Comparison surfaces, in the order they become useful

1. `help flags-as-proto`, `info build-language`, `canonicalize-flags`
2. Starlark script tests and the spec suite
3. `MODULE.bazel.lock` bytes and `mod graph --output=json`
4. `query --output=streamed_jsonproto`
5. `cquery --output=jsonproto`
6. `aquery --output=jsonproto`
7. execution log, output tree hashes, exit codes
8. build event stream, `test.xml`, runfiles
9. hit rate against a remote cache filled by real Bazel

### Corpus, in tiers

- T0: small cases cut out of the shell tests' heredocs, a few hundred, fast.
- T1: Bazel's `examples/`.
- T2: `bazel_skylib`, `rules_testing`, `rules_shell`, then `rules_cc` and `rules_java` with
  their own tests.
- T3: the 33 downstream projects from Bazel's CI.
- T4: Bazel Central Registry modules, driven by each `presubmit.yml`.
- Capstone: Blaze builds Bazel and runs its tests.

Inputs are fetched once into a locked, read-only cache with an offline registry mirror. Agents do
not edit corpus inputs.

### Upstream's integration tests

- A runfiles shim that puts Blaze at `io_bazel/src/bazel` so the 2,228 shell tests and 475 Python
  tests run unmodified.
- Run the whole suite against real pinned Bazel on this machine first. Only tests that pass there
  count. Publish the number and the skipped list.
- Results as one JSON file per run, one entry per test name.

### The gate

- `compare-tests`: every name that passed in the last accepted revision passes now. Missing and
  unrun names fail the comparison.
- `gate-compare`: the same rule for each oracle item.
- `candidate check`: scope, `rustfmt` and `clippy` before anything expensive.
- A list of protected paths (the judge scripts, the baselines, the pin file, the allow list). A
  candidate that touches one is rejected unless its batch names that path and you approved it.
- An accepted-baseline file committed to the repo, with its sha256.

### Scoreboard

One command prints, per surface: total, compared, matching, excluded and why. Agents and you read
the same numbers. This is the hill.

## Repository layout

```
blaze/
  AGENTS.md                 read order, roles, dev loop, required checks
  UPSTREAM.json             pin, oracle path and hash
  PORTING.md                the Java-to-Rust contract
  crates/                   one crate per cluster of Bazel Java packages
  tests/                    ported JUnit tests live with their crates; integration shims here
  scripts/
    oracle/                 record, replay, normalise, compare
    gate/                   candidate, compare-tests, gate-compare, accept
    state                   read and write the saved state
    facts                   one call: main, accepted revision, pin, bins, worktrees
    cargo-capped            the only way agents run Cargo
    audit/                  transcript metrics
  docs/
    research/               these notes
    agent-brief.md          pasted verbatim into every worker prompt
    queue.md                root's work queue
    state/current.json      small live state
    state/history.jsonl     append-only record of revisions, verdicts, decisions
    protected/              accepted baselines
  corpus/                   manifests only; inputs live in a locked cache outside the repo
```

Split crates from the start. Theo's port was one 605k-line crate and every edit rebuilt it; the
split came late and cost a rebase of every open lane. Bazel's 1,288 `java_library` targets give
the dependency graph. Cluster them into a few dozen crates along package lines (`vfs`, `cmdline`,
`starlark`, `skyframe-engine`, `packages`, `analysis`, `actions`, `exec`, `remote`, `bzlmod`,
`query`, `runtime`, and so on). The clustering is my suggestion; the graph is upstream's.

## The porting contract

`PORTING.md` is the document that lets many agents write files that fit together without seeing
each other's work. Have it drafted and reviewed before the first port wave. It needs at least:

- **Ownership.** One agent, one file. No edits to shared files. No Cargo. Root builds.
- **Names.** A mechanical rule from Java names to Rust names, including overloads.
- **A type table.** One Rust form per Java form: nullable references, `Optional`, interfaces,
  abstract classes and inheritance, checked exceptions to `Result`, `InterruptedException`,
  `ImmutableList` / `ImmutableMap` / `ImmutableSet` (insertion-ordered, so `IndexMap` where order
  reaches output), `HashMap` iteration, `NestedSet`, `Label`, `PathFragment`, `Artifact`,
  interning, `AutoValue` records, enums with behaviour.
- **Generated code.** How `@Option`, `@StarlarkMethod`, `@AutoCodec` and `AutoValue` are
  expressed. Probably one macro or generator each, written once by root, so port agents only
  transcribe annotations.
- **Skyframe.** How a `SkyFunction` that returns null on a missing dependency and restarts is
  written in Rust. Decide once.
- **Concurrency.** What happens to `synchronized`, executors and futures.
- **Fidelity.** Same functions, same order, same messages. No simplifying. Keep comments that
  explain behaviour.
- **Gaps.** `unported!("Class.method")` for anything missing. Never a guessed value.
- **Markers.** `// Java: path/File.java:line Class.method` on every ported item.
- **A runtime shim crate** for the Java and Guava behaviour the port depends on (string
  formatting, hashing where it reaches output, exception text).

When a rule does not cover a case: the most literal port, with a `// PORT:` comment.

## Agents

### Roles

- **Root**: owns the queue, integration and the machine's run queue. Starts workflows. Does not
  supply verdicts on its own work.
- **Port workers**: one file each, no builds, during bulk waves.
- **Lane workers**: one topic, one worktree, one branch, during hill climbing. Sized to finish
  well under 500k tokens; longer topics chain through a handoff file.
- **Skeptic**: one per lane, read-only, checks the lane against the Java before integration.
- **Regression auditor**: read-only. Compares candidate and base per name. Returns PASS or STOP
  bound to the commit and binary hashes.
- **Independent reviewer**: read-only. Reviews the change against the pinned Java and checks that
  the protected set is complete. Returns PASS or STOP.
- New auditor and reviewer for each batch. A STOP blocks acceptance and new feature work and
  cannot be overridden or shopped to another agent.

Theo's role texts in `docs/typechecker-accountability.md` are a good starting draft for the last
two.

### Two modes of work

**Bulk waves**, to get breadth quickly. Root takes a set of crates whose dependencies are already
integrated, assigns one Java file (or range) per worker, collects the files, builds once, splits
compiler errors per file and sends each back to a fix worker. Repeat until the wave builds. A
later wave starts only after the wave it depends on builds. Unit tests for the wave are ported
the same way and must pass before the wave counts.

**Hill climbing**, once a surface has an oracle. Root reads the scoreboard, groups failures into
lanes, keeps at least three workflows running, merges lanes that pass their skeptic into an
integration branch, and opens a revision. The revision runs the whole protected set. Two PASS
verdicts and the mechanical check accept it. Root merges to `main` and pushes.

### Rules worth copying into AGENTS.md on day one

Each of these cost Theo's run hours before it was written down.

- A fixed read order at the top, applied again after every context reset.
- In goal mode, no questions: choose, record the decision as a named note, continue. Enforce
  with a hook.
- Paste `docs/agent-brief.md` word for word at the start of every worker prompt.
- Every tool prints usage with `help`. Agents do not read tool source to learn usage.
- One `facts` command for everything agents otherwise search for.
- All Cargo through the capped wrapper. One target directory per worktree. No private
  `CARGO_TARGET_DIR`.
- Heavy jobs run under `systemd-run` with a memory limit and end their log with `DONE` or
  `FAIL rc=N`. Workers block on that line in the foreground; they do not poll.
- Tools live under `scripts/`, never in a temp directory.
- Reports list retained, recovered, lost, absent and unrun separately, and name the commit each
  number came from. A focused run is never reported as a full run.
- When a skeptic rejects a lane for not following upstream's design, the next round starts with
  a short written note of how the Java does it, checked read-only, before code changes. After
  three rounds root decides again whether to keep the lane.
- Real merges into `main`, never squashes. Push after every accepted revision.
- Do not message an agent inside a running workflow. Do not edit and resume a running workflow.
- Record failed revisions as well as accepted ones.

### Audit the loop

After the first few days, have an agent read the transcripts and report: calls before first edit,
time spent waiting, builds by kind, questions asked, revisions rejected for trivial reasons, raw
commands used where a wrapper exists. Fix the tool or the brief, set targets, check again. Theo's
three audits cut revision time roughly in half.

## Milestones

Each is one narrow path made exact against the oracle before the next opens. Bulk port waves can
run ahead of the milestone being scored, in dependency order, but only scored surfaces count as
done.

| | Milestone | Gate |
| --- | --- | --- |
| M0 | Referee: pin, oracle, normaliser, shim, gate scripts, scoreboard, contract, AGENTS.md | Real Bazel compared with itself scores 100% on every surface, and the upstream test pass list is recorded |
| M1 | Starlark interpreter | Script tests, spec suite, ported interpreter unit tests |
| M2 | Options, rc files, client and server startup, `help`, `info`, `version` | `flags-as-proto` and `canonicalize-flags` identical; client tests |
| M3 | File system, labels, packages, globs, BUILD evaluation with native rules | `query` identical on T0 cases without external dependencies |
| M4 | Bzlmod, repository rules, fetching | Lockfile bytes and `mod graph` identical; `query` identical on T1 and T2 |
| M5 | Evaluation engine and analysis: configurations, toolchains, transitions, aspects, providers | `cquery` and `aquery` identical on T0 to T2 |
| M6 | Local execution, action cache, sandbox, `run` | Output hashes and execution log identical; incremental rebuilds do the same work |
| M7 | `test`, runfiles, build event stream | Test results and normalised events identical; T2 test suites pass |
| M8 | Remote cache, workers | Full hit rate against a cache filled by real Bazel |
| M9 | Breadth: T3, T4, remaining shell tests, performance | Scoreboard across the corpus |
| M10 | Blaze builds Bazel | Upstream's own tests pass under Blaze |

M0 is the one to protect. Its gate (real Bazel against itself) proves the harness before any port
code can be blamed or excused by it.

## Machine

This workstation has 16 cores, 30 GB of RAM and 126 GB free on `/home` (87% used). Bazel,
Bazelisk and Go are not installed; Cargo and a JDK are.

- Disk will run out first. One Cargo target directory per worktree at this code size is tens of
  gigabytes, and Bazel output bases and repository caches add more. Free space or move target
  directories and corpus caches to another volume before starting.
- 30 GB will not hold several release builds plus real-Bazel JVMs plus agent sessions. Cap Cargo
  to one or two slots and cap every heavy job's memory.
- Theo's bottleneck was build and test capacity, not model capability, and he ended up renting CI
  runners for all heavy work. Plan for the same: a wrapper for a remote runner pool that agents
  use for builds, test suites and corpus runs, with this machine kept for editing, git and small
  scripts.
- Install Bazelisk for the oracle. Record the oracle binary's hash in `UPSTREAM.json`.

## A starting goal prompt

A draft for M0 only. I would not give the long-running port goal until M0's gate passes, because
everything after it trusts the referee.

```
Goal: build the referee for Blaze, a literal Rust port of Bazel 9.3.2. Write no port code.

Read first: docs/research/01-theo-ts-rust.md, 02-bazel-test-infrastructure.md,
03-recommendations.md.

Done when all of these hold:
1. UPSTREAM.json pins Bazel 9.3.2 with a read-only source checkout and an oracle binary, both
   verified by hash.
2. scripts/oracle records and replays real Bazel's output on a case for: flags-as-proto,
   info build-language, query, cquery and aquery (jsonproto), the lockfile, the execution log,
   output tree hashes and exit code, with a tested normaliser.
3. A T0 corpus of at least 200 small cases cut from src/test/shell, and the T1 and T2 corpora,
   are fetched into a locked offline cache.
4. A runfiles shim runs upstream's shell and Python integration tests against any binary. The
   suite has been run against real Bazel and the per-name results are committed as the baseline.
5. Comparing real Bazel with itself through the full gate scores 100% on every surface, twice,
   with every run-to-run difference on a conditioned allow list.
6. scripts/gate has candidate check, compare-tests, gate-compare and accept, with tests, and a
   protected-path list that covers all of the above.
7. PORTING.md (the Java-to-Rust contract), AGENTS.md, docs/agent-brief.md, docs/queue.md and
   the saved state exist, and an independent reviewer agent that did not write them has checked
   PORTING.md against at least ten real Java files of different kinds.
8. One scoreboard command prints total, compared, matching and excluded per surface.

Do not ask questions. Choose, record the decision in the saved state, and continue. Report what
is retained, lost, absent and unrun separately.
```

After M0, the port goal can be short, because the files carry the rules: read `AGENTS.md`, work
the queue, raise the scoreboard for the current milestone, accept nothing that loses a pass.
