//! Transcription of `com/google/devtools/build/lib/skyframe/serialization/analysis/RemoteAnalysisCachingOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/skyframe/serialization/analysis/RemoteAnalysisCachingOptions.java:31 RemoteAnalysisCachingOptions
options! {
    /// Options for caching analysis results remotely.
    pub struct RemoteAnalysisCachingOptions {
        #[option(
            name = "serialized_frontier_profile",
            default_value = "",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [BAZEL_MONITORING],
            help = "Dump a profile of serialized frontier bytes. Specifies the output path.",
        )]
        pub serialized_frontier_profile: String,

        #[option(
            name = "remote_analysis_json_log",
            default_value = "null",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "If set, a JSON file is written to this location that contains a detailed log of ",
                "the behavior of remote analysis caching. It's interpreted as a path relative ",
                "to the current working directory.",
            ),
        )]
        pub json_log: Option<String>,

        #[option(
            name = "experimental_remote_analysis_cache_mode",
            default_value = "off",
            converter = RemoteAnalysisCacheModeConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = "The transport direction for the remote analysis cache.",
        )]
        pub mode: RemoteAnalysisCacheMode,

        #[option(
            name = "experimental_remote_analysis_cache",
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = "The URL for the remote analysis caching backend.",
        )]
        pub remote_analysis_cache: String,

        #[option(
            name = "experimental_remote_analysis_cache_max_batch_size",
            default_value = "4095",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = "Batch size limit for remote analysis caching RPCs.",
        )]
        pub max_batch_size: i32,

        #[option(
            name = "experimental_remote_analysis_cache_concurrency",
            default_value = "4",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = "Target concurrency for remote analysis caching RPCs.",
        )]
        pub concurrency: i32,

        #[option(
            name = "experimental_remote_analysis_cache_deadline",
            default_value = "120s",
            converter = DurationConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = "Deadline to use for remote analysis cache operations.",
        )]
        pub deadline: Duration,

        #[option(
            name = "experimental_remote_analysis_unreachable_cache_retry_interval",
            default_value = "250ms",
            converter = DurationConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = concat!(
                "How long to wait before retrying a cache get request that failed due to an UNREACHABLE",
                " channel. This is a workaround for the client library reporting 'ready' ",
                "prematurely.",
            ),
        )]
        pub unreachable_cache_retry_interval: Duration,

        #[option(
            name = "experimental_analysis_cache_service",
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = "Locator for the AnalysisCacheService instance.",
        )]
        pub analysis_cache_service: String,

        #[option(
            name = "experimental_remote_analysis_write_proxy",
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = concat!(
                "The address of the SkycacheStorageWriteProxyService. If set, this service will be used ",
                "for uploading analysis cache data.",
            ),
        )]
        pub remote_analysis_write_proxy: String,

        #[option(
            name = "experimental_analysis_cache_key_distinguisher_for_testing",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = "An opaque string used as part of the cache key. Should only be used for testing.",
        )]
        pub analysis_cache_key_distinguisher_for_testing: Option<String>,

        #[option(
            name = "experimental_analysis_cache_enable_metadata_queries",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = "A flag to switch on/off inserting and querying the metadata db (b/425247333).",
        )]
        pub analysis_cache_enable_metadata_queries: bool,

        #[option(
            name = "experimental_analysis_cache_server_checksum_override",
            default_value = "null",
            converter = Md5Converter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = concat!(
                "If set, Blaze will use this checksum to look up entries in the remote analysis cache",
                " and not its own. WARNING: this might result in incorrect behavior. Only for",
                " debugging. It's best if the difference between the writer and the reader is only",
                " additional logging. In particular, the data structures that are being serialized ",
                " and the observable behavior of the serialization machinery must not change.",
            ),
        )]
        pub server_checksum_override: Option<HashCode>,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const REMOTE_ANALYSIS_CACHING_OPTIONS_OPTION_NAMES: &[&str] = &[
        "serialized_frontier_profile",
        "remote_analysis_json_log",
        "experimental_remote_analysis_cache_mode",
        "experimental_remote_analysis_cache",
        "experimental_remote_analysis_cache_max_batch_size",
        "experimental_remote_analysis_cache_concurrency",
        "experimental_remote_analysis_cache_deadline",
        "experimental_remote_analysis_unreachable_cache_retry_interval",
        "experimental_analysis_cache_service",
        "experimental_remote_analysis_write_proxy",
        "experimental_analysis_cache_key_distinguisher_for_testing",
        "experimental_analysis_cache_enable_metadata_queries",
        "experimental_analysis_cache_server_checksum_override",
    ];

    #[test]
    fn remote_analysis_caching_options_option_names() {
        super::super::test_util::check_option_names(
            RemoteAnalysisCachingOptions::class(),
            REMOTE_ANALYSIS_CACHING_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn remote_analysis_caching_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<RemoteAnalysisCachingOptions>(
            SKIP_ROUND_TRIP,
        );
    }
}

