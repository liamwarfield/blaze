//! Port of `remote/options/RemoteOptions.java` (the `OptionsBase` subclass and what it uses).

use blaze_options::Foreign;
use blaze_vfs::PathFragment;
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use blaze_options::{
    AssignmentConverter, BooleanConverter, ByteSizeConverter, ConversionContext, Converter,
    DurationConverter, OptionScalar, OptionsParsingException, PercentageConverter,
    RegexPatternConverter, RegexPatternOption, Value, Void, bool_or_enum_converter, enum_converter,
    option_enum, option_field_type, options,
};

use super::common_remote_options::RemoteDurationConverter;
use super::options_utils::{EmptyToNullPathFragmentConverter, PathFragmentConverter};
use super::remote_build_event_upload_mode::RemoteBuildEventUploadMode;
use super::remote_outputs_mode::RemoteOutputsMode;

// Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:43 RemoteOptions
options! {
    /// Options for remote execution and distributed caching for Bazel only.
    pub struct RemoteOptions {
        // PORT: inherits CommonRemoteOptions (its @Option fields, copied).
        #[option(
            name = "remote_download_regex",
            old_name = "experimental_remote_download_regex",
            default_value = "null",
            allow_multiple = true,
            documentation_category = REMOTE,
            effect_tags = [AFFECTS_OUTPUTS],
            converter = RegexPatternConverter,
            help = concat!(
                "Force remote build outputs whose path matches this pattern to be downloaded,",
                " irrespective of --remote_download_outputs. Multiple patterns may be specified by",
                " repeating this flag.",
            ),
        )]
        pub remote_download_regex: Vec<RegexPatternOption>,

        #[option(
            name = "experimental_remote_cache_ttl",
            default_value = "3h",
            documentation_category = REMOTE,
            effect_tags = [EXECUTION],
            converter = RemoteDurationConverter,
            help = concat!(
                "The guaranteed minimal TTL of blobs in the remote cache after their digests are recently",
                " referenced e.g. by an ActionResult or FindMissingBlobs. Bazel does several",
                " optimizations based on the blobs' TTL e.g. doesn't repeatedly call",
                " GetActionResult in an incremental build. The value should be set slightly less",
                " than the real TTL since there is a gap between when the server returns the",
                " digests and when Bazel receives them.",
            ),
        )]
        pub remote_cache_ttl: Duration,

        // RemoteOptions' own fields.
        #[option(
            name = "remote_proxy",
            old_name = "remote_cache_proxy",
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Connect to the remote cache through a proxy. Currently this flag can only be used to ",
                "configure a Unix domain socket (unix:/path/to/socket).",
            ),
        )]
        pub remote_proxy: Option<String>,

        #[option(
            name = "remote_max_connections",
            default_value = "100",
            documentation_category = REMOTE,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "Limit the max number of concurrent connections to remote cache/executor. By default the value is 100. Setting this to 0 means no limitation.\n",
                "For HTTP remote cache, one TCP connection could handle one request at one time, so Bazel could make up to --remote_max_connections concurrent requests.\n",
                "For gRPC remote cache/executor, one gRPC channel could usually handle 100+ concurrent requests (controlled by --remote_max_concurrency_per_connection), so Bazel could make around `--remote_max_connections * 100` concurrent requests.",
            ),
        )]
        pub remote_max_connections: i32,

        #[option(
            name = "remote_max_concurrency_per_connection",
            default_value = "100",
            documentation_category = REMOTE,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = "Limit the max number of concurrent requests per gRPC connection. By default the value is 100.",
        )]
        pub remote_max_concurrency_per_connection: i32,

        #[option(
            name = "remote_executor",
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "HOST or HOST:PORT of a remote execution endpoint. The supported schemas are grpc, ",
                "grpcs (grpc with TLS enabled) and unix (local UNIX sockets). If no schema is ",
                "provided Bazel will default to grpcs. Specify grpc:// or unix: schema to ",
                "disable TLS.",
            ),
        )]
        pub remote_executor: Option<String>,

        #[option(
            name = "experimental_remote_execution_keepalive",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op. Kept here for backwards compatibility.",
        )]
        pub remote_execution_keepalive: bool,

        #[option(
            name = "experimental_remote_capture_corrupted_outputs",
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            converter = PathFragmentConverter,
            help = "A path to a directory where the corrupted outputs will be captured to.",
        )]
        pub remote_capture_corrupted_outputs: Option<Foreign<PathFragment>>,

        #[option(
            name = "remote_cache_async",
            old_name = "experimental_remote_cache_async",
            default_value = "true",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If true, uploading of action results to a disk or remote cache will happen in the",
                " background instead of blocking the completion of an action. Some actions are",
                " incompatible with background uploads, and may still block even when this flag is",
                " set.",
            ),
        )]
        pub remote_cache_async: bool,

        #[option(
            name = "remote_cache",
            old_name = "remote_http_cache",
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "A URI of a caching endpoint. The supported schemas are http, https, grpc, grpcs ",
                "(grpc with TLS enabled) and unix (local UNIX sockets). If no schema is provided ",
                "Bazel will default to grpcs. Specify grpc://, http:// or unix: schema to disable ",
                "TLS. See https://bazel.build/remote/caching",
            ),
        )]
        pub remote_cache: Option<String>,

        #[option(
            name = "experimental_remote_downloader",
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "A Remote Asset API endpoint URI, to be used as a remote download proxy. The supported",
                " schemas are grpc, grpcs (grpc with TLS enabled) and unix (local UNIX sockets). If",
                " no schema is provided Bazel will default to grpcs. See: ",
                "https://github.com/bazelbuild/remote-apis/blob/master/build/bazel/remote/asset/v1/remote_asset.proto",
            ),
        )]
        pub remote_downloader: Option<String>,

        #[option(
            name = "experimental_remote_downloader_local_fallback",
            default_value = "false",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = "Whether to fall back to the local downloader if remote downloader fails.",
        )]
        pub remote_downloader_local_fallback: bool,

        #[option(
            name = "experimental_remote_downloader_propagate_credentials",
            default_value = "false",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Whether to propagate credentials from netrc and credential helper to the remote",
                " downloader server. The server implementation needs to support the new",
                " `http_header_url:<url-index>:<header-key>` qualifier where the `<url-index>` is a",
                " 0-based position of the URL inside the FetchBlobRequest's `uris` field. The",
                " URL-specific headers should take precedence over the global headers.",
            ),
        )]
        pub remote_downloader_propagate_credentials: bool,

        #[option(
            name = "remote_header",
            converter = AssignmentConverter,
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specify a header that will be included in requests: --remote_header=Name=Value. ",
                "Multiple headers can be passed by specifying the flag multiple times. Multiple ",
                "values for the same name will be converted to a comma-separated list.",
            ),
            allow_multiple = true,
        )]
        pub remote_headers: Vec<(String, String)>,

        #[option(
            name = "remote_cache_header",
            converter = AssignmentConverter,
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specify a header that will be included in cache requests: ",
                "--remote_cache_header=Name=Value. ",
                "Multiple headers can be passed by specifying the flag multiple times. Multiple ",
                "values for the same name will be converted to a comma-separated list.",
            ),
            allow_multiple = true,
        )]
        pub remote_cache_headers: Vec<(String, String)>,

        #[option(
            name = "remote_exec_header",
            converter = AssignmentConverter,
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specify a header that will be included in execution requests: ",
                "--remote_exec_header=Name=Value. ",
                "Multiple headers can be passed by specifying the flag multiple times. Multiple ",
                "values for the same name will be converted to a comma-separated list.",
            ),
            allow_multiple = true,
        )]
        pub remote_exec_headers: Vec<(String, String)>,

        #[option(
            name = "remote_downloader_header",
            converter = AssignmentConverter,
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specify a header that will be included in remote downloader requests: ",
                "--remote_downloader_header=Name=Value. ",
                "Multiple headers can be passed by specifying the flag multiple times. Multiple ",
                "values for the same name will be converted to a comma-separated list.",
            ),
            allow_multiple = true,
        )]
        pub remote_downloader_headers: Vec<(String, String)>,

        #[option(
            name = "remote_timeout",
            default_value = "60s",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            converter = RemoteDurationConverter,
            help = concat!(
                "The maximum amount of time to wait for remote execution and cache calls. For the REST",
                " cache, this is both the connect and the read timeout. Following units can be",
                " used: Days (d), hours (h), minutes (m), seconds (s), and milliseconds (ms). If",
                " the unit is omitted, the value is interpreted as seconds.",
            ),
        )]
        pub remote_timeout: Duration,

        #[option(
            name = "remote_grpc_service_config",
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            converter = EmptyToNullPathFragmentConverter,
            help = concat!(
                "Path to a gRPC service config JSON file for remote gRPC channels. This replaces the",
                " service config Bazel generates from --remote_timeout. Only a subset of the gRPC",
                " service config JSON schema is supported: top-level methodConfig entries with",
                " name objects containing service and optional method, plus timeout. Other service",
                " config fields are rejected and may be supported in the future.",
            ),
        )]
        pub remote_grpc_service_config: Option<Foreign<PathFragment>>,

        #[option(
            name = "remote_grpc_download_idle_timeout",
            default_value = "60s",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            converter = RemoteDurationConverter,
            help = concat!(
                "The maximum amount of time a remote gRPC download may go without receiving response",
                " data before Bazel cancels and retries the download. This applies to ByteStream",
                " Read. The value 0 disables the timeout. If the unit is omitted, the value is",
                " interpreted as seconds.",
            ),
        )]
        pub remote_grpc_download_idle_timeout: Duration,

        #[option(
            name = "remote_bytestream_uri_prefix",
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "The hostname and instance name to be used in bytestream:// URIs that are written into ",
                "build event streams. This option can be set when builds are performed using a ",
                "proxy, which causes the values of --remote_executor and --remote_instance_name ",
                "to no longer correspond to the canonical name of the remote execution service. ",
                "When not set, it will default to \"${hostname}/${instance_name}\".",
            ),
        )]
        pub remote_bytestream_uri_prefix: Option<String>,

        #[option(
            name = "remote_accept_cached",
            default_value = "true",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = "Whether to accept remotely cached action results.",
        )]
        pub remote_accept_cached: bool,

        #[option(
            name = "experimental_remote_require_cached",
            default_value = "false",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If set to true, enforce that all actions that can run remotely are cached, or else ",
                "fail the build. This is useful to troubleshoot non-determinism issues as it ",
                "allows checking whether actions that should be cached are actually cached ",
                "without spuriously injecting new results into the cache.",
            ),
        )]
        pub remote_require_cached: bool,

        #[option(
            name = "remote_local_fallback",
            default_value = "false",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = "Whether to fall back to standalone local execution strategy if remote execution fails.",
        )]
        pub remote_local_fallback: bool,

        #[option(
            name = "incompatible_remote_local_fallback_for_remote_cache",
            default_value = "false",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = "Whether --remote_local_fallback applies to --remote_cache.",
        )]
        pub remote_local_fallback_for_remote_cache: bool,

        #[option(
            name = "remote_local_fallback_strategy",
            default_value = "local",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = "Deprecated. See https://github.com/bazelbuild/bazel/issues/7480 for details.",
            deprecated = true,
        )]
        pub remote_local_fallback_strategy: String,

        #[option(
            name = "remote_upload_local_results",
            default_value = "true",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Whether to upload locally executed action results to the remote cache if the remote ",
                "cache supports it and the user is authorized to do so.",
            ),
        )]
        pub remote_upload_local_results: bool,

        #[option(
            name = "remote_build_event_upload",
            old_name = "experimental_remote_build_event_upload",
            default_value = "minimal",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            converter = RemoteBuildEventUploadModeConverter,
            help = concat!(
                "If set to 'all', all local outputs referenced by BEP are uploaded to remote cache.\n",
                "If set to 'minimal', local outputs referenced by BEP are not uploaded to the",
                " remote cache, except for files that are important to the consumers of BEP (e.g.",
                " test logs and timing profile). bytestream:// scheme is always used for the uri of",
                " files even if they are missing from remote cache.\n",
                "Default to 'minimal'.",
            ),
        )]
        pub remote_build_event_upload_mode: RemoteBuildEventUploadMode,

        #[option(
            name = "remote_instance_name",
            default_value = "",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = "Value to pass as instance_name in the remote execution API.",
        )]
        pub remote_instance_name: String,

        #[option(
            name = "remote_retries",
            old_name = "experimental_remote_retry_max_attempts",
            default_value = "5",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "The maximum number of attempts to retry a transient error. ",
                "If set to 0, retries are disabled.",
            ),
        )]
        pub remote_max_retry_attempts: i32,

        #[option(
            name = "remote_retry_max_delay",
            default_value = "5s",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            converter = RemoteDurationConverter,
            help = concat!(
                "The maximum backoff delay between remote retry attempts. Following units can be used:",
                " Days (d), hours (h), minutes (m), seconds (s), and milliseconds (ms). If",
                " the unit is omitted, the value is interpreted as seconds.",
            ),
        )]
        pub remote_retry_max_delay: Duration,

        #[option(
            name = "disk_cache",
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            converter = PathFragmentConverter,
            help = concat!(
                "A path to a directory where Bazel can read and write actions and action outputs. ",
                "If the directory does not exist, it will be created.",
            ),
        )]
        pub disk_cache: Option<Foreign<PathFragment>>,

        #[option(
            name = "experimental_disk_cache_gc_idle_delay",
            default_value = "5m",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            converter = DurationConverter,
            help = concat!(
                "How long the server must remain idle before a garbage collection of the disk cache",
                " occurs. To specify the garbage collection policy, set",
                " --experimental_disk_cache_gc_max_size and/or",
                " --experimental_disk_cache_gc_max_age.",
            ),
        )]
        pub disk_cache_gc_idle_delay: Duration,

        #[option(
            name = "experimental_disk_cache_gc_max_size",
            default_value = "0",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            converter = ByteSizeConverter,
            help = concat!(
                "If set to a positive value, the disk cache will be periodically garbage collected to",
                " stay under this size. If set in conjunction with",
                " --experimental_disk_cache_gc_max_age, both criteria are applied. Garbage",
                " collection occurrs in the background once the server has become idle, as",
                " determined by the --experimental_disk_cache_gc_idle_delay flag.",
            ),
        )]
        pub disk_cache_gc_max_size: i64,

        #[option(
            name = "experimental_disk_cache_gc_max_age",
            default_value = "0",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            converter = DurationConverter,
            help = concat!(
                "If set to a positive value, the disk cache will be periodically garbage collected to",
                " remove entries older than this age. If set in conjunction with",
                " --experimental_disk_cache_gc_max_size, both criteria are applied. Garbage",
                " collection occurrs in the background once the server has become idle, as",
                " determined by the --experimental_disk_cache_gc_idle_delay flag.",
            ),
        )]
        pub disk_cache_gc_max_age: Duration,

        #[option(
            name = "guard_against_concurrent_changes",
            old_name = "experimental_guard_against_concurrent_changes",
            default_value = "lite",
            converter = ConcurrentChangesCheckLevelConverter,
            documentation_category = REMOTE,
            effect_tags = [EXECUTION],
            help = concat!(
                "Set this to 'full' to enable checking the ctime of all input files of an action before",
                " uploading it to a remote cache. There may be cases where the Linux kernel delays",
                " writing of files, which could cause false positives. The default is 'lite', which",
                " only checks source files in the main repository. Setting this to 'off' disables",
                " all checks. This is not recommended, as the cache may be polluted when a source",
                " file is changed while an action that takes it as an input is executing.",
            ),
        )]
        pub guard_against_concurrent_changes: ConcurrentChangesCheckLevel,

        #[option(
            name = "remote_grpc_log",
            old_name = "experimental_remote_grpc_log",
            default_value = "null",
            category = "remote",
            documentation_category = REMOTE,
            converter = EmptyToNullPathFragmentConverter,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If specified, a path to a file to log gRPC call related details. This log consists of a",
                " sequence of serialized ",
                "com.google.devtools.build.lib.remote.logging.RemoteExecutionLog.LogEntry ",
                "protobufs with each message prefixed by a varint denoting the size of the",
                " following serialized protobuf message, as performed by the method ",
                "LogEntry.writeDelimitedTo(OutputStream).",
            ),
        )]
        pub remote_grpc_log: Option<Foreign<PathFragment>>,

        #[option(
            name = "remote_cache_compression",
            old_name = "experimental_remote_cache_compression",
            default_value = "false",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If enabled, compress/decompress cache blobs with zstd when their size is at least",
                " --experimental_remote_cache_compression_threshold.",
            ),
        )]
        pub cache_compression: bool,

        #[option(
            name = "experimental_remote_cache_compression_threshold",
            default_value = "100",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "The minimum blob size required to compress/decompress with zstd. Ineffectual unless",
                " --remote_cache_compression is set.",
            ),
        )]
        pub cache_compression_threshold: i32,

        #[option(
            name = "remote_download_outputs",
            old_name = "experimental_remote_download_outputs",
            default_value = "toplevel",
            category = "remote",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            converter = RemoteOutputsStrategyConverter,
            help = concat!(
                "If set to 'minimal' doesn't download any remote build outputs to the local machine, ",
                "except the ones required by local actions. If set to 'toplevel' behaves like ",
                "'minimal' except that it also downloads outputs of top level targets to the local ",
                "machine. Both options can significantly reduce build times if network bandwidth ",
                "is a bottleneck.",
            ),
        )]
        pub remote_outputs_mode: RemoteOutputsMode,

        #[option(
            name = "remote_download_minimal",
            old_name = "experimental_remote_download_minimal",
            default_value = "null",
            expansion = ["--remote_download_outputs=minimal"],
            category = "remote",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Does not download any remote build outputs to the local machine. This flag is an alias",
                " for --remote_download_outputs=minimal.",
            ),
        )]
        pub remote_outputs_minimal: Void,

        #[option(
            name = "remote_download_toplevel",
            old_name = "experimental_remote_download_toplevel",
            default_value = "null",
            expansion = ["--remote_download_outputs=toplevel"],
            category = "remote",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Only downloads remote outputs of top level targets to the local machine. This flag is an",
                " alias for --remote_download_outputs=toplevel.",
            ),
        )]
        pub remote_outputs_toplevel: Void,

        #[option(
            name = "remote_download_all",
            default_value = "null",
            expansion = ["--remote_download_outputs=all"],
            category = "remote",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Downloads all remote outputs to the local machine. This flag is an alias for",
                " --remote_download_outputs=all.",
            ),
        )]
        pub remote_outputs_all: Void,

        #[option(
            name = "remote_result_cache_priority",
            default_value = "0",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "The relative priority of remote actions to be stored in remote cache. ",
                "The semantics of the particular priority values are server-dependent.",
            ),
        )]
        pub remote_result_cache_priority: i32,

        #[option(
            name = "remote_execution_priority",
            default_value = "0",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "The relative priority of actions to be executed remotely. ",
                "The semantics of the particular priority values are server-dependent.",
            ),
        )]
        pub remote_execution_priority: i32,

        #[option(
            name = "remote_default_exec_properties",
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [AFFECTS_OUTPUTS],
            converter = AssignmentConverter,
            allow_multiple = true,
            help = concat!(
                "Set the default exec properties to be used as the remote execution platform ",
                "if an execution platform does not already set exec_properties.",
            ),
        )]
        pub remote_default_exec_properties: Vec<(String, String)>,

        #[option(
            name = "remote_verify_downloads",
            default_value = "true",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If set to true, Bazel will compute the hash sum of all remote downloads and ",
                " discard the remotely cached values if they don't match the expected value.",
            ),
        )]
        pub remote_verify_downloads: bool,

        #[option(
            name = "remote_download_symlink_template",
            default_value = "",
            category = "remote",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Instead of downloading remote build outputs to the local machine, create symbolic ",
                "links. The target of the symbolic links can be specified in the form of a ",
                "template string. This template string may contain {hash} and {size_bytes} that ",
                "expand to the hash of the object and the size in bytes, respectively. ",
                "These symbolic links may, for example, point to a FUSE file system ",
                "that loads objects from the CAS on demand.",
            ),
        )]
        pub remote_download_symlink_template: String,

        #[option(
            name = "bep_maximum_open_remote_upload_files",
            default_value = "-1",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Maximum number of open files allowed during BEP artifact upload.",
        )]
        pub maximum_open_files: i32,

        #[option(
            name = "remote_print_execution_messages",
            default_value = "failure",
            converter = ExecutionMessagePrintModeConverter,
            category = "remote",
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "Choose when to print remote execution messages. Valid values are `failure`, ",
                "to print only on failures, `success` to print only on successes and ",
                "`all` to print always.",
            ),
        )]
        pub remote_print_execution_messages: ExecutionMessagePrintMode,

        #[option(
            name = "experimental_remote_mark_tool_inputs",
            default_value = "false",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If set to true, Bazel will mark inputs as tool inputs for the remote executor. This ",
                "can be used to implement remote persistent workers.",
            ),
        )]
        pub mark_tool_inputs: bool,

        #[option(
            name = "experimental_remote_discard_merkle_trees",
            default_value = "true",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If set to true, discard in-memory copies of the input root's Merkle tree and associated ",
                "input mappings during calls to GetActionResult() and Execute(). This reduces ",
                "memory usage significantly, but does require Bazel to recompute them upon remote ",
                "cache misses and retries.",
            ),
        )]
        pub remote_discard_merkle_trees: bool,

        #[option(
            name = "experimental_circuit_breaker_strategy",
            documentation_category = REMOTE,
            default_value = "null",
            effect_tags = [EXECUTION],
            converter = CircuitBreakerStrategyConverter,
            help = concat!(
                "Specifies the strategy for the circuit breaker to use. Available strategies are",
                " \"failure\". On invalid value for the option the behavior same as the option is",
                " not set.",
            ),
        )]
        pub circuit_breaker_strategy: Option<CircuitBreakerStrategy>,

        #[option(
            name = "experimental_remote_failure_rate_threshold",
            default_value = "10",
            documentation_category = REMOTE,
            effect_tags = [EXECUTION],
            converter = PercentageConverter,
            help = concat!(
                "Sets the allowed number of failure rate in percentage for a specific time window after",
                " which it stops calling to the remote cache/executor. By default the value is 10.",
                " Setting this to 0 means no limitation.",
            ),
        )]
        pub remote_failure_rate_threshold: i32,

        #[option(
            name = "experimental_remote_failure_window_interval",
            default_value = "60s",
            documentation_category = REMOTE,
            effect_tags = [EXECUTION],
            converter = RemoteDurationConverter,
            help = concat!(
                "The interval in which the failure rate of the remote requests are computed. On zero or",
                " negative value the failure duration is computed the whole duration of the",
                " execution.Following units can be used: Days (d), hours (h), minutes (m), seconds",
                " (s), and milliseconds (ms). If the unit is omitted, the value is interpreted as",
                " seconds.",
            ),
        )]
        pub remote_failure_window_interval: Duration,

        #[option(
            name = "experimental_remote_cache_lease_extension",
            default_value = "false",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If set to true, Bazel will extend the lease for outputs of remote actions during the",
                " build by sending `FindMissingBlobs` calls periodically to remote cache. The",
                " frequency is based on the value of `--experimental_remote_cache_ttl`.",
            ),
        )]
        pub remote_cache_lease_extension: bool,

        #[option(
            name = "experimental_remote_scrubbing_config",
            converter = ScrubberConverter,
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Enables remote cache key scrubbing with the supplied configuration file, which must be a",
                " protocol buffer in text format (see",
                " src/main/protobuf/remote_scrubbing.proto).\n",
                "\n",
                "This feature is intended to facilitate sharing a remote/disk cache between actions",
                " executing on different platforms but targeting the same platform. It should be",
                " used with extreme care, as improper settings may cause accidental sharing of",
                " cache entries and result in incorrect builds.\n",
                "\n",
                "Scrubbing does not affect how an action is executed, only how its remote/disk",
                " cache key is computed for the purpose of retrieving or storing an action result.",
                " Scrubbed actions are incompatible with remote execution, and will always be",
                " executed locally instead.\n",
                "\n",
                "Modifying the scrubbing configuration does not invalidate outputs present in the",
                " local filesystem or internal caches; a clean build is required to reexecute",
                " affected actions.\n",
                "\n",
                "In order to successfully use this feature, you likely want to set a custom",
                " --host_platform together with --experimental_platform_in_output_dir (to normalize",
                " output prefixes).",
            ),
        )]
        pub scrubber: Option<Scrubber>,

        #[option(
            name = "experimental_remote_cache_chunking",
            default_value = "false",
            documentation_category = REMOTE,
            metadata_tags = [EXPERIMENTAL],
            effect_tags = [UNKNOWN],
            help = concat!(
                "If enabled, large blobs are split into content-defined chunks and ",
                "uploaded/downloaded in chunks, enabling deduplication across blobs. The server ",
                "must advertise SplitBlob/SpliceBlob RPCs and the parameters of the chunking ",
                "function selected by --experimental_remote_cache_chunking_function in its ",
                "capabilities.",
            ),
        )]
        pub experimental_remote_cache_chunking: bool,

        #[option(
            name = "experimental_remote_cache_chunking_function",
            default_value = "auto",
            documentation_category = REMOTE,
            metadata_tags = [EXPERIMENTAL],
            effect_tags = [UNKNOWN],
            converter = ChunkingFunctionValueConverter,
            help = concat!(
                "The content-defined chunking function used to split large blobs when ",
                "--experimental_remote_cache_chunking is enabled. If set to 'auto' (the ",
                "default), the function is negotiated with the server: FastCDC 2020 is used if ",
                "the server advertises it, otherwise RepMaxCDC. Set to 'fast_cdc_2020' or ",
                "'rep_max_cdc' to require a specific function, in which case the server must ",
                "advertise the parameters of that function in its capabilities. All clients ",
                "sharing a cache should use the same function to maximize chunk reuse.",
            ),
        )]
        pub experimental_remote_cache_chunking_function: ChunkingFunctionValue,

        #[option(
            name = "experimental_throttle_remote_action_building",
            default_value = "true",
            converter = BooleanConverter,
            documentation_category = UNDOCUMENTED,
            metadata_tags = [EXPERIMENTAL],
            effect_tags = [EXECUTION],
            help = concat!(
                "Whether to throttle the building of remote action to avoid OOM. Defaults to true.\n",
                "\n",
                "This is a temporary flag to allow users switch off the behaviour. Once Bazel is",
                " smart enough about the RAM/CPU usages, this flag will be removed.",
            ),
        )]
        pub throttle_remote_action_building: bool,

        #[option(
            name = "experimental_remote_output_service",
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "HOST or HOST:PORT of a remote output service endpoint. The supported schemas are grpc, ",
                "grpcs (grpc with TLS enabled) and unix (local UNIX sockets). If no schema is ",
                "provided Bazel will default to grpcs. Specify grpc:// or unix: schema to ",
                "disable TLS.",
            ),
        )]
        pub remote_output_service: Option<String>,

        #[option(
            name = "experimental_remote_output_service_output_path_prefix",
            default_value = "",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            help = concat!(
                "The path under which the contents of output directories managed by the",
                " --experimental_remote_output_service are placed. The actual output directory used",
                " by a build will be a descendant of this path and determined by the output",
                " service.",
            ),
        )]
        pub remote_output_service_output_path_prefix: String,
    }
}

// Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:359 RemoteOptions.RemoteBuildEventUploadModeConverter
enum_converter!(
    pub RemoteBuildEventUploadModeConverter,
    RemoteBuildEventUploadMode,
    "remote build event upload"
);

// Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:450 RemoteOptions.ConcurrentChangesCheckLevel
option_enum! {
    /// An enum for different levels of checks for concurrent changes.
    pub enum ConcurrentChangesCheckLevel { OFF, LITE, FULL }
}

// Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:456 RemoteOptions.ConcurrentChangesCheckLevel.Converter
bool_or_enum_converter!(
    pub ConcurrentChangesCheckLevelConverter,
    ConcurrentChangesCheckLevel,
    "concurrent changes check level",
    ConcurrentChangesCheckLevel::FULL,
    ConcurrentChangesCheckLevel::OFF
);

// Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:536 RemoteOptions.RemoteOutputsStrategyConverter
enum_converter!(
    pub RemoteOutputsStrategyConverter,
    RemoteOutputsMode,
    "download remote outputs"
);

// Java: com/google/devtools/build/lib/remote/Scrubber.java:46 Scrubber
/// The remote cache key scrubber of `--experimental_remote_scrubbing_config`.
///
/// PORT: Java's option holds the `Scrubber` itself (`blaze_remote::scrubber::Scrubber`, a crate
/// above this one); here it holds the parsed `Config` it is built from, whose patterns the
/// converter has compiled once (so a bad pattern fails the option as in Java).
/// `blaze_remote::scrubber::Scrubber::new(&config)` builds the scrubber. Java's `Scrubber`
/// compares patterns by identity, so two parses are never equal: equality is identity here too.
#[derive(Clone, Debug)]
pub struct Scrubber {
    pub config: std::sync::Arc<blaze_protos::remote_scrubbing::Config>,
}

