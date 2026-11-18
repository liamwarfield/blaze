# Decisions (append-only)

Each entry: date, decision, why, who decided.

- 2026-11-17. Pin Bazel 9.3.2 (commit 5951e93c), Linux x86-64 only. User.
- 2026-11-17. Starlark interpreter: `starlark-rust` 0.14.2, not a port of Bazel's Java
  interpreter. User. Consequence: Starlark-level error text may differ from Bazel; structured
  outputs (query/cquery/aquery/lockfile/exec log/output hashes/test results) must still match.
- 2026-11-17. Real projects: brotli @3dce8720 (simple, `build //...`) and re2 @2da00568 (complex,
  `test //:all //python:all`; `//app/...` excluded because it fails on the oracle too). Only these
  two until the user asks for more. User (count, policy); root (choice).
- 2026-11-17. Rust 1.97.0, edition 2024, one Cargo workspace with one crate per cluster of Bazel
  Java packages. Root.
- 2026-11-17. Crate layout follows docs/research/06-bazel-architecture-map.md section 5, with the
  changes in docs/crates.md (all options in `blaze_optdefs`; domain crates implement skyframe Key;
  in-process execution until W8). Root.
- 2026-11-17. Evaluator: blocking, memoizing `blaze_skyframe` (no restarts), keeping Java's
  getValue/getValueOrThrow/valuesMissing protocol. Endorsed by the architecture map, with its
  section 3.3 fixes. Root.
- 2026-11-17. `linux-sandbox` and `process-wrapper` come from the oracle's install base until they
  are ported; `@bazel_tools` (embedded_tools) is copied from the oracle's install base into
  Blaze's install base. Root.
- 2026-11-17. Vendor starlark-rust 0.14.2 (`starlark`, `starlark_syntax`, `starlark_derive`,
  `starlark_map`) under `third_party/starlark-rust` with `[patch.crates-io]` and a numbered patch
  queue in `third_party/starlark-rust/PATCHES.md`, per docs/research/05-starlark-rust.md 4.3. Bazel
  natives, struct, json, print, fail, depset, Label, providers, BUILD checks and tracebacks live in
  Blaze crates on the public API. Root.
- 2026-11-17. String length/indexing/order: starlark-rust's code-point semantics for now. Bazel
  9.3.2 uses UTF-8 byte strings internally; both corpus projects are ASCII, so outputs agree.
  Revisit when a non-ASCII case shows up. Root.
- 2026-11-17. re2 `query deps(...)` fails on the oracle (an unreachable `@libpfm` download in an
  unselected `select()` branch). Keep it as recorded oracle behaviour: run with `--keep_going`,
  compare exit code 3 and the partial result. Do not seed the repo cache. Root.
- 2026-11-17. Referee accepted: scripts/oracle records 13 surfaces per project; oracle vs oracle is
  100% MATCH (allow list: runfiles-manifest action keys, volatile BEP build-metric counters). Root.
- 2026-11-18. Version impersonation: Blaze reports Bazel 9.3.2's build data verbatim (label,
  target, timestamp), because rule sets (bazel_features) read the version. Root, on W1 lead's
  proposal.
- 2026-11-18. Install base: same file set as Bazel's minus A-server.jar, embedded zstd tar
  (~17 MB) in the `blaze` binary; install key is Blaze's own md5 over the file list. In-process
  server prints Bazel's "Starting local Bazel server" line per command. JVM-only `info` items
  (heap sizes, gc, java-runtime/vm) stay `unported!` for now. Root.
- 2026-11-18. Configurations live in `blaze_build_config` (BuildOptions, BuildConfigurationValue,
  output directories, Starlark exec transition) for now; 06 puts them in blaze_actions/blaze_config.
  Reconcile when the analysis lead needs it. Platform facts temporarily come from the oracle via
  `BLAZE_W4_ORACLE_PLATFORMS` until PlatformFunction is ported. Root.
- 2026-11-18. Upstream integration tests (scripts/upstream-tests): oracle baseline is 211/237
  targets and 2,878 functions passing in all 4 runs on this host; that pass list is the long-term
  hill for Blaze. The shim swaps //src:bazel for the binary under test and lets the oracle run
  the test targets. Root.
- 2026-11-18. W3: rule-definition builtins (rule/attr/provider/aspect/...) live in blaze_packages
  `starlark_defs` for now (marked PORT: move to blaze_analysis); frozen .bzl heaps live for the
  process; targets hold Arc<PackageMetadata>; `Type` is a closed enum. External repos come through
  `blaze_sky_loading::repos::ExternalRepos` (OracleRepos for dev, blaze_repo_rules for real). Root.
