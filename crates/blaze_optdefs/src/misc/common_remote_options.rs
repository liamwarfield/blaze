//! Port of `remote/options/CommonRemoteOptions.java` (the `OptionsBase` subclass and what it uses).

use std::time::Duration;

use blaze_options::{
    ConversionContext, Converter, DurationConverter, OptionsParsingException,
    RegexPatternConverter, RegexPatternOption, Value, converter::non_null, options,
};

// Java: com/google/devtools/build/lib/remote/options/CommonRemoteOptions.java:30 CommonRemoteOptions
options! {
    /// Options for remote execution and distributed caching that shared between Bazel and Blaze.
    pub struct CommonRemoteOptions {
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
    }
}

// Java: com/google/devtools/build/lib/remote/options/CommonRemoteOptions.java:61 CommonRemoteOptions.RemoteDurationConverter
/// Returns the specified duration. Assumes seconds if unitless.
#[derive(Clone, Copy, Debug, Default)]
pub struct RemoteDurationConverter;

impl Converter for RemoteDurationConverter {
    // Java: com/google/devtools/build/lib/remote/options/CommonRemoteOptions.java:66 CommonRemoteOptions.RemoteDurationConverter.convert
    fn convert(
        &self,
        input: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        let mut input = non_null(input).to_string();
        // UNITLESS_REGEX = ^[0-9]+$
        if !input.is_empty() && input.bytes().all(|b| b.is_ascii_digit()) {
            input.push('s');
        }
        DurationConverter.convert(Some(&input), /* conversionContext= */ None)
    }

    // Java: com/google/devtools/build/lib/remote/options/CommonRemoteOptions.java:74 CommonRemoteOptions.RemoteDurationConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "An immutable length of time.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<CommonRemoteOptions>(&[
            "remote_download_regex",
            "experimental_remote_cache_ttl",
        ]);
    }

    #[test]
    fn parses_flags() {
        let o = parse::<CommonRemoteOptions>(&[
            "--experimental_remote_cache_ttl=60",
            "--remote_download_regex=.*\\.jar",
            "--remote_download_regex=foo",
        ])
        .unwrap();
        assert_eq!(o.remote_cache_ttl, Duration::from_secs(60));
        assert_eq!(o.remote_download_regex.len(), 2);
        assert!(o.remote_download_regex[0].matches("a/b.jar"));
        assert_eq!(
            CommonRemoteOptions::default().remote_cache_ttl,
            Duration::from_secs(3 * 3600)
        );
    }
}
