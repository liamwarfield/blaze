# Review: cgroups, round 2 (with the hardened worker sandbox and the kept worker pool)

Reviewer: Fable 5.2, read-only. Commit under review: **e476a59** (checkpoint 37). Round 1:
`2026-11-26-cgroups.md` (STOP at 5f68157: C1 blocking; C2, C3 S2). Response:
`2026-11-26-cgroups-response.md`.

**Verdict: PASS at e476a59.** No blocking finding. One new S2 (C4) and two open items.

## Evidence

- The Rust at e476a59 (`git show`) read against the pinned Java and C++
  (`src/main/tools/daemonize.cc`, `lib/worker/SandboxedWorker.java`, `SingleplexWorker.java`,
  `WorkerModule.java`, `CleanCommand.java`, `BlazeRuntime.java`).
- One probe of mine, oracle and Blaze, server mode, `--jobs=4`
  (`~/.cache/blaze/work/review-cgroups/hard_cmdline.sh`, outputs `oracle-*.txt`, `blaze-*.txt`
  beside it). The Blaze binary is the fixer's `blaze-w3`, not a build of exactly e476a59:
  secondary evidence. Both servers were shut down; my output roots are deleted.
- The fixer's probe outputs in `~/.cache/blaze/work/workers-3/` (`seq-*.txt`, `hard-*.txt`).
- No Cargo, no test count claimed.

## Round-1 findings

| Finding | State at e476a59 |
| --- | --- |
| C1 hooks skipped on a terminating signal | Fixed |
| C2 server cgroup at startup | Fixed (`runtime.rs:1129`, with `get_cgroups_info`) |
| C3 hook order | Recorded (`docs/decisions.md:402`); doc comment corrected |
| S4 items (`read_lines`, `Mount::parse_line`, comment) | Fixed |

### C1

- Signals: `jvm_signals.rs:112-116` installs the hook-running handler for `SIGHUP`, `SIGINT` and
  `SIGTERM`, each only when not ignored at startup. That is `Terminator.setup` plus HotSpot's
  rule for the three shutdown signals (`JVM_RegisterSignal` returns without installing when the
  signal is ignored). HotSpot's C++ is not in the pinned tree; that rule is from my knowledge of
  it and from the oracle's behaviour in the fixer's table.
- `daemonize.cc:266` ignores `SIGHUP` only (then `setsid`), so the server inherits `SIGHUP`
  ignored: confirmed. It does **not** ignore `SIGINT`. The response's sentence that `SIGINT` is
  "inherited as ignored from daemonize" is wrong about the cause: in Java the server's
  `SIGINT` goes to `serverMain`'s `InterruptSignalHandler` (`BlazeRuntime.java:1243`,
  `rpcServer.interrupt()`), or was ignored in the probe because the calling shell ignored it.
  Blaze installs the same handler in server mode (`runtime.rs:439`), so an idle server survives
  `SIGINT` in both, as the table shows. The outcome is right; the explanation should be
  corrected in the response. The case where they differ is C4.
- Hooks on every path Java answers with hooks:
  - signal: `run_all_shutdown_hooks` (the `blaze_exec` list, then the server's files), then
    `_exit(128 + signal)`;
  - `BlazeRuntime::exit` (`runtime.rs:353`) and the client's `main`: the server's file hooks,
    then `std::process::exit`, whose `atexit` handler runs the `blaze_exec` list;
  - batch mode's third `SIGINT`: `capture_sigint` restores the previous disposition, which is
    this handler (read, not probed).
- The crash path still halts without hooks: `bugreport.rs:416-421` is `_exit` alone, as
  `Runtime.halt` (`BugReport.java:355`).

### Hardened worker sandbox

Read against `SandboxedWorker.java:70-246` and `WorkerModule.java:99-135`: the options record,
the `WorkerExecRoot` with the tmp dir, `getWritableDirs`, `getBindMounts` (the `TreeMap` order,
inaccessible paths on the helper file or directory, `validateBindMounts`), the cgroup made
before the command line and passed with `-C` only when it exists, `addProcess` after the start,
`finishExecution` destroying the cgroup, `destroy` in Java's order with an `IOException` ending
the cleanup, the factory limits `sandboxOptions.getLimits()` only with hardening.

Probe, the same flags on both sides (`--worker_sandboxing
--experimental_worker_sandbox_hardening`, a tmpfs path, a writable path that is a symlink, two
blocked paths, a mount pair, fake username, a 4096 MB memory limit):

- the argv of the server's `linux-sandbox` child is **identical** (`-w` order with the resolved
  symlink, `-e`, the `-M`/`-m` pairs in order, `-n -U -p`, `-C <worker_1.scope>`, the worker
  command);
