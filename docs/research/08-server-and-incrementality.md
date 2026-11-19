# 08. Persistent server and incremental evaluation: plan

Written 2026-11-19 by the planning lead (Fable 5.2, read-only), on the tree after `3ab3b35`
(uncommitted work of other agents present). Nothing here was compiled or run. Three Opus
sub-agents did bounded reading (global state, value equality, upstream tests and the referee);
their counts are marked "(sub-agent)" and give the command that produced them. Everything else I
read myself. Where something was not read, the text says so.

Paths: `J:` is `~/.cache/blaze/src/bazel-9.3.2/src/main/java/com/google/devtools/build/`,
`C:` is `~/.cache/blaze/src/bazel-9.3.2/src/main/cpp/`. Rust paths are relative to the checkout.

Contents: 0 summary; 1 client and server protocol; 2 incremental evaluation; 3 per-process
state; 4 verification; 5 wave plan; 6 decisions for root.

## 0. Summary

- The work splits into two independent tracks that meet late. **Track S (server)** makes
  `blaze` a client plus a daemon that speaks Bazel's `command_server` protocol and runs commands
  one after another, each with a fresh evaluator. That is what Bazel itself does under
  `--notrack_incremental_state` (`J:lib/skyframe/SequencedSkyframeExecutor.java:399-458`: the
  evaluator is reset before every command), so it is correct, it has an oracle, and it closes
  the lock, `shutdown`, `command.log`, `info` server keys, idle shutdown and startup-time items.
  **Track I (incrementality)** keeps the graph between commands and is correct only when
  invalidation, value equality and per-command state are all right.
- The blocking evaluator survives. Java's dirty-node state machine
  (`J:skyframe/AbstractParallelEvaluator.java:233-380`) maps onto the blocking design step by
  step: "enqueue the dep group and wait for signals" becomes "evaluate the dep group, then
  compare versions". Two-phase error bubbling is unchanged; it gains the dirty-parent branch
  Java has at `J:skyframe/ParallelEvaluator.java:395-403`.
- Blaze already stores what incrementality needs on done nodes (grouped deps, reverse deps:
  `crates/blaze_skyframe/src/evaluator.rs:83-158`) and already invalidates by deleting the
  reverse transitive closure (`evaluator.rs:646-713`). So there is a middle stage between "no
  graph" and "Bazel's graph": keep the graph, invalidate by deletion, no change pruning. It
  re-runs more than Bazel does and so does not match Bazel's per-command counts; it is a
  debugging stage, not a release.
- The hard part is not the evaluator. It is that every Rust `SkyframeExecutor` is built per
  command with the command's options, environment, reporter and services captured in closures
  (`crates/blaze_sky_top/src/skyframe_executor.rs:188-400`). A kept graph needs each captured
  input to be either a `PrecomputedValue` dependency, server-constant, or a per-command cell
  that Java also has. Section 2.5 and 3 list them.
- Recommended order: S1 to S4 (usable server, no graph kept), in parallel I1 to I3 (evaluator,
  value equality, invalidation sources) behind a flag, then I4 to I6 switch the graph on per
  phase (loading, then analysis, then execution) with the stress test as the gate.

## 1. Client and server protocol

### 1.1 What Bazel does at the pin

**Client start** (`C:blaze.cc:1603 Main`): parse options, `UpdateConfiguration`, a command
outside a workspace forces `--batch` (`blaze.cc:1686-1698`), `PrepareDirectories`
(`blaze.cc:1289`), `RunLauncher` (`blaze.cc:1516`).

**`RunLauncher`** (`blaze.cc:1516-1601`), in order:

1. `BlazeServer::AcquireLocks` (`blaze.cc:296-334`): a shared lock on
   `<install_base>.lock` (a sibling of the install base, always blocking) and an exclusive lock
   on `<output_base>/lock` (blocking only with `--block_for_lock`). Lock code:
   `C:blaze_util_posix.cc:621-762` (`TryLock`, owner information, `AcquireLock`).
