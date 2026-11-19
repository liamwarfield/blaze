//! Port of `lib/exec/SpawnLogContext.java`: the execution log action context and the helpers its
//! two formats share.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use blaze_cmdline::Label;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_protos::failure_details::{FailureDetail, failure_detail, spawn as spawn_code};
use blaze_protos::google::protobuf::{Duration as ProtoDuration, Timestamp};
use blaze_protos::tools::protos::{self, Digest, EnvironmentVariable, Platform, platform};
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path};

use crate::actions::{
    ActionInputRef, EnvironmentalExecException, ExecException, InputMapping, InputMetadataProvider,
    Spawn, SpawnResult, VirtualActionInput,
};

/// An action context providing the ability to log executed spawns.
///
/// Implementations must be safe to call concurrently.
// Java: com/google/devtools/build/lib/exec/SpawnLogContext.java:58 SpawnLogContext
pub trait SpawnLogContext: Send + Sync {
    /// Logs an executed spawn.
    ///
    /// May be called concurrently.
    ///
    /// - `spawn`: the spawn to log
    /// - `input_metadata_provider`: provides metadata for the spawn inputs
    /// - `input_map`: the mapping from input paths to action inputs (built lazily)
    /// - `file_system`: the filesystem containing the spawn inputs and outputs, which might be an
    ///   action filesystem when building without the bytes
    /// - `timeout`: the timeout the spawn was run under
    /// - `result`: the spawn result
    ///
    /// PORT: Java's `logSpawn` also throws `IOException`, which its only caller
    /// (`AbstractSpawnStrategy.exec`) turns into an `EnvironmentalExecException`; that conversion
    /// ([`spawn_log_io_exception`]) happens here instead.
    // Java: com/google/devtools/build/lib/exec/SpawnLogContext.java:72 SpawnLogContext.logSpawn
    fn log_spawn(
        &self,
        spawn: &dyn Spawn,
        input_metadata_provider: &dyn InputMetadataProvider,
        input_map: &dyn Fn() -> Arc<InputMapping>,
        file_system: &FileSystemRef,
        timeout: Duration,
        result: &SpawnResult,
    ) -> Result<(), ExecException>;

    /// Logs an internal symlink action, which is not backed by a spawn.
    ///
    /// May be called concurrently.
    // Java: com/google/devtools/build/lib/exec/SpawnLogContext.java:100 SpawnLogContext.logSymlinkAction
    fn log_symlink_action(&self, action: &dyn SpawnLogAction) -> io::Result<()>;

    /// Finishes writing the log and performs any required post-processing.
    // Java: com/google/devtools/build/lib/exec/SpawnLogContext.java:104 SpawnLogContext.close
    fn close(&self) -> io::Result<()>;

    /// Whether the log should be published to the build event protocol.
    // Java: com/google/devtools/build/lib/exec/SpawnLogContext.java:107 SpawnLogContext.shouldPublish
    fn should_publish(&self) -> bool;
}

/// The view of an `AbstractAction` (in practice a `SymlinkAction`) that
/// [`SpawnLogContext::log_symlink_action`] reads. Seam with `blaze_actions`.
// Java: com/google/devtools/build/lib/actions/AbstractAction.java AbstractAction
pub trait SpawnLogAction {
    /// `getPrimaryInput()`: `None` for a symlink to an absolute path.
    fn get_primary_input(&self) -> Option<ActionInputRef>;
    /// `getPrimaryOutput()`.
    fn get_primary_output(&self) -> ActionInputRef;
    /// `getOwner().getLabel()`.
    fn get_owner_label(&self) -> Option<Label>;
    /// `getMnemonic()`.
    fn get_mnemonic(&self) -> String;
}

/// The `EnvironmentalExecException` `AbstractSpawnStrategy.exec` throws when `logSpawn` fails
/// with an `IOException`.
// Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:198 AbstractSpawnStrategy.exec
pub fn spawn_log_io_exception(e: &io::Error) -> ExecException {
    EnvironmentalExecException::with_cause(
        e,
        FailureDetail {
            message: "IOException while logging spawn".to_string(),
            category: Some(failure_detail::Category::Spawn(
                blaze_protos::failure_details::Spawn {
                    code: spawn_code::Code::SpawnLogIoException as i32,
                    ..Default::default()
                },
            )),
        },
    )
    .into()
}