// ---- hand-written below this line (preserved by the generator) ----

/// Guava `HashCode`, holding the hash bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct HashCode(pub Vec<u8>);

impl HashCode {
    /// Guava `HashCode.fromString`: an even number (at least two) of lower-case hex digits.
    pub fn from_string(string: &str) -> Option<HashCode> {
        if string.len() < 2 || !string.len().is_multiple_of(2) {
            return None;
        }
        let digit = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        let b = string.as_bytes();
        (0..b.len())
            .step_by(2)
            .map(|i| Some((digit(b[i])? << 4) | digit(b[i + 1])?))
            .collect::<Option<Vec<u8>>>()
            .map(HashCode)
    }

    /// `HashCode.bits()`.
    pub fn bits(&self) -> usize {
        self.0.len() * 8
    }
}

// Guava HashCode.toString: lower-case hex.
impl OptionScalar for HashCode {
    fn java_to_string(&self) -> String {
        self.0.iter().map(|b| format!("{:02x}", b)).collect()
    }
}
option_field_type!(HashCode, "com.google.common.hash.HashCode");

// Java: com/google/devtools/build/lib/skyframe/serialization/analysis/RemoteAnalysisCachingOptions.java:34 RemoteAnalysisCachingOptions.Md5Converter
/// A converter for MD5 checksums.
#[derive(Clone, Copy, Debug, Default)]
pub struct Md5Converter;

impl Converter for Md5Converter {
    // Java: com/google/devtools/build/lib/skyframe/serialization/analysis/RemoteAnalysisCachingOptions.java:36 RemoteAnalysisCachingOptions.Md5Converter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let Some(input) = input.filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        match HashCode::from_string(input) {
            // Hashing.md5().bits() == 128.
            Some(result) if result.bits() == 128 => Ok(Some(Value::new(result))),
            _ => Err(OptionsParsingException::new(
                "Blaze checksum must be exactly 32 hex characters",
            )),
        }
    }
    fn get_type_description(&self) -> String {
        String::new()
    }
}

// Java: com/google/devtools/build/lib/skyframe/serialization/analysis/RemoteAnalysisCachingOptions.java:91 RemoteAnalysisCachingOptions.RemoteAnalysisCacheMode
option_enum! {
    /// The transport direction for the remote analysis cache.
    pub enum RemoteAnalysisCacheMode { UPLOAD, DUMP_UPLOAD_MANIFEST_ONLY, DOWNLOAD, OFF }
}

impl RemoteAnalysisCacheMode {
    // Java: com/google/devtools/build/lib/skyframe/serialization/analysis/RemoteAnalysisCachingOptions.java:108 RemoteAnalysisCachingOptions.RemoteAnalysisCacheMode.requiresBackendConnectivity
    /// Returns true if the selected mode needs to connect to a backend.
    pub fn requires_backend_connectivity(self) -> bool {
        match self {
            RemoteAnalysisCacheMode::UPLOAD | RemoteAnalysisCacheMode::DOWNLOAD => true,
            RemoteAnalysisCacheMode::DUMP_UPLOAD_MANIFEST_ONLY | RemoteAnalysisCacheMode::OFF => {
                false
            }
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/serialization/analysis/RemoteAnalysisCachingOptions.java:120 RemoteAnalysisCachingOptions.RemoteAnalysisCacheMode.serializesValues
    /// Returns true if the mode serializes *values*.
    pub fn serializes_values(self) -> bool {
        match self {
            RemoteAnalysisCacheMode::UPLOAD
            | RemoteAnalysisCacheMode::DUMP_UPLOAD_MANIFEST_ONLY => true,
            RemoteAnalysisCacheMode::DOWNLOAD | RemoteAnalysisCacheMode::OFF => false,
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/serialization/analysis/RemoteAnalysisCachingOptions.java:129 RemoteAnalysisCachingOptions.RemoteAnalysisCacheModeConverter
enum_converter!(
    pub RemoteAnalysisCacheModeConverter,
    RemoteAnalysisCacheMode,
    "Frontier serialization mode"
);

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, parse_err};
    use super::*;

    #[test]
    fn checksum_override() {
        let o = parse::<RemoteAnalysisCachingOptions>(&[
            "--experimental_analysis_cache_server_checksum_override=0123456789abcdef0123456789abcdef",
        ]);
        assert_eq!(
            OptionScalar::java_to_string(&o.server_checksum_override.unwrap()),
            "0123456789abcdef0123456789abcdef"
        );
        assert_eq!(
            parse_err::<RemoteAnalysisCachingOptions>(&[
                "--experimental_analysis_cache_server_checksum_override=ABCD"
            ]),
            "While parsing option --experimental_analysis_cache_server_checksum_override=ABCD: Blaze checksum must be exactly 32 hex characters"
        );
        let o = parse::<RemoteAnalysisCachingOptions>(&[
            "--experimental_remote_analysis_cache_mode=download",
        ]);
        assert!(o.mode.requires_backend_connectivity());
    }
}
