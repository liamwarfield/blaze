# Response to review 2026-11-18: bzlmod, repository fetching, client, rc handling

Implementer response to `docs/reviews/2026-11-18-bzlmod-repos.md` (verdict STOP).

Tree: uncommitted working tree on top of `0015449`, with other agents' uncommitted work in other
crates. Work was split over three sub-implementers (client and rc; fetch internals and the bzlmod
model; repository-rule contexts); the wiring in `setup.rs`, the registry fetcher and all
compiling, testing and verification were done by the lead implementer of this response.

Paths: `R/` is `crates/`, `J/` is `src/main/java/com/google/devtools/build/lib/` at the pin,
`C/` is `src/main/cpp/` at the pin.

## Verification (final tree)

| Check | Result |
| --- | --- |
| `rustfmt --edition 2024 --check` on the 75 changed or new `.rs` files | clean |
| `clippy --all-targets --no-deps -D warnings` on the six crates | clean |
| `cargo test -p blaze_client` | 145 passed, 0 failed, 13 ignored |
| `cargo test -p blaze_options` | 407 passed, 0 failed, 150 ignored |
| `cargo test -p blaze_repo_fetch` | 326 passed, 0 failed, 14 ignored |
| `cargo test -p blaze_bzlmod_model` | 117 passed, 0 failed, 10 ignored |
| `cargo test -p blaze_bzlmod` | 220 passed, 0 failed, 35 ignored |
| `cargo test -p blaze_repo_rules` | 80 passed, 0 failed, 133 ignored |
| `scripts/w5r/compare-repos --project brotli` | 67/67 MATCH, lockfile MATCH, hidden 11/11 |
| `scripts/w5r/compare-repos --project re2` | 81/81 MATCH, lockfile MATCH, hidden 20/21 |
| `scripts/w1/compare-info --project brotli` | MATCH 42, MISMATCH 0, BLAZE-UNPORTED 15 |
| `scripts/w1/compare-info --project re2` | MATCH 42, MISMATCH 0, BLAZE-UNPORTED 15 |

Notes on the table:

- All Cargo ran through `scripts/cargo-capped`. These are full runs of each crate's tests, not
  focused runs. `--no-deps` is needed for clippy because `blaze_query_engine` (not in this area)
  currently fails `-D warnings`.
- re2's hidden lockfile was 20/21 before this work too (`docs/workerd.md:100`): the missing
  extension needs `apple_common.dotted_version`, which is outside this area.
- compare-info was 39 MATCH when the task was written; it is 42 now because of other agents'
  work. No case mismatches.
- workerd was not run.

## S1

### S1-1. Two commands in one output base: fixed

- Where: `R/blaze_client/src/blaze.rs:271` (`BlazeServer::communicate`). The locks are no longer
  released before `server.run_command`. Both the exclusive output base lock and the shared
  install base lock are held until the process exits, as `RunBatchMode` does.
- `acquire_lock` (`R/blaze_client/src/platform.rs:496`) was re-read against
  `C/blaze_util_posix.cc:692` and needed no change: OFD lock with POSIX fallback, owner file,
  500 ms retry, exit code 9.
- Messages are the pin's: "Another command holds the output base lock: \n<owner>", "Waiting for
  it to complete...", "Exiting because the output base lock is held and --noblock_for_lock was
  given." The wording "…on the client..." does not exist at 9.3.2.
- Server-side semantics (`J/runtime/BlazeCommandDispatcher.java:239-266`) do not apply: with one
  command per process the second client always waits or exits in `AcquireLock`, which is what
  Bazel prints in `--batch` mode. A `// PORT:` comment at the site says so.
- Tests: `R/blaze_client/tests/client_test.rs` (new). Lock behaviour:
  `acquire_lock_writes_owner_information`, `acquire_lock_noblock_fails_while_the_lock_is_held`,
  `acquire_lock_succeeds_after_release`, `acquire_lock_block_waits_for_release`,
  `shared_locks_do_not_exclude_each_other`. The fix:
  `communicate_keeps_the_client_locks_for_the_whole_command`,
  `client_holds_locks_while_the_in_process_server_runs`, `client_holds_locks_in_batch_mode`,
  `test_noblock_for_lock_with_batch` (from `client_test.sh`),
  `noblock_for_lock_without_batch_fails_on_the_client_lock`.
- Ignored with the upstream names kept: `test_noblock_for_lock_reuse_server`,
  `test_noblock_for_lock_new_server`, `test_multiple_commands_same_output_base`. They expect the
  server's wording and need a long-lived server.
