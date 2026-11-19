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

## 2. Incremental evaluation

### 2.1 What Bazel's graph does between builds

`J:skyframe/` is 85 files, 16,607 lines. The parts that exist only for incrementality:

**Versions.** `AbstractInMemoryMemoizingEvaluator.evaluate`
(`J:skyframe/AbstractInMemoryMemoizingEvaluator.java:118-194`) takes the next graph version:
`Version.constant()` when edges are not kept, else `IntVersion` 0, 1, 2, ... (`:495-503`). Each
node has a `NodeVersion` (last changed, last evaluated; `NodeVersion.java`, 55 lines).

**Two node entry classes.** `IncrementalInMemoryNodeEntry` (539 lines) keeps compressed
`GroupedDeps`, reverse deps and the version; `NonIncrementalInMemoryNodeEntry` (304 lines) keeps
none and is used when `keepEdges` is false (`--notrack_incremental_state`, or `--batch` with
`--discard_analysis_cache`: `J:lib/skyframe/SequencedSkyframeExecutor.java:410-413`). Blaze's
`DoneEntry` (`crates/blaze_skyframe/src/evaluator.rs:83-94`) is in between: it keeps deps and
reverse deps but no version.

**Start of every evaluation** (`AbstractInMemoryMemoizingEvaluator.java:134-152`):

1. keys still in flight when the previous evaluation ended, and unsuccessfully rewound keys,
   are queued for deletion (`:134-135`, from `DirtyAndInflightTrackingProgressReceiver`);
2. `differencer.getDiff(graph, lastVersion, thisVersion)` returns keys changed without a new
   value and keys with a new value (`Differencer.java`, `RecordingDifferencer`,
   `SequencedRecordingDifferencer`: the lists `invalidate(keys)` and `inject(map)` filled since
   the last evaluation);
3. `pruneInjectedValues` (`:432-465`): an injected value equal (`equals`) to the dep-less value
   already in the graph is dropped; an injected key that has deps is dirtied instead;
4. `performInvalidation` (`:481-493`): `EagerInvalidator.delete` then
   `EagerInvalidator.invalidate`;
5. `injectValues` (`:468-479`, `ParallelEvaluator.injectValues :599-632`).

**Invalidation** (`InvalidatingNodeVisitor.java`, 570 lines). `DirtyingNodeVisitor`
(`:402-570`) marks the given keys `CHANGE` and walks reverse deps marking them `DIRTY`.
`NodeEntry.markDirty` (`IncrementalInMemoryNodeEntry.java:287-333`) moves the value and the
deps into a `DirtyBuildingState` ("last build value", "last build direct deps") and returns the
reverse deps so the visitor goes on. `DeletingNodeVisitor` (`:229-400`) removes nodes and, when
edges are kept, their whole reverse transitive closure, and removes the reverse-dep edge from
each surviving dep (`:362-376`). `MemoizingEvaluator.delete(predicate)`
(`AbstractInMemoryMemoizingEvaluator.java:212-225`) queues every dirty node and every node the
predicate matches; `deleteDirty(window)` (`:243-255`) queues dirty nodes older than
`--version_window_for_dirty_node_gc`.

**Evaluating a dirty node** (`AbstractParallelEvaluator.Evaluate.maybeHandleDirtyNode`,
`AbstractParallelEvaluator.java:233-380`; states in `NodeEntry.java:60-91`, `DirtyType :95-135`,
`DirtyBuildingState.java`, 306 lines). While the node is in `CHECK_DEPENDENCIES`:

- take the next dep group of the last build (`getNextDirtyDirectDeps`,
  `DirtyBuildingState.java:244`); groups are checked in order so that a dep that is no longer
  needed is never evaluated (the comment at `:238-246` gives the BUILD-file example);
- if the group is the error-transience dep and that node is newer, force a rebuild
  (`:254-262`);
- without keep_going, a done dep in error stops the evaluation here (`:264-299`);
- re-add the group as temporary deps, register for the deps, wait for their signals
  (`:307-339`). `signalDep` (`DirtyBuildingState.java:178-200`) compares the child's version
  with this node's last-evaluated version: a newer child means `NEEDS_REBUILDING`; when every
  dep of every group signalled without being newer the node is `VERIFIED_CLEAN`.

Then: `VERIFIED_CLEAN` → `markClean` (value kept, `IncrementalInMemoryNodeEntry.java:336-348`),
the stored events are replayed (`:354`), the receiver is told `evaluated(..., versionChanged =
false)`, parents are signalled with the unchanged version; `NEEDS_REBUILDING` → the function
runs from scratch (`:373-377`).

