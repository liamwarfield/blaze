# Review: credential helper (commit 3353c77, parent e297ee3)

Verdict: **STOP**, bound to commit 3353c77.

Reviewer: read-only. Read from the commit (`git show 3353c77:<path>`, `git diff e297ee3 3353c77`),
never from the working tree. No Cargo, no build, no `aspect` run. No test was executed by this
review; test claims below are about what the test sources assert, not about results.

Line numbers are those of the files at 3353c77. Java paths are relative to
`~/.cache/blaze/src/bazel-9.3.2/src/main/java/com/google/devtools/build/lib/`.

Third-party behaviour (Gson 2.11.0, Caffeine 3.1.8, grpc-auth 1.66.0, google-auth 1.24.1,
`java.time`) is not in the source cache. Statements about it are from my knowledge of those
versions, not from reading them here; each is marked "(recalled)".

## Scope read

Rust: `crates/blaze_repo_fetch/src/authandtls/**`, `src/runtime/**`, the six test files and the
copied `test_credential_helper.py`; `crates/blaze_repo_rules/src/setup.rs` (credential part) and
`tests/setup_test.rs` (credential tests); `crates/blaze_remote/src/authandtls/**`,
`tests/google_auth_utils_test.rs`, `src/remote_module.rs` (`create_credentials`, its caller,
`command_line_path_factory`); `crates/blaze_runtime/src/bazel_modules.rs`
(`CredentialModule::before_command`). Followed out of the listed files where the behaviour is
decided: `blaze_remote/src/grpc/channel.rs` (`start_call`, `service_uri`),
`blaze_remote/src/util/utils.rs` (`grpc_aware_error_message`, `refresh_if_unauthenticated*`),
`blaze_runtime/src/commands/query.rs` (exit code of the set-up error),
`blaze_repo_fetch/src/downloader/http_connector_multiplexer.rs`, `blaze_optdefs` converter,
`blaze_repo_fetch/src/uri.rs` (host API only).

Java: every file named in the task, plus `remote/util/Utils.java`, `RemoteSpawnCache.java`,
`RemoteSpawnRunner.java`, `RemoteExecutionService.java`,
`bazel/repository/downloader/HttpConnectorMultiplexer.java`, `failure_details.proto`.

## Blocking findings

### B1 (S1). "Error initializing credential helper" exits 36, Bazel exits 2

- Rust: `crates/blaze_repo_rules/src/setup.rs:520-523` returns `Err("Error initializing
  credential helper".to_string())`. The only consumer,
  `crates/blaze_runtime/src/commands/query.rs:197-209`
  (`SkyframeExecutorCreationError::ExternalRepos`), turns every such string into
  `ExitCode::LOCAL_ENVIRONMENTAL_ERROR` (36) with
  `ExternalRepository.Code.EXTERNAL_REPOSITORY_UNKNOWN`. `build`, `query`, `cquery`, `aquery`
  and `mod` all create the executor through this function.
- Java: `bazel/BazelRepositoryModule.java:473-481` exits with
  `detailedExitCode("Error initializing credential helper", Code.CREDENTIALS_INIT_FAILURE)`;
  `failure_details.proto:261` gives that code `exit_code: 2`.
- Failing scenario: `blaze build --credential_helper=no-such-helper //x` with no such file on
  `PATH` (or `--credential_helper=%nosuchroot%/h`, or a relative path with a `/`). Bazel exits 2
  with failure detail `CREDENTIALS_INIT_FAILURE`; Blaze exits 36 with
  `EXTERNAL_REPOSITORY_UNKNOWN`.
- Why it was missed: `tests/setup_test.rs:807-822` asserts the string and the error event only.
  The exit code has no test and no oracle record.
- Not checked: the exact `ERROR:` lines Blaze prints for this failure
  (`query.rs` reports `e.to_string()` of the creation error; I did not read its `Display`).
  The fix should be compared with the oracle for text as well as exit code.

### B2 (S1 for the valid-input case; secret exposure for the invalid-input case). On the gRPC path an unchecked exception from the credentials is a panic; in Java it is a failed call

