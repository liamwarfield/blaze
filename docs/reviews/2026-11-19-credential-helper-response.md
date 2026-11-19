# Response: credential helper review (STOP on 3353c77)

Implementer's answers to `docs/reviews/2026-11-19-credential-helper.md`, finding by finding. The
fixes are in the uncommitted working tree on top of 3353c77. Line numbers refer to that tree.

## Blocking

### B1. "Error initializing credential helper" exits 36, Bazel exits 2: fixed

- Oracle record (Bazel 9.3.2, `~/.cache/blaze/bench/ws/brotli`, `--nosystem_rc --nohome_rc
  --output_base=…/ob-credcheck-oracle --host_jvm_args=-Xmx2g query //:brotlicommon
  --repository_cache=… --credential_helper=aw-remote-ext.buildremote.stairwell.io=no-such-helper-xyz`,
  under `systemd-run -p MemoryMax=5G`, server shut down after): exit code 2, empty stdout, stderr
  (after the rc-file warning and "Starting local Bazel server"):

  ```
  ERROR: Could not find file with name 'no-such-helper-xyz' on PATH '<client PATH>'
  ERROR: Error initializing credential helper
  ```

- Fix: `crates/blaze_runtime/src/commands/query.rs` `external_repos_detailed_exit_code` maps the
  set-up message `Error initializing credential helper` (a fixed string in
  `BazelRepositoryModule.java:473-481`) to `ExternalRepository.CREDENTIALS_INIT_FAILURE` through
  `DetailedExitCode::of`, which takes the exit code from the proto metadata
  (`failure_details.proto:261`, exit code 2). Every other set-up message keeps the existing
  36/`EXTERNAL_REPOSITORY_UNKNOWN` mapping. The set-up still returns its `AbruptExitException` as
  a message only; recognising the one fixed message is marked `// PORT:`.
- Evidence: the same Blaze command (debug build of this tree, same `PATH`) exits 2 and its
  stderr is byte-identical to the oracle's (`diff` empty).
- Tests: `blaze_runtime` `commands::query::tests::credential_helper_init_failure_exits_2` (exit
  code 2, category and code) and `other_repository_setup_failures_exit_36`;
  `blaze_repo_rules` `setup_test::credential_helper_not_on_path_is_an_init_failure` (the event
  `Could not find file with name '…' on PATH '…'` from `CommandLinePathFactory.java:146` and the
  message from `BazelRepositoryModule.java:479`).
- Related, not fixed (pre-existing, outside the credential helper): the other
  `BazelRepositoryModule.beforeCommand` exits (`BAD_REPO_CONTENTS_CACHE`,
  `BAD_DOWNLOADER_CONFIG` exit 2, ...) still go through the 36/`EXTERNAL_REPOSITORY_UNKNOWN`
  fallback. A proper fix makes the set-up return a failure code with its message.

### B2. An unchecked exception from the credentials is a panic on the gRPC path: fixed

- Third-party behaviour, checked in the oracle's install base (`A-server.jar`, disassembled with
  `javap -c` from the JDK 25 toolchain), not recalled:
  - google-auth `com.google.auth.Credentials.blockingGetToCallback`: the call to
    `getRequestMetadata(URI)` is covered by an exception-table entry for `java/lang/Throwable`
    whose handler calls `RequestMetadataCallback.onFailure`.
  - grpc-auth `GoogleAuthLibraryCallCredentials$1.onFailure`: for a `Retryable` exception that
    is retryable, `Status.UNAVAILABLE.withDescription("Credentials failed to obtain metadata")
    .withCause(e)`; otherwise `Status.UNAUTHENTICATED.withDescription("Failed computing
    credential metadata").withCause(e)`. Neither `CredentialHelperException` nor the two
    runtime exceptions are `Retryable`.
- Fix:
  - `crates/blaze_repo_fetch/src/authandtls/credentialhelper/unchecked_exception.rs` (new):
    `UncheckedException::throw(class, message)` panics with an `UncheckedException` payload;
    `catch_unchecked(f)` turns that payload into `Err` and lets any other panic continue (so
    `unported!` stays loud). The two reachable Java runtime exceptions use it:
    `ArithmeticException` (`Duration.toNanos()` in `credential_cache_expiry.rs`, `Duration.toMillis()`
    in `credential_helper.rs`, `WallTicker`) and `IllegalArgumentException`
    (`GetCredentialsResponseBuilder::build`, duplicate header name).
  - `crates/blaze_remote/src/authandtls/google_auth_utils.rs`: the blocking
    `get_request_metadata` runs inside `catch_unchecked` on the blocking thread, so an unchecked
    exception becomes the call's error, as `blockingGetToCallback` does.
  - `crates/blaze_remote/src/grpc/channel.rs`: the status is
    `UNAUTHENTICATED` with description exactly `Failed computing credential metadata` and the
    error as its source (`withCause`), no longer with the error text appended.
  - `crates/blaze_remote/src/util/utils.rs` `grpc_aware_error_message`: appends `": " +
    getMessage()` of the first `CredentialHelperException` in the status' cause chain
    (`Utils.java:372-378`). A `CredentialHelperException` is now a `blaze_repo_fetch`
    `IoException` of the new kind `IoExceptionKind::CredentialHelperException`
    (`errors.rs`); `credential_helper_exception.rs` converts into it.