**Change pruning.** When a rebuilt node commits, `setValue`
(`IncrementalInMemoryNodeEntry.java:139-163`) asks
`dirtyBuildingState.unchangedFromLastBuild(value)` (`DirtyBuildingState.java:214-219`): the new
value is not a `NotComparableSkyValue` and `lastBuildValue.equals(newValue)`. If so the node
keeps its old value object and its old last-changed version, so its parents are verified clean.
Deps of the last build that the rebuild did not request again lose their reverse-dep edge at
commit (`SkyFunctionEnvironment.java:84-96` `oldDeps`; the commit code itself I did not read
line by line).

**Errors.** `ErrorTransienceValue` is injected again at every evaluation that does work
(`ParallelEvaluator.java:197-198`), so every node with a transient error is dirty in the next
one; `SkyframeExecutor.invalidateTransientErrors` is called from `sync`
(`J:lib/skyframe/SkyframeExecutor.java:3064`). Blaze has this already
(`evaluator.rs:661-671`). Error bubbling handles a dirty parent by forcing it to rebuild
(`ParallelEvaluator.java:395-403`).

**Events.** A node's value is stored with its events (`ValueWithMetadata.java`, 253 lines): a
`NestedSet<Reportable>` of the events the function reported plus the event sets of its deps,
subject to `EventFilter.shouldPropagate` (`SkyFunctionEnvironment.java:300-331`). They are
replayed when a node is found done or clean (`AbstractParallelEvaluator.replay :833`, callers
`:354` and `ParallelEvaluator.java:115, 338, 445, 485`), each event once per command:
`EmittedEventState` is cleared by `SkyframeExecutor` at `:1178`, `:1733` and `:4467`. This is
why a warning from analysis appears again on the next build of the same target. Blaze reports
events once when the node commits and stores nothing (`crates/blaze_skyframe/src/lib.rs:124`).

**Progress receiver** (`EvaluationProgressReceiver.java`, 146 lines: `dirtied`, `deleted`,
`enqueueing`, `stateStarting`, `stateEnding`, `evaluated`, `changePruned`). Users at the pin
that matter for output: the in-flight tracking above; `InMemoryMemoizingEvaluator.ProgressReceiver`
(`InMemoryMemoizingEvaluator.java:44-136`, evaluation statistics); the
`SkyframeProgressReceiver` of `SkyframeExecutor` (counts behind "N packages loaded, M targets
configured" and the build metrics; `SequencedSkyframeExecutor.java:244-258` feeds the
incremental build monitor from dirtied `FileKey`s); `SkyframeBuildView`'s
`ActionLookupValueProgressReceiver` (`J:lib/skyframe/SkyframeBuildView.java:149-156, 1262-1268`:
whether any action lookup value was evaluated or dirtied decides whether the action conflict
check runs again).

**Rewinding** (`SkyFunction.Reset`, `DirtyType.REWIND`,
`AbstractParallelEvaluator.java:569-575, 872-910`, `resetEvaluationFromScratch`
`IncrementalInMemoryNodeEntry.java:390-411`, `J:lib/skyframe/rewinding/`, 1,708 lines): not
ported and not planned here (decision 2026-11-19, "Lost remote inputs").

### 2.2 How file-system and option changes become invalidations

All in `J:lib/skyframe/SkyframeExecutor.java` (4,910 lines) and
`SequencedSkyframeExecutor.java` (1,026 lines) unless noted. Per command,
`CommandEnvironment.syncPackageLoading` (`J:lib/runtime/CommandEnvironment.java:851`) calls
`SequencedSkyframeExecutor.sync` (`:261-319`):

1. reset the evaluator if `decideKeepIncrementalState` said so (`:274-293`);
2. `SkyframeExecutor.sync` (`:3040-3066`): `preparePackageLoading` (`:1693`) injects the
   precomputed values (`PrecomputedValue.X.set(injectable(), v)` at `:1518-1540`, `:1600`
   `BUILD_ID`, `:1766` `PATH_PACKAGE_LOCATOR`, `:1826`, `:3089` `ACTION_ENV`; an injection of an
   equal value is pruned, see 2.1 step 3), `setDeletedPackages`
   (`SequencedSkyframeExecutor.java:373-397`: invalidates the `PACKAGE_LOOKUP` nodes of packages
   whose deleted state changed), a changed package path invalidates the package-locator
   dependent functions (`onPkgLocatorChange :362`), `invalidateTransientErrors`;
