//! Port of `lib/remote/util/Utils.java`: utility methods for the remote package.

use std::fmt;
use std::future::Future;
use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use blaze_exec::actions::{
    ActionInputRef, EnvironmentalExecException, ExecException, Spawn, SpawnMetrics, SpawnResult,
    SpawnResultBuilder, Status as SpawnResultStatus, execution_requirements, spawns,
};
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_protos::google::protobuf::{Any, Timestamp};
use blaze_vfs::PathFragment;
use futures::StreamExt;
use futures::stream::FuturesOrdered;
use indexmap::IndexMap;
use prost::Message as _;

use crate::common::bulk_transfer_exception::BulkTransferException;
use crate::common::cache_not_found_exception::CacheNotFoundException;
use crate::common::errors;
use crate::common::output_digest_mismatch_exception::OutputDigestMismatchException;
use crate::common::remote_cache_client::ActionKey;
use crate::common::remote_execution_capabilities_exception::RemoteExecutionCapabilitiesException;
use crate::execution_status_exception::{self, ExecutionStatusException};
use crate::reapi::rpc::{
    BadRequest, DebugInfo, Help, LocalizedMessage, PreconditionFailure, QuotaFailure, RequestInfo,
    ResourceInfo, RetryInfo, Status,
};
use crate::reapi::{Action, ActionResult, Digest, Platform};
use crate::util::digest_util::DigestUtil;

/// Returns the result of a future, blocking the calling thread (`Utils.getFromFuture`; see
/// [`crate::runtime::get_from_future`] for how interruption maps).
// Java: com/google/devtools/build/lib/remote/util/Utils.java:98 Utils.getFromFuture
pub use crate::runtime::get_from_future;

/// `StringUtilities.bytesCountToDisplayString`, which the remote package imports statically.
///
/// PORT: not part of `Utils.java`; re-exported here so that the remote package has one place to
/// take it from (the port lives in `blaze_repo_fetch`).
// Java: com/google/devtools/build/lib/util/StringUtilities.java:118 StringUtilities.bytesCountToDisplayString
pub use blaze_repo_fetch::downloader::download_progress_event::bytes_count_to_display_string;

/// `lib/authandtls/CallCredentialsProvider.java` (re-exported for the callers of `Utils`).
pub use crate::authandtls::call_credentials_provider::CallCredentialsProvider;

/// Returns the (exec root relative) path of a spawn output that should be made available via
/// `SpawnResult.getInMemoryOutput(ActionInput)`.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:132 Utils.getInMemoryOutputPath
pub fn get_in_memory_output_path(spawn: &dyn Spawn) -> Option<PathFragment> {
    let output_path = spawn
        .get_execution_info()
        .get(execution_requirements::REMOTE_EXECUTION_INLINE_OUTPUTS);
    if let Some(output_path) = output_path {
        return Some(PathFragment::create(output_path));
    }
    None
}

/// Constructs a `SpawnResult`.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:142 Utils.createSpawnResult
pub fn create_spawn_result(
    digest_util: &DigestUtil,
    action_key: &ActionKey,
    exit_code: i32,
    cache_hit: bool,
    runner_name: &str,
    in_memory_output: Option<InMemoryOutput>,
    execution_start_timestamp: Timestamp,
    execution_completed_timestamp: Timestamp,
    spawn_metrics: SpawnMetrics,
    mnemonic: &str,
) -> SpawnResult {
    let mut builder = SpawnResultBuilder::new();
    builder
        .set_status(if exit_code == 0 {
            SpawnResultStatus::Success
        } else {
            SpawnResultStatus::NonZeroExit
        })
        .set_exit_code(exit_code)
        .set_runner_name(&if cache_hit {
            format!("{runner_name} cache hit")
        } else {
            runner_name.to_string()
        })
        .set_cache_hit(cache_hit)
        .set_start_time(timestamp_to_instant(execution_start_timestamp))
        .set_wall_time_in_ms(
            // `(int) Duration.between(start, completed).toMillis()`
            (timestamp_to_epoch_nanos(execution_completed_timestamp)
                - timestamp_to_epoch_nanos(execution_start_timestamp))
            .div_euclid(1_000_000) as i64 as i32,
        )
        .set_spawn_metrics(spawn_metrics)
        .set_remote(true)
        .set_digest(Some(digest_util.as_spawn_log_proto(action_key)));
    if exit_code != 0 {
        builder.set_failure_detail(FailureDetail {
            message: format!("{mnemonic} returned a non-zero exit code when running remotely"),
            category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
                code: failure_details::spawn::Code::NonZeroExit as i32,
                ..Default::default()
            })),
        });
    }
    if let Some(in_memory_output) = in_memory_output {
        builder.set_in_memory_output(
            in_memory_output.get_output().clone(),
            in_memory_output.get_contents().to_vec(),
        );
    }
    builder.build()
}