2. `ExtractData` (install base extraction; ported, see 1.3).
3. `BlazeServer::Connect` (`blaze.cc:1757-1810`): read `server/server_info.rawproto`
   (`command_server.ServerInfo`: pid, address, request cookie, response cookie;
   `src/main/protobuf/command_server.proto:230-243`), accept only `127.0.0.1:`, `[::1]:` or
   `[0:0:0:0:0:0:0:1]:` addresses, `VerifyServerProcess` (`C:blaze_util_linux.cc:211`: the pid's
   start time must equal the one saved by `WriteSystemSpecificProcessIdentifier`,
   `blaze_util_linux.cc:189`), open an insecure channel with proxies off, then `Ping` with the
   request cookie and compare the response cookie (`blaze.cc:1735-1755`,
   `--connect_timeout_secs`).
4. `shutdown` with no server: return at once (`blaze.cc:1542-1548`).
5. `EnsureCorrectRunningVersion` (`blaze.cc:1148-1190`): `<output_base>/install` is a symlink to
   the install base; if it differs, kill the server and relink.
6. `GetServerExeArgs` (`blaze.cc:371`), then `KillRunningServerIfDifferentStartupOptions`
   (`blaze.cc:1090-1141`): compare `server/cmdline` (NUL-joined arguments of the running server)
   with the new vector as multisets, ignoring the volatile options of `IsVolatileArg`
   (`blaze.cc:996-1016`: `--option_sources=`, `--max_idle_secs=`, `--connect_timeout_secs=`,
   `--local_startup_timeout_secs=`, `--client_debug=`, `--preemptible=`, `-XX:HeapDumpPath=`).
   On a difference: the warning "Running Bazel server needs to be killed, because the following
   startup options are different:" with the "Only in old server" / "Only in new server" lines,
   then `KillRunningServer`.
7. `RunServerMode` (`exec-server`, `blaze.cc:711`), `RunBatchMode` (`blaze.cc:754`) or
   `RunClientServerMode` (`blaze.cc:1200`).

**Starting the daemon** (`StartServerAndConnect`, `blaze.cc:936-994`): unlink
`server/command_port` and the OOM file, create `server/` with mode 0700
(`blaze.cc:678`), `EnsurePreviousServerProcessTerminated` (`blaze.cc:916-933`: a verified live
pid from `server.pid.txt` is killed, "Killed non-responsive server process (pid=N)"), write
`server/cmdline`, `chdir` to the workspace, `SetScheduling`, print "Starting local Bazel server
(<label>) and connecting to it...", `ExecuteDaemon` (`C:blaze_util_posix.cc:408` onward). The
daemon is not forked by the client: the client `posix_spawn`s the install base's `daemonize`
tool (`src/main/tools/daemonize.cc`, 445 lines) with `-l <jvm.out> -p <server.pid.txt> [-a]
[-c <cgroup>] [-s <systemd wrapper>] -- <exe> <argv...>`; `daemonize` does `setsid`, redirects
output to `server/jvm.out`, writes the pid file and execs the server with
`argv[0] = "bazel(<workspace basename>)"` (`blaze.cc:382`). A socket pair kept open by the server
tells the client if it dies during startup (`SocketBlazeServerStartup`,
`blaze_util_posix.cc:367-399`). `ConnectOrDie` (`blaze.cc:857-912`) retries `Connect` every
100 ms for `--local_startup_timeout_secs`, prints "... still trying to connect to local Bazel
server (N) after S seconds ..." every 10 s, and on a dead server prints "Server crashed during
startup. Now printing <jvm.out>" and exits 37.

**Before the command** (`RunClientServerMode`, `blaze.cc:1209-1245`): if `/proc/<pid>/cwd` of
the server is not the workspace or ends in " (deleted)", kill the server and start again.