/// A Java `String` held in its "internal" form (Latin-1 chars holding UTF-8 bytes), as Java
/// protos store strings that Bazel sets without `StringEncoding.internalToUnicode`. Their UTF-8
/// encoding on the wire is therefore "double encoded" for non-ASCII text.
// Java: com/google/devtools/build/lib/util/StringEncoding.java StringEncoding.unicodeToInternal
///
/// Seam (decisions.md 2026-11-18 "Strings"): `to_internal` gives the internal string that `s`
/// stands for, whether or not Blaze's strings are internal yet
/// (`blaze_util::string_encoding::STRINGS_ARE_INTERNAL`).
pub(crate) fn internal(s: &str) -> String {
    blaze_util::string_encoding::to_internal(s).into_owned()
}

/// `StringEncoding.internalToUnicode(s)` of the internal string that `s` stands for.
///
/// Seam (decisions.md 2026-11-18 "Strings"): while
/// `blaze_util::string_encoding::STRINGS_ARE_INTERNAL` is false Blaze's strings are Unicode
/// already and this is the identity; once it is true it is `internal_to_unicode`.
// Java: com/google/devtools/build/lib/util/StringEncoding.java:126 StringEncoding.internalToUnicode
pub(crate) fn internal_to_unicode(s: &str) -> String {
    if blaze_util::string_encoding::strings_are_internal() {
        blaze_util::string_encoding::internal_to_unicode(s).into_owned()
    } else {
        s.to_string()
    }
}

/// `StringEncoding.unicodeToInternal(s)`, giving the string Blaze holds where Java holds the
/// internal string (the same seam as [`internal_to_unicode`]).
// Java: com/google/devtools/build/lib/util/StringEncoding.java:137 StringEncoding.unicodeToInternal
fn unicode_to_internal(s: &str) -> String {
    if blaze_util::string_encoding::strings_are_internal() {
        blaze_util::string_encoding::unicode_to_internal(s).into_owned()
    } else {
        s.to_string()
    }
}

/// `String.compareTo`: lexicographic order of UTF-16 code units. For proto strings (which hold
/// exactly the chars of the Java string) this is Java's order.
pub(crate) fn java_string_compare(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        return a.cmp(b);
    }
    a.encode_utf16().cmp(b.encode_utf16())
}

/// A `String` key ordered as Java's `String.compareTo` (for `TreeMap<String, _>` ports).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct JavaOrderedString(pub String);

impl Ord for JavaOrderedString {
    fn cmp(&self, other: &Self) -> Ordering {
        java_string_compare(&self.0, &other.0)
    }
}