/// `Instant.ofEpochSecond(timestamp.getSeconds(), timestamp.getNanos())`, as nanoseconds since
/// the epoch.
fn timestamp_to_epoch_nanos(timestamp: Timestamp) -> i128 {
    timestamp.seconds as i128 * 1_000_000_000 + timestamp.nanos as i128
}

/// `timestampToInstant(timestamp)`.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:185 Utils.timestampToInstant
pub fn timestamp_to_instant(timestamp: Timestamp) -> SystemTime {
    // `Instant.ofEpochSecond(seconds, nanoAdjustment)` normalizes the nanoseconds into
    // [0, 999_999_999].
    let epoch_nanos = timestamp_to_epoch_nanos(timestamp);
    let seconds = epoch_nanos.div_euclid(1_000_000_000);
    let nanos = epoch_nanos.rem_euclid(1_000_000_000) as u32;
    if seconds >= 0 {
        UNIX_EPOCH + Duration::new(seconds as u64, nanos)
    } else {
        UNIX_EPOCH - Duration::from_secs(seconds.unsigned_abs() as u64)
            + Duration::from_nanos(nanos as u64)
    }
}

/// `Code.forNumber(code).getValueDescriptor().getName()`.
///
/// PORT: `google.rpc.Code` is not among the generated protos; its numbers and names are those
/// of `io.grpc.Status.Code` (`google/rpc/code.proto` defines the canonical gRPC codes).
fn rpc_code_name(code: i32) -> &'static str {
    if !(0..=16).contains(&code) {
        // `Code.forNumber` returns null for a number that is not a code.
        panic!("NullPointerException: Code.forNumber({code})");
    }
    errors::code_name(tonic::Code::from_i32(code))
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:189 Utils.statusName
fn status_name(code: i32) -> String {
    // 'convert_underscores' to 'Convert Underscores'
    let name = rpc_code_name(code);
    name.split('_')
        .map(|word| {
            format!(
                "{}{}",
                word[0..1].to_ascii_uppercase(),
                word[1..].to_ascii_lowercase()
            )
        })
        .collect::<Vec<String>>()
        .join(" ")
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:197 Utils.errorDetailsMessage
fn error_details_message(details: &[Any]) -> Result<String, prost::DecodeError> {
    let mut messages = String::new();
    for detail in details {
        messages += &format!("  {}\n", error_detail_message(detail)?);
    }
    Ok(messages)
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:206 Utils.durationMessage
fn duration_message(duration: &blaze_protos::google::protobuf::Duration) -> String {
    // this will give us seconds, might want to consider something nicer (graduating ms, s, m, h,
    // d, w?)
    blaze_protos::json::durations_to_string(duration.seconds, duration.nanos)
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:212 Utils.retryInfoMessage
fn retry_info_message(retry_info: &RetryInfo) -> String {
    format!(
        "Retry delay recommendation of {}",
        // `getRetryDelay()` is the default instance when the field is not set.
        duration_message(&retry_info.retry_delay.unwrap_or_default())
    )
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:216 Utils.debugInfoMessage
fn debug_info_message(debug_info: &DebugInfo) -> String {
    let mut message = String::new();
    if !debug_info.stack_entries.is_empty() {
        message += &format!(
            "Debug Stack Information:\n  {}",
            debug_info.stack_entries.join("\n  ")
        );
    }
    if !debug_info.detail.is_empty() {
        if !debug_info.stack_entries.is_empty() {
            message += "\n";
        }
        message += &format!("Debug Details: {}", debug_info.detail);
    }
    message
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:231 Utils.quotaFailureMessage
fn quota_failure_message(quota_failure: &QuotaFailure) -> String {
    let mut message = "Quota Failure".to_string();
    if !quota_failure.violations.is_empty() {
        message += ":";
    }
    for violation in &quota_failure.violations {
        message += &format!("\n    {}: {}", violation.subject, violation.description);
    }
    message
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:242 Utils.preconditionFailureMessage
fn precondition_failure_message(precondition_failure: &PreconditionFailure) -> String {
    let mut message = "Precondition Failure".to_string();
    if !precondition_failure.violations.is_empty() {
        message += ":";
    }
    for violation in &precondition_failure.violations {
        message += &format!(
            "\n    ({}) {}: {}",
            violation.r#type, violation.subject, violation.description
        );
    }
    message
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:259 Utils.badRequestMessage
fn bad_request_message(bad_request: &BadRequest) -> String {
    let mut message = "Bad Request".to_string();
    if !bad_request.field_violations.is_empty() {
        message += ":";
    }
    for field_violation in &bad_request.field_violations {
        message += &format!(
            "\n    {}: {}",
            field_violation.field, field_violation.description
        );
    }
    message
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:270 Utils.requestInfoMessage
fn request_info_message(request_info: &RequestInfo) -> String {
    format!(
        "Request Info: {} => {}",
        request_info.request_id, request_info.serving_data
    )
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:274 Utils.resourceInfoMessage
fn resource_info_message(resource_info: &ResourceInfo) -> String {
    let mut message = format!(
        "Resource Info: {}: name='{}', owner='{}'",
        resource_info.resource_type, resource_info.resource_name, resource_info.owner
    );
    if !resource_info.description.is_empty() {
        message += &format!(", description: {}", resource_info.description);
    }
    message
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:289 Utils.helpMessage
fn help_message(help: &Help) -> String {
    let mut message = "Help".to_string();
    if !help.links.is_empty() {
        message += ":";
    }
    for link in &help.links {
        message += &format!("\n    {}: {}", link.description, link.url);
    }
    message
}

/// `detail.is(clazz)`: whether the type name of the type URL (what follows its last `/`) is the
/// full name of the message type.
// Java: com/google/protobuf/Any.java Any.is
fn any_is(detail: &Any, full_name: &str) -> bool {
    // Java: com/google/protobuf/Any.java Any.getTypeNameFromTypeUrl
    let type_name = match detail.type_url.rfind('/') {
        Some(pos) => &detail.type_url[pos + 1..],
        None => "",
    };
    type_name == full_name
}

/// `detail.unpack(clazz)` for a `detail` that [`any_is`] that class (every call below checks it
/// first, so the "Type of the Any message does not match the given class." failure of
/// `Any.unpack` cannot happen here).
// Java: com/google/protobuf/Any.java Any.unpack
fn any_unpack<M: prost::Message + Default>(detail: &Any) -> Result<M, prost::DecodeError> {
    M::decode(&detail.value[..])
}

/// `TextFormatEscaper.escapeBytes`.
// Java: com/google/protobuf/TextFormatEscaper.java TextFormatEscaper.escapeBytes
fn text_format_escape_bytes(input: &[u8]) -> String {
    let mut builder = String::with_capacity(input.len());
    for &b in input {
        match b {
            0x07 => builder.push_str("\\a"),
            0x08 => builder.push_str("\\b"),
            0x0c => builder.push_str("\\f"),
            b'\n' => builder.push_str("\\n"),
            b'\r' => builder.push_str("\\r"),
            b'\t' => builder.push_str("\\t"),
            0x0b => builder.push_str("\\v"),
            b'\\' => builder.push_str("\\\\"),
            b'\'' => builder.push_str("\\'"),
            b'"' => builder.push_str("\\\""),
            // Only ASCII characters between 0x20 (space) and 0x7e (tilde) are printable. Other
            // byte values must be escaped.
            0x20..=0x7e => builder.push(b as char),
            _ => {
                builder.push('\\');
                builder.push((b'0' + ((b >> 6) & 3)) as char);
                builder.push((b'0' + ((b >> 3) & 7)) as char);
                builder.push((b'0' + (b & 7)) as char);
            }
        }
    }
    builder
}

/// `detail.toString()`: the text format of the `Any`, with the default (empty) type registry,
/// which prints the two fields without expanding the packed message.
// Java: com/google/protobuf/TextFormat.java TextFormat.Printer.printToString
fn any_to_string(detail: &Any) -> String {
    let mut text = String::new();
    if !detail.type_url.is_empty() {
        text += &format!(
            "type_url: \"{}\"\n",
            text_format_escape_bytes(detail.type_url.as_bytes())
        );
    }
    if !detail.value.is_empty() {
        text += &format!("value: \"{}\"\n", text_format_escape_bytes(&detail.value));
    }
    text
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:300 Utils.errorDetailMessage
fn error_detail_message(detail: &Any) -> Result<String, prost::DecodeError> {
    if any_is(detail, "google.rpc.RetryInfo") {
        return Ok(retry_info_message(&any_unpack::<RetryInfo>(detail)?));
    }
    if any_is(detail, "google.rpc.DebugInfo") {
        return Ok(debug_info_message(&any_unpack::<DebugInfo>(detail)?));
    }
    if any_is(detail, "google.rpc.QuotaFailure") {
        return Ok(quota_failure_message(&any_unpack::<QuotaFailure>(detail)?));
    }
    if any_is(detail, "google.rpc.PreconditionFailure") {
        return Ok(precondition_failure_message(&any_unpack::<
            PreconditionFailure,
        >(detail)?));
    }
    if any_is(detail, "google.rpc.BadRequest") {
        return Ok(bad_request_message(&any_unpack::<BadRequest>(detail)?));
    }
    if any_is(detail, "google.rpc.RequestInfo") {
        return Ok(request_info_message(&any_unpack::<RequestInfo>(detail)?));
    }
    if any_is(detail, "google.rpc.ResourceInfo") {
        return Ok(resource_info_message(&any_unpack::<ResourceInfo>(detail)?));
    }
    if any_is(detail, "google.rpc.Help") {
        return Ok(help_message(&any_unpack::<Help>(detail)?));
    }
    Ok(format!(
        "Unrecognized error detail: {}",
        any_to_string(detail)
    ))
}

/// `Locale.getDefault().toLanguageTag()`.
///
/// PORT: the Bazel client starts the server JVM with `-Duser.country= -Duser.language=
/// -Duser.variant=` (src/main/cpp/blaze.cc:436), so the default locale is `Locale.ROOT`, whose
/// language tag is "und".
const DEFAULT_LOCALE_LANGUAGE_TAG: &str = "und";

// Java: com/google/devtools/build/lib/remote/util/Utils.java:328 Utils.localizedStatusMessage
fn localized_status_message(status: &Status) -> Result<String, prost::DecodeError> {
    let language_tag = DEFAULT_LOCALE_LANGUAGE_TAG;
    for detail in &status.details {
        if any_is(detail, "google.rpc.LocalizedMessage") {
            let message = any_unpack::<LocalizedMessage>(detail)?;
            if message.locale == language_tag {
                return Ok(message.message);
            }
        }
    }
    Ok(status.message.clone())
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:342 Utils.executionStatusExceptionErrorMessage
fn execution_status_exception_error_message(
    e: &ExecutionStatusException,
) -> Result<String, prost::DecodeError> {
    let status = e.get_original_status();
    Ok(format!(
        "{}: {}\n{}",
        status_name(status.code),
        localized_status_message(status)?,
        error_details_message(&status.details)?
    ))
}

/// `io.grpc.Status.toString()`.
// Java: io/grpc/Status.java Status.toString
fn grpc_status_to_string(status: Option<&tonic::Status>) -> String {
    match status {
        Some(status) => format!(
            "Status{{code={}, description={}, cause=null}}",
            errors::code_name(status.code()),
            status.message()
        ),
        // `Status.UNKNOWN.withCause(e)`.
        None => "Status{code=UNKNOWN, description=null, cause=null}".to_string(),
    }
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:352 Utils.grpcAwareErrorMessage
fn grpc_aware_error_message_of_io_exception(e: &io::Error) -> String {
    let err_status = execution_status_exception::status_from_throwable(e);
    // `e.getCause() instanceof ExecutionStatusException`
    if let Some(cause) = errors::downcast_ref::<ExecutionStatusException>(e) {
        // Display error message returned by the remote service.
        return match execution_status_exception_error_message(cause) {
            Ok(message) => format!("Remote Execution Failure:\n{message}"),
            // PORT: Java appends `Throwables.getStackTraceAsString(protoEx)`; Rust has no stack
            // trace, so only its first line (the exception's `toString()`) is printed.
            Err(proto_ex) => format!(
                "Error occurred attempting to format an error message for {}: com.google.protobuf.InvalidProtocolBufferException: {}\n",
                grpc_status_to_string(err_status),
                proto_ex
            ),
        };
    }
    if let Some(err_status) = err_status
        && err_status.code() != tonic::Code::Unknown
    {
        // Display error message returned by the gRPC library, prefixed by the status code.
        let mut sb = String::new();
        sb.push_str(errors::code_name(err_status.code()));
        sb.push_str(": ");
        // PORT: `errStatus.getDescription()` is null for a status without a message (the
        // `grpc-message` trailer was absent), and `StringBuilder.append(null)` prints "null".
        // tonic has no null description: an empty message stands for it.
        if err_status.message().is_empty() {
            sb.push_str("null");
        } else {
            sb.push_str(err_status.message());
        }
        // If the error originated from a credential helper, print additional debugging
        // information.
        // PORT: `CredentialHelperException` (lib/authandtls/credentialhelper) is not ported, so
        // no status can have one in its cause chain; when it is, append `": " + getMessage()`
        // of the first one found in `err_status.source()` here.
        return sb;
    }
    e.to_string()
}

/// `grpcAwareErrorMessage(error, verboseFailures)`.
///
/// PORT: every `Throwable` of the remote package is an `io::Error` here (docs/remote.md,
/// convention 2); the one that is not an `IOException` in Java is `InterruptedException`
/// (`errors::is_interrupted`).
///
/// PORT: with `verbose_failures` Java appends `"\n" + getStackTraceAsString(error)`. Rust has no
/// stack traces, so nothing is appended and `verbose_failures` has no effect.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:385 Utils.grpcAwareErrorMessage
pub fn grpc_aware_error_message(error: &io::Error, verbose_failures: bool) -> String {
    let mut error_message = if !errors::is_interrupted(error) {
        grpc_aware_error_message_of_io_exception(error)
    } else {
        // `error.getMessage()` of an `InterruptedException`, which the remote package always
        // creates without a message (null).
        String::new()
    };

    if error_message.is_empty() {
        // `error.getClass().getSimpleName()`
        error_message = if errors::is_interrupted(error) {
            "InterruptedException".to_string()
        } else {
            "IOException".to_string()
        };
    }

    // On --verbose_failures print the whole stack trace (see the PORT note above).
    let _ = verbose_failures;

    error_message
}

/// `downloadAsActionResult(actionDigest, downloadFunction)`: downloads the blob of the action
/// key with `download_function` and parses it as an `ActionResult`; `None` (Java: null) when the
/// download fails with a `CacheNotFoundException`.
///
/// PORT: Java hands the function a `ByteString.Output` to write to. Here the function takes the
/// (empty) output buffer by value and gives it back when the download is done, so that the
/// returned future does not borrow a local of this function.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:406 Utils.downloadAsActionResult
pub async fn download_as_action_result<F, Fut>(
    action_digest: &ActionKey,
    download_function: F,
) -> io::Result<Option<ActionResult>>
where
    F: FnOnce(Digest, Vec<u8>) -> Fut,
    Fut: Future<Output = io::Result<Vec<u8>>>,
{
    let data: Vec<u8> = Vec::with_capacity(/* initialCapacity= */ 1024);
    let download = download_function(action_digest.digest().clone(), data);
    let result: io::Result<ActionResult> = match download.await {
        Ok(data) => ActionResult::decode(&data[..]).map_err(invalid_protocol_buffer_exception),
        Err(e) => Err(e),
    };
    match result {
        Ok(action_result) => Ok(Some(action_result)),
        Err(e) if errors::downcast_ref::<CacheNotFoundException>(&e).is_some() => Ok(None),
        Err(e) => Err(e),
    }
}

/// `InvalidProtocolBufferException` (an `IOException`).
fn invalid_protocol_buffer_exception(e: prost::DecodeError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// `verifyBlobContents(expected, actual)`.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:424 Utils.verifyBlobContents
pub fn verify_blob_contents(expected: &Digest, actual: &Digest) -> io::Result<()> {
    if expected != actual {
        return Err(OutputDigestMismatchException::new(expected.clone(), actual.clone()).into());
    }
    Ok(())
}

/// `buildAction(command, inputRoot, platform, timeout, cacheable, salt)`.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:430 Utils.buildAction
pub fn build_action(
    command: Digest,
    input_root: Digest,
    platform: Option<Platform>,
    timeout: Duration,
    cacheable: bool,
    salt: Option<Vec<u8>>,
) -> Action {
    let mut action = Action {
        command_digest: Some(command),
        input_root_digest: Some(input_root),
        ..Default::default()
    };
    if !timeout.is_zero() {
        action.timeout = Some(blaze_protos::google::protobuf::Duration {
            seconds: timeout.as_secs() as i64,
            nanos: 0,
        });
    }
    if !cacheable {
        action.do_not_cache = true;
    }
    if let Some(platform) = platform {
        action.platform = Some(platform);
    }
    if let Some(salt) = salt {
        action.salt = salt;
    }
    action
}

/// An in-memory output file.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:456 Utils.InMemoryOutput
#[derive(Debug, Clone)]
pub struct InMemoryOutput {
    output: ActionInputRef,
    contents: bytes::Bytes,
}

impl InMemoryOutput {
    /// `new InMemoryOutput(output, contents)`.
    // Java: com/google/devtools/build/lib/remote/util/Utils.java:460 InMemoryOutput.<init>
    pub fn new(output: ActionInputRef, contents: bytes::Bytes) -> InMemoryOutput {
        InMemoryOutput { output, contents }
    }

    /// `getOutput()`.
    // Java: com/google/devtools/build/lib/remote/util/Utils.java:465 InMemoryOutput.getOutput
    pub fn get_output(&self) -> &ActionInputRef {
        &self.output
    }

    /// `getContents()`.
    // Java: com/google/devtools/build/lib/remote/util/Utils.java:469 InMemoryOutput.getContents
    pub fn get_contents(&self) -> &bytes::Bytes {
        &self.contents
    }
}

/// Whether `Status.fromThrowable(t)` is `UNAUTHENTICATED` or `PERMISSION_DENIED`.
fn is_unauthenticated_or_permission_denied(t: &io::Error) -> bool {
    matches!(
        execution_status_exception::status_from_throwable(t).map(|status| status.code()),
        Some(tonic::Code::Unauthenticated | tonic::Code::PermissionDenied)
    )
}

/// Call an asynchronous code block. If the block throws unauthenticated error, refresh the
/// credentials using [`CallCredentialsProvider`] and call it again.
///
/// If any other exception thrown by the code block, it will be caught and wrapped in the
/// returned future.
///
/// PORT: a Rust `call` cannot throw before it returns its future, so Java's two paths (a throw
/// from `call.call()` and a failed future) are the one `Err` of the awaited future.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:481 Utils.refreshIfUnauthenticatedAsync
pub async fn refresh_if_unauthenticated_async<V, F, Fut>(
    mut call: F,
    call_credentials_provider: &dyn CallCredentialsProvider,
) -> io::Result<V>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = io::Result<V>>,
{
    match call().await {
        Ok(v) => Ok(v),
        Err(e) => {
            refresh_if_unauthenticated_async_on_exception(e, call, call_credentials_provider).await
        }
    }
}

// Java: com/google/devtools/build/lib/remote/util/Utils.java:497 Utils.refreshIfUnauthenticatedAsyncOnException
async fn refresh_if_unauthenticated_async_on_exception<V, F, Fut>(
    t: io::Error,
    mut call: F,
    call_credentials_provider: &dyn CallCredentialsProvider,
) -> io::Result<V>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = io::Result<V>>,
{
    if is_unauthenticated_or_permission_denied(&t) {
        // Java: `try { refresh(); return call.call(); } catch (Throwable tt) { t.addSuppressed(tt); }`
        // The future of the second call is returned as it is; only a failed refresh falls
        // through to the original error.
        // PORT: `t.addSuppressed(tt)` has no counterpart (an `io::Error` carries no suppressed
        // exceptions).
        if call_credentials_provider.refresh().is_ok() {
            return call().await;
        }
    }

    Err(t)
}

/// Same as [`refresh_if_unauthenticated_async`] but calling a synchronous code block.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:515 Utils.refreshIfUnauthenticated
pub fn refresh_if_unauthenticated<V>(
    mut call: impl FnMut() -> io::Result<V>,
    call_credentials_provider: &dyn CallCredentialsProvider,
) -> io::Result<V> {
    match call() {
        Ok(v) => Ok(v),
        Err(e) => {
            if is_unauthenticated_or_permission_denied(&e) {
                // Java: `try { refresh(); return call.call(); } catch (Exception ex) { e.addSuppressed(ex); }`
                // A failure of the second call is suppressed; the first error is the one thrown.
                // PORT: `e.addSuppressed(ex)` has no counterpart.
                if call_credentials_provider.refresh().is_ok()
                    && let Ok(v) = call()
                {
                    return Ok(v);
                }
            }

            Err(e)
        }
    }
}

/// `Spawns.mayBeCachedRemotely(Map<String, String> executionInfo)`.
///
/// PORT: `blaze_exec::actions::spawns` has this method for a `Spawn` only.
// Java: com/google/devtools/build/lib/actions/Spawns.java:45 Spawns.mayBeCachedRemotely
fn may_be_cached_remotely(execution_info: &IndexMap<String, String>) -> bool {
    spawns::may_be_cached_info(execution_info)
        && !execution_info.contains_key(execution_requirements::NO_REMOTE)
        && !execution_info.contains_key(execution_requirements::NO_REMOTE_CACHE)
}

/// `shouldUploadLocalResultsToRemoteCache(remoteOptions, executionInfo)`.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:543 Utils.shouldUploadLocalResultsToRemoteCache
pub fn should_upload_local_results_to_remote_cache(
    remote_options: &RemoteOptions,
    execution_info: &IndexMap<String, String>,
) -> bool {
    remote_options.remote_upload_local_results
        && may_be_cached_remotely(execution_info)
        && !execution_info.contains_key(execution_requirements::NO_REMOTE_CACHE_UPLOAD)
}

/// Waits for all transfers to finish.
///
/// If interrupted, all remaining transfers are canceled.
///
/// PORT: Java's transfers are futures that are already running, and this method waits for them
/// one after the other. Rust futures run only when polled, so the transfers are driven together
/// here, on the calling thread, and their results are taken in the order of `transfers`. A
/// transfer that ends with an interruption (`errors::is_interrupted`) is Java's
/// `InterruptedException`: the transfers after it are dropped, which cancels them.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:555 Utils.waitForBulkTransfer
pub fn wait_for_bulk_transfer<I, F, T>(transfers: I) -> io::Result<()>
where
    I: IntoIterator<Item = F>,
    F: Future<Output = io::Result<T>>,
{
    let mut bulk_transfer_exception: Option<BulkTransferException> = None;
    let mut interrupted_exception: Option<io::Error> = None;
    let mut pending: FuturesOrdered<F> = transfers.into_iter().collect();
    crate::runtime::block_on(async {
        // Wait for all transfers to finish.
        while let Some(transfer) = pending.next().await {
            match transfer {
                Ok(_) => {}
                Err(e) if errors::is_interrupted(&e) => {
                    interrupted_exception = Some(e);
                    break;
                }
                Err(e) => {
                    bulk_transfer_exception
                        .get_or_insert_with(BulkTransferException::new)
                        .add(e);
                }
            }
        }
    });
    // `transfer.cancel(true)` for the remaining transfers.
    drop(pending);
    if let Some(interrupted_exception) = interrupted_exception {
        // PORT: `interruptedException.addSuppressed(bulkTransferException)` has no counterpart.
        return Err(interrupted_exception);
    }
    if let Some(bulk_transfer_exception) = bulk_transfer_exception {
        return Err(bulk_transfer_exception.into());
    }
    Ok(())
}

/// `mergeBulkTransfer(transfers)`: completes when all transfers have, with a
/// `BulkTransferException` of their errors if any failed, or with the interruption of the first
/// transfer (in the order of `transfers`) that was interrupted.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:592 Utils.mergeBulkTransfer
pub async fn merge_bulk_transfer<I, F>(transfers: I) -> io::Result<()>
where
    I: IntoIterator<Item = F>,
    F: Future<Output = io::Result<()>>,
{
    // `Futures.whenAllComplete(transfers)`
    let results = futures::future::join_all(transfers).await;
    let mut bulk_transfer_exception: Option<BulkTransferException> = None;

    for transfer in results {
        let error = match transfer {
            Ok(()) => continue,
            Err(e) if errors::is_interrupted(&e) => return Err(e),
            Err(e) => e,
        };

        bulk_transfer_exception
            .get_or_insert_with(BulkTransferException::new)
            .add(error);
    }

    if let Some(bulk_transfer_exception) = bulk_transfer_exception {
        return Err(bulk_transfer_exception.into());
    }

    Ok(())
}

/// `createExecExceptionForCredentialHelperException(e)`.
///
/// PORT: `CredentialHelperException` (lib/authandtls/credentialhelper) is not ported; the
/// exception is taken as anything that prints its message.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:637 Utils.createExecExceptionForCredentialHelperException
pub fn create_exec_exception_for_credential_helper_exception(
    e: &dyn fmt::Display,
) -> ExecException {
    EnvironmentalExecException::with_cause(
        e,
        FailureDetail {
            message: "Exec failed due to CredentialHelperException".to_string(),
            category: Some(failure_detail::Category::RemoteOptions(
                failure_details::RemoteOptions {
                    code: failure_details::remote_options::Code::CredentialsReadFailure as i32,
                },
            )),
        },
    )
    .into()
}

/// `createExecExceptionFromRemoteExecutionCapabilitiesException(e)`.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:650 Utils.createExecExceptionFromRemoteExecutionCapabilitiesException
pub fn create_exec_exception_from_remote_execution_capabilities_exception(
    e: &RemoteExecutionCapabilitiesException,
) -> ExecException {
    EnvironmentalExecException::with_cause(
        e.get_cause(),
        FailureDetail {
            message: "Failed to query remote execution capabilities".to_string(),
            category: Some(failure_detail::Category::RemoteExecution(
                failure_details::RemoteExecution {
                    code: failure_details::remote_execution::Code::CapabilitiesQueryFailure as i32,
                },
            )),
        },
    )
    .into()
}