3. `handleDiffs` (`SkyframeExecutor.java:3774-3865`): for each package path entry ask
   `DiffAwarenessManager.getDiff` (`DiffAwarenessManager.java`, 240 lines). Diff awareness
   strategies at the pin: `LocalDiffAwareness` (221 lines) with `WatchServiceDiffAwareness`
   (337 lines), only with `--watchfs`; otherwise there is no diff information.
   - With a diff (`handleDiffsWithCompleteDiffInformation :3923-3939`):
     `FileSystemValueCheckerInferringAncestors` (496 lines) checks only the modified paths.
   - Without (`handleDiffsWithMissingDiffInformation :3950-4086`): apply pending invalidation
     (`applyInvalidation :4098`, an evaluation of no keys), then `FilesystemValueChecker`
     (804 lines) `getDirtyKeys` over **every done value in the graph** (`:4027-4030`) with a
     `MissingDiffDirtinessChecker` for the package roots and an `ExternalDirtinessChecker` for
     the file types chosen by `--experimental_check_output_files`,
     `--experimental_check_external_repository_files`,
     `--experimental_check_external_other_files` and by what `ExternalFilesHelper` saw
     (`ExternalFilesKnowledge`, `:3960-3965`). A checker makes the new `FileStateValue` or
     `DirectoryListingStateValue` from the file system and compares it with the old one
     (`SkyValueDirtinessChecker.check :56-70`, `DirtinessCheckerUtils.java:45-75`). This is the
     default path: with no `--watchfs`, every command stats every file the graph knows.
   - `handleChangedFiles` (`:4121`) hands the result to the differencer (keys to invalidate,
     and new values to inject where the checker already computed them).
   - `handleClientEnvironmentChanges` and `handleRepositoryEnvironmentChanges`
     (`:3874-3916`): one injected `EnvironmentVariableValue` per variable, removed variables
     invalidated.
4. Analysis: `SkyframeBuildView.setConfiguration` (`J:lib/skyframe/SkyframeBuildView.java:350-369`)
   diffs the top-level `BuildOptions` with the previous command's (`OptionsDiff`), prints
   "Build option --x has changed, discarding analysis cache" (`:230-340`) and calls
   `handleAnalysisInvalidatingChange` (`SkyframeExecutor.java:1218`,
   `SequencedSkyframeExecutor.java:672-721`: deletes configured nodes by the predicate
   `shouldDeleteOnAnalysisInvalidatingChange`). `--discard_analysis_cache` and
   `clearAnalysisCache` (`:1476`, `SequencedSkyframeExecutor.java:466`) drop analysis nodes
   after the build.
5. Execution: `detectModifiedOutputFiles` (`SequencedSkyframeExecutor.java:481-520`) runs
   `FilesystemValueChecker.getDirtyActionValues` (`FilesystemValueChecker.java:213-300`) over
   every `ACTION_EXECUTION` value: stat each output and compare with the stored
   `FileArtifactValue` (`couldBeModifiedSince`, `:399`); dirty actions are invalidated. Then the
   action cache decides whether a re-evaluated action really runs (`ActionCacheChecker`,
   `J:lib/actions/ActionCacheChecker.java`, 880 lines; ported:
   `crates/blaze_exec/src/actions/action_cache_checker.rs`, with the persistent cache in
   `crates/blaze_exec/src/actions/cache/compact_persistent_action_cache.rs`). Also:
   `setSaltAndDeleteActionsIfChanged` (`SkyframeExecutor.java:2063-2071`); the `BUILD_ID`
   dependency of actions that depend on the build id; `TimestampGranularityMonitor`
   (`J:lib/util/io/TimestampGranularityMonitor.java`, 206 lines), which makes the command wait
   at its end so that a later edit cannot share a timestamp with a file it read;
   `BlazeWorkspace.recordLastExecutionTime` (`J:lib/runtime/BlazeWorkspace.java:283-295`).
   `ActionExecutionValue` equality lets unchanged outputs prune everything above an action.
6. Cleanups between phases that exist only because the graph is kept: deleting the conflict
   finding nodes (`SkyframeExecutor.java:2560`, `:2612`), `clearLoadedPackages` (`:1680`),
   `deleteOldNodes` (`SequencedSkyframeExecutor.java:779`).

`PACKAGE_LOOKUP` and `CONTAINING_PACKAGE_LOOKUP` need no special invalidation beyond step 2:
they depend on the `FILE` nodes of BUILD files, which step 3 dirties.

### 2.3 What has to change in `blaze_skyframe`

The crate is 4,796 lines without tests. The model in `src/lib.rs:1-136` stays. Changes, each a
literal port of the Java named:

| # | Change | Java | Rust site today |
| --- | --- | --- | --- |
| E1 | Graph version counter; `NodeVersion` on done entries; `keep_edges` switch | `AbstractInMemoryMemoizingEvaluator.java:495-503`, `NodeVersion.java`, `IntVersion.java` | `evaluator.rs:83-94`, `527-546` |
| E2 | `Differencer` API: `invalidate(keys)`, `inject(map)`, applied at the next `evaluate`; `delete(pred)`, `delete_dirty(window)` | `RecordingDifferencer`, `AbstractInMemoryMemoizingEvaluator.java:134-152, 212-255, 424-493` | `inject` applies at once by deleting the closure (`evaluator.rs:646-713`) |
| E3 | Node state `Dirty` with `DirtyBuildingState` (last value, last deps, lifecycle) and `mark_dirty`; `DirtyingNodeVisitor`, `DeletingNodeVisitor` | `IncrementalInMemoryNodeEntry.java:287-348`, `DirtyBuildingState.java`, `InvalidatingNodeVisitor.java` | `NodeState` has `InProgress` and `Done` only (`evaluator.rs:140-143`) |
| E4 | Dirty check when a dirty node is claimed: per group of the last build, evaluate the group (blocking), compare each dep's last-changed version with this node's last-evaluated version; clean → keep value; else run the function | `AbstractParallelEvaluator.java:233-380`, `DirtyBuildingState.java:178-200` | `compute_claimed` (`evaluator.rs:1150`) |
| E5 | Change pruning at commit; drop reverse-dep edges of deps not requested again | `IncrementalInMemoryNodeEntry.java:139-163`, `DirtyBuildingState.java:214-219` | `committed` / `finish` (`evaluator.rs:1398-1487`) |
| E6 | Value equality hook (below) | `SkyValue.equals`, `NotComparableSkyValue` | `Key::Value: Send + Sync + 'static` (`lib.rs:192-197`) |
| E7 | Dirty parent in error bubbling; `isDirty` in the cycle detector | `ParallelEvaluator.java:395-403`, `SimpleCycleDetector.java` | `evaluator.rs:1921`, `simple_cycle_detector.rs:184` |
| E8 | Events stored with the value and replayed; `EventFilter`; `EmittedEventState` cleared per command | `ValueWithMetadata.java`, `SkyFunctionEnvironment.java:300-331`, `EventFilter.java` | `lib.rs:124`, `evaluator.rs:292-311`, `1880` |
| E9 | `EvaluationProgressReceiver` (`dirtied`, `deleted`, `evaluated` with `versionChanged`, `changePruned`) and the in-flight tracking receiver | `EvaluationProgressReceiver.java`, `DirtyAndInflightTrackingProgressReceiver.java` | absent (`lib.rs:115-116`) |
| E10 | `pruneInjectedValues` | `AbstractInMemoryMemoizingEvaluator.java:432-465` | "values here have no equality" (`evaluator.rs:643-645`) |
| E11 | `FunctionHermeticity` on `SkyFunctionName` (only `NONHERMETIC` functions may see the file system; used by `SkyFunctionEnvironment.java:176` for the max transitive source version, which Bazel OSS does not use for pruning; port the enum, skip the version) | `FunctionHermeticity.java`, `SkyFunctionName.java` | `crates/blaze_sky_loading/src/sky_functions.rs:4-6` |

Not in this list on purpose: `getState`, `registerDependencies`, `dependOnFuture`, partial
reevaluation, `SkyFunction.Reset`. None is needed for incrementality in a blocking evaluator.

**E4 in the blocking model.** Java registers the node with a dep group, returns, and is
re-enqueued when the last dep signals. Blaze evaluates the group with the code path that
`get_values_and_exceptions` already uses (`Run::eval_keys`, `evaluator.rs:1600`) and then reads
the versions. The observable order is Java's: groups in last-build order, nothing of group n+1
touched before group n is known clean. The differences to mark `// PORT:` are the same two the
crate already documents for normal evaluation (requests are evaluated when made; a dep in
error under no-keep-going is seen as "not done"). The deps recorded per group are the requests
of the one blocking run instead of the union over Java's restarts; each engine is consistent
with what its own function read, and the cycle detector already relies on this.

**Nodes dropped after a failed evaluation.** `Run::cleanup` (`evaluator.rs:2105`) drops what
Java would not have committed. With E3 it must do what Java does at the next `evaluate`: delete
the in-flight keys and their reverse transitive closure (`:134`, `EagerInvalidator.delete` with
`traverseGraph = keepEdges`). A dirty node that was being checked is therefore deleted, not put
back as dirty.

**E6, the equality hook.** Values are `Arc<dyn Any + Send + Sync>` and 91 `Key` impls exist.
Recommended form: a provided method on `Key`,

```rust
/// Java: SkyValue.equals as used by DirtyBuildingState.unchangedFromLastBuild.
/// Default: not comparable (Java identity equality or NotComparableSkyValue).
fn values_equal(_old: &Self::Value, _new: &Self::Value) -> bool { false }
```

