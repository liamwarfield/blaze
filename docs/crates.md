# Crate plan

Source of truth for crate names, contents and dependency order: `docs/research/06-bazel-architecture-map.md`
section 5 (crate list, DAG, cycle breaks). Changes from that plan, decided by root:

- `blaze_optdefs`: every `OptionsBase` subclass lives here (one module per Java class), not spread
  over the crates that own the Java classes. Options are leaf data, and one crate lets several
  agents transcribe in parallel. Depends on util, options, cmdline, vfs. Converters that need
  higher-level types are ported here when small; otherwise the field uses a string and a `// PORT:`
  note names the converter to apply at the use site.
- Domain crates implement `blaze_skyframe::Key` for their own key types (Java's direction), so
  `blaze_cmdline` and `blaze_vfs` may depend on `blaze_skyframe`.
- Single process first: `blaze` runs the command in-process (no gRPC server) until wave W8; the
  client still computes the same output base, install base and directory layout as Bazel.
- `blaze_starlark` depends on `blaze_cmdline` and `blaze_collect` (it holds the Starlark `Label`
  and `depset` values). This inverts 06 §5.2; `blaze_cmdline` and `blaze_collect` must never depend
  on `blaze_starlark`.
- `blaze_protos`: generated proto types plus Java-compatible binary and JsonFormat encoders. Use
  its `JavaMessage` API (`to_byte_array`, `write_delimited_to`, the JSON printer), not prost's
  `encode`. Map fields are `Vec<Entry>` in Java insertion order.
- `blaze_repo_fetch`: repository cache, downloader, decompressors, PatchUtil (not in 06's list).
- `blaze_bzlmod_model`: Starlark-free Bzlmod (versions, selection, discovery, registry, lockfile).
- `blaze_server`: `lib/server` (GrpcServerImpl, CommandManager, ServerWatcherRunnable,
  PidFileWatcher, ShutdownHooks, TerminalSizeMonitor, idle task manager) lives in its own crate
  below `blaze_runtime`, not in `blaze_runtime` as 06 §5 says (decision 2026-11-19, D3). It also
  owns `CommandDispatcher` (with `LockingMode`, `UiVerbosity`) and `BlazeCommandResult` from
  `lib/runtime`, re-exported by `blaze_runtime`. Depends on protos, util, vfs, events, shell;
  never on `blaze_runtime` or anything above it. The gRPC stack is tonic on tokio, as in
  `blaze_remote`; the client side of the protocol is in `blaze_client`.