- For the integration lead: the comments at `R/blaze_runtime/src/runtime.rs:479-481` and
  `dispatcher.rs:112-113` are now true in substance but should name the client lock.

### S1-2. `--downloader_config`, `--distdir`, `.netrc`, `--credential_helper`: fixed

- Where: `R/blaze_repo_rules/src/setup.rs:274` (`prepare_repository_functions_with`), a port of
  that block of `J/bazel/BazelRepositoryModule.java:276-490` in Java's order.
  - `.netrc` (`:442`): a parse error is Java's warning.
  - `--downloader_config` (`:452`): a missing or unparsable file stops the command with Java's
    "Failed to parse downloader config..." message.
  - `--distdir` (`:496`): relative entries are relative to the workspace.
  - `--credential_helper` (`:482`): the helper protocol is not ported, so a set flag is
    `unported!("GoogleAuthUtils.newCredentialHelperProvider", ...)`.
- One download stack: `R/blaze_bzlmod/src/registry_fetcher.rs` is now only
  `DownloadManagerRegistryFetcher`, over the same `Arc<DownloadManager>` the repository rules
  use (Java hands one manager to all functions). The stand-in is deleted:
  `registry_fetcher/http.rs`, `registry_fetcher/download_cache.rs`,
  `LocalDownloadManagerFetcher`, the `repo-fetch` feature and the crate's `ureq` dependency.
  This also closes the S3 item "`HTTPS_PROXY` panics on registry downloads" and the S4 item "two
  HTTP stacks".
- Tests: `R/blaze_repo_rules/tests/setup_test.rs` (new):
  `downloader_config_is_applied_to_repository_and_registry_downloads`,
  `bad_downloader_config_stops_the_command`, `distdir_is_consulted_before_the_network`,
  `netrc_credentials_are_sent_to_registries_and_repository_downloads`,
  `unparsable_netrc_is_a_warning`, `credential_helper_fails_loudly`.
- `R/blaze_bzlmod/tests/registry_fetcher_test.rs` now drives the seam (23 tests).
  `download_and_read_one_url_for_bzlmod_falls_back_to_next_rewritten_url` is un-ignored and
  ported. The in-memory `DownloadCacheTest` slice went with the stand-in; that Java test is
  ported in `R/blaze_repo_fetch/tests/download_cache_test.rs`.

### S1-3. Default repository cache location: fixed, needs one change by the integration lead

- Where: `setup.rs:194` (`to_workspace_path`, Java `toPath`) and the cache block of
  `prepare_repository_functions_with`. The default is
  `<output_user_root>/cache/repos/v1`; relative `--repository_cache` and `--repo_contents_cache`
  resolve against the workspace.
- `FetchSetup` has no output user root and is constructed by `blaze_sky_top`, so the value comes
  through a new `CommandInputs` struct (`setup.rs:75`) and the new entry point. The old entry
  points still compile and behave as follows (`setup.rs:227`):
  - output base is the client's default `<root>/<md5(workspace)>`: its parent is the output
    user root;
  - any other output base with no `--repository_cache`: the setup returns an error naming the
    flag. It no longer puts a cache next to the output base.
- Until the lead passes the output user root, `blaze --output_base=X build` without
  `--repository_cache` fails with that error.
- Tests: `default_repository_cache_is_under_the_output_user_root`,
  `default_repository_cache_without_an_output_user_root`,
  `relative_repository_cache_and_repo_contents_cache_are_relative_to_the_workspace`.
- `blaze-mod-dev` has no output user root either and now requires `--repository_cache`.

### S1-4. `%workspace%` in `--override_repository`: fixed

- Where: `setup.rs:207` (`get_absolute_path`, `J/bazel/BazelRepositoryModule.java:700`).
  `--override_module` and `--inject_repository` already went through the same port in
  `R/blaze_bzlmod/src/skyframe/config.rs`.
- Test: `override_repository_expands_workspace_and_resolves_against_the_working_directory`.

### S1-5. `java.io.IOException: ` prefix: fixed

- Where: one helper, `repository_function_exception`
  (`R/blaze_repo_rules/src/starlark/base_external_context.rs:114`), over a new `JavaThrowable`
  (`R/blaze_repo_rules/src/repository/exceptions.rs:128`) that carries the Java class name.
  Every `RepositoryFunctionException` site in `base_external_context.rs`,
  `repository_context.rs` and `path.rs` goes through it.
- File system errors map to the class `unix_jni.cc` throws (`FileNotFoundException`,
  `FileAccessException`, ...).