impl PartialOrd for JavaOrderedString {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Computes the environment variables, sorted by name.
// Java: com/google/devtools/build/lib/exec/SpawnLogContext.java:110 SpawnLogContext.getEnvironmentVariables
pub fn get_environment_variables(spawn: &dyn Spawn) -> Vec<EnvironmentVariable> {
    let environment = spawn.get_environment();
    // `ImmutableSortedMap.copyOf(environment)`: the keys are internal strings, whose Java order is
    // the byte order of their UTF-8 form.
    let mut entries: Vec<(&String, &String)> = environment.iter().collect();
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    entries
        .into_iter()
        .map(|(name, value)| EnvironmentVariable {
            name: internal_to_unicode(name),
            value: internal_to_unicode(value),
        })
        .collect()
}

/// Computes the execution platform.
// Java: com/google/devtools/build/lib/exec/SpawnLogContext.java:126 SpawnLogContext.getPlatform
pub fn get_platform(
    spawn: &dyn Spawn,
    remote_options: Option<&RemoteOptions>,
) -> Result<Option<Platform>, ExecException> {
    let Some(exec_platform) = get_platform_proto(spawn, remote_options)? else {
        return Ok(None);
    };
    Ok(Some(Platform {
        properties: exec_platform
            .into_iter()
            .map(|(name, value)| platform::Property { name, value })
            .collect(),
    }))
}

/// `PlatformUtils.getPlatformProto(spawn, remoteOptions)`, returning the sorted properties of the
/// `build.bazel.remote.execution.v2.Platform`.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:102 PlatformUtils.getPlatformProto
fn get_platform_proto(
    spawn: &dyn Spawn,
    remote_options: Option<&RemoteOptions>,
) -> Result<Option<Vec<(String, String)>>, ExecException> {
    let default_exec_properties: Vec<(String, String)> = match remote_options {
        Some(r) => r.get_remote_default_exec_properties().into_iter().collect(),
        None => Vec::new(),
    };

    if !should_produce_platform_proto(spawn, &default_exec_properties) {
        // Execution platform is null or functionally empty
        return Ok(None);
    }

    let mut properties: HashMap<String, String> = HashMap::new();
    let combined_exec_properties = spawn.get_combined_exec_properties();
    if !combined_exec_properties.is_empty() {
        // Apply default exec properties if the execution platform does not already set
        // exec_properties
        match spawn.get_execution_platform() {
            Some(p) if !p.exec_properties.is_empty() => {
                properties.extend(combined_exec_properties);
            }
            _ => {
                properties.extend(default_exec_properties.iter().cloned());
                properties.extend(combined_exec_properties);
            }
        }
    } else if let Some(execution_platform) = spawn.get_execution_platform() {
        if !execution_platform.remote_execution_properties.is_empty() {
            // Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:128 PlatformUtils.getPlatformProto
            let mut platform_builder =
                blaze_protos::build::bazel::remote::execution::v2::Platform::default();
            // Try and get the platform info from the execution properties.
            if let Err(e) = blaze_protos::text_format::merge_platform(
                &internal_to_unicode(&execution_platform.remote_execution_properties),
                &mut platform_builder,
            ) {
                let message = format!(
                    "Failed to parse remote_execution_properties from platform {}",
                    execution_platform.label
                );
                // Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:179 PlatformUtils.createFailureDetail
                return Err(crate::actions::UserExecException::with_cause(
                    &e,
                    FailureDetail {
                        message,
                        category: Some(failure_detail::Category::Spawn(
                            blaze_protos::failure_details::Spawn {
                                code: spawn_code::Code::InvalidRemoteExecutionProperties as i32,
                                ..Default::default()
                            },
                        )),
                    },
                )
                .into());
            }

            for property in &platform_builder.properties {
                properties.insert(
                    unicode_to_internal(&property.name),
                    unicode_to_internal(&property.value),
                );
            }
        } else {
            properties.extend(
                execution_platform
                    .exec_properties
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone())),
            );
        }
    }

    if properties.is_empty() {
        properties.extend(default_exec_properties);
    }

    // `additionalProperties` is empty for this overload.

    // setName(internalToUnicode(entry.getKey())).setValue(internalToUnicode(entry.getValue()))
    let mut sorted: Vec<(String, String)> = properties
        .into_iter()
        .map(|(name, value)| (internal_to_unicode(&name), internal_to_unicode(&value)))
        .collect();
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:43 PlatformUtils.sortPlatformProperties
    sorted.sort_by(|a, b| java_string_compare(&a.0, &b.0));
    Ok(Some(sorted))
}

// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:76 PlatformUtils.shouldProducePlatformProto
fn should_produce_platform_proto(
    spawn: &dyn Spawn,
    default_exec_properties: &[(String, String)],
) -> bool {
    if let Some(execution_platform) = spawn.get_execution_platform() {
        if !execution_platform.exec_properties.is_empty() {
            return true;
        }
        if !execution_platform.remote_execution_properties.is_empty() {
            return true;
        }
    }
    if !spawn.get_combined_exec_properties().is_empty() {
        return true;
    }
    if !default_exec_properties.is_empty() {
        return true;
    }
    false
}

