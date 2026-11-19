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
- 2026-11-18. Analysis crates: `blaze_analysis` (lib/analysis core, ConfiguredTargetKey, toolchain
  context types, platform providers, ctx/actions), `blaze_rules` (native rule factories, cc/py/
  apple natives, fragments' Starlark values; depends on blaze_sky_loading for W3's environment
  composition), `blaze_sky_analysis` (CONFIGURED_TARGET, producers, toolchain functions,
  PlatformFunction, late-bound default resolution), `blaze_sky_top` (one evaluator per command:
  loading + bzlmod + configuration + analysis; BuildView/AnalysisPhaseRunner), `blaze_cquery`
  (query2 cquery/aquery; 06 puts these in blaze_query, which W3 owns). Design and ownership:
  crates/blaze_analysis/DESIGN.md. Analysis lead.
- 2026-11-18. Infos are frozen Starlark values (`OwnedFrozenValue`), native ones Rust Starlark
  types with the loading-phase builtin provider identity; rule implementations run in a fresh
  Module per configured target, actions recorded from Starlark are built after freezing in
  registration order. Analysis natives replace W3's loading stand-ins by composing
  `BazelStarlarkEnvironment` from outside (blaze_rules::bazel_environment). Analysis functions
  capture `AnalysisServices` in closures (the evaluator's single client is the configuration
  functions'). Analysis lead.
- 2026-11-18. Until real fetching is wired, analysis commands use the OracleRepos seam from
  `$BLAZE_ORACLE_REPOS` or `~/.cache/blaze/w3/<workspace basename>.repos.json`
  (blaze_sky_top::external_repos). `LateBoundDefault` carries a resolver name; analysis
  resolves it (blaze_sky_analysis::late_bound_defaults). Analysis lead, root-approved.
- 2026-11-18. The user asked to try workerd (cloudflare/workerd @6ee5f171) with Blaze. Added to
  corpus/projects.json as a stress project; target `build //src/workerd/server:workerd`. It pins
  Bazel 9.3.1; the oracle stays 9.3.2. User (request); root (target choice).
- 2026-11-18. Strings: supersedes the 2026-11-17 code-point ruling. Blaze mirrors Bazel's
  internal string form (option A of docs/research/07-byte-strings.md): Starlark sources and
  repository/module reads are decoded as Latin-1 so each Starlark string holds one char per UTF-8
  byte, Rust `String`s carry that form, and conversion happens only at OS boundaries and at the
  places Java calls StringEncoding. Migration per 07 section 5 after the build/test integration
  lands (it touches many crates); brotli/re2 must stay byte-identical at every step. Trigger:
  workerd's aspect_tools_telemetry sha1.bzl. Root.
- 2026-11-18. Reviewers and auditors run on Fable 5.2; rules in AGENTS.md "Reviewers". First
  review round covers the evaluator, execution, actions, loading, analysis, bzlmod/repos, and an
  integrity audit of the referee and tests. User (model choice); root (scope).
- 2026-11-18. Referee tightened and gate adopted after the integrity audit (auditor PASS, round
  2A): order items for cquery/aquery, missing/failed surfaces fail, allow-list caps, commit-bound
  recordings, per-name test baseline (docs/state), protected-path hooks installed. Open follow-ups
  from the auditor (next protected batch): `diff_paths(limit=200)` can hide a non-allowed diff
  behind 200+ allowed ones (R2A-1); verify the recorded commit against the binary; refuse dirty
  trees for `scoreboard --write`; item-name ratchet in compare-scores. Root.
- 2026-11-18. TLS trust for downloads stays `webpki-roots` only (no system store). Bazel 9.3.2
  uses the embedded JDK 25.0.2's own `lib/security/cacerts` (a regular file with 105 CA entries
  in the install base, not linked to the system store), so real Bazel does not trust locally
  installed CAs either; the Mozilla-derived webpki set is the closest available. Adding native
  roots would trust more than Bazel does. Bazel's way to add CAs, `--host_jvm_args=
  -Djavax.net.ssl.trustStore...` (and `keyStore`), fails loudly in the client (exit 2). Exact
  parity would mean reading `<install_base>/embedded_tools/jdk/lib/security/cacerts` (PKCS12);
  not done. bzlmod/repos implementer, review 2026-11-18 follow-up.
- 2026-11-18. In-process locking: the client keeps the output base lock (exclusive) and the
  install base lock (shared) for the whole in-process command, as blaze.cc does in `--batch`
  mode; contention shows the client's `AcquireLock` messages and exit 9 with
  `--noblock_for_lock`, not the server's "Another command (pid=...) is running" wording, until
  there is a server (W8). A live server process found through `server/server.pid.txt` that is
  not this process (a real Bazel server sharing the default output base) makes Blaze exit 36
  before it changes anything; Bazel's own client would connect to it or kill it. bzlmod/repos
  implementer, review 2026-11-18 follow-up.
- 2026-11-18. HTTP transport: `ureq` is replaced by `blaze_repo_fetch`'s own HTTP/1.1 GET over
  `TcpStream`/rustls (`downloader/socket_transport.rs`), because `ureq` 2 accepts a chunked body
  cut inside a chunk as complete and gives no way to detect it. Body framing follows the JDK's
  `HttpURLConnection` ("Premature EOF" for a truncated chunked body; a short `Content-Length`
  body ends normally). Trust roots unchanged (`webpki-roots`). SOCKS proxies are `unported!`.
  bzlmod/repos implementer, review round 2 (N1).
- 2026-11-18. Remote caching and execution are now a priority (they were excluded from the first
  target). New crate `blaze_remote`; order: disk cache, remote cache (gRPC), remote execution,
  build-without-the-bytes. Referee: Bazel's own remote worker as the server, the oracle's gRPC
  log as the reference, and cache interchange with real Bazel (hit rate) as the headline number.
  A Fable 5.2 review follows the port. User (priority); root (plan).
- 2026-11-18. Models: Fable 5.2 for reviewers, auditors and leads; Opus 5.5 for all other agents
  (implementers, fixers, leads' sub-workers). Passed explicitly at launch. User.
- 2026-11-18. Analysis review round 3 PASS. Rulings it asked for: (1) `resource_set` on Starlark
  actions is ported in blaze_actions/blaze_analysis; the call from blaze_exec is assigned to the
  execution owner; until it lands the callback is not honoured and this is a documented gap, not
  a loud failure (failing loudly would stop every cc link). (2) cquery request-round model
  (S2-5/R2-3): accepted as an order-only deviation in one tie-break; the `graph-order` probe
  scenario stays as a visible known difference. (3) Probe evidence: Blaze's probe transcripts
  must be saved next to the oracle's so counts can be re-derived (R3-2). Root.
- 2026-11-19. Remote review (docs/reviews/2026-11-19-remote.md): STOP at ef66f55, six S1 in the
  integration (no action file system for remote-only outputs, capabilities check, interrupts,
  eviction). Ruling on the reviewer's question: for output symlink paths, Tree node names and
  server-log keys Blaze stays identical to Bazel (which does not confine them) rather than
  hardening beyond it; each site is marked and listed in docs/remote.md. Flagged to the user as
  a decision they may want to overrule. Root.
- 2026-11-19. Integrity audit round 2B: PASS; test baseline moved to ef66f55. Referee batch 2
  (auditor's open list) was partly committed by a checkpoint before review (root's error); the
  auditor is reviewing it as round 2C, and AGENTS.md "Commits" now keeps in-progress protected
  changes out of checkpoints. Accepted score files stay as recorded; the next accepted label
  uses the new referee with commit verification. Root.
- 2026-11-19. TLS for remote endpoints: no hand-written certificate verifier. rustls/webpki
  rejects X.509 v1 certificates, which Bazel (JDK/Netty) accepts; Blaze fails closed and lists
  it as a known deviation (docs/remote.md). Root, on the remote lead's recommendation.
- 2026-11-19. Lost remote inputs: action rewinding (Bazel's default) is not ported and stays a
  loud failure naming `--norewind_lost_inputs`; with that flag every lost-input source must
  purge stale entries and fail with Bazel's message and exit 39. Root.
- 2026-11-19. First run of Bazel's own remote shell tests against Blaze (baseline binary before
  the round-2 fixes): 110 of 253 oracle-passing functions pass; causes classified in
  docs/remote.md. Root (recording the remote lead's measurement).
- 2026-11-19. Disk-cache action-result integrity check: kept on regardless of
  `--rewind_lost_inputs` while action rewinding is unported (Java skips it when rewinding is on,
  RemoteModule.java:700-706). Known deviation: such entries are misses where Bazel reports hits;
  restore Java's expression when rewinding is ported. Root, on the remote lead's evidence.
- 2026-11-19. workerd: `blaze build //src/workerd/server:workerd` completes on the ASCII-README
  copy with Bazel's summary line and a byte-identical output binary (sha256 cdedf3f9…, verified
  by root from both output bases). Byte-strings migration started so the pristine checkout
  passes. Root.
