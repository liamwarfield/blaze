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
