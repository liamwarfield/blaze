# Response: bes (Build Event Service client), first round, 2026-11-26

Response to `docs/reviews/2026-11-26-bes.md` (STOP at a673e29, blocking BES-1), by the
implementer (task `ea-4`). The fixes below were committed by root in checkpoint 41 (9b5671c8);
the corrections in "Found while porting the tests" are in the working tree on top of 38cd662a.
Test counts are from that tree.

## BES-1 (S1): a timed-out background upload that finishes before the next command — fixed

`crates/blaze_runtime/src/commands/bep.rs` `TimedCloseFuture` now latches its timeout as
`Futures.withTimeout` does (Java `BuildEventServiceModule.java:585-628`):

- On construction with a non-zero `--bes_timeout`, a timer thread (Java: the single-thread
  scheduled executor `bes-close-<name>-%d`) waits for the close future until the deadline and,
  if it is still pending then, sets `timed_out` for good.
- `outcome` answers `TimedOut` whenever `timed_out` is set, also when the close future
  completed after the deadline. A wait that reaches the deadline with the future pending sets
  the flag itself (the timer fires at the deadline).

So the next command's `wait_for_previous_invocation` takes the "failed due to a network
timeout" branch for a late completion, as Java's `ExecutionException(TimeoutException)` does.
Test: `bes_module_tests::before_second_command_no_wait_bes_timeout_late_completion_warning`
(the review's case: `nowait_for_upload_complete`, the close completes after `--bes_timeout`
and before the next command; the warning is expected). The same holds for the half-close
future (`fully_async`), which goes through the same type.

## BES-2 (S2): the cached client is cleared after every command — fixed

`crates/blaze_runtime/src/bazel_modules.rs` `BazelBuildEventServiceModule`: `before_command`
records `--keep_backend_connections` for every command (Java reads `besStreamOptions` in
`beforeCommand` before the allowed-commands check), and `after_command` clears the client
when the command has no streamer and the option is false. A command with a streamer still
clears it at the end of `BepCommandSupport::after_command`, after the transports closed and the
results URL was reported, and not when closing failed (Java's `afterCommand` throws from
`closeBepTransports` before it gets to `clearBesClient`).

## BES-3 (S2): waiting for uploads — local part fixed, remote part shown unreachable

- Local file uploads: `uploader.rs` `wait_for_uploads` now waits interruptibly
  (`PathConverterFuture::get_timeout`, new in `blaze_bep`, polled every 10 ms with the
  uploader thread's interrupt flag). A cancelled close (timeout, next command) interrupts the
  uploader thread, the wait ends with `InterruptedException`, and the dropped future cancels
  the upload, as Java's `get()` does.
- Remote uploads (`waitForRemoteUploads`): not ported, now with the evidence in the code. In
  Bazel only `BuildToolLogs.remoteUploads()` returns futures (`BuildToolLogs.java:69`), filled by
  `BuildToolLogCollection.addUriFuture` from three places: the Skyframe memory dump with
  `--experimental_stream_log_file_uploads` (`BuildTool.java:743`), `ExecutionGraphModule`'s
  action dump (`ExecutionGraphModule.java:984`) and `BuildEventArtifactInstrumentationOutput`
  (`:44`). Each of the three fails loudly in Blaze (`unported_options.rs` and
  `profiler_init.rs`), so no event can reach the uploader with a pending remote upload. Porting
  the futures would add code no input can run; I left them out and ask for that to be accepted.

## BES-4 (S2): an unexpected interrupt — fixed

`uploader.rs` `run`: the interrupt that was not caused by a cancel now runs the `finally` part
first (releases the uploader, completes the close future normally, as Java's `finally` does
after `checkState` throws from its `catch` block), then fails with
`IllegalStateException: Unexpected interrupt on BES uploader thread`.

## S3: `BazelBuildEventServiceModuleTest` — ported where it can run

All 47 methods are in `crates/blaze_runtime/src/commands/bep.rs` (`mod bes_module_tests`), plus
the BES-1 regression test.

- **23 run and pass**: the three upload modes, `--bes_timeout`, the second command's waits and
  the error-on-complete cases, plus `testKeywords` and `testMakeGrpcMetadata`. Java runs whole
  builds against an in-process `DelayingPublishBuildEventService`. Blaze has no in-process build
  harness, so these tests drive the module's closing and waiting code (`close_pending_uploads`,
  `wait_for_previous_invocation`, factored out of the methods for this) with a fake transport.
  Its futures behave as the delaying service makes the real transport's behave:
  `setDelayBeforeClosingStream` (half-closed at once, closed late),
  `setDelayBeforeHalfClosingStream` (both late) and `setErrorMessage` (the close fails). Times
  are scaled down (`--bes_timeout` 200 ms, `WAIT_FOR_LAST_INVOCATION_TIMEOUT` 500 ms through a
  test override of `getMaxWaitForPreviousInvocation`, as the Java test overrides it; delays
  3 s). The uploader's own behaviour under those services is covered by `blaze_bes`' transport
  tests (20 of 21 run and pass; `test_file_upload_fails` is ignored, see below).