- Two sites correctly have no prefix, as in Java: `getPathFromLabel` (uses the cause's message)
  and `path.realpath` (throws the `IOException` directly).
- Tests: `R/blaze_repo_rules/tests/repository_context_messages_test.rs` (new, 16 tests),
  including `download_and_extract_failure_keeps_io_exception_class_name` (the oracle's re2
  text), `test_disable_download_should_prevent_downloading`,
  `bad_url_keeps_io_exception_class_name`, `file_failure_keeps_io_exception_class_name`,
  `read_failure_keeps_file_not_found_exception_class_name`; 10 unit tests in `exceptions.rs`.
- Residual: `blaze_vfs` drops errno, so EACCES and EINVAL are recognised by `ErrorKind` plus the
  strerror suffix; rarer errnos print `java.io.IOException`.

### S1-6. `--nofetch` text: fixed

- Where: `R/blaze_repo_rules/src/repository/repository_fetch_function.rs:632`. The other
  messages in that function were compared with the Java and match.
- Test: `test_repository_and_nofetch`.

## S2

1. **Shared output base with a live Bazel server: fixed (refuses).**
   - `find_live_foreign_server` (`R/blaze_client/src/blaze.rs:945`) reads
     `server/server.pid.txt` and verifies the process as
     `EnsurePreviousServerProcessTerminated` does. `run_launcher` checks before taking the lock
     and again with the lock held, so nothing under the output base is changed, not even the
     lock file.
   - Blaze exits 36 (`LOCAL_ENVIRONMENTAL_ERROR`) with "a running Bazel server (pid=N) owns the
     output base ... Run 'bazel shutdown' ... or ... a different --output_user_root or
     --output_base."
   - What Bazel's own client does instead (in the `// PORT:` comment): it connects; kills the
     server if the install base or startup options differ, or with SIGKILL if it does not
     answer; then starts a new one.
   - A stale pid file works as before. `kill_server_process` stays `unported!` and has no
     caller.
   - Tests: `find_live_foreign_server_without_pid_file`, `..._with_stale_pid_file`,
     `..._with_live_server`, `client_refuses_output_base_of_live_foreign_server` (snapshot of
     the output base unchanged across four commands), `client_ignores_stale_server_pid_file`,
     `client_starts_without_server_pid_file`.
   - Not changed: with no live server Blaze still shares Bazel's output base. Giving Blaze its
     own output user root is a root decision.
2. **HTTP downloader options: fixed.** `setup.rs:246` (`http_downloader_from_options`, port of
   `J/runtime/CommandEnvironment.java:302-325`). The registry path uses the same downloader.
   Tests: `http_connector_attempts_come_from_the_http_option`,
   `http_max_parallel_downloads_must_be_positive`. Needs `CommonCommandOptions` from the lead;
   without it the option defaults (8, 0s, 8, 1.0) apply.
3. **Non-positive values: fixed.** Retries are set only when `>= 0`; a non-positive
   `--experimental_scale_timeouts` or `--http_timeout_scaling` gives Java's warning and 1.0.
   Test: `non_positive_scales_and_negative_retries_are_ignored_with_java_s_warnings`.
4. **Repo contents cache lock and checks: fixed.** Both `BAD_REPO_CONTENTS_CACHE` checks with
   Java's text, and `acquire_shared_lock` (`setup.rs:421`). The lock is released when the
   command's services are dropped (Java: `afterCommand`). The GC idle task needs an idle server
   and is not scheduled. Tests: `repo_contents_cache_inside_the_main_repo_is_refused`,
   `output_base_inside_the_repo_contents_cache_is_refused`,
   `repo_contents_cache_shared_lock_is_held_for_the_command` (probed from a second process).
5. **Truncated body: fixed.** `R/blaze_repo_fetch/src/downloader/streams.rs:274,315`: the
   transport's short-body error becomes `ContentLengthMismatchException` "Bytes read N but
   wanted M", which the download manager retries. Tests:
   `R/blaze_repo_fetch/tests/check_content_length_input_stream_test.rs` (new, 9; Java has no
   test class for this).
6. **Blank tar mode: fixed.** `R/blaze_repo_fetch/src/decompressor/compressed_tar.rs:78`: ports
   of Commons Compress `TarUtils.parseOctalOrBinary` and friends for `mode` and `mtime`. Tests:
   `tests/tar_header_fields_test.rs` (new, 8) and 4 unit tests.
7. **Non-UTF-8 bytes: left for the byte-strings migration, marked.** 56 `// PORT: byte-strings`
   comments across the six crates, one at every `from_utf8_lossy` / `to_string_lossy` site
   outside tests (and at lossy path conversions in `setup.rs` and `registry_fetcher.rs`), each
   saying what Java keeps. Not marked: `R/blaze_client/src/util/logging.rs:406`, a test buffer.
8. **Unpadded SRI: fixed.** `R/blaze_repo_fetch/src/downloader/checksum.rs:159`: a port of
   `java.util.Base64.Decoder.decode0`. 7 unit tests, including
   `from_subresource_integrity_accepts_missing_padding`; expected values were checked against a
   real JDK.
9. **`print()` as it happens: fixed.** `R/blaze_repo_rules/src/starlark/thread.rs:99`
   (`DebugPrintHandler`) forwards DEBUG and WARNING events to the environment's listener during
   the call; the replays are removed. Tests: `repository_rule_print_is_reported_when_it_runs`,
   `module_extension_print_is_reported_when_it_runs`.
   - Implementation note: the handler reaches the Skyframe environment through a thread-local
     stack holding a second `SkyEnv` handle, which bends the documented contract of
     `SkyEnv::scope`. It adds no `unsafe` of its own, but a reviewer should look at it.
   - Not asserted: order relative to `execute(quiet = False)`, which writes to the process
     stderr where the test handler cannot see it. The tests order prints against
     `report_progress` posts instead.
10. **Process-wrapper options: fixed.** `RepoServices` has `process_wrapper_kill_delay` and
    `process_wrapper_graceful_sigterm`, set in `setup.rs` from `LocalExecutionOptions` (absent
    options: no `--kill_delay`, as Java). Command line built at
    `base_external_context.rs:222`. Tests:
    `process_wrapper_options_come_from_local_execution_options` and three in
    `base_external_context.rs`. Needs `LocalExecutionOptions` from the lead; until then no kill
    delay is passed, which is Java's behaviour for a command without those options.
11. **`rctx.attr` repr: fixed, and it is not a struct.** `J/bazel/repository/starlark/
    StarlarkRepositoryContext.java:170` returns the `RepoDefinition` record, a `Structure` with
    no `repr`. So `repr`/`str`/`print` give
    `<unknown object com.google.devtools.build.lib.bazel.repository.RepoDefinition>` and `type()`
    is `"RepoDefinition"`. `R/blaze_repo_rules/src/repository/repo_definition.rs:97`. Tests:
    `repository_ctx_attr_is_the_repo_definition_structure`,
    `repository_ctx_attr_unknown_attribute_message`, `repository_ctx_attr_has_no_to_json`,
    `test_attr` (un-ignored).
12. **`selectedYankedVersions` order: fixed.** The order does reach the lockfile (the map is
    built in iteration order and Gson copies it unsorted), so the review's assumption holds.
    `R/blaze_bzlmod_model/src/resolution.rs:318` replays `java.util.HashMap` order
    (`java_hash.rs`, new). `ModuleKey` is a record: on the embedded OpenJDK 25 its hash is
    `31 * name.hashCode() + version.hashCode()`. A bucket that would become a tree bin is
    `unported!`. Tests: `selected_yanked_versions_are_in_java_hash_map_order` and 3 unit tests,
    with values from a real JDK.
13. **`Table::column()` order: fixed.** `R/blaze_bzlmod_model/src/dep_graph.rs:44`: dense or
    sparse order as Guava chooses. Tests: `tests/immutable_table_test.rs` (new, 7), expected
    orders printed by Guava 33.5.0.
14. **Tag kwargs read at the end of the module file: fixed.** Values of tags, `use_repo_rule`
    proxies, `archive_override` and `git_override` are kept on the file's heap and converted
    when that file finishes (`R/blaze_bzlmod/src/module_file/module_file_globals.rs:445`).
    `single_version_override` and `multiple_version_override` copy during the call in Java, so
    they were already right. Tests: 5 in `tests/module_file_globals_test.rs`, including
    `tag_attribute_values_are_read_after_the_module_file_has_been_executed`.
15. **`package_lookup`: fixed.** New `PackageLookup` trait
    (`R/blaze_bzlmod/src/skyframe/module_file_function.rs:615`) in `BzlmodConfig`;
    `setup.rs:612` installs `PackageLookupFunctionLookup`, backed by the real PACKAGE_LOOKUP, for
    every evaluator built through `setup`. The default stand-in fails loudly on a `.bazelignore`
    or `ignore_directories()`. Tests: 6 in `tests/module_file_function_test.rs` and
    `include_uses_package_lookup_with_bazelignore` in
    `R/blaze_repo_rules/tests/repository_e2e_test.rs`.

## TLS trust store (S3): decided, no code change to the trust roots

The embedded JDK 25.0.2 ships its own `lib/security/cacerts` (105 CA entries, a regular file,
not linked to the system store). Real Bazel at the pin therefore does not trust locally installed
CAs. `webpki-roots` stays the only root set, being the closest; adding native roots would trust
more than Bazel does. Bazel's mechanism for private CAs, `--host_jvm_args=
-Djavax.net.ssl.trustStore...`, now fails loudly in the client with exit 2
(`R/blaze_client/src/startup_options.rs:1174`; tests `client_rejects_host_jvm_args_trust_store`,
`host_jvm_args_trust_store_options_are_not_ported`). Recorded in `docs/decisions.md`. Exact
parity would be reading the install base's `cacerts`; deferred.

## Other S3 and S4 items touched

- Fixed: the three `testSingleVersionOverridePatches*` tests are ported and pass.
- Fixed: the three ignored rc_file tests are ported in `R/blaze_client/tests/rc_file_test.rs`
  (they need `StartupOptions`, which `blaze_options` cannot depend on).
- Fixed: `rc/option_processor.rs:382` uses `access(R_OK)`; test
  `can_read_file_uses_access_and_does_not_open_a_fifo`.
- Fixed: `lockfile.rs:844`, an out-of-range `lockFileVersion` is BAD_LOCKFILE; test
  `lock_file_version_beyond_int_range_is_a_bad_lockfile`.
- Fixed: `exceptions.rs` prints an `EvalException` cause as message with stack, no class name.
- Fixed: `OS_ARCH` / `OS_NAME` come from one place (`starlark/os.rs:38,55`).
- Disputed: "`rctx.os.environ` is mutable" is Java's behaviour. The getter returns an
  `ImmutableMap` (`J/bazel/repository/starlark/StarlarkOS.java:56`) and `MethodDescriptor.call`
  converts it with `Starlark.fromJava(result, mu)`, a fresh mutable dict per access. Kept, with
  a test (`environ_is_a_new_mutable_dict_on_each_access`).

## Not established items

- Commons Compress does not reject a bad tar header checksum (`checkSumOK` is stored and never
  read by `TarArchiveInputStream`); the `tar` crate does. Blaze therefore fails on such an
  archive where Bazel extracts it. Not fixed: the check is inside the crate's iterator. The
  failure is an error, not silent.
- The lockfile writer does not sort `selectedYankedVersions` (S2-12).
- The java-diff-utils fuzz port was not compared.

## Deferred

- Credential helper protocol (`unported!` when the flag is set).
- Exit codes of setup failures: `prepare_repository_functions_with` returns Java's message as a
  `String`, so `BAD_REPO_CONTENTS_CACHE`, `BAD_DOWNLOADER_CONFIG` and
  `CREDENTIALS_INIT_FAILURE` are not distinguishable by the caller.
- Remaining S3 items: `--vendor_dir`, `--incompatible_require_repo_extension_metadata`,
  floats in tag kwargs, cancellation of unawaited downloads, SIGINT, SOCKS proxies.
- Remaining S4 items: `build.rs` digest check, `blaze-fetch-dev`'s hard-coded install base,
  `MissingChecksumException` recognised by prefix.
- Noticed, not changed: `ureq`'s chunked decoder treats a connection closed mid-chunk as end of
  stream where Java throws "Premature EOF"; `decompressor/ar.rs` decodes name bytes >= 0x80 as
  Latin-1 where Commons Compress uses US-ASCII.

## Changes the integration lead must make

1. `R/blaze_sky_top/src/skyframe_executor.rs:186`: call
   `prepare_repository_functions_with(&setup, &CommandInputs { output_user_root,
   common_command_options, auth_and_tls_options, local_execution_options }, fs)` instead of
   `prepare_repository_functions_in`. This needs the output user root
   (`ServerDirectories::get_output_user_root`) in `SkyframeExecutorInputs`. Without it: S1-3
   holds only for default output bases, the `--http_*` flags and `--credential_helper` are not
   seen, and no process-wrapper kill delay is passed.
2. Correct the comments at `R/blaze_runtime/src/runtime.rs:479-481` and
   `R/blaze_runtime/src/dispatcher.rs:112-113` to say that the client's file locks, held for the
   whole in-process command, are what excludes a second Blaze process.
3. `ImplementationOutcome.events` (`blaze_repo_rules::starlark::thread`) is always empty now;
   nothing outside the crate reads it.
4. `R/blaze/src/main.rs:39` (`std::env::args()`) panics on non-UTF-8 argv; belongs with the
   byte-strings migration.
