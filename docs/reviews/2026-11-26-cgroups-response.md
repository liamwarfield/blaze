# Response to the review of the cgroup port, round 1, 2026-11-26

Implementer: `workers-3` (Opus 5.5). Review: `docs/reviews/2026-11-26-cgroups.md`, STOP at
5f68157. Tree: uncommitted on top of d212972 (HEAD when measured). Blaze binary:
`~/.cache/blaze/work/workers-3/bin/blaze-w3` (sha256 d584d347…). Pre-fix binary: root's
`~/.cache/blaze/work/root-runs/blaze-5f68157`. Probe scripts and outputs:
`~/.cache/blaze/work/workers-3/` (`c1_probe.sh`, `c1_delegated.sh`).

## C1 (S1) fixed

- `crates/blaze_runtime/src/jvm_signals.rs`: the terminating-signal thread now calls
  `run_all_shutdown_hooks()` before `_exit`. It runs the `blaze_exec` hook list (the workers'
  process hooks and `VirtualCgroup::delete_instance`, the list the `atexit` handler runs on a
  normal exit), then the server's `ShutdownHooks` (its files). This is Java's `Shutdown.exit`.
- The JVM's `Terminator` also handles `SIGINT`. `install` now installs the same handler for
  `SIGINT`, so batch mode's `captureSigint` reverts to it after the second interrupt, and a
  third one runs the hooks and exits with 130, as in Java.
- HotSpot leaves a shutdown signal that is ignored when the process starts ignored
  (`JVM_RegisterSignal` / `os::Posix::is_sig_ignored`). The oracle shows it for `SIGHUP`:
  `daemonize.cc` ignores `SIGHUP` (only that signal), so Bazel's server survives it. It survives
  `SIGINT` for another reason: `serverMain` installs its own `InterruptSignalHandler`
  (corrected in round 2; the first version of this text said `daemonize` ignores both).
  `install` now skips `SIGHUP`, `SIGINT` and `SIGTERM` when ignored at startup. Without that
  check, the first version of this fix made Blaze's server exit on `SIGHUP`, where Bazel's keeps
  running.
- Other exit paths:
  - `BlazeRuntime::exit` (`System.exit`) and a return from `main` reach `std::process::exit`, so
    the `atexit` handler runs the `blaze_exec` hooks, after the server's file hooks.
  - The halts (`BugReport.halt`, the pid-file watcher, an uncaught panic without a handler) run
    none, as `Runtime.halt` runs none.
- Probe (`c1_probe.sh`), daemon mode, my agent's own cgroup (the parent slice is writable):
  - build `//p:pw_0` (a persistent proto worker) with `--experimental_worker_use_cgroups_on_linux`;
  - read the server pid;
  - check `blaze_<pid>_spawns.slice` beside the server's cgroup and the worker process;
  - send the signal; check again.

| binary | signal | before | after |
| --- | --- | --- | --- |
| oracle | TERM | slice present, worker alive | server gone, worker gone, slice removed (one run) or left behind (another run) |
| Blaze pre-fix | TERM | slice present (with `worker_1.scope`), worker alive | server gone, worker gone, **slice and `worker_1.scope` left behind** |
| Blaze fixed | TERM | slice present, worker alive | server gone, worker gone, slice removed (three runs) |
| oracle | HUP | slice present, worker alive | server alive (ignored), unchanged |
| Blaze fixed | HUP | same | server alive, unchanged |
| oracle | INT | same | server alive (its own interrupt handler), unchanged |
| Blaze fixed | INT | same | server alive, unchanged |

- The oracle's own `SIGTERM` run left its slice behind once: its hooks run concurrently, and
  the cgroup hook can run before the worker has left the cgroup. Blaze's newest-first order (C3)
  ends the worker first, so the slice is always removed.
- Worker processes end in every case. Here the proto worker also exits when its stdin closes.
- I removed the cgroups my probe servers left behind (`blaze_1114637_spawns.slice` from the
  pre-fix binary, `blaze_1242489_spawns.slice` from the oracle).
- **For root:** other `blaze_<pid>_spawns.slice` directories of dead processes are in
  `…/user@1000.service/app.slice/` (1181293, 1025856, 1181205 when I looked). They are not mine
  and I left them.
- Batch mode's third `SIGINT` was not probed (the window is hard to hit by hand). The path is
  the same thread and function as `SIGTERM`.

## C2 (S2) fixed

`BlazeRuntime`'s construction (`crates/blaze_runtime/src/runtime.rs`, after the command name
cache, as in `BlazeRuntime.java:268`) calls `VirtualCgroup::get_instance()`. It keeps
`cgroups_info`, with `get_cgroups_info()` (`BlazeRuntime.getCgroupsInfo`).

Probe, daemon mode:
- `info server_pid` on a fresh server: `blaze_<pid>_spawns.slice` exists before any build;
  `shutdown` removes it.
- In-process mode: the command's process creates its slice and removes it at exit.

Because of this, the cgroup hook is always registered before any worker hook, so the
newest-first order of C3 holds by construction.

## C3 (S2) comment corrected