**Server start** (`J:lib/runtime/BlazeRuntime.java:911 main`, `:1200 serverMain`):
`newRuntime` (`:1318`: startup options, file system, a shared install base lock that is never
released `:1396-1412`, directories, modules' `globalInit`/`blazeStartup`/`serverInit`), read the
pid file the launcher wrote (`:1215`), `PidFileWatcher` (`J:lib/server/PidFileWatcher.java`: a
thread that halts the JVM with exit 37 when the pid file is deleted or names another pid),
`ShutdownHooks` (`J:lib/server/ShutdownHooks.java`: delete the server files at exit),
`BlazeCommandDispatcher`, `GrpcServerImpl.create` with two random cookies of 16 bytes
(`J:lib/server/GrpcServerImpl.java:116-143`, hex of `b + 128` without padding `:374-383`),
SIGINT handler that interrupts in-flight commands (`:1242-1249`), `serve()`.

**`serve()`** (`GrpcServerImpl.java:439-499`): bind `[::1]:<--command_port>` (0 = any), fall
back to `127.0.0.1`; start the idle and memory thread if `--max_idle_secs > 0`; write
`server/command_port`, `request_cookie`, `response_cookie` and, by write-then-rename,
`server_info.rawproto` (`:501-527`); block until shut down.

**A command** (`GrpcServerImpl.run :678`, `executeCommand :539-675`): reject a bad cookie or an
empty client description with exit 36 and a failure detail; preempt preemptible commands;
register a `RunningCommand` (random UUID, the thread, `J:lib/server/CommandManager.java`); send a
first `RunResponse` with the cookie and the command id (`:587-589`); build an `OutErr` of two
`RpcOutputStream`s that cut output into chunks of at most 8192 bytes, each a `RunResponse` with
cookie and command id (`:249-306`), sent through a `BlockingStreamObserver` that blocks on gRPC
flow control and does not react to interrupts (`:161-206`); decode `arg` and `startup_options`
as Latin-1 (`:784`); parse the invocation policy; call
`dispatcher.exec(policy, args, outErr, WAIT|ERROR_OUT, QUIET|NORMAL, clientDescription,
firstContactTime, startupOptions, idleTaskResults, commandExtensions, reporter,
terminalSizeMonitor)`; hand the result's idle tasks to the command manager; send the final
response (`finished`, `exit_code`, `termination_expected`, `exec_request`, `failure_detail`,
extensions, `:645-666`); if the result says shutdown, `server.shutdown()` (`:672`).

**The command lock** is inside the server
(`J:lib/runtime/BlazeCommandDispatcher.java:228-269`): one `currentClientDescription`; a second
command prints "Another command (pid=N) is running. Waiting for it to complete on the server
(server_pid=M)..." and polls every 500 ms (`WAIT`), or prints "Another command (pid=N) is
running. Exiting immediately." and returns `ANOTHER_COMMAND_RUNNING` (exit 9, `ERROR_OUT`). The
client's own file locks are released as soon as the `Run` call is sent
(`C:blaze.cc:2067-2073`): they protect only startup. After a `shutdown` every later command on
that server gets "Server shut down explicitly by client pid=N" (`:283-288`, `:322-328`).

**Client side of a command** (`BlazeServer::Communicate`, `C:blaze.cc:2018-2206`): build the
`RunRequest` (cookie, `block_for_lock`, `quiet`, `preemptible`, `client_description = "pid=N"`,
args = command, the logging args of `AddLoggingArgs` `blaze.cc:638`, command arguments;
invocation policy; startup options with sources). Note `blaze.cc:2055-2059` calls
`add_startup_options()` twice per option, so the request carries an empty `StartupOption` after
each real one; the port must send the same bytes. Then: release the locks, start the cancel
thread, read responses: check the cookie on each (`:2088`), write `standard_output` and
`standard_error` to fd 1 and 2, on a broken pipe print "Cannot write to standard output;
exiting..." and cancel (`:2120-2125`), remember the command id. After the stream: if
`termination_expected`, drop the channel and wait up to 60 s for the server process to exit,
then kill it (`:2141-2150`); a failed stream or no `finished` message prints "Server terminated
abruptly (error code: N, error message: '...', log file: '...')" and takes the exit code from
`<output_base>/exit_code_to_use_on_abrupt_exit` or the OOM file (`blaze.cc:1421-1449`); an
`exec_request` with `should_exec` (the `run` command) clears and sets environment variables,
changes directory and `execv`s the binary from the client (`:2166-2196`); otherwise exit with
`exit_code` (or 36 after a broken pipe, then the signal is re-raised by
`PropagateSignalOrExit`, `C:blaze_util_posix.cc:221`).

**Cancel and signals** (`C:blaze_util_posix.cc:144-219`, `blaze.cc:1812-1924`): SIGINT and
SIGTERM in the client write one byte to a pipe; the cancel thread sends `Cancel(cookie,
command_id)` with a 10 s deadline once the command id is known, after printing "Bazel caught
interrupt signal; cancelling pending invocation." The third SIGINT kills the server process
("Bazel caught third interrupt signal; server killed. ...") and `_exit(1)`s
(`blaze_util_posix.cc:150-167`). SIGQUIT is forwarded to the
server pid (a JVM thread dump into `jvm.out`). SIGWINCH triggers `UpdateTerminalSize`. SIGPIPE is
recorded and re-raised at exit. On the server, `CommandManager.doCancel`
(`CommandManager.java:99-115`) interrupts the command's thread and starts a watcher that warns
after 10 s if the command is still there. The server is in its own session, so a terminal
Ctrl-C never reaches it or its children directly.

**Idle and memory** (`J:lib/server/ServerWatcherRunnable.java:36-205`): when no command is
registered, the deadline is now + `--max_idle_secs` (client default 3 hours, 15 s when
`$TEST_TMPDIR` is set: `C:startup_options.cc:115`). With `--shutdown_on_low_sys_mem`, after 5
minutes idle it reads `/proc/meminfo` every 5 s and shuts down when free RAM is below 5 % of
the total and below 1 GiB (`:38-41`, `ProcMeminfoLowMemoryChecker.check :117-128`). Idle tasks
(`J:lib/server/IdleTaskManager.java`, `CommandManager.idle/busy :152-170`): the previous
command's `BlazeCommandResult.getIdleTasks()` are scheduled after their delays when the last
command leaves, interrupted when the next arrives, and their results handed to that next
command. Implementations at the pin (grep `IdleTask`): `GcAndInternerShrinkingIdleTask`,
`InstallBaseGarbageCollectorIdleTask`, `DiskCacheGarbageCollectorIdleTask`
(`J:lib/remote/disk/`), and tasks created in `BlazeWorkspace`, `CommandEnvironment`,
`RemoteModule` and `LocalRepoContentsCache` (files named by the grep; bodies not read).

**Batch mode** (`BlazeRuntime.batchMain :1085-1194`): one command with `LockingMode.ERROR_OUT`,
SIGINT interrupts the main thread ("Bazel received an interrupt"), the second SIGINT restores
the default handler; an `ExecRequest` is run as a child process and its exit code forwarded.

### 1.2 What is ported and what is missing

`crates/blaze_client` (11 files, 5,413 lines) ports option processing, rc files, startup
options, the workspace layout, archive extraction, the file locks
(`platform.rs:391-640`), `verify_server_process` (`platform.rs:1004`),
`get_server_exe_args` (`blaze.rs:461`), `build_run_request` (`blaze.rs:414`),
`ensure_correct_running_version` (`blaze.rs:1167`), the launcher skeleton and the signal handler
with an in-process cancel thread (`signal_handler.rs`). The server half is a trait,
`InProcessServer` (`server.rs:34-54`), implemented by `crates/blaze/src/main.rs` on
`BlazeRuntime::batch_main` (`crates/blaze_runtime/src/runtime.rs:214`) and
`BlazeRuntime::server_main_single_command` (`runtime.rs:288`), which already takes a
`RunRequest` and returns a `RunResponse`.

Missing or stubbed (each is a `// PORT:` or an `unported!` today):

| Item | Where it stops today |
| --- | --- |
| `ExecuteDaemon`, `SocketBlazeServerStartup`, `ConnectOrDie`, `WriteSystemSpecificProcessIdentifier` | `blaze.rs:246-262`, `1086-1146` (in-process) |
| `BlazeServer::Connect` / `TryConnect` over gRPC, cookies | `blaze.rs:246`, `424` |
| `Communicate` streaming, lock release, broken pipe, abrupt exit, waiting for termination | `blaze.rs:315`, `335`, `408` |
| `KillRunningServer`, `KillServerProcess`, `AwaitServerProcessTermination` | `blaze.rs:264` `unported!`, `platform.rs:648` `unported!` |
| `KillRunningServerIfDifferentStartupOptions` (`AreStartupOptionsDifferent`, `IsVolatileArg`) | `blaze.rs:1157` `unported!` |
| `RunServerMode` (`exec-server`) | `blaze.rs:877` `unported!` |
| Server cwd check (`GetProcessCWD`) | `blaze.rs:1258` |
| SIGQUIT forwarding, third-SIGINT kill, SIGWINCH | `signal_handler.rs:91`, `110`, `123` |
| `BlazeRuntime.serverMain` | `runtime.rs:155` `unported!` |
| The server command lock (`WAIT`) | `dispatcher.rs:135` `unported!` |
| `GrpcServerImpl`, `CommandManager`, `ServerWatcherRunnable`, `PidFileWatcher`, `ShutdownHooks`, `IdleTaskManager`, `IdleTask`s, `TerminalSizeMonitor`, `InstallBaseGarbageCollector` (`J:lib/server`, 12 files, 2,138 lines) | no Rust |
| `InvocationPolicyParser` and enforcement | `runtime.rs:240`, `328` `unported!` |
| `BlazeModule` hooks | the Rust trait has 5 of Java's hooks (`crates/blaze_runtime/src/module.rs:52-97`: `global_init`, `server_init`, `before_command`, `after_command`, `blaze_shutdown`); Java has `blazeStartup`, `workspaceInit`, `getOutputListener`, `commandComplete`, `blazeShutdownOnCrash`, `executorInit` and others (`J:lib/runtime/BlazeModule.java:83-492`) |
| `CommandLogModule` (`command.log`, `info command_log`) | no Rust; it is a `getOutputListener` module (`J:lib/runtime/CommandLogModule.java:46-116`), not a server feature, but it needs that hook |

Blaze-only behaviour that goes away with the server: the client holding both locks for the whole
command (decision 2026-11-18 "In-process locking"), `find_live_foreign_server` exiting 36
(`blaze.rs:1017-1055`), and "Starting local Bazel server" printed for every command.

Direct process I/O that a server cannot have (the command's output must go through the
request's `OutErr`). Counted with `grep -rnE` over `crates/`, excluding tests, `src/bin`,
`build.rs`, `blaze_client` where noted:

| Pattern | Sites by crate |
| --- | --- |
| `println!` / `eprintln!` | blaze_runtime 2 + 2, blaze_starlark 1 + 1 |
| `io::stdout()` / `io::stderr()` | blaze_client 12 (fine: client), blaze_options 2, blaze_events 2, blaze_rules 1, blaze_analysis 1 |
| `OutErr::system_out_err()` | blaze_runtime 7, blaze_repo_rules 2, blaze_events 2 |
| `process::exit(` | blaze_options 2, blaze_client 2 (fine), blaze 1 (fine) |
| `env::var*` | blaze_runtime 2, blaze_build_config 2, blaze_shell 1, blaze_protos 1, blaze_optdefs 1 |
| `current_dir()` | blaze_runtime 2, blaze_shell 1 |

Each non-client site must be read in package S2: in a daemon, stdout and stderr are `jvm.out`,
the environment and the working directory are the first client's, and `process::exit` kills
every later command.

### 1.3 Which crate gets what

`docs/research/06-bazel-architecture-map.md:488` puts `lib/server` in `blaze_runtime`. I
recommend one change (decision D3): a new crate **`blaze_server`** for `lib/server`, placed
below `blaze_runtime`, so that the server implementer does not share a crate with the command
implementers and the tonic/tokio server code stays in one place.

| Java / C++ | Crate | Notes |
| --- | --- | --- |
| `C:blaze.cc` client-server paths, `blaze_util_posix.cc`/`_linux.cc` daemon, kill, cwd, signals | `blaze_client` | gRPC client stub from `blaze_protos` (`command_server.proto` is already generated: `crates/blaze_protos/build.rs:37`); tonic client on a current-thread runtime |
| `J:lib/server/*` (2,138 lines) | new `blaze_server` | depends on `blaze_protos`, `blaze_vfs`, `blaze_util`, `blaze_events` (OutErr); owns the `CommandDispatcher` trait (`J:lib/runtime/CommandDispatcher.java`, 98 lines) and `BlazeCommandResult`, re-exported by `blaze_runtime` so no caller changes. `// PORT:` note at both. |
| `BlazeRuntime.serverMain`, `BlazeCommandDispatcher` lock, module hooks, `CommandLogModule`, `BlazeWorkspace` state | `blaze_runtime` | `serverMain` constructs `GrpcServerImpl` as in Java |
| `src/main/tools/daemonize.cc` | not ported now | use the oracle install base's `daemonize`, as for `linux-sandbox` (decision 2026-11-17); `clean --expunge_async` already does (`crates/blaze_runtime/src/commands/clean.rs:162`) |
| `main` | `blaze` | one binary. The daemon is `blaze` started by `daemonize` with the argument vector `GetServerExeArgs` builds (`argv[0] = "bazel(<ws>)"`, the JVM flags, `-jar <jar>`, the startup options), which `ServerStartup` already models (`crates/blaze_client/src/server.rs:11-27`). `main` recognises that shape and calls `serverMain` with `startup_args()`. |

### 1.4 Wire compatibility as a test oracle

Blaze should speak the real protocol (decision D1). It costs nothing extra (the proto is
generated, tonic is in the workspace for `blaze_remote`) and it gives two oracles that no
recorded transcript can:

- **Real Bazel client driving a Blaze server.** The C++ client starts whatever is at
  `<server_javabase>/bin/java` (`C:startup_options.cc:497-505`), and its only check is that the
  file is executable and readable (`SanityCheckJavabase`, `startup_options.cc:538-566`). So
  `bazel --server_javabase=<dir>` with `<dir>/bin/java` a symlink to `blaze` makes the real
  client extract its own install base, write `cmdline`, run `daemonize`, wait for
  `server_info.rawproto`, ping, send `RunRequest`, cancel and shut down against Blaze's server.
  What it takes: Blaze's `main` must accept the JVM-shaped argument vector (skip everything up
  to and including `-jar <path>`), the server must work with Bazel's install base (it has the
  same embedded tools; `A-server.jar` is ignored), and server-side startup-option parsing must
  accept everything the C++ client sends. This tests the server with a client we did not write.
- **Blaze client driving a real Bazel server.** With `--install_base=<oracle install base>` the
  Blaze client's `GetServerExeArgs` names the oracle's JDK and `A-server.jar`, so `daemonize`
  starts a real Bazel server and the Rust client must connect, stream, cancel, detect option
  changes and shut it down. What it takes: a test-only way to make the client exec the JVM path
  instead of itself (the path `StartupOptions::GetExe` computes is already ported; the choice
  "self or JVM" is one branch). This tests the client with a server we did not write.

Both need no change to protected paths; they are new scripts under `scripts/server/`. Limits:
`RunRequest.command_extensions` and `RunResponse.command_extensions` are Google-internal
(ignored, as the proto says unknown types are); `preemptible` is three lines in
`CommandManager`; `UpdateTerminalSize` only matters for `--curses=yes`, which is not ported.
gRPC details that must match for the C++ client: service path
`/command_server.CommandServer/Run`, plain HTTP/2 on loopback, server streaming; tonic
interoperates with grpc-c++ on these. I did not test that claim; the first acceptance step of
package S3 is exactly that test.

