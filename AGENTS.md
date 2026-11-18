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