- Rust: `crates/blaze_remote/src/authandtls/google_auth_utils.rs:541-545` runs
  `creds.get_request_metadata` in `spawn_blocking` and, when that panics, calls
  `std::panic::resume_unwind`. Two "unchecked exception" panics are reachable from a helper
  answer:
  1. `crates/blaze_repo_fetch/src/authandtls/credentialhelper/credential_cache_expiry.rs:13-15`
     (`to_nanos`, "ArithmeticException: long overflow"), reached from `expire_after_create`
     inside `CredentialCache::get`.
  2. `crates/blaze_repo_fetch/src/authandtls/credentialhelper/get_credentials_response.rs:98-109`
     (`build`, "IllegalArgumentException: Multiple entries with same key: ..."). The message
     contains the values of both header entries.
- Java: both exceptions are thrown the same way with the same texts
  (`CredentialCacheExpiry.java:56` `Duration.toNanos()`; `ImmutableMap.Builder.buildOrThrow`
  through `GetCredentialsResponse.java:162`), so the two Rust panics are faithful in
  themselves. The difference is the caller. gRPC applies call credentials through
  `com.google.auth.Credentials.getRequestMetadata(URI, Executor, RequestMetadataCallback)`,
  whose `blockingGetToCallback` catches `Throwable` and calls `onFailure`;
  `GoogleAuthLibraryCallCredentials` then fails the call with
  `Status.UNAUTHENTICATED.withDescription("Failed computing credential metadata").withCause(e)`
  (recalled). The exception text is only in the cause, which Bazel prints with
  `--verbose_failures` at most (`remote/util/Utils.java:367-380` appends a cause's message
  only for `CredentialHelperException`).