- Header values and stderr: the panic hook prints an `UncheckedException` only when no
  `catch_unchecked` is active on the thread. On the gRPC path nothing is printed, and the user
  sees `UNAUTHENTICATED: Failed computing credential metadata` (the cause's message is not shown,
  as in Java, since it is not a `CredentialHelperException`). On the downloader path the
  exception is still uncaught, as in Bazel (`HttpConnectorMultiplexer.java:141-148` catches
  `IOException` only): Blaze panics and the hook prints `thread '…' panicked at …:` then
  `java.lang.IllegalArgumentException: Multiple entries with same key: …`, the exception's
  `toString()`, which Bazel's crash report also prints. `UncheckedException`'s `Debug` shows the
  class only.
- Tests: `blaze_remote` `google_auth_utils_test::call_credentials_turn_unchecked_exceptions_into_errors`
  (no panic, the error carries the exception) and
  `grpc_aware_error_message_shows_only_credential_helper_causes` (helper failure: description
  plus the helper message; unchecked or other cause: description only);
  `blaze_repo_fetch` `credential_helper_test::far_future_expires_is_an_arithmetic_exception`
  (`"expires": "9999-12-31T23:59:59Z"` through `CredentialHelperCredentials`),
  `caught_unchecked_exception_prints_nothing` (run with `--nocapture`: the placeholder values do
  not appear in the output; checked, 0 occurrences), `uncaught_unchecked_exception_is_a_panic`;
  `get_credentials_response_test::duplicate_header_names_are_an_illegal_argument`.
- Not done: no end-to-end gRPC run with a misbehaving helper.

## Non-blocking

### N1. The Gson reader rejects documents Gson accepts: fixed (ported, not `unported!`)

- Detecting lenient syntax by the failing character turned out wrong (Gson reads the
  `not json` of `credential_helper_test::error_parsing_output` as an unquoted string, which the
  adapter then rejects with its own message), so the lenient grammar is ported in `gson.rs`:
  unquoted and single-quoted names and strings, `#`, `//` and `/* */` comments (a document of
  comments only is empty), `=`/`=>` name separators, `;` separators, the `)]}'\n` prefix,
  missing array elements as `null`, case-insensitive keywords, a leading byte order mark, a
  leading `0` followed by a digit read as a literal, and keyword/number ends decided by
  `isLiteral`, all from Gson's `JsonReader.doPeek`/`nextNonWhitespace`/`nextUnquotedValue`
  as I know them (Gson's source is not in the cache; not disassembled).
- Remaining difference: Gson's number reader also rejects some numbers by `long` range rules
  and reads them as literals; the port reads every JSON number as a number. A number is never
  accepted where the adapters expect a string, so this only changes the message of an adapter
  error.
- Tests: `get_credentials_response_test::lenient_syntax_is_read_like_gson`,
  `a_leading_byte_order_mark_is_skipped`, `standard_syntax_errors_stay_syntax_errors`.

### N2. Helper text through the status description; false "not ported" comments: fixed

- The description and `grpcAwareErrorMessage` are fixed as described under B2.
- Comments rewritten: `util/utils.rs` (`create_exec_exception_for_credential_helper_exception`
  and the cause-chain loop), `remote_spawn_runner.rs` module comment, `remote_spawn_cache.rs`,
  `remote_execution_service.rs`. They now say why the `catch (CredentialHelperException e)`
  clauses have no counterpart: over gRPC the exception only arrives as a status' cause, where
  Java's catch does not fire either; the direct throwers (HTTP cache, remote downloader) are not
  ported.
- Open: the `JavaUri::create` failure in `GoogleAuthLibraryCallCredentials` still becomes the
  generic credentials status; grpc-auth's own service-URI failure status was not checked.

### N3. No end-to-end evidence: partly addressed

- The missing-helper case is recorded above (oracle and Blaze, identical stderr, exit 2). The
  success case (`=aspect`) exits 0 with `//:brotlicommon`. No helper-failure-during-gRPC
  scenario was run, and `docs/remote.md`'s evidence table was not edited (not my file).

### N4. Honest gaps: unchanged, one update

- `CredentialHelperException` now has its own `IoExceptionKind` (so the type exists before the
  HTTP cache is ported). The rest of N4 stands as written.

### N5. Stale comments: fixed

- `credential_cache_expiry.rs` and `credential_module.rs` now say that
  `blaze_runtime`'s `CredentialModule` calls `before_command` for every command; the missing-hook
  `unported!` text no longer claims the runtime does not.

### N6 to N11: open

- N6 (`// Java:` line numbers in `credentialhelper/*`), N7 (`java_time.rs` error texts and
  year-limit range check), N8 (`gson.rs` `\u` with `+`, form feed; the form feed is now a
  literal terminator through `is_literal`, the `+` remains), N9
  (`command_line_path_factory.rs` first character class), N10 (cache slots kept per URI,
  `invalidate_all` during a computation), N11 (tests without a Java counterpart) are not
  addressed.

### N12. `StaticCredentials` derives `Debug`: fixed

- `crates/blaze_repo_fetch/src/authandtls/mod.rs`: hand-written `Debug` printing the URIs and
  header names only.

## Test runs (this tree)

- `blaze_repo_fetch` (all targets): every target passes; credential targets:
  `credential_helper_test` 18 passed, 1 ignored (real `aspect` helper);
  `get_credentials_response_test` 16 passed; `credential_module_test` 10;
  `credential_helper_provider_test` 8; `command_line_path_factory_test` 14;
  `get_credentials_request_test` 7.
- `blaze_repo_rules` `setup_test`: 20 passed.
- `blaze_remote` `google_auth_utils_test`: 15 passed.
- `blaze_runtime`: `check` clean; `commands::query::tests`: 2 passed.
- Clippy (`--no-deps`, `--tests`) on `blaze_repo_fetch`, `blaze_remote`, `blaze_runtime`: no
  warnings. `rustfmt --edition 2024 --check` clean on every touched file.