- the child is in `blaze_<pid>_spawns.slice/worker_1.scope`, `memory.max` 4294967296,
  `memory.swap.max` 0, in both;
- a mount target that does not exist fails the action with the same message and exit code;
- stderr lines (`INFO`, `ERROR`) are equal after removing times.

### Worker pool kept across commands, `clean`

- `clean.rs:343-355` delivers `CleanStartingEvent` where Java posts it (after `canUseAsync`,
  before the clean itself) to its two subscribers; `WorkerModule::clean_starting` is
  `WorkerModule.java:75-84` (reporter, then `shutdownPool` with the same message and flags).
- Probe: after `clean` the worker process is gone, its `-tmp` directory and its cgroup are
  removed and the work dir remains, in both; after `shutdown` the server's slice is gone in
  both. The whole probe output of Blaze equals the oracle's.
- The fixer's sequence (`seq-oracle.txt`, `seq-blaze.txt`: same worker across two builds, a new
  one after its file changes, junk and crashing workers) differs only in the Java stack frames
  of an "Exception details" block, a declared gap (`docs/decisions.md:403`).

## New findings

### S2

**C4. `InterruptSignalHandler` is installed even when `SIGINT` was ignored at startup.**

- Rust: `interrupt.rs:131-136` calls `libc::signal(SIGINT, on_sigint)` unconditionally (server
  mode `runtime.rs:439`, batch mode `capture_sigint`).
- Java: `InterruptSignalHandler` registers through `sun.misc.Signal.handle`, which goes through
  the same `JVM_RegisterSignal` rule the fix now cites: a shutdown signal ignored at startup
  stays ignored, for Bazel's handler too.
- Input: `bazel --batch build //x &` from a non-interactive shell (background jobs start with
  `SIGINT` ignored), then `kill -INT <pid>`. Java: nothing happens. Blaze: prints `Bazel
  received an interrupt` and interrupts the build. The same holds for a server whose client was
  started that way and that gets `SIGINT` directly.
- Not probed; the Rust is unambiguous and the Java side rests on the HotSpot rule above. Older
  than this change, but it is the other half of the rule C1 introduced.
- Masked: the client cancels over gRPC, and nobody signals a backgrounded batch build with
  `SIGINT`. Fix: skip the installation when `jvm_signals::is_ignored(SIGINT)` held at startup.

### Open items (do not block)

- Two shutdowns at once: the signal thread and a normal exit both call `run_shutdown_hooks`;
  the second finds the list empty and lets the process exit while the first may still be
  destroying workers. Java serializes this (a second `System.exit` blocks until the first
  halts). A `SIGTERM` in the last milliseconds of a normal exit only.
- A panic in one hook ends the remaining hooks of that list (Java runs each hook on its own
  thread).
- In the shutdown hook of a hardened worker (`singleplex_worker.rs:427-445`) the inaccessible
  helpers are removed with the work dir and not one by one as `SandboxedWorker.destroy` does; the
  result is the same.
- S3, unchanged: the old cgroup implementation (`CgroupsInfo`) stays `unported!`.

## Not read

- `worker_factory.rs`, `worker_spawn_runner.rs`, `worker_multiplexer.rs`, the 31 new worker
  tests, the metrics-collector refactoring beyond a skim, the idle-task changes of `runtime.rs`
  (another area).
- `linux_sandbox_util` and `sandbox_helpers::mount_additional_paths` were checked through the
  probe only.

## Verdict

**PASS**, bound to e476a59. Open, not blocking: C4 (S2), the two shutdown-race items, the wrong
sentence about `SIGINT` in the response.