captured by `EvaluatorBuilder::typed_function::<K, _>` into the function table (functions
registered through the raw `SkyFunction` trait register a comparator explicitly or stay "not
comparable"). The default never prunes, which is always correct, so all 91 impls keep compiling
and each key opts in with `a == b` exactly where the Java value class has value equality. A
wrong opt-in in the direction "Rust says equal, Java says different" is still a correct build
as long as the Rust equality covers everything a parent can observe, but it changes the counts
Bazel prints; in the other direction it only costs time. Either way the reviewer checks each
opt-in against the Java `equals`.

### 2.4 Does two-phase bubbling survive?

Yes. Bubbling runs after every evaluator thread has stopped, over nodes that are in progress
or done; it computes nothing new. Kept graphs add three cases, all in Java already: a parent on
the bubbling path that is dirty is forced to rebuild before its function is re-run
(`ParallelEvaluator.java:395-403`); a done top-level node replays its stored events (E8);
errors built on the way up are still never written to the graph, so the next command
re-evaluates the failed path, which is what Java does through the in-flight deletion. The
restarting design would not make any of this simpler, and every ported SkyFunction (and the
rule that Starlark evaluation cannot suspend, PORTING.md "Skyframe") is written against the
blocking environment. Recommendation D5: keep one evaluator and port the node-entry state
machine into it.

@@SECTION-2-5@@

### 2.6 The `PORT: skipped, incremental` sites

`grep -rn "PORT: skipped, incremental" crates --include=*.rs` gives 28 lines; the looser
`grep -rniE "PORT:[^\n]*incremental"` gives 35 (it adds comments worded differently). All 35,
by crate, with the package (section 5) that owns each:

| Crate | Sites | What each needs |
| --- | ---: | --- |
| blaze_skyframe | 4 | `lib.rs:48` (statement of the model), `evaluator.rs:545` (versions, diffs, in-flight deletion), `evaluator.rs:1921` (dirty parent in bubbling), `simple_cycle_detector.rs:184` (`isDirty`): all I1 |
| blaze_sky_loading | 6 | `sky_functions.rs:6` (function hermeticity: I1 E11); `actions/file_state_value.rs:6-7` (`compareContents`, timestamp granularity notifications: I3); `file_state.rs:133` and the type comment at `:98` (external files knowledge: I3); `skyframe_executor.rs:202` (`PACKAGE_LOOKUP` invalidation on deleted packages: I3) |
| blaze_runtime | 8 | `dispatcher.rs:82`, `runtime.rs:116` (idle tasks: S4); `dispatcher.rs:629` (TSGM, client env and package path synced into the server's executor: I3); `command_environment.rs:574` (`decideKeepIncrementalState` bookkeeping: I3, the reset switch; `heuristicallyDropNodes` stays out); `bazel_modules.rs:164` (credential cache reset in `beforeCommand`: S2); `:315` (`--watchfs` diff awareness registration: I7); `:553` (digest cache sizing in `executorInit`/`commandComplete`: S2); `:827` (`workspaceInit`, `commandComplete`: S2) |
| blaze_sky_top | 5 | `skyframe_executor.rs:668` (delete conflict-finding nodes), `skyframe_build_view.rs:195` (`enableAnalysis`), `:359`, `:402` (`resetActionConflictsStoredInSkyframe`), `build_view.rs:297` (`ArtifactFactory.noteAnalysisStarting`): all I5 |
| blaze_build_config | 3 | `skyframe.rs:216`: Java requests the `FILE` node of the platform mappings file and Blaze does not. With a kept graph this is a missing dependency edge, so an edit of that file would not invalidate: I2 (and the feature is unported anyway). `fragments/test_configuration.rs:4` (`SHOULD_INVALIDATE_FOR_OPTION_DIFF`: I5). `transitions.rs:47` (`BuildOptionsCache`: a cache, performance only; optional) |
| blaze_sky_exec | 3 | `action_execution_function.rs:368` (`BUILD_ID` dependency: I6); `skyframe_action_executor.rs:2615` (`notifyDependenceOnFileTime`: I3/I6); `action_execution_function.rs:347` (remote analysis caching: not planned) |
| blaze_cquery | 3 | `buildtool/post_analysis_query_processor.rs:233` (`deleteOldNodes`/`applyInvalidation`: I5); `commands/cquery_command.rs:162`, `commands/aquery_command.rs:105` (remote analysis cache keys: not planned) |
| blaze_query | 1 | `common/query_transitive_package_preloader.rs:162` (the empty evaluation that leaves the graph consistent: I4) |
| blaze_remote | 1 | `lease_service.rs:82` (delete `ACTION_EXECUTION` values with expired remote outputs: I6) |
| blaze_optdefs | 1 | `fragment/platform_mapping_key.rs:9` (`SkyKeyInterner`): nothing; keys compare by value |

So 30 of the 35 need work, 3 are out of scope (remote analysis caching), 1 is optional and 1
needs nothing. The marked sites are the small part: the per-command construction of the
executor (section 2.5, G1) is not marked anywhere because it was a design choice
(`docs/decisions.md` 2026-11-18, "one evaluator per command").

@@SECTION-3@@

## 4. Verification

### 4.1 Three references

1. **Real Bazel, default flags** (kept graph). The reference for Track I: which actions ran,
   the "N processes" line, "(P packages loaded, T targets configured)", the analysis-cache
   warnings, per command of a sequence.
2. **Real Bazel with `--notrack_incremental_state --nokeep_state_after_build`.** The evaluator
   is reset before each command (`SequencedSkyframeExecutor.java:448-453, 274-278`). This is the
   reference for the first server release (decision D2): a sequence recorded this way should
   show cold counts on every command while the server, the lock, `command.log` and the on-disk
   action cache behave as in a normal server. Expected, not verified: I did not record such a
   sequence. The first step of package S5 is to record one and read it. (Both flags are given
   because `--notrack_incremental_state` alone prints a warning,
   `SequencedSkyframeExecutor.java:423-430`.)
3. **Blaze against itself.** A long-lived server with a kept graph must produce, after any
   sequence, what a fresh server on a copy of the same workspace state produces (4.5). This
   needs no oracle run per step and isolates Track I bugs from porting gaps.

### 4.2 What the referee records today, and what it needs (sub-agent, files read)

- `scripts/oracle/record` runs, per project, on one output user root and **one server**, with
  startup flags `--output_user_root=<root> --nohome_rc --nosystem_rc` only (`record:344`): `mod
  graph`, the build or test with exec log and BEP, the queries, cquery, aquery, `info`, then
  `shutdown` (`record:346-401`). So the recorded build is the second command of a warm server.
  `scripts/oracle/README.md` ("Recording") says the build runs on a fresh server; that sentence
  and the script disagree. Not a defect for today's scores, but root should know before a
  Blaze server is compared: from then on Blaze's side is a warm sequence too.
- `kill_servers` (`record:128-150`) finds servers through `<root>/*/server/server.pid.txt`. A
  Blaze server that writes the pid file is handled with no change.
- There is no surface that is a sequence with edits. `build_stderr` keeps "INFO: N processes:
  ..." and "INFO: Analyzed ... (P packages loaded, T targets configured)" verbatim
  (`normalise.py:119-157`); `bep` compares `buildMetrics` except the allow-listed
  `buildGraphMetrics.{evaluated,built,changed,cleaned,dirtied}Values` and two more counters
  (`allow.json`, "build-metrics-volatile-counters"); `execlog` has one item per spawn
  (`surfaces.py:378-401`).
- `scripts/w4/probe` (not a protected path) runs each scenario's `COMMANDS` on one output base
  and one oracle server, with `shutdown` only at the end (`probe:163-183, 283`). Step kinds are
  `run`, `aquery`, `cat`, `stat` (`probe:30-40`); **no step can write, touch or delete a
  file.** 41 scenarios, 765 steps (630 `run`). Its masked view hides exactly the things a
  server changes: "server start", "warm-server counts", "warm-server analysis cache",
  "failed-build process count" (`probe:115-143`). `ORACLE.txt` files already contain warm
  behaviour (for example `exec-tree` step 12: "(0 packages loaded, 0 targets configured)",
  "INFO: 1 process: 1 internal.").
- `scripts/upstream-tests` adds no startup options and does nothing about inner servers.
  Bazel's own harness does: `src/test/shell/testenv.sh` uses one output user root and one
  workspace per shard, tears down with `bazel clean` and shuts the server down once at suite
  exit (`testenv.sh:656-705`), so **one server serves every test function of a shard**; the
  Python base class shuts down after every method (`src/test/py/bazel/test_base.py:170-171`).
  With `$TEST_TMPDIR` set the server's idle limit is 15 s.

Needed, in order of use:

| Tool | Change | Protected? |
| --- | --- | --- |
| `scripts/w4/probe` | New step kinds: `write <path>` (content from a sibling file), `append`, `touch`, `rm`, `mv`, `env NAME=VALUE` (client environment of later steps), `bg <name> run ...` / `wait <name>` / `signal <name> INT` (lock and cancel scenarios), `sleep`. A third oracle column recorded with reference 2. Un-mask the four warm-server masks once the Blaze side is a server (keep them selectable so old counts can be re-derived). | no |
| new `scripts/server/` | `interop` (the two wire tests of 1.4), `seq` (record and compare sequences on the corpus projects: per step the exit code, normalised stderr, the spawn set from `--execution_log_json_file`, BEP `buildMetrics` action, package and target counts, output hashes; reuses `scripts/oracle` normalisers by import, does not edit them), `stress` (4.5). | no (new); moves under the gate when accepted, by the auditor |
| `scripts/oracle` | Nothing in wave 1. Later, a `sequence` surface per project, and a ruling on whether the allow-listed `buildGraphMetrics` counters become compared (D9). | yes: root names it |

### 4.3 Upstream integration tests that need a server (sub-agent)

Method: every `test_*` shell function and `def test*` method in the files behind the 251
in-scope targets of `corpus/upstream-tests/oracle-9.3.2.json` (2,671 functions in 248 files;
2,491 in the pass list by this extraction) was classified by regular expressions on the
function body only. Helpers are not followed, so the counts are lower bounds; the scripts are in
the session scratchpad and the patterns are in the sub-agent's report. Each function counted
once, under its strongest label:

| Label | Files | Functions | In oracle pass list |
| --- | ---: | ---: | ---: |
| S1 server lifecycle (shutdown, pid, startup options, batch, lock, signals) | 43 | 157 | 154 |
| S2 `command.log`, server log | 8 | 14 | 13 |
| S3 asserts in-memory reuse (process counts after an edit, "packages loaded", `--watchfs`, `--skyframe_state`, discard flags) | 20 | 71 | 62 |
| S4 other server-only (`dump`, `config`, idle GC, concurrent commands, memory flags) | 12 | 21 | 19 |
| M at least two commands with an edit between (incremental correctness) | 59 | 185 | 176 |

S1 to S4 together: 263 functions, about 248 passing on the oracle. Of the 157 S1 functions, 82
only call `bazel shutdown` as a reset and pass on a server-less Blaze; the other 75 (72
passing) assert lifecycle. 971 functions (910 passing) run at least two Bazel commands.

Files that are mostly about this plan (functions / passing on the oracle):

| File | Count | Closes with |
| --- | --- | --- |
| `src/test/shell/integration/client_test.sh` | 47 / 47 (S1 30, S4 5) | S1, S3 |
| `src/test/shell/bazel/client_test.sh` | 7 / 7 (install base and action cache GC) | S3, S4 |
| `src/test/shell/integration/startup_options_test.sh` | 6 / 6 | S1 |
| `src/test/shell/integration/bazel_command_log_test.sh` | 5 / 5 | S2 |
| `src/test/shell/integration/client_sigint_test.sh` | 2 / 2 | S1, S3 |
| `src/test/shell/integration/server_logging_test.sh` | 1 / 1 | S2 |
| `src/test/py/bazel/bazel_server_mode_test.py` | 2 / 2 | S3 |
| `src/test/shell/integration/run_test.sh` | 29 / 29 (3 on the `--batch` ExecRequest path) | S1 |
| `src/test/shell/integration/discard_graph_edges_test.sh` | 25 / 25 | I5, I6 |
| `src/test/shell/integration/discard_analysis_cache_test.sh` | 5 / 5 | I5 |
| `src/test/shell/integration/nonincremental_builds_test.sh` | 4 / 4 (two use `jmap`: JVM only) | I5 |
| `src/test/shell/integration/watchfs_test.sh` | 4 / 4 | I7 |
| `src/test/shell/bazel/check_external_files_test.sh` | 10 / 10 | I3, I7 |
| `src/test/shell/bazel/external_correctness_test.sh` | 12 / 12 | I3, I4 |
| `src/test/shell/integration/aquery_test.sh` | 52 / 52 (10 use `--skyframe_state`) | I7 |
| `src/test/shell/integration/dump_test.sh` | 7 / 6 | I7 (memory dumps: not planned) |
| `src/test/shell/integration/info_test.sh` | 7, target fails on the oracle here | S2 |
| `src/test/py/bazel/bzlmod/bazel_lockfile_test.py` | 36 multi-command with edits, 29 use shutdown | I3, I4 |
| `src/test/shell/bazel/starlark_repository_test.sh` | 20 multi-command with edits | I3, I4 |

No file named `incremental*`, `skyframe*` or `analysis_cache*` exists at the pin.
`thread_dump_test.sh`, `output_filter_test.sh` and `profiler_test.sh` exist but have no test
target. `docs/upstream-tests.md` (the ranked causes of the v0.1.0 run) did not exist when this
was written; the run's log ends "Executed 236 out of 237 tests: 76 tests pass, 1 fails to
build, and 160 fail locally", unclassified.

The M functions run today, because a server-less Blaze is trivially correct after an edit. They
become the regression suite of Track I: every one must stay green at I4, I5 and I6, and because
of the shared server per shard a wrong kept graph also shows up as a failure in an unrelated
later function of the same file.

### 4.4 Java and C++ unit tests to port with each package (sub-agent, `grep -c "@Test"`)

- `lib/server`: `GrpcServerTest` 17, `InstallBaseGarbageCollectorTest` 7, `PidFileWatcherTest`
  6, `IdleTaskManagerTest` 5, `ServerWatcherRunnableTest` 5, `CommandManagerTest` 4,
  `ShutdownHooksTest` 2: 46 (S3, S4).
- `lib/runtime`: `BlazeCommandDispatcherTest` 21, `CommandInterruptionTest` 9,
  `BlazeRuntimeTest` 14 (S2).
- `src/test/cpp`: `blaze_util_test.cc` 28, `blaze_util_posix_test.cc` 3 and the option tests
  (S1; whether the option tests are already ported I did not check).
- Evaluator: `crates/blaze_skyframe/src/tests/not_ported.rs` holds 122 ignored stubs. By
  reason: 58 dirty nodes, versions, change pruning; 7 `delete`/`deleteDirty`; 6 progress
  receiver; 6 stored events; 2 value equality; 1 inject-replaces-node: **80 that I1 must turn
  green**. The other 42 stay ignored (14 rewinding, 5 partial reevaluation, 4 `dependOnFuture`,
  4 `SkyKeyComputeState`, and smaller groups). New ports for I1:
  `IncrementalInMemoryNodeEntryTest` 32, `EagerInvalidatorTest` 14, `ReverseDepsUtilityTest` 7.
- Invalidation (I3): `FilesystemValueCheckerTest` 52,
  `FileSystemValueCheckerInferringAncestorsTest` 21, `DirtinessCheckerUtilsTest` 10,
  `SequencedSkyframeExecutorTest` 35, `pkgcache/IncrementalLoadingTest` 19; with `--watchfs`
  (I7) `DiffAwarenessManagerTest` 11, `LocalDiffAwarenessTest` 15.
- Execution (I6): `TimestampBuilderTest` 11, `OutputsInvalidationIntegrationTest` 9;
  `ActionCacheCheckerTest` 44 belongs to the existing port.

### 4.5 Stress design: incremental equals clean

**Evaluator level** (package I1, inside `crates/blaze_skyframe` tests, no Bazel involved). A
seeded generator builds a random graph of functions over integer leaves (the `GraphTester`
harness in `src/tests.rs` already expresses this), including functions that fail, fail
transiently, form cycles under some leaf values, and request different deps depending on a
dep's value. A random sequence of operations follows: change leaves, inject, `invalidate`,
`delete(pred)`, evaluate with or without keep_going, interrupt at a random point. After every
evaluation, for every requested root: value, error and cycle info equal those of a fresh
evaluator given the same leaves; the set of reported events is equal; and no function ran
whose transitive leaves are unchanged since its last successful run (minimality, a weaker
statement than Java's exact counts, which the 80 ported tests pin).

**Build level** (`scripts/server/stress`, packages I4 to I6). Workspaces: a generated one that
contains every construct with its own invalidation path (sources, globs, subpackages,
`.bzl` loads and macros, a Starlark rule with an aspect, `select` on a `config_setting`, a
genrule tree artifact, a test, a `local_repository`/`local_path_override`, a module
extension reading a file and an environment variable, a symlinked source), plus copies of
brotli and re2. A seed gives a sequence of 30 steps; each step is one mutation and one
command.

Mutations: edit a source (semantic; comment only; `touch` only); edit a BUILD file (comment
only; add or remove a dep; add, remove or rename a target); edit a `.bzl` (used function;
unused function); add or delete a file matched by a glob; create or delete a BUILD file inside
a package (changes package boundaries); retarget a symlink; turn a file into a directory; edit
`MODULE.bazel`; change a flag (`-c opt`, `--copt`, `--define`, a Starlark flag, `--action_env`,
`--keep_going`); change a client environment variable a repository rule reads; delete or
overwrite an output under `bazel-bin`; introduce a syntax error and repair it at a later step;
revert an earlier mutation exactly. Commands: `build`, `test`, `query`, `cquery`, `aquery`,
occasionally `clean`, `shutdown`, a build interrupted by SIGINT after a random delay, two
commands at once.

Checks after each step, with the incremental server A and a fresh server B (new output base,
copy of the workspace as it is at that step, same Blaze binary):

| Item | Compared how |
| --- | --- |
| exit code | equal |
| `query`, `cquery`, `aquery` output | equal as normalised items (the oracle surfaces' normalisers) |
| output tree | sha256 and exec bit of every file under `bazel-bin`; existence under `bazel-testlogs` |
| action keys | equal (from `aquery`) |
| error and warning messages | equal as a set per command (this catches lost replayed events) |
| test results | equal statuses |
| lockfile | byte equal |
| not compared between A and B | process counts, cache hits, "packages loaded": these legitimately differ and go to the oracle comparison |

And the same sequence on real Bazel (reference 1) gives the comparison that B cannot: the spawn
set per step from the execution log, the "N processes" line, the analysis counts and the
analysis-cache messages. A third run with Blaze under `--notrack_incremental_state` separates
"kept graph is wrong" from "this command is wrong anyway".

A failing seed is reduced by removing steps while the failure stays (delta debugging on the
step list) and stored as a `scripts/w4/scenarios` directory, which is why 4.2 adds edit steps
to the probe. Runs go under `systemd-run --user --collect --wait -p MemoryMax=12G -p
MemorySwapMax=0` (AGENTS.md). Gate for switching a phase on (I4, I5, I6): N seeds green on the
generated workspace and on both corpus projects, N chosen by root; I suggest 200 seeds of 30
steps per phase as a first bar, and a standing nightly run afterwards.

@@SECTION-5@@