`run_shutdown_hooks`' doc comment now says "newest first", a declared deviation (Java starts
the hooks concurrently in no order). The module comment names the signal path too. Root has
accepted the order itself; I did not edit `docs/decisions.md`.

## S4

- `read_lines` now ends a line at `\n`, `\r` or `\r\n`, as `BufferedReader.readLine`. Test:
  `read_lines_ends_a_line_at_a_lone_carriage_return`.
- `Mount::parse_line` now uses Java's pattern itself (`regex`, with Java's ASCII `\s`/`\S` and
  `.` classes, anchored as `Matcher.matches`), so an empty `file` group matches as in Java. Test:
  `mount_parse_line_matches_an_empty_file_group`.
- `SandboxOptions::get_limits` comment corrected: the value of the last occurrence wins; the
  position is the first occurrence's here and the last one's in Guava, which nothing reads.

## Also changed with this round (hardening, S3 of the review)

`--experimental_worker_sandbox_hardening` is ported:
- `WorkerModule.buildStarting` builds `WorkerSandboxOptions`, and the factory's default limits
  are `sandboxOptions.getLimits()`.
- `SandboxedWorker.createProcess` adds the `linux-sandbox` wrapper with its writable
  directories, tmpfs, bind mounts, inaccessible paths, `-C` cgroup directories and fake
  hostname/username.
- `destroy` cleans the helpers, the cgroup, the work dir and the tmp dir.

The old implementation (`CgroupsInfo`, `--noincompatible_use_new_cgroup_implementation`) stays
`unported!`.

## Tests

`scripts/cargo-capped test -p blaze_exec`: 1003 passed, 0 failed, 126 ignored, including the 37
cgroup tests (35 ported from Java plus the two above).

## Round 2

Review: `docs/reviews/2026-11-26-cgroups-round2.md`, PASS at e476a59. Implementer: `workers-4`
(Opus 5.5). Tree: uncommitted on top of 7ef5c86 (HEAD when measured). Binary: `~/.cache/blaze/work/workers-4/bin/
blaze-w4` (sha256 a7694206…). Probes ran under the memory rule of 2026-11-26: `--max_idle_secs=60`,
one output root per side.

### C4 fixed

- Probe (`~/.cache/blaze/work/workers-4/c4_probe.sh`): `--batch build //slow:slow` (a 6 s
  genrule) started from a subshell with `trap '' INT`, then `kill -INT` after 3 s.
  - Oracle: the build completes (exit 0).
  - Blaze before the fix: `Bazel received an interrupt`, `ERROR: build interrupted`, exit 8.
- Fix: `InterruptSignalHandler::new` (`crates/blaze_runtime/src/interrupt.rs`) installs nothing
  when `SIGINT` is ignored, as `JVM_RegisterSignal` leaves an ignored signal ignored. `uninstall`
  is then a no-op. Blaze after the fix: the build completes, exit 0.
- Regression check with `SIGINT` at its default (a Python launcher resets it, fresh genrule
  command each run): both binaries print `Bazel received an interrupt` and
  `ERROR: build interrupted`, exit 8.

### The explanation corrected

The text of round 1 above is corrected: `daemonize.cc` ignores only `SIGHUP`
(`daemonize.cc:266`); the server survives `SIGINT` because `serverMain` installs its own
handler (`BlazeRuntime.java:1243`).

### Open items made safe

- **A signal during a normal exit.** `blaze_exec::worker::shutdown_hooks::begin_shutdown` ports
  the lock of `Shutdown.exit`. The first thread that starts a shutdown owns it for the rest of
  the process; a later call from the same thread returns, and any other thread blocks forever.
  It is taken:
  - by the terminating-signal thread before it runs the hooks;
  - by `BlazeRuntime::exit` before the server's file hooks (the `atexit` handler then runs on
    the same thread);
  - by the `atexit` handler itself (a return from `main`).

  A signal that arrives while a normal exit runs the hooks now waits, and the process ends when
  that exit does.
- **A panic in one hook.** `run_shutdown_hooks` runs each hook under `catch_unwind`, runs the
  rest, then reports the panics to the default uncaught exception handler (each Java hook is a
  thread of its own). `run_all_shutdown_hooks` and `BlazeRuntime::exit` guard each list the same
  way, so a panic in the worker/cgroup list does not skip the server's file hooks.
  - Test: `crates/blaze_exec/tests/shutdown_hooks_test.rs`
    (`a_panicking_hook_does_not_skip_the_others`).
  - The server's own `ShutdownHooks::run_hooks` (in `blaze_server`, another area) is unchanged.
- **Probes after the change:**
  - `SIGTERM` with a live worker, daemon mode: server gone, worker gone, slice removed.
  - `shutdown`: the server exits in 0.17 s and its slice is removed.

### Correction to S4 of round 1

The round-1 review said Guava's `buildKeepingLast` keeps the position of a key's last occurrence,
and the comment on `SandboxOptions::get_limits` was changed to say so. Measured with the
oracle's own Guava (`A-server.jar`, `~/.cache/blaze/work/workers-4/guava-probe/`): the builder
`{""=default, Foo=foo, ""=generic}` builds `{=generic, Foo=foo}`. The first position is kept with
the last value, which is what the code does. The comment now says this.