- Failing scenario 1 (valid helper answer): the helper for a `grpcs://` remote cache answers
  with headers and `"expires": "9999-12-31T23:59:59Z"`. That is a well-formed answer; any
  `expires` more than about 292 years ahead overflows `toNanos()`. Bazel: every call fails
  with `UNAUTHENTICATED: Failed computing credential metadata`, handled as any other failed
  remote call (warning or build error, Bazel's exit code). Blaze: a panic on a runtime thread,
  re-raised into the caller.
- Failing scenario 2 (invalid helper answer, secret exposure): the helper answers with the
  same header name twice inside `headers`. Bazel on the gRPC path: the same
  `UNAUTHENTICATED` status, header values not printed by default. Blaze: the panic message
  with both header values is produced on the blocking thread, where the panic hook prints it
  to stderr, before `resume_unwind` runs.
- Not a difference on the downloader path: `HttpConnectorMultiplexer.java:141-148` catches
  `IOException` only, so Bazel crashes there too, with the same message.
- Not checked: what the process does after `resume_unwind` (which task or thread it reaches,
  final exit code). I did not trace it and nothing was run.

## Non-blocking findings

### N1 (S2). The hand-written Gson reader rejects documents Gson accepts

- `crates/blaze_repo_fetch/src/authandtls/credentialhelper/gson.rs:9-20` (module comment) and
  `:186-249` (`do_peek`). `new Gson().fromJson` reads leniently (recalled: Gson 2.11 sets
  `Strictness.LENIENT` when the Gson instance has no strictness). The port accepts RFC 8259
  only.
- Scenario: a shell helper that prints its answer with single-quoted names and strings
  (`echo "{'headers': {...}}"`), or unquoted names. Bazel accepts and authenticates. Blaze
  reports `Failed to get credentials ... error parsing output. stderr: ...`; the downloader
  then continues without credentials (warning), the gRPC path fails `UNAUTHENTICATED`.
- Also accepted by Gson and rejected here: a leading UTF-8 byte order mark, comments, `;` and
  `=` separators, the `)]}'` prefix, non-standard numbers in unknown fields.
- Disclosed in the module comment, but it fails with a wrong error text, not `unported!`.
  Either port the lenient syntax or fail with `unported!` when the first parse fails on a
  lenient-only construct.

### N2 (S2). The helper's error text reaches the user through the status description, not through `grpcAwareErrorMessage`

- `crates/blaze_remote/src/grpc/channel.rs:485-487` builds
  `Status::unauthenticated("Failed computing credential metadata: {e}")`. Java's description is
  exactly `Failed computing credential metadata` (recalled) and
  `remote/util/Utils.java:372-378` appends `": " + getMessage()` only when a
  `CredentialHelperException` is in the cause chain.
- For a helper failure the final text is the same. It differs for any other error from the
  credentials (the `JavaUri::create` failure at `google_auth_utils.rs:532-533`, a fallback
  error) and anywhere the description is used without `grpc_aware_error_message`.
- The comments that justify the missing port are now false: `util/utils.rs:467-471` and
  `:796-800`, `remote_spawn_cache.rs:166-169`, `remote_spawn_runner.rs:6-7`, `:449`, `:636`,
  `remote_execution_service.rs:1450-1452` all say `CredentialHelperException` "is not ported".
- This line of `channel.rs` is not changed by 3353c77; the commit makes it reachable.

### N3 (S2). No end-to-end evidence

- `docs/remote.md` has no credential helper scenario in its evidence table, and no oracle
  record exists for one (success, helper failure, bad path). Everything above the unit tests
  rests on reading. B1 is the kind of defect an oracle run of the failure cases finds.

### N4 (S3). Honest gaps, checked for behaviour change

- `getAuthenticationType`, `hasRequestMetadata`, `hasRequestMetadataOnly` are absent
  (`credential_helper_credentials.rs:16-19`). Their only Bazel caller is
  `remote/http/AbstractHttpHandler.java:70` (HTTP cache, which fails with `unported!`). On the
  ported paths this cannot change behaviour.
- Exceptions as the `IOException` kind (`credential_helper_exception.rs:59-63`,
  `command_line_path_factory.rs:47-58`): Java tests the class in `RemoteSpawnCache.java:184`,
  `RemoteSpawnRunner.java:244` and `:352`, `RemoteExecutionService.java:533`, and
  `Utils.java:374`. The four `catch` clauses fire only when the exception is thrown directly,
  which happens on the HTTP cache and the remote downloader (both not ported); over gRPC it
  arrives wrapped in a status. `Utils.java:374` is covered by N2. So no behaviour change today;
  the type must exist before the HTTP cache is ported. The `cause` is dropped at
  `credential_helper_exception.rs:61`; Java shows it only in a `--verbose_failures` stack
  trace.
- `--google_credentials` / `--google_default_credentials`: `unported!`
  (`blaze_remote/src/authandtls/google_auth_utils.rs:643-656`). HTTP cache plaintext warning:
  `unported!` (`remote_module.rs:979-986`).
- `credential_cache_expiry.rs:47-57`: no default duration before the first
  `before_command` is `unported!` where Java uses `Duration.ZERO`. Marked `// PORT:`.

### N5 (S4). Stale comments

- `credential_cache_expiry.rs:24-27` and `credential_module.rs:68-70` say the runtime does not
  call `before_command` yet. `crates/blaze_runtime/src/bazel_modules.rs:181-194` does.

### N6 (S4). `// Java:` line numbers are off by one to five lines in `credentialhelper/*`

- Examples: `credential_helper.rs:18` says `:42`, the class is at 43; `:56` says `:67`,
  `getCredentials` is at 69; `:149` says `:155`, `spawnSubprocess` is at 160;
  `credential_helper_provider.rs` says 37/67/85/91/110/121/126, Java has 38/66/83/89/108/119/124.
  `GoogleAuthUtils` and `RemoteModule` markers are exact.

### N7 (S4). `java_time.rs`, error cases only

- `:132-136`: the JDK shortens the quoted text to 64 characters plus `...` in
  `DateTimeParseException`; the port quotes it whole.
- `:234-237`: an offset hour of 24 to 59 is a parse error here and a resolve error
  ("Zone offset not in valid range") in the JDK.
- `:270-281`: no range check after rolling months and days at the year limits.
- All three change the text of an error or turn an error into a far-away instant for inputs
  with a nine-digit year. I checked the accepted grammar (sign rules of `yyyy`, fixed two-digit
  fields, `Z` or `+HH:MM`, case sensitivity, lenient month, day and time roll-over, offset
  taking precedence over the formatter's zone) against the JDK from memory and found no
  difference on input the JDK accepts.

### N8 (S4). `gson.rs` details

- `:364`: `u16::from_str_radix` accepts a leading `+` in a `\u` escape; Gson does not.
- `:251-272`: form feed is missing from the literal terminators.
- `:338`: an unpaired surrogate becomes U+FFFD (disclosed).

### N9 (S4). `command_line_path_factory.rs:64-67`

- The first character class excludes line terminators; Java's `[^%]` accepts them. Affects
  only a path whose first character is a line terminator.

### N10 (S4). Cache

- `credential_cache.rs:94-96`, `:130-134`: a slot is kept for every URI ever asked, including
  URIs without a helper (one per downloaded URL). Not observable, grows with the build.
- `:181-183`: `invalidate_all` during a computation orphans the slot; a second caller can then
  run the helper concurrently. Caffeine blocks the invalidation instead (recalled). Only around
  `refresh()`.
- No test drives `get` from two threads; the one-helper-per-URI property rests on the per-key
  mutex (`:115-116`), which I read as correct.

### N11 (S4). Tests added by the port

- `get_credentials_response_test.rs` (`parse_expires_resolves_leniently`,
  `serialize_writes_gson_bytes`, `parse_errors_carry_the_adapter_messages`),
  `get_credentials_request_test.rs` (`serialize_is_html_safe_like_gson`),
  `credential_module_test.rs` (`get_computes_once_until_expiry`,
  `get_stores_neither_null_nor_errors`) have no Java counterpart; their expectations come from
  reading the JDK, Gson and Caffeine, not from a Java test or the oracle. The ones I derived
  independently agree. They are labelled as additions in the files.

### N12 (S4, not from this commit). `StaticCredentials` derives `Debug`

- `crates/blaze_repo_fetch/src/authandtls/mod.rs:29`: the derive prints the header map. I
  found no place that formats it.

## What I checked and found equal to Java

- Helper selection (`credential_helper_provider.rs`): exact host, then `*.` suffix walking up
  the parents (so `*.example.com` also matches `example.com`), then the default; a URI without
  a host takes the default. Host comparison is case-sensitive on `URI.getHost()` as in Java;
  the scope is normalised at option parsing (`blaze_optdefs`, not changed by this commit, not
  reviewed here beyond reading that it calls an `IDN.toASCII` port). For gRPC the URI is
  `https://<authority without :443>/<service>` (`channel.rs:908-911`).
- Subprocess (`credential_helper.rs:150-172`): argv is the helper path and `get`, no shell;
  environment is the client environment; working directory is the workspace when there is one;
  the request is written and stdin closed, write errors ignored; timeout from
  `--credential_helper_timeout`. Timeout kill is `blaze_shell`'s, which I did not review.
- All seven error texts of `CredentialHelper.getCredentials`, character for character. Helper
  stdout is never put in a message; stderr is, as in Java.
- Request JSON: html-safe escaping as `new Gson()`.
- Response adapter (`get_credentials_response.rs:151-234`): the six adapter messages, unknown
  keys skipped, repeated `headers` objects merged, last `expires` wins.
- Secrets: `GetCredentialsResponse` has a hand-written `Debug` that prints header names only
  (`:22-30`); the builder and the cache node have no `Debug`; no message built in the reviewed
  files contains a header value except the faithful `IllegalArgumentException` text of B2; the
  gRPC log interceptor reads only the `RequestMetadata` header; tests use placeholder values,
  and the ignored real-helper test prints header names only.
- Cache: expiry is `expires - now` or the default duration, expired when
  `now - variableTime >= 0`, zero duration stores an entry that is expired on the next read,
  no extension on read, `clean` or a changed default clears the cache, a null or failed
  computation stores nothing. The process-wide instance is keyed by URI only and shared by the
  downloader and the remote module, as Java's one module per server; Blaze runs one command per
  process, so it holds less than Java's does.
- Wiring: `new_credentials` builds Google credentials (not ported, loud), else `.netrc` with
  the warning on a parse failure, and wraps the result in the helper credentials. No helper for
  the URI falls back; a helper that answers with no headers does not fall back (as Java).
  Downloader: a helper failure is the warning `Error retrieving auth headers, continuing
  without: ...` and the download continues. gRPC: `spawn_blocking`, `UNAUTHENTICATED`, then
  `refresh()` (at most once per second) drops the whole cache and the call is repeated once.
  `RemoteModule`: `CREDENTIALS_INIT_FAILURE` through `handle_init_failure`.
- Tests: every Java test of `CredentialHelperTest` (10), `CredentialHelperProviderTest` (8),
  `CredentialModuleTest` (7), `GetCredentialsRequestTest` (4), `GetCredentialsResponseTest` (9),
  `CommandLinePathFactoryTest` (11) and `GoogleAuthUtilsTest` (6) has a Rust test of the same
  name. `test_credential_helper.py` is byte-identical to Java's. One test is ignored in these
  files (`real_aspect_helper_returns_authorization`, needs a login).
  `RemoteModuleTest.testNetrc_netrcWithoutRemoteCache` is `#[ignore]` with a reason.

## Re-check

B1 and B2 fixed (or shown wrong with Java evidence), then this role re-reads the fixing commit.
For B1 an oracle run of `--credential_helper=<missing>` should be recorded (exit code and
stderr). For B2 the re-check is that a panic from the credentials becomes the status Java
produces and that no header value is printed on the way.

## Round 2 (commit 3bb3adf, fixes for the STOP on 3353c77)

Verdict: **PASS**, bound to commit 3bb3adf, for the credential helper area.

Same rules as round 1: read from the commit (`git show 3bb3adf:<path>`,
`git diff 3353c77 3bb3adf`), nothing run. Test counts and the oracle record are the
implementer's (`docs/reviews/2026-11-19-credential-helper-response.md`); I read the test sources,
not their results. Gson's source is still not available, so the grammar check below is against
`JsonReader` 2.11 as I know it (recalled). The implementer's `javap` evidence for google-auth
and grpc-auth agrees with what I recalled in round 1.

### B1: fixed

- `crates/blaze_runtime/src/commands/query.rs:212-236` (`external_repos_detailed_exit_code`)
  gives the message `Error initializing credential helper` the failure detail
  `ExternalRepository.CREDENTIALS_INIT_FAILURE` through `DetailedExitCode::of`; the response
  records the oracle run (exit 2, two `ERROR:` lines) and a byte-identical Blaze stderr.
- Matching on the message text: acceptable for this finding. The match is exact equality with
  a constant that only `setup.rs:522` produces, and no other set-up message can equal it (the
  others carry a path or a parse error). It is marked `// PORT:`. It does not hide anything
  that was visible before.
- It does not fix the other exits of `BazelRepositoryModule.beforeCommand`, and the response
  says so. They are wrong today and are not part of this area: `BAD_DOWNLOADER_CONFIG` and
  the four `BAD_REPO_CONTENTS_CACHE` exits are exit code 2 in Bazel
  (`failure_details.proto:259-262`, `BazelRepositoryModule.java:353-412`, `:440-446`) and 36
  with `EXTERNAL_REPOSITORY_UNKNOWN` in Blaze. See R2-1.

### B2: fixed

- `credentialhelper/unchecked_exception.rs` (new), used at the four throw sites
  (`credential_cache_expiry.rs:14-18`, `credential_helper.rs:169-171`, `wall_ticker.rs:36-38`,
  `get_credentials_response.rs:104-111`); `blaze_remote/src/authandtls/google_auth_utils.rs:136-145`
  wraps the blocking call in `catch_unchecked`; `grpc/channel.rs:533-538` builds
  `UNAUTHENTICATED` with the description exactly `Failed computing credential metadata` and the
  error as source; `util/utils.rs:260-271`, `:486-494` append the message of the first
  `CredentialHelperException` in the cause chain, as `Utils.java:372-378`.
- The panic hook (`unchecked_exception.rs:60-81`), questions from the lead:
  - Can it swallow an unrelated panic or an `unported!`? No. A payload that is not an
    `UncheckedException` goes to the previous hook unconditionally (`:64-66`), whatever
    `CATCHING` is. `catch_unchecked` re-raises any other payload (`:112-115`).
  - Can it swallow an `UncheckedException` on another thread while one thread is catching? No.
    `CATCHING` is thread-local (`:53-56`); the hook runs on the panicking thread and reads that
    thread's count.
  - Is anything that it silences lost? No. Silence requires `CATCHING > 0` on the panicking
    thread, and then the enclosing `catch_unchecked` on that thread receives the payload and
    returns it as `Err`. The closure at `google_auth_utils.rs:137` is synchronous, so the count
    cannot leak across an `.await`; the decrement at `:109` runs on both outcomes.
  - Is the hook left replaced? Yes, permanently, from the first throw (`Once`). It delegates,
    so the only lasting effect is the print format of an uncaught `UncheckedException`. The
    workspace `Cargo.toml` has no `panic = "abort"` profile; with one, a caught throw would
    abort silently, so that setting must not be added without revisiting this file. No other
    production code sets a panic hook (`git grep set_hook`: tests and `starlark_conformance`
    only). If a later hook replaces this one, an uncaught `UncheckedException` prints as an
    opaque payload, which leaks nothing.
- Uncaught on the downloader path the hook prints the exception's `toString()`, header values
  included for the duplicate-name case. That is Bazel's behaviour there (crash with the same
  text), as noted in round 1. `UncheckedException`'s `Debug` prints the class only (`:35-42`).
- Cause chain: I traced `Status` (source `io::Error` holding the `IoException`) through
  `status_to_io_error` and `StatusRuntimeException`, and found no place in `blaze_remote/src`
  that rebuilds a status from code and message on this path. The unit test builds the status
  by hand; there is still no end-to-end run with a failing helper against a gRPC endpoint, so
  the printed line for that case rests on reading.

### N1, the lenient reader (`gson.rs:176-343`)

Compared with `JsonReader.doPeek`, `nextNonWhitespace`, `consumeNonExecutePrefix`,
`peekKeyword`, `peekNumber`, `isLiteral`, `nextQuotedValue`, `readEscapeCharacter` (recalled).
Structure, separators, comments, the non-execute prefix, missing array elements, the literal
terminator set and the byte order mark agree.

What becomes a header value: a double-quoted string, a single-quoted string, or an unquoted
literal (token `STRING` in Gson too; includes `01`, `NaN`, `1.5.2`, `+1`). Numbers, booleans
and `null` are rejected by the adapter in both. I found no input the port accepts as a header
value that Gson rejects, except:

- `:457`: a `\u` escape with a leading `+` (`\u+041`) is accepted; Gson fails with "Malformed
  Unicode escape". Open from round 1 (N8), S4.

Other differences, all S4 and none on a well-formed answer:

- A document that is only the `)]}'` prefix line: Gson returns null ("process exited without
  output"); the port fails the first peek ("error parsing output") (`:190-198`, `:604-612`).
- Gson gives up on a number longer than its buffer (1024 characters) and reads it as an
  unquoted string; the port reads a number.
- The port parses a quoted string when it is peeked, Gson when it is consumed; an adapter type
  error can therefore be replaced by a string syntax error. Both are `JsonSyntaxException`s
  with the same outer message.
- The response's "remaining difference" (numbers outside `long` read as literals by Gson) is
  not one as far as I know: `peekNumber` returns `PEEKED_NUMBER` for those, a `NUMBER` token.

### N2, N5, N12: fixed

- N2: description and cause as above; the six comments now state the real reason the
  `catch (CredentialHelperException)` clauses have no counterpart. `CredentialHelperException`
  is `IoExceptionKind::CredentialHelperException` (`errors.rs:42-43`,
  `credential_helper_exception.rs:60-64`).
- N5: `credential_cache_expiry.rs:27-30`, `credential_module.rs:68-69` say the runtime calls
  `before_command`.
- N12: `authandtls/mod.rs:34-45`, `Debug` prints URIs and header names.

### New findings (non-blocking)

**R2-1 (S2). A new test takes its expectation from Blaze and pins a value Java contradicts.**
`crates/blaze_runtime/src/commands/query.rs:376-380`
(`other_repository_setup_failures_exit_36`) asserts exit code 36 for "could not acquire lock".
Bazel's message is `could not acquire lock on repo contents cache` with
`BAD_REPO_CONTENTS_CACHE`, exit code 2 (`BazelRepositoryModule.java:409-412`). The underlying
wrong exit codes are an S1 in the repository set-up area (older code, disclosed in the
response): `blaze build --downloader_config=<file with a syntax error> //x` exits 2 in Bazel and
36 in Blaze. The lead should record that against the set-up area; the test should stop
asserting 36 for a Java message, and the set-up should return a failure code with its message
so that `query.rs` does not match text at all.

**R2-2 (S4).** The new tests `lenient_syntax_is_read_like_gson`,
`a_leading_byte_order_mark_is_skipped`, `standard_syntax_errors_stay_syntax_errors` have
expectations from reading Gson, not from a Java test or the oracle (same class as N11). The
cases I derived agree.

### Still open from round 1

N3 (no gRPC run with a failing helper; `docs/remote.md` has no helper row), N4 (honest gaps),
N6 (`// Java:` line numbers), N7, N8 (`\u+`), N9, N10, N11.

### Regressions

None found in the credential files. `remote_module.rs`' credential wiring is unchanged between
the two commits. The rest of 3bb3adf (lazy channel, action cache metadata, strategy registry,
`build.rs`) belongs to the remote wiring review and was not reviewed here.