impl PartialEq for Scrubber {
    fn eq(&self, other: &Scrubber) -> bool {
        std::sync::Arc::ptr_eq(&self.config, &other.config)
    }
}

impl OptionScalar for Scrubber {
    // Java: Object.toString (`Scrubber` does not override it): the class name and identity hash.
    fn java_to_string(&self) -> String {
        blaze_util::unported!("Scrubber.toString (Object.toString: identity hash)")
    }
}

option_field_type!(
    Scrubber,
    "com.google.devtools.build.lib.remote.Scrubber",
    raw = unreachable!("Scrubber fields are always nullable")
);

/// `Scrubber.ConfigParseException`'s message, or the parsed `Config`: the reading part of
/// `Scrubber.parse(configPath)`, and the patterns `new Scrubber(config)` compiles, compiled in
/// its order (`SpawnScrubber.<init>`: mnemonic, label, kind, omitted inputs, argument
/// replacement sources; an empty matcher field is `.*`).
// Java: com/google/devtools/build/lib/remote/Scrubber.java:73 Scrubber.parse
pub fn parse_scrubbing_config(
    config_path: &str,
) -> Result<blaze_protos::remote_scrubbing::Config, String> {
    use blaze_protos::remote_scrubbing::{Config, config};
    // Files.newBufferedReader(Paths.get(configPath)): UTF-8, malformed input is an error.
    // PORT: the IOException messages are the JDK's for the common cases (NoSuchFileException
    // and AccessDeniedException carry the path, MalformedInputException "Input length = <n>");
    // any other I/O error carries the OS error text.
    let bytes = match std::fs::read(blaze_util::string_encoding::internal_to_platform(
        config_path,
    )) {
        Ok(bytes) => bytes,
        Err(e) => {
            return Err(match e.kind() {
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                    config_path.to_string()
                }
                _ => e.to_string(),
            });
        }
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(e) => match e.error_len() {
            Some(n) => return Err(format!("Input length = {n}")),
            None => blaze_util::unported!(
                "Scrubber.parse (MalformedInputException of a truncated UTF-8 sequence at the end of the file)"
            ),
        },
    };
    let mut builder = Config::default();
    if let Err(e) = blaze_protos::text_format::get_parser().merge(text, &mut builder) {
        return Err(e.to_string());
    }
    // new Scrubber(builder.build()): PatternSyntaxException is
    // "in regex '<pattern>': <message>".
    let compile = |regex: &str| -> Result<(), String> {
        blaze_util::java_regex::Pattern::compile(regex, 0)
            .map(|_| ())
            .map_err(|message| format!("in regex '{regex}': {message}"))
    };
    let empty_to_all = |s: &str| -> String {
        if s.is_empty() {
            ".*".to_string()
        } else {
            s.to_string()
        }
    };
    for rule_proto in &builder.rules {
        let default_matcher = config::Matcher::default();
        let matcher_proto = rule_proto.matcher.as_ref().unwrap_or(&default_matcher);
        compile(&empty_to_all(&matcher_proto.mnemonic))?;
        compile(&empty_to_all(&matcher_proto.label))?;
        compile(&empty_to_all(&matcher_proto.kind))?;
        let default_transform = config::Transform::default();
        let transform_proto = rule_proto.transform.as_ref().unwrap_or(&default_transform);
        for omitted_input in &transform_proto.omitted_inputs {
            compile(omitted_input)?;
        }
        for r in &transform_proto.arg_replacements {
            compile(&r.source)?;
        }
    }
    Ok(builder)
}

// Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:832 RemoteOptions.ScrubberConverter
#[derive(Clone, Copy, Debug, Default)]
struct ScrubberConverter;

impl Converter for ScrubberConverter {
    // Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:835 RemoteOptions.ScrubberConverter.convert
    fn convert(
        &self,
        path: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        let Some(path) = path else {
            return Ok(None);
        };
        match parse_scrubbing_config(path) {
            Ok(config) => Ok(Some(Value::new(Scrubber {
                config: std::sync::Arc::new(config),
            }))),
            Err(message) => Err(OptionsParsingException::new(format!(
                "Failed to parse ScrubbingConfig: {message}"
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:844 RemoteOptions.ScrubberConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "Converts to a Scrubber".into()
    }
}

// Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:884 RemoteOptions.ExecutionMessagePrintMode
option_enum! {
    /// An enum for specifying different modes for printing remote execution messages.
    pub enum ExecutionMessagePrintMode {
        FAILURE, // Print execution messages only on failure
        SUCCESS, // Print execution messages only on success
        ALL, // Print execution messages always
    }
}

// Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:890 RemoteOptions.ExecutionMessagePrintMode.Converter
enum_converter!(
    pub ExecutionMessagePrintModeConverter,
    ExecutionMessagePrintMode,
    "execution message print mode"
);

impl ExecutionMessagePrintMode {
    // Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:896 RemoteOptions.ExecutionMessagePrintMode.shouldPrintMessages
    pub fn should_print_messages(&self, success: bool) -> bool {
        (!success && *self == ExecutionMessagePrintMode::FAILURE)
            || (success && *self == ExecutionMessagePrintMode::SUCCESS)
            || *self == ExecutionMessagePrintMode::ALL
    }
}

// Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:904 RemoteOptions.CircuitBreakerStrategy
option_enum! {
    /// An enum for specifying different strategy for circuit breaker.
    pub enum CircuitBreakerStrategy { FAILURE }
}

// Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:908 RemoteOptions.CircuitBreakerStrategy.Converter
enum_converter!(
    pub CircuitBreakerStrategyConverter,
    CircuitBreakerStrategy,
    "CircuitBreaker strategy"
);

// Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:916 RemoteOptions.ChunkingFunctionValue
option_enum! {
    /// An enum for specifying the content-defined chunking function.
    pub enum ChunkingFunctionValue {
        // Negotiate with the server: FastCDC 2020 if advertised, otherwise RepMaxCDC.
        AUTO,
        FAST_CDC_2020,
        REP_MAX_CDC,
    }
}

// Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:923 RemoteOptions.ChunkingFunctionValue.Converter
enum_converter!(
    pub ChunkingFunctionValueConverter,
    ChunkingFunctionValue,
    "chunking function"
);

impl RemoteOptions {
    // Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:853 RemoteOptions.maxOutboundMessageSize
    /// The maximum size of an outbound message sent via a gRPC channel.
    ///
    /// PORT: a non-`@Option` field in Java (settable by tests only); `options!` structs hold only
    /// `@Option` fields, so it is the constant initial value here.
    pub const MAX_OUTBOUND_MESSAGE_SIZE: i32 = 1024 * 1024;

    // Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:791 RemoteOptions.getEffectiveChunkingFunction
    /// Returns the chunking function to use for chunked cache transfers, or `None` if chunking is
    /// disabled.
    pub fn get_effective_chunking_function(&self) -> Option<ChunkingFunctionValue> {
        if self.experimental_remote_cache_chunking {
            Some(self.experimental_remote_cache_chunking_function)
        } else {
            None
        }
    }

    // Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:856 RemoteOptions.isRemoteCacheEnabled
    /// Returns `true` if remote cache or disk cache is enabled.
    pub fn is_remote_cache_enabled(&self) -> bool {
        !self.remote_cache.as_deref().unwrap_or("").is_empty()
            || !self.disk_cache.as_ref().is_none_or(|d| d.is_empty())
            || self.is_remote_execution_enabled()
    }

    // Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:863 RemoteOptions.isRemoteExecutionEnabled
    /// Returns `true` if remote execution is enabled.
    pub fn is_remote_execution_enabled(&self) -> bool {
        !self.remote_executor.as_deref().unwrap_or("").is_empty()
    }

    // Java: com/google/devtools/build/lib/remote/options/RemoteOptions.java:871 RemoteOptions.getRemoteDefaultExecProperties
    /// Returns the default exec properties specified by the user or an empty map if nothing was
    /// specified. Use this method instead of directly accessing the field.
    pub fn get_remote_default_exec_properties(&self) -> BTreeMap<String, String> {
        // Don't use `ImmutableSortedMap.copyOf` directly because it crashes on duplicate keys.
        let mut map = HashMap::new();
        for (k, v) in &self.remote_default_exec_properties {
            map.insert(k.clone(), v.clone());
        }
        map.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<RemoteOptions>(&[
            "remote_download_regex",
            "experimental_remote_cache_ttl",
            "remote_proxy",
            "remote_max_connections",
            "remote_max_concurrency_per_connection",
            "remote_executor",
            "experimental_remote_execution_keepalive",
            "experimental_remote_capture_corrupted_outputs",
            "remote_cache_async",
            "remote_cache",
            "experimental_remote_downloader",
            "experimental_remote_downloader_local_fallback",
            "experimental_remote_downloader_propagate_credentials",
            "remote_header",
            "remote_cache_header",
            "remote_exec_header",
            "remote_downloader_header",
            "remote_timeout",
            "remote_grpc_service_config",
            "remote_grpc_download_idle_timeout",
            "remote_bytestream_uri_prefix",
            "remote_accept_cached",
            "experimental_remote_require_cached",
            "remote_local_fallback",
            "incompatible_remote_local_fallback_for_remote_cache",
            "remote_local_fallback_strategy",
            "remote_upload_local_results",
            "remote_build_event_upload",
            "remote_instance_name",
            "remote_retries",
            "remote_retry_max_delay",
            "disk_cache",
            "experimental_disk_cache_gc_idle_delay",
            "experimental_disk_cache_gc_max_size",
            "experimental_disk_cache_gc_max_age",
            "guard_against_concurrent_changes",
            "remote_grpc_log",
            "remote_cache_compression",
            "experimental_remote_cache_compression_threshold",
            "remote_download_outputs",
            "remote_download_minimal",
            "remote_download_toplevel",
            "remote_download_all",
            "remote_result_cache_priority",
            "remote_execution_priority",
            "remote_default_exec_properties",
            "remote_verify_downloads",
            "remote_download_symlink_template",
            "bep_maximum_open_remote_upload_files",
            "remote_print_execution_messages",
            "experimental_remote_mark_tool_inputs",
            "experimental_remote_discard_merkle_trees",
            "experimental_circuit_breaker_strategy",
            "experimental_remote_failure_rate_threshold",
            "experimental_remote_failure_window_interval",
            "experimental_remote_cache_lease_extension",
            "experimental_remote_scrubbing_config",
            "experimental_remote_cache_chunking",
            "experimental_remote_cache_chunking_function",
            "experimental_throttle_remote_action_building",
            "experimental_remote_output_service",
            "experimental_remote_output_service_output_path_prefix",
        ]);
    }

    #[test]
    fn parses_flags() {
        let o = parse::<RemoteOptions>(&[
            "--remote_cache=grpc://cache",
            "--remote_header=a=b",
            "--remote_timeout=30",
            "--remote_download_minimal",
            "--guard_against_concurrent_changes",
            "--remote_default_exec_properties=b=2",
            "--remote_default_exec_properties=a=1",
            "--remote_default_exec_properties=b=3",
            "--experimental_disk_cache_gc_max_size=2G",
            "--remote_download_regex=x",
        ])
        .unwrap();
        assert_eq!(o.remote_cache.as_deref(), Some("grpc://cache"));
        assert_eq!(o.remote_headers, vec![("a".into(), "b".into())]);
        assert_eq!(o.remote_timeout, Duration::from_secs(30));
        assert_eq!(o.remote_outputs_mode, RemoteOutputsMode::MINIMAL);
        assert_eq!(
            o.guard_against_concurrent_changes,
            ConcurrentChangesCheckLevel::FULL
        );
        assert_eq!(o.disk_cache_gc_max_size, 2 * 1024 * 1024 * 1024);
        assert_eq!(o.remote_download_regex.len(), 1);
        assert!(o.is_remote_cache_enabled());
        assert!(!o.is_remote_execution_enabled());
        let props: Vec<(String, String)> =
            o.get_remote_default_exec_properties().into_iter().collect();
        assert_eq!(
            props,
            vec![("a".into(), "1".into()), ("b".into(), "3".into())]
        );
    }

    #[test]
    fn defaults() {
        let o = RemoteOptions::default();
        assert_eq!(o.remote_outputs_mode, RemoteOutputsMode::TOPLEVEL);
        assert!(!o.is_remote_cache_enabled());
        assert_eq!(o.get_effective_chunking_function(), None);
        assert!(ExecutionMessagePrintMode::ALL.should_print_messages(true));
        assert!(!ExecutionMessagePrintMode::FAILURE.should_print_messages(true));
        let o = parse::<RemoteOptions>(&["--disk_cache="]).unwrap();
        assert!(!o.is_remote_cache_enabled());
    }

    // Java tests: src/test/java/com/google/devtools/build/lib/remote/options/RemoteOptionsTest.java

    #[test]
    fn test_default_value_of_exec_properties() {
        let options = RemoteOptions::default();
        assert!(options.get_remote_default_exec_properties().is_empty());
    }

    #[test]
    // Java mutates the defaults instance in place.
    #[allow(clippy::field_reassign_with_default)]
    fn test_remote_default_exec_properties() {
        let mut options = RemoteOptions::default();
        options.remote_default_exec_properties = vec![
            ("ISA".into(), "x86-64".into()),
            ("OSFamily".into(), "linux".into()),
        ];
        let properties = options.get_remote_default_exec_properties();
        assert_eq!(
            properties,
            BTreeMap::from([
                ("OSFamily".to_string(), "linux".to_string()),
                ("ISA".to_string(), "x86-64".to_string()),
            ])
        );
    }

    #[test]
    // Java mutates the defaults instance in place.
    #[allow(clippy::field_reassign_with_default)]
    fn test_remote_default_exec_properties_with_duplicates() {
        let mut options = RemoteOptions::default();
        options.remote_default_exec_properties = vec![
            ("foo".into(), "bar".into()),
            ("qux".into(), "quux".into()),
            ("foo".into(), "baz".into()),
        ];
        let properties = options.get_remote_default_exec_properties();
        assert_eq!(
            properties,
            BTreeMap::from([
                ("foo".to_string(), "baz".to_string()),
                ("qux".to_string(), "quux".to_string()),
            ])
        );
    }

    #[test]
    fn test_remote_timeout_options_converter_without_unit() {
        let seconds = 60;
        let convert = RemoteDurationConverter
            .convert(Some(&seconds.to_string()), None)
            .unwrap()
            .unwrap()
            .get::<Duration>();
        assert_eq!(Duration::from_secs(seconds), convert);
    }

    #[test]
    fn test_remote_timeout_options_converter_with_unit() {
        let milliseconds = 60;
        let convert = RemoteDurationConverter
            .convert(Some(&format!("{}ms", milliseconds)), None)
            .unwrap()
            .unwrap()
            .get::<Duration>();
        assert_eq!(Duration::from_millis(milliseconds), convert);
    }

    #[test]
    fn test_remote_maximum_open_files_default() {
        let options = RemoteOptions::default();
        let default_max = options.maximum_open_files;
        assert_eq!(default_max, -1);
    }

    #[test]
    fn remote_grpc_download_idle_timeout_defaults_to60_seconds() {
        // Keep gRPC downloads protected by default with the same timeout used by the HTTP remote
        // cache. Users can still explicitly set the option to zero to disable it.
        let options = RemoteOptions::default();
        assert_eq!(
            options.remote_grpc_download_idle_timeout,
            Duration::from_secs(60)
        );
    }

    #[test]
    fn test_remote_grpc_log_with_empty_string() {
        let options =
            parse::<RemoteOptions>(&["--remote_grpc_log=test.log", "--remote_grpc_log="]).unwrap();
        assert!(options.remote_grpc_log.is_none());
    }
}
