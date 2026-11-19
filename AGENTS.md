# Blaze: agent rules

Blaze is a Rust port of Bazel 9.3.2 for Linux x86-64. Its Starlark interpreter is
`starlark-rust` (crate `starlark` 0.14.2), by the user's decision. Everything else ports Bazel's
Java at the pinned source.

## Read before work

1. This file.
2. `PORTING.md` (the porting contract), at least the sections your task touches.
3. `docs/STATUS.md` (where things stand) and `docs/decisions.md` (rulings; append-only).
4. `docs/research/` only when your task asks.

Read them again after a context reset.

## Fixed facts

- Pin: `UPSTREAM.json`. Bazel source (read-only): `~/.cache/blaze/src/bazel-9.3.2`.
  Oracle binary: `~/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64`.
  The oracle's install base (embedded `@bazel_tools`, `linux-sandbox`, `process-wrapper`):
  `~/.cache/blaze/oracle-out/ur/install/58cf1ecbff441bac762b0050d6da21e9`.
- Real projects: `corpus/projects.json`. Only brotli (simple) and re2 (complex) until the user
  asks for more. Checkouts in `~/.cache/blaze/corpus/`, read-only for agents.
- Repository cache shared by oracle and Blaze: `~/.cache/blaze/repo-cache`.
- Oracle records: `~/.cache/blaze/oracle-records/` (see `scripts/oracle`).

## Rules

- Port, do not redesign. Behaviour, ordering, output formats and error text follow the Java at the
  pin. Mark every ported item `// Java: <path relative to src/main/java>:<line> Class.method`.
- Never guess a value. Anything not yet ported calls `unported!("Class.method")` (fails loudly).
- A new crate is a workspace member as soon as its directory exists, and a half-made crate breaks
  every other agent's build. Create it in one step: `Cargo.toml` and a compiling `src/lib.rs`
  (empty is fine) before any other file.
- Edit only the files or crates your task gives you. Do not edit another agent's crate.
- All Cargo goes through `scripts/cargo-capped` (it caps jobs and memory). Never run bare `cargo`.
  Do not set `CARGO_TARGET_DIR`.
- Heavy jobs that are not Cargo (oracle runs, big scripts) run under
  `systemd-run --user --collect --wait -p MemoryMax=12G -p MemorySwapMax=0 ...`.
- Wait for a job by blocking in the foreground (Bash timeout up to 600000 ms), not by short polls.
- Tools live under `scripts/`, never in `/tmp`. Every tool prints usage with `help`.
- Do not commit unless your task says so. Root commits.
- Run `rustfmt --edition 2024` on every `.rs` file you change before you report.
- Do not ask questions. Choose the option you would recommend, say which in your report, continue.
- Report results as text in your final answer: what you did, file list, test counts (passed,
  failed, ignored, not ported), open problems. Name the commit or tree your numbers came from.
  Never report a focused run as a full run.

## Tests

- Port the JUnit tests of each class you port, into the crate's `tests/` or `#[cfg(test)]`.
  Keep the Java test names in snake_case. A test you cannot port yet is `#[ignore = "reason"]`,
  not deleted.
- The oracle (real Bazel at the pin) is the judge for end-to-end behaviour. Expected outputs come
  from the oracle, never from Blaze.
- Do not edit `scripts/oracle`, `scripts/gate`, `corpus/` or recorded baselines unless your task
  names them.

## Reviewers

- Models, by the user's decision (2026-11-18): reviewers, regression auditors and leads (agents
  that plan a wave, hand out files and integrate) run on Fable 5.2 (`model: "fable"`). Everything
  else runs on Opus 5.5 (`model: "opus"`): implementers, fixers, and every sub-worker a lead
  starts. Always pass the model explicitly when starting an agent; do not rely on inheritance.
- A reviewer is read-only: it does not edit code, tests, expectations, scripts or baselines, and
  does not run Cargo. It reviews the Rust against the pinned Java and the oracle evidence, writes
  its findings to `docs/reviews/<date>-<area>.md`, and returns one verdict, PASS or STOP, bound to
  the commit it read.
- A STOP names the blocking findings. Root cannot overrule it with a summary or a passing test;
  the finding is fixed (or shown wrong with Java evidence) and the same reviewer role re-checks.
- Findings are ranked: S1 wrong output or wrong behaviour on valid input; S2 a deviation that is
  masked today (corpus-specific shortcut, guessed value, weakened check, expectation taken from
  Blaze); S3 missing port marked honestly (`unported!`, `#[ignore]`); S4 style.

## Commits (root)

- Never commit part of a tree that other uncommitted files depend on. Before pushing, check the
  commit builds from a clean checkout (`git worktree add`, then
  `scripts/cargo-capped check --workspace --all-targets`).
- A score is reported with the commit it was measured at, the binary used, and its denominator
  (including what was excluded or not ported).
- Checkpoint commits must not include in-progress changes to protected paths. Stage them out
  (`git add -A && git reset -q -- $(grep -v '^#' scripts/gate/protected-paths.txt | awk 'NF{print $NF}')`,
  then add back only reviewed or task-named protected files); referee and gate changes are
  committed after the auditor's verdict. `BLAZE_PROTECTED_OK` is never set for a checkpoint.

## Protected paths and accepted state

- Protected paths are listed in `scripts/gate/protected-paths.txt` (referee, gate, hooks, corpus
  and baselines, `docs/state`, review reports, the pin, this file, PORTING.md, the starlark-rust
  patch list). Do not edit one unless your task names it.
- Root installs the hooks once per clone: `scripts/git-hooks/install`. A commit that touches a
  protected path needs `BLAZE_PROTECTED_OK="<batch>: <why>"`. Never `git commit --no-verify`.
- A label is recorded once: `blaze-<commit>` from a clean checkout of that commit. `record
  --force` only when root says so.
- A number in README or STATUS cites its label (`corpus/scores/<label>.md`) or the baseline
  commit (`scripts/gate/baseline show`). Acceptance steps and rules: `docs/state/README.md`.
