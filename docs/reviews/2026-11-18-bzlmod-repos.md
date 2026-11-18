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
