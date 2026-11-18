# Review 2026-11-18: bzlmod, repository fetching, client, rc handling

Reviewer: independent, read-only (Fable 5.2). No code, test, script or baseline was edited and no
Cargo was run. Nothing was executed: every finding is from reading the Rust against the pinned
Java/C++ (`~/.cache/blaze/src/bazel-9.3.2`) and, where stated, the recorded oracle output.

Area: `crates/blaze_bzlmod_model`, `crates/blaze_bzlmod`, `crates/blaze_repo_rules`,
`crates/blaze_repo_fetch`, `crates/blaze_client`, `crates/blaze_options` (rc handling).

## Verdict: STOP

Blocking: S1-1 to S1-6. The ports of the individual classes are mostly literal (DownloadManager,
DownloadCache, HttpConnector, HashInputStream, IndexRegistry, the decompressors, PatchUtil,
Selection, Discovery, marker files, lockfile reuse). The blocking problems are in the wiring
around them: `crates/blaze_repo_rules/src/setup.rs` (the stand-in for
`BazelRepositoryModule.beforeCommand`) and the in-process client.

## Commit and tree

- `git rev-parse HEAD` = `e4379753f8b645eee3b9f49e54c5051c57414634`.
- The tree was dirty and moving: 85 dirty paths at the start, 91 at the end.
- Dirty files inside the area, all read as on disk:
  - `crates/blaze_bzlmod/Cargo.toml`
  - `crates/blaze_bzlmod/src/skyframe/module_file_function.rs`
  - `crates/blaze_repo_fetch/src/decompressor/compressed_tar.rs`
  - `crates/blaze_repo_rules/src/bin/blaze-fetch-dev.rs` (only line 95 read)
  - `crates/blaze_repo_rules/src/bzlmod/module_extension_context.rs`
  - `crates/blaze_repo_rules/src/registration.rs`
  - `crates/blaze_repo_rules/src/setup.rs`
  - `crates/blaze_repo_rules/src/starlark/context.rs`
- `Cargo.lock` is dirty too (read for the `ureq` / `webpki-roots` entries).
- The review was split in four: I read integrity and extraction; three forks of this reviewer
  read the client and rc handling, resolution, and extensions and repository rules. I re-read
  the Rust and Java lines of every S1 and of the S2 findings marked "re-read". S2 findings
  marked "fork" were verified on both sides by the fork only.

Paths below: `R/` is `crates/`, `J/` is `src/main/java/com/google/devtools/build/lib/` at the pin,
`C/` is `src/main/cpp/` at the pin.

## Coverage

Full, both sides:

- `blaze_repo_fetch`: `downloader/{download_manager, http_downloader, http_connector,
  http_connector_multiplexer, http_stream, http_utils, checksum, streams}.rs`,
  `cache/{download_cache, local_repo_contents_cache}.rs`, `authandtls/{mod, netrc}.rs`,
  `decompressor/{mod, compressed_tar, strip_prefixed_path, zip, sevenz, compressed}.rs`,
  `file_system_lock.rs`, `fs_utils.rs`.
- `blaze_bzlmod`: `registry_fetcher.rs`, `registry_fetcher/download_cache.rs`,
  `module_file/**`, `skyframe/{module_file_function, resolution_function, dep_graph_function,
  lock_file, registry_functions, config, precomputed, extension_placeholder, mod}.rs`.
- `blaze_bzlmod_model`: `selection.rs`, `discovery.rs`, `resolution.rs`, `version.rs`,
  `module_key.rs`, `module.rs`, `dep_graph.rs`, `module_extension.rs`, `lockfile.rs`,
  `external_deps_exception.rs`, `checksum.rs`; `index_registry.rs` lines 100-640.
- `blaze_repo_rules`: `setup.rs`, `registration.rs`, `services.rs`, `precomputed.rs`,
  `env_handle.rs`, `external_repos.rs`, `attr_values.rs`, `repository/{repository_fetch_function,
  digest_writer, repo_recorded_input, repo_environment_function, repo_definition_function,
  repository_utils, directory_tree_digest, repo_rule, attribute_utils, exceptions}.rs`,
  `bzlmod/{single_extension_eval_function, runnable_extension, eval_thread_context,
  module_extension_context, module_extension_metadata}.rs`, `starlark/{base_external_context,
  context, thread, os, path, execution_result, repository_context}.rs`.
- `blaze_client`: `blaze.rs`, `archive_utils.rs`, `option_processor.rs`, `server.rs`,
  `workspace_layout.rs`, `util/{file, path, md5, numbers, exit_code}.rs`; `startup_options.rs`
  and `platform.rs` for the parts named in the client section. Also `crates/blaze/{build.rs,
  src/main.rs, src/embedded.rs}` (embedded archive and key).
- `blaze_options`: `rc/{rc_file, option_processor, handler, tokenize, client_options,
  sem_ver}.rs`, `global_rc.rs`.

Sampled:

- `blaze_repo_fetch`: `decompressor/patch_util.rs` (file handling, `apply`, path checks; the
  java-diff-utils fuzz port was not diffed), `decompressor/ar.rs`, `downloader/url_rewriter.rs`
  (`update_auth_headers`, `rewrite`), `downloader/proxy_helper.rs`, `errors.rs`.
- `blaze_bzlmod`: `registry_fetcher/http.rs` (proxy check, transport, stream reading, checksum),
  `inspector.rs`.
- `blaze_bzlmod_model`: `repo_spec.rs`, `facts.rs`, `gson.rs`, `registry.rs`.
- `blaze_repo_rules`: `bzlmod/{single_extension_usages, facts}.rs`,
  `starlark/{repository_module, repo_metadata}.rs`, `repository/repo_definition.rs`,
  `semantics_fingerprint.rs`.
- `blaze_client`: `util/logging.rs`, JVM argument building, rlimits and scheduling.

Not reviewed: `blaze_bzlmod/src/modcommand/**`, `blaze_bzlmod_model/src/java_uri.rs`,
`blaze_repo_fetch/src/{uri, repo_utils, java_hash}.rs`, `authandtls/netrc_parser.rs`,
`downloader/{download_progress_event}.rs`, `blaze_repo_rules/src/bzlmod/{starlark_bazel_module,
repo_mapping_entries, tag_class}.rs`, `repository/repository_resolved_event.rs`, the dev
binaries, `blaze_options` outside `rc/`, and all test files except where a finding names one.

Checked and found faithful (no finding):

- Checksum before use and before cache insertion on every download path; cache key type follows
  the checksum's type; `canonical_id` marker files; distdir lookup by checksum; cached entries
  re-hashed on read.