- 2026-11-19. workerd on the untouched checkout (README emoji in place), binary of the tree that
  became 2574fef: `build //src/workerd/server:workerd` exit 0, `9713 processes: 3333 internal,
  6379 linux-sandbox, 1 local`, output binary sha256 cdedf3f9… equal to Bazel's (root hashed
  both output bases; log `~/.cache/blaze/work/workerd-logs/blaze.build.pristine.log`). Root.
- 2026-11-19. Tag `v0.1.0` ("Basic functionality") at 5a4e4a0 on the user's request, as a
  GitHub prerelease. It is not an accepted state: the accepted baseline stays ef66f55 until the
  remote re-review and the byte-strings review are in. Root.
- 2026-11-19. Fidelity target: the user said they are unsure how identical to Bazel everything
  has to be and has not ruled. Until they do, the contract is unchanged (identical output and
  error text). Root's recommendation on record: keep outputs, action graph and keys, lockfile,
  query results and the remote protocol identical; relax console text, error wording and
  timing lines. Root.
- 2026-11-19. Wave after v0.1.0 (Opus implementers, one each): `run`; `mod dump_repo_mapping` /
  `show_repo` / `tidy`; protobuf TextFormat and `--build_event_text_file`; `help <command>` and
  topics, `fetch`, `canonicalize-flags`, `license`; and a first full run of Bazel's integration
  suite against the v0.1.0 binary, whose ranked cause list (docs/upstream-tests.md) orders the
  next wave. Execution-crate items (exclusive tests, `unused_inputs_list`, compile action
  templates) wait until the remote lead is out of those crates. The persistent server (W8) is
  deferred until the next accepted commit. Root.