/// Determines whether an action input is a directory, avoiding I/O if possible.
///
/// Do not call for action outputs.
// Java: com/google/devtools/build/lib/exec/SpawnLogContext.java:144 SpawnLogContext.isInputDirectory
pub fn is_input_directory(
    input: &ActionInputRef,
    input_metadata_provider: &dyn InputMetadataProvider,
) -> io::Result<bool> {
    if input.is_directory() {
        return Ok(true);
    }
    if input.is_symlink() {
        return Ok(false);
    }
    // Virtual action inputs are always files.
    if input.as_virtual_action_input().is_some() {
        return Ok(false);
    }
    // Source artifacts are always of file type, but may be directories in the filesystem.
    if input.as_artifact().is_some_and(|a| a.is_source_artifact()) {
        let metadata = input_metadata_provider.get_input_metadata(input)?;
        return Ok(metadata.is_some_and(|m| m.get_type().is_directory()));
    }
    Ok(false)
}

/// A sink that hashes and counts what is written (`DigestOutputStream` over a null stream).
struct HashingWriter {
    hasher: sha2::Sha256,
    size: i64,
}

impl Write for HashingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        sha2::Digest::update(&mut self.hasher, buf);
        self.size += buf.len() as i64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `DigestUtil.compute(DeterministicWriter, HashFunction)`: the digest of a virtual input's
/// contents, as (lowercase hex hash, size).
// Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:125 DigestUtil.compute
pub(crate) fn compute_virtual_digest(
    input: &dyn VirtualActionInput,
    digest_hash_function: DigestHashFunction,
) -> io::Result<(String, i64)> {
    match digest_hash_function {
        DigestHashFunction::Sha256 => {
            let mut writer = HashingWriter {
                hasher: <sha2::Sha256 as sha2::Digest>::new(),
                size: 0,
            };
            input.write_to(&mut writer)?;
            let hash = sha2::Digest::finalize(writer.hasher);
            Ok((hex::encode(hash), writer.size))
        }
    }
}

/// Computes the digest of an `ActionInput` or its path.
///
/// Will try to obtain the digest from cached metadata first, falling back to digesting the
/// contents manually.
///
/// PORT: the `XattrProvider` parameter is the file system's fast digest
/// (`Path.getFastDigest`), which is what `SyscallCache`'s provider delegates to.
// Java: com/google/devtools/build/lib/exec/SpawnLogContext.java:170 SpawnLogContext.computeDigest
pub fn compute_digest(
    input: Option<&ActionInputRef>,
    path: &Path,
    input_metadata_provider: Option<&dyn InputMetadataProvider>,
    digest_hash_function: DigestHashFunction,
    include_hash_function_name: bool,
) -> io::Result<Digest> {
    let mut builder = Digest::default();

    if include_hash_function_name {
        builder.hash_function_name = digest_hash_function.to_string();
    }

    if let Some(input) = input {
        if let Some(virtual_action_input) = input.as_virtual_action_input() {
            let (hash, size) = compute_virtual_digest(virtual_action_input, digest_hash_function)?;
            builder.hash = hash;
            builder.size_bytes = size;
            return Ok(builder);
        }

        if let Some(input_metadata_provider) = input_metadata_provider {
            // Try to obtain a digest from the input metadata.
            // PORT: Java also falls back on an IllegalStateException; Rust panics instead.
            if let Ok(Some(metadata)) = input_metadata_provider.get_input_metadata(input)
                && let Some(digest) = metadata.get_digest()
            {
                builder.hash = hex::encode(digest);
                builder.size_bytes = metadata.get_size();
                return Ok(builder);
            }
            // Pass through to local computation.
        }
    }

    // Obtain a digest from the filesystem.
    let status = path.stat()?;
    // Java: com/google/devtools/build/lib/vfs/DigestUtils.java:139 DigestUtils.getDigestWithManualFallback
    let digest = match path.get_fast_digest()? {
        Some(digest) => digest,
        None => path.get_digest()?,
    };
    builder.hash = hex::encode(digest);
    builder.size_bytes = status.get_size();
    Ok(builder)
}