- Redirect handling (`mergeUrls`, user info only to the same origin, 40 redirects, `http`/`https`
  only), per-URL credential lookup after a redirect, no TLS verification bypass anywhere.
- `registryFileHashes` handling in `IndexRegistry.doGrabFile` for all four modes, including the
  `--lockfile_mode=error` message and the "previously not found" case.
- Zip and tar extraction follow the Java, including what the Java does not check: neither side
  rejects `..` in regular entry names, and setuid bits pass through `chmod(mode | 0400)`. The zip
  symlink uplevel check and the 7z uplevel check are present.
- PatchUtil's "outside of external repository" check, rename and delete handling.
- No brotli/re2/workerd special-casing and no oracle seam in the `src` of these crates.

## S1: wrong or unsafe behaviour on valid input

### S1-1. Two Blaze commands can run in one output base at once

- Rust: `R/blaze_client/src/blaze.rs:296-307` (`BlazeServer::communicate`) logs "Released the
  client-side locks", calls `self.release_locks()`, then runs the command in-process with
  `server.run_command(startup, request)`.
- The only command lock after that is a process-local mutex,
  `R/blaze_runtime/src/dispatcher.rs:112-113` ("In-process there is one command per server, so
  the lock is always free"). `R/blaze_runtime/src/runtime.rs:479-481` says the install base lock
  "is held by the client for the whole in-process run", which is not true on this path. A grep
  for `fcntl`/`flock` finds file locks only in `blaze_client/src/platform.rs` and
  `blaze_repo_fetch`.
- C++/Java: `C/blaze.cc:2067-2073` releases the locks because one long-lived server serialises
  its clients (`J/runtime/BlazeCommandDispatcher.java:239-266`) and takes its own shared install
  base lock (`J/runtime/BlazeRuntime.java:1396-1408`).
- Scenario: terminal A runs `blaze build //...`; terminal B runs `blaze test //x` in the same
  workspace. Bazel makes B wait ("Another command (pid=...) is running") or exit 9 with
  `--noblock_for_lock`. Blaze runs both against the same `external/`, marker files, execroot,
  action cache and `MODULE.bazel.lock`. `--noblock_for_lock` never fails.
- Fix direction: keep both client locks held across `run_command`, as `run_batch_mode` already
  does, and keep Bazel's wait and error messages. Correct the `runtime.rs` comment.

### S1-2. `--downloader_config` is accepted and ignored; `--distdir`, `.netrc` and `--credential_helper` too

- Rust: `R/blaze_repo_rules/src/setup.rs:165-178` builds the `DownloadManager` and calls only
  `set_disable_download` and `set_retries`. No non-test code calls `set_url_rewriter`,
  `set_netrc_creds`, `set_credential_factory` or `set_distdir`.
- The registry path has setters that fail loudly
  (`R/blaze_bzlmod/src/registry_fetcher.rs:263-307`: `set_downloader_config`,
  `set_netrc_creds_from_client_env`, `set_credential_helpers`), and the type's doc comment says
  "configuring either fails loudly" (:214-216). Nothing calls them: `setup.rs:200-204` only
  passes the cache path, the reporter and the retry count.
- Java: `J/bazel/BazelRepositoryModule.java:423-427` (netrc), `:432-446` (rewriter; a parse error
  aborts because "this config file may be required for security purposes, and the build must not
  proceed ignoring it"), `:448-477` (credential helpers), `:484` (distdir).
- Scenario: `.bazelrc` has `common --downloader_config=allow.cfg` with `block *` and
  `allow mirror.corp.example`. Bazel refuses every other host. Blaze downloads from the original
  URLs of every repository rule and registry, with no warning.
- Lesser effects: `--distdir` is never consulted; registry files and rewritten URLs get no
  `.netrc` credentials (the `http_archive` netrc path is Starlark and does work).
- Fix direction: port that block of `beforeCommand` for both download stacks. Until then, fail
  when any of these flags is set.

### S1-3. Default repository cache is not under the output user root

- Rust: `R/blaze_repo_rules/src/setup.rs:144`
  `None => setup.output_base.parent().map(|p| p.join("cache/repos/v1")),`
- Java: `J/bazel/BazelRepositoryModule.java:325-330` uses
  `getServerDirectories().getOutputUserRoot().getRelative(DEFAULT_CACHE_LOCATION)`.
- Scenario: `blaze --output_base=/tmp/ob build //...` with no `--repository_cache`. Bazel uses
  `~/.cache/bazel/_bazel_$USER/cache/repos/v1`. Blaze uses `/tmp/cache/repos/v1`, including the
  repo contents cache `contents/` below it. Contents-cache entries are reused after a marker
  check only, so on a shared machine another user who creates `/tmp/cache/repos/v1/contents/...`
  first can supply repository contents.
- Also: a relative `--repository_cache` or `--repo_contents_cache` is used as given
  (`setup.rs:143,158`); Java resolves it against the workspace (`toPath`, `:715-728`).
- Fix direction: carry `output_user_root` in `FetchSetup`; port `toPath`.

### S1-4. `--override_repository=name=%workspace%/path` is not expanded

- Rust: `R/blaze_repo_rules/src/setup.rs:227-232` joins a relative path with the working
  directory and nothing else.
- Java: `J/bazel/BazelRepositoryModule.java:506` calls `getAbsolutePath`, which replaces
  `%workspace%` first (`:700-708`).
- Scenario: `--override_repository=foo=%workspace%/third_party/foo`, the form the flag's help
  documents. Blaze resolves `<cwd>/%workspace%/third_party/foo`.
- `R/blaze_bzlmod/src/skyframe/config.rs:211-220` (`get_absolute_path`) already does this for
  `--override_module` and `--inject_repository`. Reuse it.

### S1-5. Download and file errors from context methods lose the `java.io.IOException: ` prefix

- Rust: `R/blaze_repo_rules/src/starlark/base_external_context.rs` uses
  `eval_error(e.get_message())` for failed downloads (`complete_download`, about :898, and
  `download_and_extract`, about :1789) and `eval_error(e.to_string())` in `create_file` and
  `read_file` (about :1938, :1970). `repository_context.rs:120-122` does it correctly with
  `e.to_java_string()`.
- Java: `J/bazel/repository/starlark/StarlarkBaseExternalContext.java:671, 1165` throw
  `new RepositoryFunctionException(e, Transience.TRANSIENT)`; its message is `e.toString()`, and
  `net/starlark/java/eval/MethodDescriptor.java:295` wraps it in an `EvalException`. The same
  holds for `getUrls` ("urls not set", "Bad URL: ...", :468-491).
- Oracle evidence:
  `~/.cache/blaze/oracle-records/oracle-9.3.2/re2/raw/query_deps_label_kind.stderr` has
  `Error in download_and_extract: java.io.IOException: Error downloading [https://netcologne...`.
  Blaze prints `Error in download_and_extract: Error downloading [...`. This is the corpus case
  that decisions.md compares by exit code and partial result only.
- Fix direction: one helper that builds the message from the cause's Java `toString()`, used by
  every `RepositoryFunctionException(IOException)` site.

### S1-6. `--nofetch` error text differs

- Rust: `R/blaze_repo_rules/src/repository/repository_fetch_function.rs:630-634`
  "...External repository {} not found and fetching is disabled."
- Java: `J/bazel/repository/RepositoryFetchFunction.java:399-404`
  "... not found and fetching repositories is disabled."

## S2: deviations that are masked today

1. **Blaze shares real Bazel's output base, and a live Bazel server makes it mutate that state
   and then panic.** (re-read)
   - `R/blaze_client/src/platform.rs:777-800` (`get_cache_dir`) gives
     `~/.cache/bazel/_bazel_$USER`, so Blaze computes the same output base as Bazel
     (`docs/crates.md`, "Single process first").
   - With an idle Bazel server for the workspace: `ensure_correct_running_version`
     (`blaze.rs:1012-1040`) repoints `<output_base>/install`; `start_server_and_connect`
     (`blaze.rs:934`) deletes `server/command_port`; `ensure_previous_server_process_terminated`
     (`blaze.rs:900`) reaches `unported!("KillServerProcess")` (`platform.rs:605-609`).
   - C++: `C/blaze_util_posix.cc` `KillServerProcess` kills the stale server and continues.
   - Result: every `blaze` command in that workspace panics, and the Bazel server is orphaned.
     Apart from the panic, both tools then trust each other's `external/` marker files and
     action cache.
   - Fix direction: give Blaze its own output user root until the server exists, or check the
     pid file before changing anything.
2. **HTTP downloader built from the wrong options.** (re-read)
   - `R/blaze_repo_rules/src/setup.rs:165-170` passes
     `--experimental_repository_downloader_retries` as the connector attempt count and
     `--experimental_scale_timeouts` as the HTTP timeout scale;
     `R/blaze_bzlmod/src/registry_fetcher.rs:240` hard-codes `(8, ZERO, 8, 1.0)`.
   - Java: `J/runtime/CommandEnvironment.java:302-325` uses `--http_connector_attempts`,
     `--http_connector_retry_max_timeout`, `--http_max_parallel_downloads`,
     `--http_timeout_scaling`.
   - Effect: 5 connect attempts by default for repository rules where Bazel makes 8; the four
     `--http_*` flags do nothing.
3. **Non-positive option values panic.** (re-read)
   - `setup.rs:178` calls `set_retries` unconditionally (asserts `>= 0`,
     `download_manager.rs:132-135`); `setup.rs:195` passes `experimental_scale_timeouts` raw.
   - Java: `BazelRepositoryModule.java:305` sets retries only when `>= 0`; `:310-321` warns and
     uses 1.0 for a non-positive scale.
4. **Repo contents cache is used without the shared lock or the location checks.** (re-read)
   - `acquire_shared_lock` (`R/blaze_repo_fetch/src/cache/local_repo_contents_cache.rs:173`) has
     no caller. `setup.rs` has none of the `BAD_REPO_CONTENTS_CACHE` checks.
   - Java: `BazelRepositoryModule.java:335-417` (lock at :407).
   - Scenario: a real Bazel server sharing the cache runs its idle GC, takes the exclusive lock
     that Blaze never contends for, and moves an entry to `_trash` while a Blaze output base
     symlinks to it.
5. **A truncated HTTP body is not retried by the repository-rule downloader.** (re-read)
   - `ureq` reports a body shorter than `Content-Length` as `UnexpectedEof`;
     `R/blaze_repo_fetch/src/downloader/streams.rs:55-66` passes it on and
     `errors.rs:170` maps it to `EOFException`, which `is_retryable_exception`
     (`download_manager.rs:493-497`) does not retry.
   - Java: the JDK stream ends normally, `CheckContentLengthInputStream.java:71` throws
     `ContentLengthMismatchException`, and `DownloadManager.java:390-393` retries it.
   - The registry stack already handles this (`R/blaze_bzlmod/src/registry_fetcher/http.rs`,
     PORT note above `read_all_bytes_and_close`).
6. **A tar entry with a blank mode field fails extraction.** (re-read)
   - `R/blaze_repo_fetch/src/decompressor/compressed_tar.rs:212` `entry.header().mode()?`; the
     `tar` crate rejects a field of NULs or spaces. The uncommitted change in this file gives
     `mtime` the Commons Compress treatment (`commons_compress_octal_is_zero`) but not `mode`.
   - Java: `J/bazel/repository/decompressor/CompressedTarFunction.java:147`
     `entry.getMode() | 0400`, where Commons Compress returns 0.
7. **Non-UTF-8 bytes are replaced with U+FFFD on several paths.** PORTING.md "Strings" forbids
   lossy conversion; decisions.md records the byte-string migration as pending.
   - Tar entry and link names: `compressed_tar.rs:147`, `strip_prefixed_path.rs:96-98`. Java
     keeps the raw bytes (`CompressedTarFunction.java:88-91`). An archive member `caf\xe9.h` is
     written as `caf\xef\xbf\xbd.h`, and two such names can collide. (re-read)
   - `rctx.read` and `execute` output: `R/blaze_repo_rules/src/starlark/execution_result.rs:145-147`
     against `StarlarkBaseExternalContext.java:1643` (ISO-8859-1). (re-read)
   - rc files, client environment, cwd: `R/blaze_options/src/rc/rc_file.rs:66`,
     `rc/tokenize.rs:70`, `R/blaze_client/src/platform.rs` (`get_env`, `get_processed_env`),
     `util/file.rs:534`. (fork)
8. **SRI strings without padding are rejected.** (re-read)
   - `R/blaze_repo_fetch/src/downloader/checksum.rs:79-83` uses the strict `base64` engine.
   - Java: `J/bazel/repository/downloader/Checksum.java` `base64Decode` uses
     `Base64.getDecoder()`, which accepts missing padding. The sibling
     `R/blaze_bzlmod_model/src/checksum.rs:241` ports that correctly.
   - Scenario: `rctx.download(integrity = "sha256-<43 characters>")` fails with "Invalid base64".
9. **`print()` output is buffered until the implementation returns.** (re-read)
   - `R/blaze_repo_rules/src/starlark/thread.rs:46-58`, replayed at
     `repository_fetch_function.rs:906` and `runnable_extension.rs:521`.
   - Java reports at once (`RepositoryFetchFunction.java:656`). Order relative to
     `execute(quiet = False)` output and direct warnings changes.
10. **Process-wrapper kill delay and graceful SIGTERM are fixed values.** (re-read)
    `base_external_context.rs:245-246` against `J/runtime/ProcessWrapper.java:72-76`, which
    reads `LocalExecutionOptions` and passes no kill delay for commands without them.
11. **`rctx.attr` repr is invented.** (re-read) `R/blaze_repo_rules/src/repository/repo_definition.rs:96-101`,
    with the comment "nothing in the corpus prints `rctx.attr`".
12. **`selectedYankedVersions` order in a newly written lockfile.** (re-read up to the lockfile
    builder) `R/blaze_bzlmod_model/src/resolution.rs:296-320` iterates in discovery order;
    `J/bazel/bzlmod/BazelModuleResolutionFunction.java:211-238` iterates a `HashMap`. Two or
    more allowed yanked versions give a lockfile that is not byte-identical.
13. **`Table::column()` order for dense tables.** (fork)
    `R/blaze_bzlmod_model/src/dep_graph.rs:22-56`, used at :150. Guava's `DenseImmutableTable`
    (chosen when cells exceed half of rows times columns) iterates a column in row order, not
    cell order. Repo-mapping entry order differs when modules use the same extensions in
    different orders.
14. **Tag and override attribute values are copied when the tag is called.** (fork)
    `R/blaze_bzlmod/src/module_file/module_file_globals.rs:361-396` against
    `J/bazel/bzlmod/ModuleFileGlobals.java:592-600`. `l = ["a"]; ext.tag(x = l); l.append("b")`
    gives the extension `["a"]` where Bazel gives `["a", "b"]`.
15. **`package_lookup` stand-in for `include()` and patch labels.** (re-read)
    `R/blaze_bzlmod/src/skyframe/module_file_function.rs:503-534` ignores `--package_path`,
    `--deleted_packages` and `.bazelignore`, where `ModuleFileFunction.java:466-490, 896-915`
    uses PACKAGE_LOOKUP.

## S3: missing ports, marked or loud

Reachable from ordinary flags or environments:

- `HTTPS_PROXY`/`HTTP_PROXY` in the client environment panics on any registry download
  (`R/blaze_bzlmod/src/registry_fetcher/http.rs:330`). The repository-rule stack supports HTTP
  proxies; SOCKS proxies fail there because `ureq` is built without `socks-proxy` (not marked).
- TLS trust: `ureq` with rustls and `webpki-roots` only (`Cargo.toml:29`). The system store is
  not read and there is no equivalent of the JVM trust store options, so private CAs, internal
  mirrors and TLS-inspecting proxies fail with "TLS error". Verification is not weakened.
- A non-UTF-8 byte in any `MODULE.bazel` panics at default flags
  (`R/blaze_bzlmod/src/module_file/compiled_module_file.rs:106-109`); Java warns and continues.
- `--vendor_dir` (`index_registry.rs:210`, `repository_fetch_function.rs:462`).
- `--incompatible_require_repo_extension_metadata` (`repository_fetch_function.rs:347`,
  `runnable_extension.rs:558`), `--experimental_repo_remote_exec`, wasm methods.
- Floats and big ints in tag or override kwargs (`module_file_globals.rs:373,390`),
  `module_ctx.facts[missing]` (`bzlmod/facts.rs:288`), attribute kinds outside the handled set
  (`attr_values.rs:146`).
- Unawaited `block = False` downloads are joined, not cancelled, when a rule fails
  (`base_external_context.rs:711-730`).
- No SIGINT handling (`blaze.rs:1110`); `--extra_classpath`, `exec-server`, a non-empty
  `--invocation_policy`, `--digest_function` other than SHA-256.
- The three `testSingleVersionOverridePatches*` tests are empty `#[ignore]` stubs
  (`R/blaze_bzlmod/tests/module_file_function_test.rs:1228-1236`), while the code they cover is
  an uncommitted change. Three rc_file tests are ignored for a dependency that now exists
  (`R/blaze_options/tests/rc_file_test.rs:275,299,304`).

Count: 13 items.

## S4

- Two separate HTTP stacks (`blaze_repo_fetch/src/downloader` and
  `blaze_bzlmod/src/registry_fetcher/http.rs`) that already differ in proxy support, truncated
  body handling and base64 decoding. The file says it is a temporary stand-in.
- `R/blaze_repo_rules/src/repository/exceptions.rs:134` formats an `EvalException` cause with a
  class-name prefix; Java's `toString` is the message with stack. Unreachable today.
- `lockfile.rs:838-840`: `"lockFileVersion": 99999999999` panics; Java reports BAD_LOCKFILE.
- `module_file_function.rs:862-878` recognises `MissingChecksumException` by message prefix.
- `rc/option_processor.rs:361-364` opens the file where C++ uses `access(R_OK)`; a FIFO blocks.
- `crates/blaze/build.rs:61` embeds the on-disk oracle install base without checking it against
  a pinned digest.
- `rctx.os.environ` is mutable (`starlark/os.rs:106`); `OS_ARCH` is hard-coded in
  `digest_writer.rs:38` while `os.rs` maps aarch64.
- `bin/blaze-fetch-dev.rs:95` hard-codes the oracle install base path.

Count: 8 items.

## Not established

- Whether Commons Compress rejects tar headers with a bad checksum (the `tar` crate does). The
  Commons Compress sources are not in the pinned tree.
- Whether the lockfile serializer sorts `selectedYankedVersions` (S2-12 assumes it does not).
- The java-diff-utils fuzz port in `patch_util.rs` was not compared line by line.

---

# Round 2 (2026-11-18)

Same reviewer role, same read-only rules. Re-check of
`docs/reviews/2026-11-18-bzlmod-repos-response.md` against the code at the commit below and the
pinned Java/C++. Nothing was executed and no Cargo was run; the test counts and oracle
comparisons in the response are the implementer's and were not reproduced.

## Verdict: PASS

Bound to `f938830b1a93d2f7e22efd88cafdfdbb8aaab567`. The six crates of the area had no
uncommitted changes when read. All six S1 findings are closed in the area. The PASS covers the
six crates; it does not cover the one caller outside them:

- `crates/blaze_sky_top/src/skyframe_executor.rs:186` still calls
  `prepare_repository_functions_in`. Through that entry point the real binary gets the
  `--downloader_config`, `.netrc`, `--distdir`, cache-location, lock and `%workspace%` fixes, but
  not `--credential_helper` (silently ignored), the four `--http_*` flags (defaults used) or the
  process-wrapper kill delay (none passed). `--output_base=X` without `--repository_cache`
  fails with an error instead of misplacing the cache. These close when the caller moves to
  `prepare_repository_functions_with`.

## S1

| Finding | Status | Evidence |
| --- | --- | --- |
| S1-1 concurrent commands | CLOSED | `R/blaze_client/src/blaze.rs` `communicate` no longer calls `release_locks`; `run_launcher` takes the locks before `extract_data`, in the order of `C/blaze.cc:1529-1537`. Nothing closes the descriptors before exit. |
| S1-2 downloader config, distdir, netrc | CLOSED | `R/blaze_repo_rules/src/setup.rs` `prepare_repository_functions_with` sets netrc, rewriter and distdir in Java's order with Java's messages; `UrlRewriter::get_downloader_url_rewriter` and `new_credentials_from_netrc` match `UrlRewriter.java:87-120, 313-346`. The old entry points delegate to it. `--credential_helper` is `unported!` when the options reach it. |
| S1-3 cache location | CLOSED | Default is `<output_user_root>/cache/repos/v1`; without an output user root it is derived only when the output base is `<root>/<md5(workspace)>`, else an error. `to_workspace_path` ports `toPath`. |
| S1-4 `%workspace%` | CLOSED | `setup.rs` `get_absolute_path`, used for `--override_repository`. |
| S1-5 `java.io.IOException: ` prefix | CLOSED | `repository_function_exception` over `JavaThrowable`; the download, file and read sites in `base_external_context.rs` go through it. The message chain was re-read against `MethodDescriptor.java:295`. |
| S1-6 `--nofetch` text | CLOSED | `repository_fetch_function.rs:632` has "fetching repositories is disabled". |

Answers to the points raised for S1-1:

- **Held for the whole command:** yes, both locks, until the process exits.
- **Lock acquisition against install-base extraction:** unchanged from the C++: shared install
  base lock, then exclusive output base lock, then extraction into a sibling directory and an
  atomic rename. No new race.
- **Live real-Bazel server:** `find_live_foreign_server` reads `server/server.pid.txt` and
  `server.starttime` only. It runs before the lock is taken and again with the lock held, so
  nothing under the output base is written before the exit 36. A Bazel client starts its server
  while holding the output base lock, so the second check sees it. While Blaze holds the lock a
  Bazel client waits in `AcquireLock`. I accept the refusal. A pid file whose pid was reused and
  whose `server.starttime` is missing gives a false refusal; the C++ would kill that process.

## Unified download stack

Removing `registry_fetcher/http.rs` and `registry_fetcher/download_cache.rs` lost no check.

- Registry files go through `DownloadManager::download_and_read_one_url_for_bzlmod`
  (`R/blaze_repo_fetch/src/downloader/download_manager.rs`). A cached entry is re-hashed before
  use; a download with a checksum passes `HashInputStream` to end of stream before the bytes are
  returned or cached; a download without one is cached under its own SHA-256.
- `IndexRegistry.do_grab_file` (`registryFileHashes`, all four modes) is unchanged.
- `R/blaze_bzlmod/tests/registry_fetcher_test.rs` has `download_and_read_checksum_mismatch`,
  `checksum_mismatch_is_not_cached` and `poisoned_cache_entry_is_downloaded_again` over the new
  seam.
- The checksum conversion between the two crates' `Checksum` types goes by string form and
  keeps the SRI flag.

## S2

| # | Finding | Status | Evidence |
| --- | --- | --- | --- |
| 1 | Shared output base, live Bazel server | CLOSED | Refusal as above; `kill_server_process` has no caller. Sharing the output base when no server is live is unchanged and is root's decision. |
| 2 | HTTP downloader options | CLOSED in the crate | `http_downloader_from_options` ports `CommandEnvironment.java:302-325`. Needs the caller change above. |
| 3 | Non-positive values | CLOSED | Retries set only when `>= 0`; Java's two warnings and 1.0. |
| 4 | Repo contents cache lock and checks | CLOSED | Both `BAD_REPO_CONTENTS_CACHE` checks with Java's text; `acquire_shared_lock` is called. See N3. |
| 5 | Truncated body with `Content-Length` | CLOSED | `streams.rs`: the transport's short-body error becomes end of stream below `CheckContentLengthInputStream`, and `RetryingInputStream` no longer resumes on it. |
| 6 | Blank tar mode | CLOSED | `parse_tar_header_numbers` replaces the `tar` crate's accessors for `mode` and `mtime`. Commons Compress sources are not in the tree; the port follows the implementer's reading of the class file. |
| 7 | Non-UTF-8 bytes | STILL OPEN | Left for the byte-strings migration (decisions.md) and marked `// PORT: byte-strings`. Tar entry names are still written with U+FFFD. |
| 8 | Unpadded SRI | CLOSED | `java_base64_decode` follows `Base64.Decoder.decode0`. |
| 9 | `print()` buffering | CLOSED | See "Print handler" below. |
| 10 | Process-wrapper options | CLOSED in the crate | Needs the caller change above. |
| 11 | `rctx.attr` repr | CLOSED | Default `StarlarkValue.repr` text and type `RepoDefinition`. |
| 12 | `selectedYankedVersions` order | CLOSED | `java_hash_map_order` over `ModuleKey.java_hash_code`; `Version.hashCode` (`Version.java:204-205`) re-derived by hand and matches. |
| 13 | Dense table order | CLOSED | `Table` reorders rows and columns when the dense condition holds. |
| 14 | Tag kwargs | CLOSED | Values stay on the file's heap and are converted in `resolve_deferred_attribute_values` after execution. Mechanism read; tests not run. |
| 15 | `package_lookup` | CLOSED | `setup.rs` installs `PackageLookupFunctionLookup` over PACKAGE_LOOKUP; the stand-in is `unported!` on a `.bazelignore`. |

## Print handler (S2-9)

`R/blaze_repo_rules/src/starlark/thread.rs`. The behaviour is right: DEBUG and WARNING events are
not stored for replay (`J/events/Event.java:75-80`), so reporting them to the environment's
listener at once matches Java.

The thread-local stack is correct in use:

- The guard pops on return and on unwind, so the handle does not outlive the call.
- `SkyEnv` is not `Send` and the stack is per thread; the evaluation runs on the calling thread.
- A nested implementation on the same thread pushes its own environment and pops it first.
- No native in the crate emits a Starlark event while it holds a borrow of the environment, so
  `report` never overlaps another `&mut Environment`.

One defect remains, filed as N2: the second handle is made by reborrowing through the first
(`SkyEnv::scope(payload.base.env.get(), |env| env)`), so every later `base.env.get()`
invalidates it under Stacked Borrows. I do not expect a miscompile, but Miri would reject it.

## TLS decision

Accepted. The embedded JDK in the install base has its own `lib/security/cacerts` (a 128 KB
regular file), so Bazel at the pin does not read the system store either. `webpki-roots` only is
a defensible stand-in, and `--host_jvm_args=-Djavax.net.ssl.trustStore*` / `keyStore*` now fails
with exit 2. Two limits, both S3:

- The two root sets are not the same in either direction. Exact parity is reading the install
  base's `cacerts` (deferred by the implementer).
- `--server_javabase` pointing at a JDK with another trust store changes Bazel's trust and is
  still accepted and ignored.

## `rctx.os.environ`: withdrawn

The implementer is right. `StarlarkOS.java:56` returns an `ImmutableMap`, and
`MethodDescriptor.java:315` converts it with `Starlark.fromJava(result, mu)`, which is
`Dict.copyOf(mutability, map)` (`Starlark.java:215-216`): a new mutable dict on each access.

## NEW

- **N1 (S2). A chunked response cut off inside a chunk is accepted as complete.**
  - `ureq` 2.12.1 `src/chunked/decoder.rs`: inside a chunk, `self.source.read(buf)` returning 0
    is passed on as `Ok(0)`. Java's chunked stream throws "Premature EOF".
  - With a checksum the hash check catches it. Without one (`rctx.download` with no `sha256` or
    `integrity`, or a registry file not yet in the lockfile) the truncated bytes are returned
    and cached under their own SHA-256, and a registry file's hash goes into
    `registryFileHashes`.
  - Mitigation today: registry requests ask for gzip, and a truncated gzip stream fails in
    `GzipInputStream`.
  - The implementer noticed this and left it. It existed in round 1; I missed it.
  - Fix direction: detect `Transfer-Encoding: chunked` and verify the terminating chunk, or
    decode chunks in the transport wrapper.
- **N2 (S4). The print handler's second `SkyEnv` handle** (above). Fix direction: an explicit
  alias method on `SkyEnv` that copies the raw pointer under a stated contract, or a clonable
  listener from `blaze_skyframe`.
- **N3 (S4). The repo contents cache lock is a process-wide POSIX lock**
  (`R/blaze_repo_fetch/src/file_system_lock.rs`). A second setup in the same process, then a
  drop of the first, releases it for both. One setup per process today
  (`blaze-fetch-dev` builds its evaluator in two separate code paths).
- **N4 (S2). A tar header with a bad checksum** fails in the `tar` crate where Commons Compress
  extracts. Round 1 "not established"; the implementer confirmed it and left it. The failure is
  loud.

## Still open from round 1, not blocking

- S2-7 (byte strings).
- S3: `--vendor_dir`, `--incompatible_require_repo_extension_metadata`, credential helpers,
  floats in tag kwargs, cancellation of unawaited downloads, SIGINT, SOCKS proxies, non-UTF-8
  `MODULE.bazel`.
- S4: `build.rs` digest check, `MissingChecksumException` by message prefix, the dev binary's
  hard-coded install base.
- Setup failures return a message only, so the caller cannot map them to
  `BAD_DOWNLOADER_CONFIG` or `BAD_REPO_CONTENTS_CACHE` exit codes.

Closed S3/S4 items that I confirmed by reading: the `HTTPS_PROXY` panic and the duplicate HTTP
stack (both files are gone). The other S3/S4 fixes listed in the response (rc tests, `access`,
`lockFileVersion`, `EvalException` cause text, `OS_ARCH`) were not re-read.

---

# Round 3 (2026-11-18)

Narrow scope, same read-only rules: the hand-written HTTP transport that replaced `ureq`, and
the fixes for N2, N3 and N4. No Cargo was run and Blaze was not run. I disassembled classes with
`javap` to check two claims about libraries whose sources are not in the pinned tree:
`commons-compress-1.26.1.jar` (from the upstream-tests cache) and the JDK's
`sun.net.www.protocol.http.HttpURLConnection` (system JDK 11, not the embedded 25).

## Verdict: STOP (for these changes)

One blocking finding, R3-1, in the new transport's request construction. Everything else in
scope passes: TLS setup, response framing, redirects, proxy scoping, checksum-before-cache, the
tar checksum leniency, `SkyEnv::alias` and the lock table.

## Tree

- `git rev-parse HEAD` = `03409d9a843bac8e19e19af31c65e3e3b2b08392`, with uncommitted changes.
  The verdict is bound to the working tree as read, not to a commit.
- Files read in full: `R/blaze_repo_fetch/src/downloader/socket_transport.rs` (untracked),
  `R/blaze_repo_fetch/src/decompressor/tar_checksum.rs` (untracked),
  `R/blaze_repo_fetch/src/file_system_lock.rs`, `R/blaze_repo_rules/src/env_handle.rs`.
- Diffs read: `downloader/{http_connector, streams, mod}.rs`, `decompressor/{compressed_tar,
  mod}.rs`, `R/blaze_repo_rules/src/starlark/thread.rs`, `R/blaze_repo_fetch/Cargo.toml`.
- Not read: the changed and new tests beyond their names.
- `download_manager.rs`, `http_downloader.rs`, `http_stream.rs` and `http_utils.rs` are
  unchanged since round 2.

## R3-1 (blocking). Request headers are written without the JDK's checks

- Rust: `socket_transport.rs:616-618` writes every caller header as `{k}: {v}\r\n`.
  `HttpConnector::connect` (`http_connector.rs`) passes on the `headers` argument of
  `rctx.download` / `download_and_extract`, the `auth` patterns and the credentials unchanged.
  Nothing between Starlark and the socket validates a name or a value.
- JDK: `HttpURLConnection.addRequestProperty`, which `J/bazel/repository/downloader/
  HttpConnector.java:149` calls, does two things first (confirmed in the bytecode):
  - `checkMessageHeader`: a name containing LF or `:`, or a value containing LF not followed by
    a space or tab, throws `IllegalArgumentException` ("Illegal character(s) in message header
    field: ..." / "... value: ..."). `HttpConnector.java:245-246` turns that into
    `UnrecoverableHttpException`.
  - `isRestrictedHeader`: `Host`, `Connection` (other than `close`), `Content-Length`,
    `Transfer-Encoding`, `Upgrade`, `Via`, `Keep-Alive`, `Origin`, `Trailer`,
    `Content-Transfer-Encoding` and the two `Access-Control-Request-*` names are dropped
    silently.
- Scenario 1 (valid input, different request): `rctx.download(url, headers = {"Host":
  "other.example"})`. Bazel sends `Host` from the URL. Blaze sends `Host: other.example`
  (`socket_transport.rs:604` suppresses its own `Host` when the caller has one), so the server
  answers for another virtual host. With `--downloader_config`, which filters by URL host, this
  reaches a blocked name through an allowed front. `Transfer-Encoding: chunked` or
  `Content-Length: 5` on the GET likewise go out.
- Scenario 2 (regression): a token read from a file with its trailing newline,
  `headers = {"Authorization": "Bearer " + rctx.read(f)}`. Bazel fails with "Illegal
  character(s) in message header value". Blaze sends `Authorization: Bearer tok\n\r\n`, which
  ends the header block early; the remaining header lines reach the server as the start of a
  second request. The `ureq` transport rejected such a header (`Header::validate`), so this
  part is new with this change.
- Fix direction: port `checkMessageHeader` and `isRestrictedHeader` to the point where
  `HttpConnector` adds request properties, with the JDK's messages, and have the transport
  refuse CR or LF in anything it writes as a last line of defence.
- Reach: the URL parts are safe. `JavaUri` rejects control characters and whitespace
  (`R/blaze_repo_fetch/src/uri.rs:89`), so the request target, `Host` and the `CONNECT` line
  cannot carry CR or LF, including after a redirect.

## Transport: what was checked and holds

- **TLS.** `https` is decided by the URL scheme alone; there is no path from a TLS failure to a
  plain connection. rustls with the default verifier checks the chain against `webpki-roots`
  and the certificate against `ServerName` built from the URL host; SNI is on by default. TLS
  1.2 and 1.3. No ALPN is offered, so the peer speaks HTTP/1.1. Through a proxy, TLS runs
  inside the `CONNECT` tunnel against the origin's name. Certificate errors carry
  "certificate" in rustls's text, so `HttpConnector` makes them unrecoverable as Java does.
- **Close without `close_notify`** is treated as end of stream (`read_raw`). The JDK does the
  same by default, and the framing or the checksum then decides. A close-delimited HTTPS body
  with no checksum can be truncated undetected on both.
- **Framing.** Chunked takes precedence over `Content-Length`; `Transfer-Encoding` must equal
  `chunked`; the last header of a name wins. All three match the JDK. A chunked body ending
  anywhere before the empty line after the trailer is "Premature EOF". A short fixed-length
  body ends normally and `CheckContentLengthInputStream` reports it. 1xx, 204 and 304 have no
  body. Only GET is sent.
- **Limits.** Header lines are capped at 100 KB and 1000 fields; chunk-size lines at 100 KB.
  Bodies are streamed. Every read and write has the read timeout; the JDK has no overall
  deadline either.
- **Redirects and credentials.** Still in `HttpConnector` (40 hops, `mergeUrls`, `http`/`https`
  only, per-URL credential lookup at each hop). `Proxy-Authorization` goes to the proxy only:
  in the `CONNECT` request for https, on the absolute-form request for http, never inside the
  tunnel.
- **Connections** are not reused, so a mis-framed response cannot desynchronise a later one.
- **Proxies.** HTTP proxies by absolute form or `CONNECT`; SOCKS is `unported!`.
- **gzip** is still handled in `HttpStreamFactory`, as Java does; the transport does not decode.
- **Checksum before cache.** The transport only replaces the byte source under the unchanged
  `Retrying` / `CheckContentLength` / `Gzip` / `Hash` stack, and the cache writes in
  `DownloadManager` still follow a complete read.

## Transport: smaller deviations (not blocking)

- **R3-2 (S3). TLS and proxy paths have no test.** The implementer says so; TLS was tried by
  hand and the proxy code was never run. For new code that carries credentials and verifies
  peers, that is the main residual risk.
- **R3-3 (S4). Response header folding.** A continuation line (leading space or tab) is parsed
  as a header of its own if it contains a colon; the JDK appends it to the previous value. A
  line without a colon is dropped.
- **R3-4 (S4). Stricter chunk syntax than the JDK.** A chunk-size line must end in CR LF and be
  all hex before `;`; the JDK reads the leading hex digits and does not require the CR there.
  A lenient server fails with "missing CR" or "Bogus chunk size".
- **R3-5 (S4). `Transfer-Encoding: chunked` on an `HTTP/1.0` response** is ignored
  (`socket_transport.rs:638`). I believe the JDK decodes it regardless of version; not
  verified.
- **R3-6 (S4). `Proxy-Authorization` is written twice** for http through an authenticated
  proxy: once by `HttpConnector` and once by the transport (`:613`).
- **R3-7 (S4). Unbounded loops under a hostile server:** interim 1xx responses and trailer
  lines have no count limit. Each is bounded per read by the timeout, as in the JDK.
- **R3-8 (S4). Every resolved address is tried** with the full connect timeout; the JDK tries
  one. Slower failure, more successes.
- **R3-9 (S4). `Accept: */*` and no `Authorization` from URL user info.** Neither matches
  `ureq`; the second matches the JDK, which does not send user info either.

## N4. Tar checksum leniency: CLOSED

- Confirmed from the 1.26.1 bytecode: `TarUtils.verifyCheckSum` starts with
  `parseOctal(header, 148, 8)`; `TarArchiveEntry.parseTarHeaderUnwrapped` stores its result in
  `checkSumOK`; `TarArchiveInputStream` never reads `isCheckSumOK`; and
  `parseTarHeader` wraps `IllegalArgumentException` as `IOException("Corrupted TAR archive.")`.
- `tar_checksum.rs` does exactly that for the checksum field: `parse_octal` on bytes 148-156,
  then the field is rewritten to the sum the `tar` crate computes (`tar` 0.4.46
  `next_entry_raw`). Header positions follow the crate: size field, PAX `size` only for
  non-extension headers, PAX state dropped after any entry that does not describe the next one,
  stop at a zero or short block. I compared each rule with the crate's source.
- Two limits, both stated in the code: headers after a GNU sparse entry keep the crate's check,
  and the other header fields are not made lenient here (a blank `size` still fails in the
  crate; `uid`/`gid` are not parsed at all, where Commons Compress fails on a non-octal one).
  So the checksum rule equals Commons Compress; whole-header parity does not.

## N2. `SkyEnv::alias`: CLOSED

- `alias` copies the raw pointer, so both handles have the provenance `scope` created and a use
  of one no longer invalidates the other. `report` now takes a shared reference through
  `SkyEnv::listener` and holds it for one `handle` call.
- Sound under the stated contract, which the code keeps: the guard drops the alias before
  `call_implementation` returns, the handle is not `Send`, and no native emits a Starlark event
  while it holds a reference from the first handle. Not run under Miri, by the implementer or
  by me.

## N3. Lock table: CLOSED

- `file_system_lock.rs` keeps one descriptor per locked file (keyed by device and inode) and
  counts shared holders, so the first of two setups no longer releases the lock for the second.
  A conflicting second lock in one process panics as the JVM throws.
- The table mutex is held while waiting for another process's lock (documented). An in-process
  GC run while the shared lock is held would panic; nothing schedules one.

## N1. Chunked truncation: CLOSED by the new transport

Closed in substance; the transport that closes it is the subject of R3-1.

## Re-check needed

R3-1 only. The same reviewer role should read the header check where it is added and confirm
the JDK's two messages and the restricted list.

---

# Round 4 (2026-11-18)

Narrow scope, same read-only rules: the answer to R3-1, the test CA, and regressions from the
round-3 response. No Cargo and no Blaze run. I used `javap` on the embedded JDK 25.0.2
(`--system <install base>/embedded_tools/jdk`) and on `commons-compress-1.26.1.jar`, and
`openssl` to inspect the three files under `testdata/`.

## Verdict: PASS (for these changes)

Bound to the uncommitted working tree on `03409d9a843bac8e19e19af31c65e3e3b2b08392` as read. A
commit of this tree should be named to me if the verdict is to be tied to a hash.

Read in full: `R/blaze_repo_fetch/src/downloader/socket_transport.rs` (non-test part, lines
1-800, and the test CA section), `decompressor/tar_checksum.rs` (non-test part), the diff of
`downloader/http_connector.rs`, `Cargo.toml` and `R/blaze_repo_fetch/Cargo.toml`. Tests were
read by name only, except the TLS helpers.

## R3-1. Request header checks: CLOSED

Compared with the bytecode of `sun.net.www.protocol.http.HttpURLConnection` in the embedded JDK:

| Point | JDK 25.0.2 | Port (`http_connector.rs`) |
| --- | --- | --- |
| Name check | LF or `:` in the name throws "Illegal character(s) in message header field: " + name | same test, same text |
| Value check | an LF not followed by space or tab throws "Illegal character(s) in message header value: " + value | same loop, same text |
| Folding | LF followed by space or tab is allowed | allowed |
| Restricted set | twelve names in `static {}`: `Access-Control-Request-Headers`, `Access-Control-Request-Method`, `Connection`, `Content-Length`, `Content-Transfer-Encoding`, `Host`, `Keep-Alive`, `Origin`, `Trailer`, `Transfer-Encoding`, `Upgrade`, `Via` | the same twelve |
| `sec-` prefix | `startsWith("sec-")` after lower-casing | same |
| Case | `toLowerCase(Locale.ROOT)` then set lookup | lower-cased, then lookup |
| `Connection: close` | allowed (`equalsIgnoreCase`) | allowed |
| Restricted header | dropped silently | dropped silently |

- The checks sit where Java calls `addRequestProperty` / `setRequestProperty`
  (`HttpConnector.java:149`), and the error becomes `UnrecoverableHttpException` with the JDK's
  message, as `HttpConnector.java:245-246` does. It is not retried and nothing is sent.
- The transport re-checks every header it writes, refuses a caller `Host`, and always writes
  `Host` from the URL (`socket_transport.rs` `check_request_header`, `request_head`). The head
  is built before the socket is opened.
- Stricter than the JDK, and marked: CR in a name and a bare CR in a value are refused.
- Same as the JDK, and therefore left: other control bytes in values, and names with spaces. A
  header named `Host ` (trailing space) is not restricted in either and goes out as its own
  line beside the real `Host`.

## Test CA and key: no production path

- `include_bytes!("testdata/...")` appears three times, all inside `#[cfg(test)] mod tests` of
  `socket_transport.rs` (lines 801 onward). No other file in the three crates names `testdata`,
  and there is no build script.
- The only way to pass a TLS configuration is the private `open_connection(..., tls)`;
  `SocketTransport::open` passes `None`, which is the `webpki-roots` configuration.
- `test_ca.der`: self-signed, `CA:TRUE`. `test_server.der`: `CN=localhost`, SAN `localhost`,
  `CA:FALSE`, issued by the test CA. `test_server_key.pk8` matches the server certificate's
  public key and not the CA's. The CA's key is not in the tree, so the checked-in key cannot
  issue certificates.

## Other round-3 items

- R3-2 (tests): added; names read, not run by me.
- R3-3 folding, R3-4 chunk syntax, R3-5 chunked on HTTP/1.0, R3-6 single `Proxy-Authorization`,
  R3-8 one address, R3-9 request head order: read in the code and consistent with the
  response. Chunk truncation is still "Premature EOF" at every point.
- R3-7: unchanged, as the JDK.
- `ureq` is gone from `Cargo.toml` and `Cargo.lock`.

## Tar header parity

- `uid`, `gid` and `size` now follow `parseOctalOrBinary`. Confirmed in the bytecode: the three
  8-byte fields go through the non-lenient instance method, `size` through the static one, and
  a negative size is `IOException("broken archive, entry with negative size")`.
- The size field is rewritten only when the crate would read a different value, and the
  reader's own data count uses the rewritten header, so header positions stay in step.
- Extended sparse headers are passed through by the `isextended` flags at offsets 482 and 504,
  as the crate reads them.

## New, not blocking

- **R4-1 (S4).** A trailer line has no length cap (`read_chunk_line(reader, None)`); round 3's
  transport capped it at 100 KB. The JDK has no cap either. A hostile server can grow memory.
- **R4-2 (S4).** Sparse entry contents, `devmajor`/`devminor` and the GNU time fields were not
  compared with Commons Compress (stated by the implementer).

## Still open from earlier rounds, not blocking

- `blaze_sky_top` must call `prepare_repository_functions_with` (round 2).
- S2-7 byte strings; the S3 list of round 1; the false refusal for a reused pid;
  `--server_javabase` and TLS trust.
- No end-to-end test of a proxy configured through the environment; SOCKS is `unported!`.