- **25 are `#[ignore]` with the reason**:
  - two are `@Ignore("b/246912214")` in Java;
  - three are OOM handling (JVM-only);
  - `testConnectivityFailureDisablesBesStreaming`: no `ConnectivityStatusProvider` (Bazel's
    status is always OK);
  - `..._postsEvent`: `BuildEventServiceUploadCompleteEvent` has no subscriber in Bazel and is
    not posted;
  - the rest need an in-process build (transport creation from options, results-URL lines,
    retries against a server, file transports with a late-closing stream, flagsets), with
    where the checked behaviour is covered named in each reason.

## S4

- The forced shutdown (`force_shutdown_build_event_streamer`) now stops at the first failure,
  as `Futures.allAsList(...).get()` does, then cancels the rest
  (`cancelAndResetPendingUploads`).
- After a cancel the half-close future is cancelled too: the close future's cancel listener
  cancels it (Java: `halfCloseFuture.setFuture(closeFuture)` on completion), and `finish_run`
  cancels it instead of completing it when the close future was cancelled.
- `Status.toString()` printing a source chain instead of a stack trace: unchanged (declared;
  a Java stack trace cannot be reproduced).

## Found while porting the tests

A new finding against my own code (S2, open): after `abortStream` the server does not see the
call fail. grpc-java's `onError` resets the HTTP/2 stream, so the server's call fails with
`CANCELLED`. `test_file_upload_fails` waits for that server-side error, and it hangs in Blaze.

- Blaze stops reading responses, but hyper pipes the request stream in a task of its own,
  which ends only when the stream ends.
- `ReferenceCountedChannel::bidi_streaming` (blaze_remote) cannot cancel a call whose request
  side is open.
- tonic's encoder cannot turn a stream item into a body error, which hyper would send as a reset.

So the server sees a normal half-close without the last events: at once in a673e29, and when the
stream context is dropped in checkpoint 41. My earlier claim that checkpoint 41 fixes it was
wrong: I had not run the test to the end.

The client's own outcome is as in Java: the status set by `abortStream`, the exit code, and the
message. The code now says this (`PORT: deviation` at `BesGrpcStreamContext::abort_stream`), and
the test is `#[ignore]` with this reason. A fix needs a cancellable bidi-streaming call in
blaze_remote's channel; that crate belongs to another agent.

## Probes against a live backend

On the tree before checkpoint 41 (binary `blaze-d3`), against the oracle and the crate's logging server, in one
reused workspace per side:

- `test_bes_upload_failure_does_not_block_run` by hand: `bazel run --bes_backend=localhost:1234
  --bes_upload_mode=wait_for_upload_complete`. The oracle reports the upload failure and runs
  the binary (exit 0). Blaze exited 38 without running it, because the build commands' own
  `afterCommand` path dropped the exec request. That path now keeps it
  (`BlazeCommandResult.execute(execRequest, moduleExitCode)`, `commands/build.rs`). Now equal
  apart from the action-cache counts of a rerun.
- The successful upload, a stream the server fails once, and an unreachable backend were equal
  in the previous round (requests, console, exit code).

Not yet probed: one run per upload mode against a backend that closes late, which would show
BES-1 by observation. The test above covers it at the module level.