/// `Instant.toEpochMilli()`.
fn to_epoch_milli(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => {
            let before = e.duration().as_nanos();
            -(before.div_ceil(1_000_000) as i64)
        }
    }
}

/// `Timestamps.fromMillis(long)`.
// Java: com/google/protobuf/util/Timestamps.java fromMillis
pub(crate) fn timestamp_from_millis(milliseconds: i64) -> Timestamp {
    Timestamp {
        seconds: milliseconds.div_euclid(1000),
        nanos: (milliseconds.rem_euclid(1000) * 1_000_000) as i32,
    }
}

// Java: com/google/devtools/build/lib/exec/SpawnLogContext.java:217 SpawnLogContext.getSpawnMetricsProto
pub fn get_spawn_metrics_proto(result: &SpawnResult) -> protos::SpawnMetrics {
    let metrics = result.get_metrics();
    let mut builder = protos::SpawnMetrics::default();
    if metrics.total_time_in_ms != 0 {
        builder.total_time = Some(millis_to_proto(metrics.total_time_in_ms));
    }
    if metrics.parse_time_in_ms != 0 {
        builder.parse_time = Some(millis_to_proto(metrics.parse_time_in_ms));
    }
    if metrics.network_time_in_ms != 0 {
        builder.network_time = Some(millis_to_proto(metrics.network_time_in_ms));
    }
    if metrics.fetch_time_in_ms != 0 {
        builder.fetch_time = Some(millis_to_proto(metrics.fetch_time_in_ms));
    }
    if metrics.queue_time_in_ms != 0 {
        builder.queue_time = Some(millis_to_proto(metrics.queue_time_in_ms));
    }
    if metrics.setup_time_in_ms != 0 {
        builder.setup_time = Some(millis_to_proto(metrics.setup_time_in_ms));
    }
    if metrics.upload_time_in_ms != 0 {
        builder.upload_time = Some(millis_to_proto(metrics.upload_time_in_ms));
    }
    if metrics.execution_wall_time_in_ms != 0 {
        builder.execution_wall_time = Some(millis_to_proto(metrics.execution_wall_time_in_ms));
    }
    if metrics.process_outputs_time_in_ms != 0 {
        builder.process_outputs_time = Some(millis_to_proto(metrics.process_outputs_time_in_ms));
    }
    if metrics.retry_time_in_ms() != 0 {
        builder.retry_time = Some(millis_to_proto(metrics.retry_time_in_ms()));
    }
    builder.input_bytes = metrics.input_bytes;
    builder.input_files = metrics.input_files;
    builder.memory_estimate_bytes = metrics.memory_estimate_bytes;
    builder.measured_memory_peak_bytes = metrics.measured_memory_peak_bytes;
    builder.input_bytes_limit = metrics.input_bytes_limit;
    builder.input_files_limit = metrics.input_files_limit;
    builder.output_bytes_limit = metrics.output_bytes_limit;
    builder.output_files_limit = metrics.output_files_limit;
    builder.memory_bytes_limit = metrics.memory_bytes_limit;
    if metrics.time_limit_in_ms != 0 {
        builder.time_limit = Some(millis_to_proto(metrics.time_limit_in_ms));
    }
    if let Some(start_time) = result.get_start_time() {
        builder.start_time = Some(timestamp_from_millis(to_epoch_milli(start_time)));
    }
    builder
}

/// `Durations.fromMillis(t)`.
// Java: com/google/devtools/build/lib/exec/SpawnLogContext.java:269 SpawnLogContext.millisToProto
pub fn millis_to_proto(t: i32) -> ProtoDuration {
    let millis = t as i64;
    // `Durations.normalizedDuration(millis / 1000, millis % 1000 * 1_000_000)`: both parts have
    // the sign of `millis`, so no normalization applies.
    ProtoDuration {
        seconds: millis / 1000,
        nanos: ((millis % 1000) * 1_000_000) as i32,
    }
}