- 2026-11-19. New goal from the user: complete the port of Bazel to Rust. Root's reading: every
  NEEDED Java class (Linux x86-64, per PORTING.md "Skip only") ported, the persistent server and
  incremental builds included, Bazel's own integration suite as the outer measure
  (`docs/upstream-tests.md`), and every area with a PASS verdict. Planning inputs started:
  a file- and method-level coverage map (`docs/coverage/`, Opus) and a plan for the server and
  incremental evaluation (`docs/research/08-server-and-incrementality.md`, Fable lead, read-only).
  Byte-strings review round 1 is STOP at 2574fef (six S1 at boundary sites); two Opus fixers
  started (byte-string findings; interpreter differences in lexer and string methods). Root.
- 2026-11-19. Persistent server and incremental builds: plan accepted
  (`docs/research/08-server-and-incrementality.md`, decisions D1-D14 there). Root rules as the
  plan recommends: D1 the server is wire-compatible with Bazel's client (interop tests both
  ways); D2 the first server discards the graph between commands (Bazel's
  `--notrack_incremental_state` behaviour is the oracle); D3 `lib/server` goes to a new crate
  `blaze_server` below `blaze_runtime`; D4 `daemonize` comes from the oracle's install base for
  now; D5 the blocking evaluator stays and gains Java's node-entry state machine (versions,
  dirtiness, change pruning); D6 the kept graph is behind a server environment variable with
  levels, off by default, raised per accepted gate; D7 frozen Starlark heaps live in an arena
  owned by the executor; D8 only Bazel's non-JVM memory levers are ported; D9 incremental
  numbers that must equal Bazel's: the set of actions run, "N processes", "packages loaded,
  targets configured", analysis-cache messages (not graph-metric node counters); D10 value
  equality for change pruning is an opt-in method on `Key`, default "not comparable"; D11
  sequence and stress tools under `scripts/server/`, protected after the auditor reviews them;
  D12 (referee README says fresh server, script records on a warm one) goes to the auditor;
  D13 packages I1 (evaluator), S1 (client daemon and gRPC) and S3 (`blaze_server`) start now,
  before the next accepted commit, because they touch only `blaze_skyframe`, `blaze_client` and
  a new crate; D14 invocation policy later. Not ported, per the plan: action rewinding stays
  as ruled, restart-model devices, JVM memory machinery, Skyfocus, remote analysis caching.
  Root.
